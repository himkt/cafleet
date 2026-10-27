use super::input::InputGuard;
use super::*;
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::sync::Arc;
use std::time::Instant;

mod drain;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputDrain {
    Pending,
    Quiescent {
        human_generation: u64,
        output_seq: u64,
    },
}

pub(super) struct DrainedInput {
    instance: Arc<()>,
    turn: u64,
    human_generation: u64,
    output_seq: u64,
    health: ObservationHealth,
}

impl DrainedInput {
    pub(super) fn evidence(&self) -> InputDrain {
        InputDrain::Quiescent {
            human_generation: self.human_generation,
            output_seq: self.output_seq,
        }
    }
}

pub(crate) enum ProgrammaticWrite {
    Complete { bytes: usize },
    Partial { bytes: usize },
    Pending,
}

pub(crate) trait ByteInput {
    fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize>;
}
pub(crate) trait PtyIo: ByteInput {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    fn resize(&mut self, size: TerminalSize) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
pub(crate) trait Display {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    fn try_flush(&mut self) -> io::Result<()>;
}
pub(crate) trait ScreenObserver {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()>;
    fn input_health(&self) -> Option<ObservationHealth> {
        None
    }
    fn failures_are_fatal(&self) -> bool {
        false
    }
    fn parser_failure(&mut self) -> Result<()> {
        Ok(())
    }
}
#[derive(Default)]
pub(crate) struct TransportProgress {
    pub child_eof: bool,
    pub outer_eof: bool,
    pub child_bytes: usize,
    pub human_bytes: usize,
    pub display_bytes: usize,
    pub observation_error: Option<String>,
    pub observation_failure: Option<ObservationFailure>,
}
pub(crate) struct TransportLimits {
    pub input_bytes: usize,
    pub display_bytes: usize,
    pub bytes_per_tick: usize,
}
pub(crate) struct Transport {
    terminal: TerminalState,
    limits: TransportLimits,
    input: VecDeque<u8>,
    display: VecDeque<u8>,
    outer_eof: bool,
    child_eof: bool,
    human_generation: u64,
    input_turn: u64,
    input_instance: Arc<()>,
    input_drain: Option<DrainedInput>,
    observation_complete: bool,
    #[cfg(test)]
    observation_hooks: Option<std::sync::Arc<dyn ObservationHooks>>,
    #[cfg(test)]
    observation_phase: ObservationPhase,
    #[cfg(test)]
    deferred_child_bytes: Option<Vec<u8>>,
}
enum AvailableRead {
    Skipped,
    WouldBlock,
    Interrupted,
    Eof,
    Data(Vec<u8>),
}
impl AvailableRead {
    fn available(self) -> Option<Vec<u8>> {
        match self {
            Self::Data(bytes) => Some(bytes),
            Self::Eof => Some(Vec::new()),
            Self::Skipped | Self::WouldBlock | Self::Interrupted => None,
        }
    }
}
fn read_classified(input: &mut impl ByteInput, capacity: usize) -> io::Result<AvailableRead> {
    if capacity == 0 {
        return Ok(AvailableRead::Skipped);
    }
    let mut bytes = vec![0; capacity.min(16384)];
    match input.try_read(&mut bytes) {
        Ok(count) => {
            assert!(count <= bytes.len(), "reader respects supplied capacity");
            bytes.truncate(count);
            Ok(if count == 0 {
                AvailableRead::Eof
            } else {
                AvailableRead::Data(bytes)
            })
        }
        Err(cause) if cause.kind() == ErrorKind::WouldBlock => Ok(AvailableRead::WouldBlock),
        Err(cause) if cause.kind() == ErrorKind::Interrupted => Ok(AvailableRead::Interrupted),
        Err(cause) => Err(cause),
    }
}
fn read_available(input: &mut impl ByteInput, capacity: usize) -> io::Result<Option<Vec<u8>>> {
    read_classified(input, capacity).map(AvailableRead::available)
}
fn write_pending(
    queue: &mut VecDeque<u8>,
    limit: usize,
    mut write: impl FnMut(&[u8]) -> io::Result<usize>,
) -> Result<usize> {
    if queue.is_empty() {
        return Ok(0);
    }
    let bytes = queue.make_contiguous();
    let count = match write(&bytes[..bytes.len().min(limit)]) {
        Ok(0) => return Err(error("terminal write made no progress")),
        Ok(count) => count,
        Err(cause) if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => 0,
        Err(cause) => return Err(io_error(cause)),
    };
    ensure(
        count <= bytes.len().min(limit),
        "terminal writer exceeded supplied bytes",
    )?;
    queue.drain(..count);
    Ok(count)
}
impl Transport {
    pub(crate) fn new(size: TerminalSize, limits: TransportLimits) -> Result<Self> {
        ensure(
            limits.input_bytes > 0 && limits.display_bytes > 0 && limits.bytes_per_tick > 0,
            "transport limits must be positive",
        )?;
        Ok(Self {
            terminal: TerminalState::new(size)?,
            limits,
            input: VecDeque::new(),
            display: VecDeque::new(),
            outer_eof: false,
            child_eof: false,
            human_generation: 0,
            input_turn: 0,
            input_instance: Arc::new(()),
            input_drain: None,
            observation_complete: true,
            #[cfg(test)]
            observation_hooks: None,
            #[cfg(test)]
            observation_phase: ObservationPhase::ChildOutput,
            #[cfg(test)]
            deferred_child_bytes: None,
        })
    }
    pub(crate) fn tick(
        &mut self,
        human: &mut impl ByteInput,
        pty: &mut impl PtyIo,
        display: &mut impl Display,
        observer: &mut impl ScreenObserver,
    ) -> Result<TransportProgress> {
        self.invalidate_input_drain();
        let mut progress = TransportProgress::default();
        if !self.outer_eof
            && let Some(bytes) = read_available(
                human,
                (self.limits.input_bytes - self.input.len()).min(self.limits.bytes_per_tick),
            )
            .map_err(io_error)?
        {
            self.outer_eof = bytes.is_empty();
            progress.human_bytes = bytes.len();
            self.retain_human(bytes);
        }
        write_pending(&mut self.input, self.limits.bytes_per_tick, |bytes| {
            pty.try_write(bytes)
        })?;
        if let Some(bytes) = self.read_child(pty)? {
            self.child_eof = bytes.is_empty();
            progress.child_bytes = bytes.len();
            self.display.extend(bytes.iter().copied());
            let observation_complete_before = self.observation_complete;
            if !bytes.is_empty() {
                self.observation_complete = false;
            }
            #[cfg(test)]
            if !bytes.is_empty()
                && let Some(hooks) = &self.observation_hooks
                && let Err(cause) =
                    hooks.before(self.observation_phase, ObservationOperation::Parser)
            {
                progress.observation_error = Some(cause.message().into_owned());
                progress.observation_failure = Some(ObservationFailure::Parser);
            }
            if progress.observation_failure.is_none() {
                match self.terminal.feed_observed_classified(&bytes, observer) {
                    Ok(()) => {}
                    Err(TerminalObservationError::Parser(cause)) => {
                        observer.parser_failure()?;
                        return Err(cause);
                    }
                    Err(TerminalObservationError::Observer(cause)) => {
                        if observer.failures_are_fatal() {
                            return Err(cause);
                        }
                        progress.observation_error = Some(cause.message().into_owned());
                        progress.observation_failure = Some(ObservationFailure::Sink);
                    }
                }
            }
            if progress.observation_failure.is_none() {
                self.observation_complete = observation_complete_before;
            }
        }
        progress.display_bytes = self.flush_display(display)?;
        progress.outer_eof = self.outer_eof;
        progress.child_eof = self.child_eof;
        Ok(progress)
    }
    fn read_child(&mut self, pty: &mut impl PtyIo) -> Result<Option<Vec<u8>>> {
        #[cfg(test)]
        if let Some(bytes) = self.deferred_child_bytes.take() {
            return Ok(Some(bytes));
        }
        if self.child_eof {
            return Ok(None);
        }
        read_available(
            pty,
            (self.limits.display_bytes - self.display.len()).min(self.limits.bytes_per_tick),
        )
        .map_err(io_error)
    }
    #[cfg(test)]
    pub(super) fn prefetch_child_bytes(&mut self, pty: &mut impl PtyIo) -> Result<usize> {
        self.invalidate_input_drain();
        if self.deferred_child_bytes.is_some() {
            return Ok(0);
        }
        if let Some(bytes) = self.read_child(pty)? {
            if bytes.is_empty() {
                self.child_eof = true;
            } else {
                let count = bytes.len();
                self.deferred_child_bytes = Some(bytes);
                return Ok(count);
            }
        }
        Ok(0)
    }
    pub(crate) fn flush_display(&mut self, display: &mut impl Display) -> Result<usize> {
        self.invalidate_input_drain();
        let count = write_pending(&mut self.display, self.limits.bytes_per_tick, |bytes| {
            display.try_write(bytes)
        })?;
        match display.try_flush() {
            Ok(()) => {}
            Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
            Err(cause) => return Err(io_error(cause)),
        }
        Ok(count)
    }
    pub(crate) fn resize(&mut self, pty: &mut impl PtyIo, size: TerminalSize) -> Result<()> {
        self.invalidate_input_drain();
        size.validate()?;
        pty.resize(size)?;
        self.terminal.resize(size)
    }
    fn invalidate_input_drain(&mut self) {
        self.input_drain = None;
        self.input_turn = self
            .input_turn
            .checked_add(1)
            .expect("input drain turn fits u64");
    }
    fn retain_human(&mut self, bytes: Vec<u8>) {
        self.human_generation = self
            .human_generation
            .checked_add(u64::try_from(bytes.len()).expect("human read count fits u64"))
            .expect("human input generation fits u64");
        self.input.extend(bytes);
    }
    pub(super) fn human_generation(&self) -> u64 {
        self.human_generation
    }
    pub(super) fn take_input_drain(&mut self) -> Option<DrainedInput> {
        self.input_drain.take()
    }
    pub(super) fn validate_input_drain(
        &self,
        proof: &DrainedInput,
        deadline: Instant,
    ) -> Result<()> {
        self.validate_observation_drain(proof, deadline)?;
        ensure(
            proof.health.eligibility == InputEligibility::ComposerEmpty
                && proof.health.visibility == Visibility::NotVisible,
            "input drain is not eligible for composer input",
        )
    }

    pub(super) fn validate_observation_drain(
        &self,
        proof: &DrainedInput,
        deadline: Instant,
    ) -> Result<()> {
        check_deadline(Instant::now(), deadline)?;
        ensure(
            Arc::ptr_eq(&self.input_instance, &proof.instance)
                && self.input_turn == proof.turn
                && self.human_generation == proof.human_generation
                && self.terminal.snapshot().screen_seq == proof.output_seq,
            "input drain binding changed",
        )?;
        ensure(
            self.observation_complete
                && !self.outer_eof
                && !self.child_eof
                && self.input.is_empty()
                && self.display.len() < self.limits.display_bytes,
            "input transport is no longer quiescent",
        )?;
        proof.health.validate()?;
        ensure(
            !proof.health.degraded
                && proof.health.pending_count == 0
                && !proof.health.pending_evidence
                && proof.health.diagnostic.is_none(),
            "observation drain is not healthy",
        )
    }

    pub(super) fn write_programmatic(
        &mut self,
        pty: &mut impl PtyIo,
        bytes: &[u8],
        guard: &mut InputGuard<'_>,
        deadline: Instant,
    ) -> Result<ProgrammaticWrite> {
        let effective = guard.begin_write(self, bytes, deadline)?;
        self.invalidate_input_drain();
        check_deadline(Instant::now(), effective)?;
        guard.mark_io_possible();
        let result = pty.try_write(bytes);
        let recorded = guard.record_write_result(&result, bytes.len());
        if let Err(recording) = recorded {
            return Err(match result {
                Err(primary) => CafleetError::WithDiagnostics {
                    cause: Box::new(io_error(primary)),
                    diagnostics: vec![recording.message().into_owned()],
                },
                Ok(_) => recording,
            });
        }
        check_deadline(Instant::now(), effective)?;
        match result {
            Ok(count) if count == bytes.len() => Ok(ProgrammaticWrite::Complete { bytes: count }),
            Ok(0) => Err(error("native input write returned zero bytes")),
            Ok(count) => Ok(ProgrammaticWrite::Partial { bytes: count }),
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                Ok(ProgrammaticWrite::Pending)
            }
            Err(cause) => Err(io_error(cause)),
        }
    }

    pub(crate) fn snapshot(&self) -> ScreenSnapshot {
        self.terminal.snapshot()
    }
    #[cfg(test)]
    pub(super) fn set_observation_hooks(
        &mut self,
        hooks: std::sync::Arc<dyn ObservationHooks>,
    ) -> Result<()> {
        ensure(
            self.observation_hooks.is_none(),
            "transport observation hooks install once",
        )?;
        self.observation_hooks = Some(hooks);
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn set_observation_phase(&mut self, phase: ObservationPhase) {
        self.observation_phase = phase;
    }
    #[cfg(test)]
    pub(super) fn order_shutdown_parsing_after_native_exit(
        &self,
        deadline: std::time::Instant,
    ) -> bool {
        self.observation_hooks
            .as_ref()
            .is_some_and(|hooks| hooks.order_shutdown_parsing_after_native_exit(deadline))
    }
    pub(crate) fn pending_input_bytes(&self) -> usize {
        self.input.len()
    }
    pub(crate) fn pending_display_bytes(&self) -> usize {
        self.display.len()
    }
    pub(crate) fn discard_display_tail(&mut self) -> usize {
        self.invalidate_input_drain();
        #[cfg(test)]
        if let Some(bytes) = self.deferred_child_bytes.take() {
            self.observation_complete = false;
            self.display.extend(bytes);
        }
        let count = self.display.len();
        self.display.clear();
        count
    }
}
