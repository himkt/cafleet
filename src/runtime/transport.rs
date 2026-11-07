use crate::runtime::input::InputGuard;
use crate::runtime::*;
use nix::sys::termios;
use nix::sys::termios::SetArg;
use nix::sys::termios::Termios;
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

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
    pub(super) fn human_generation(&self) -> u64 {
        self.human_generation
    }
    pub(super) fn output_seq(&self) -> u64 {
        self.output_seq
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
    pub(crate) child_eof: bool,
    pub(crate) outer_eof: bool,
    pub(crate) child_bytes: usize,
    pub(crate) human_bytes: usize,
    pub(crate) forwarded_bytes: usize,
    pub(crate) display_bytes: usize,
    pub(crate) observation_error: Option<String>,
    pub(crate) observation_failure: Option<ObservationFailure>,
}
pub(crate) struct TransportLimits {
    pub(crate) input_bytes: usize,
    pub(crate) display_bytes: usize,
    pub(crate) bytes_per_tick: usize,
}
pub(crate) struct Transport {
    terminal: TerminalState,
    limits: TransportLimits,
    input: VecDeque<u8>,
    display: VecDeque<u8>,
    outer_eof: bool,
    child_eof: bool,
    human_generation: u64,
    replies: ReplyClassifier,
    input_turn: u64,
    input_instance: Arc<()>,
    input_drain: Option<DrainedInput>,
    drain_blocker: Option<String>,
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
            replies: ReplyClassifier::default(),
            input_turn: 0,
            input_instance: Arc::new(()),
            input_drain: None,
            drain_blocker: None,
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
        let mut progress = TransportProgress {
            human_bytes: self.drain_outer(human)?,
            ..TransportProgress::default()
        };
        progress.forwarded_bytes =
            write_pending(&mut self.input, usize::MAX, |bytes| pty.try_write(bytes))?;
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
    /// One outer-terminal read returns at most the line discipline's buffer, so
    /// a longer terminal reply must reach the child within the same tick: a
    /// child that sees only its first part may parse the rest as keys.
    fn drain_outer(&mut self, human: &mut impl ByteInput) -> Result<usize> {
        self.expire_reply_prefix();
        let mut forwarded = 0;
        while !self.outer_eof {
            let capacity =
                (self.limits.input_bytes - self.input.len()).min(self.limits.bytes_per_tick);
            let Some(bytes) = read_available(human, capacity).map_err(io_error)? else {
                break;
            };
            if bytes.is_empty() {
                self.close_outer();
            } else {
                forwarded += bytes.len();
                self.retain_human(bytes);
            }
        }
        Ok(forwarded)
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
        let classified = self.replies.feed(&bytes, Instant::now());
        self.count_manual(classified);
        self.input.extend(bytes);
    }
    fn expire_reply_prefix(&mut self) {
        let classified = self.replies.expire(Instant::now());
        self.count_manual(classified);
    }
    fn close_outer(&mut self) {
        let classified = self.replies.finish();
        self.count_manual(classified);
        self.outer_eof = true;
    }
    fn count_manual(&mut self, classified: Classified) {
        self.human_generation = self
            .human_generation
            .checked_add(u64::try_from(classified.manual).expect("manual byte count fits u64"))
            .expect("human input generation fits u64");
    }
    pub(super) fn human_generation(&self) -> u64 {
        self.human_generation
    }
    #[cfg(test)]
    pub(super) fn backdate_reply_prefix(&mut self, by: std::time::Duration) {
        self.replies.backdate(by);
    }
    pub(super) fn take_input_drain(&mut self) -> Option<DrainedInput> {
        self.input_drain.take()
    }
    pub(super) fn drain_blocker(&self) -> Option<&str> {
        self.drain_blocker.as_deref()
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
                && !self.replies.pending()
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

const ROUND_LIMIT: usize = 8;

struct DrainBudget {
    human: usize,
    child: usize,
    manual: usize,
    display: usize,
}

fn unsettled_health(health: &ObservationHealth) -> Option<String> {
    if health.degraded {
        Some(match &health.diagnostic {
            Some(code) => format!("observation degraded: {code}"),
            None => "observation degraded".into(),
        })
    } else if health.pending_count != 0 {
        Some("observation transitions pending".into())
    } else if health.pending_evidence {
        Some("observation evidence pending".into())
    } else {
        None
    }
}

struct DeadlineObserver<'a, O> {
    inner: &'a mut O,
    deadline: Instant,
}
impl<O: ScreenObserver> ScreenObserver for DeadlineObserver<'_, O> {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
        check_deadline(Instant::now(), self.deadline)?;
        self.inner.observe(screen)?;
        check_deadline(Instant::now(), self.deadline)
    }

    fn failures_are_fatal(&self) -> bool {
        true
    }

    fn parser_failure(&mut self) -> Result<()> {
        check_deadline(Instant::now(), self.deadline)?;
        self.inner.parser_failure()?;
        check_deadline(Instant::now(), self.deadline)
    }
}

impl Transport {
    pub(crate) fn quiesce_for_input(
        &mut self,
        human: &mut impl ByteInput,
        pty: &mut impl PtyIo,
        display: &mut impl Display,
        observer: &mut impl ScreenObserver,
        deadline: Instant,
    ) -> Result<InputDrain> {
        self.invalidate_input_drain();
        self.drain_blocker = None;
        check_deadline(Instant::now(), deadline)?;
        let mut remaining = DrainBudget {
            human: self.limits.bytes_per_tick,
            child: self.limits.bytes_per_tick,
            manual: self.limits.bytes_per_tick,
            display: self.limits.bytes_per_tick,
        };
        for _ in 0..ROUND_LIMIT {
            check_deadline(Instant::now(), deadline)?;
            self.expire_reply_prefix();
            let human_read = if self.outer_eof {
                AvailableRead::Skipped
            } else {
                read_classified(
                    human,
                    remaining
                        .human
                        .min(self.limits.input_bytes - self.input.len()),
                )
                .map_err(io_error)?
            };
            let human_quiet = matches!(human_read, AvailableRead::WouldBlock);
            let mut interrupted = matches!(human_read, AvailableRead::Interrupted);
            match human_read {
                AvailableRead::Data(bytes) => {
                    remaining.human -= bytes.len();
                    self.retain_human(bytes);
                }
                AvailableRead::Eof => self.close_outer(),
                _ => {}
            }
            check_deadline(Instant::now(), deadline)?;
            if remaining.manual > 0 && !self.input.is_empty() {
                let written = write_pending(&mut self.input, remaining.manual, |bytes| {
                    pty.try_write(bytes)
                })?;
                remaining.manual -= written;
                check_deadline(Instant::now(), deadline)?;
            }
            let child_read = self.read_drain_child(
                pty,
                remaining
                    .child
                    .min(self.limits.display_bytes - self.display.len()),
            )?;
            let child_quiet = matches!(child_read, AvailableRead::WouldBlock);
            interrupted |= matches!(child_read, AvailableRead::Interrupted);
            match child_read {
                AvailableRead::Data(bytes) => {
                    remaining.child -= bytes.len();
                    self.display.extend(bytes.iter().copied());
                    self.observe_drain_child(&bytes, observer, deadline)?;
                }
                AvailableRead::Eof => self.child_eof = true,
                _ => {}
            }
            check_deadline(Instant::now(), deadline)?;
            if remaining.display > 0 && !self.display.is_empty() {
                let written = write_pending(&mut self.display, remaining.display, |bytes| {
                    display.try_write(bytes)
                })?;
                remaining.display -= written;
                check_deadline(Instant::now(), deadline)?;
            }
            match display.try_flush() {
                Ok(()) => {}
                Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                Err(cause) => return Err(io_error(cause)),
            }
            check_deadline(Instant::now(), deadline)?;
            let ended = if interrupted {
                Some("terminal input interrupted")
            } else if self.outer_eof {
                Some("outer terminal closed")
            } else if self.child_eof {
                Some("native output closed")
            } else {
                None
            };
            if let Some(reason) = ended {
                return Ok(self.pending(reason.into()));
            }
            if human_quiet
                && child_quiet
                && self.input.is_empty()
                && self.display.len() < self.limits.display_bytes
            {
                if self.replies.pending() {
                    return Ok(self.pending("terminal reply pending".into()));
                }
                if !self.observation_complete {
                    return Ok(self.pending("observation incomplete".into()));
                }
                let health = observer.input_health();
                check_deadline(Instant::now(), deadline)?;
                let Some(health) = health else {
                    return Ok(self.pending("observation health unavailable".into()));
                };
                if let Some(reason) = unsettled_health(&health) {
                    return Ok(self.pending(reason));
                }
                let output_seq = self.terminal.snapshot().screen_seq;
                check_deadline(Instant::now(), deadline)?;
                self.input_drain = Some(DrainedInput {
                    instance: self.input_instance.clone(),
                    turn: self.input_turn,
                    human_generation: self.human_generation,
                    output_seq,
                    health,
                });
                return Ok(InputDrain::Quiescent {
                    human_generation: self.human_generation,
                    output_seq,
                });
            }
        }
        Ok(self.pending("terminal output still arriving".into()))
    }

    fn pending(&mut self, reason: String) -> InputDrain {
        self.drain_blocker = Some(reason);
        InputDrain::Pending
    }

    fn read_drain_child(&mut self, pty: &mut impl PtyIo, capacity: usize) -> Result<AvailableRead> {
        if capacity == 0 || self.child_eof {
            return Ok(AvailableRead::Skipped);
        }
        #[cfg(test)]
        if let Some(mut bytes) = self.deferred_child_bytes.take() {
            assert!(!bytes.is_empty(), "prefetch retains positive child bytes");
            let rest = bytes.split_off(bytes.len().min(capacity));
            if !rest.is_empty() {
                self.deferred_child_bytes = Some(rest);
            }
            return Ok(AvailableRead::Data(bytes));
        }
        read_classified(pty, capacity).map_err(io_error)
    }

    fn observe_drain_child(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
        deadline: Instant,
    ) -> Result<()> {
        let observation_complete_before = self.observation_complete;
        self.observation_complete = false;
        check_deadline(Instant::now(), deadline)?;
        #[cfg(test)]
        if let Some(hooks) = &self.observation_hooks {
            hooks.before(self.observation_phase, ObservationOperation::Parser)?;
            check_deadline(Instant::now(), deadline)?;
        }
        let mut bounded = DeadlineObserver {
            inner: observer,
            deadline,
        };
        match self.terminal.feed_observed_classified(bytes, &mut bounded) {
            Ok(()) => {
                self.observation_complete = observation_complete_before;
                check_deadline(Instant::now(), deadline)
            }
            Err(TerminalObservationError::Observer(cause)) => Err(cause),
            Err(TerminalObservationError::Parser(cause)) => match bounded.parser_failure() {
                Ok(()) => Err(cause),
                Err(notification) => Err(CafleetError::WithDiagnostics {
                    cause: Box::new(cause),
                    diagnostics: vec![notification.message().into_owned()],
                }),
            },
        }
    }
}

pub(crate) const REPLY_PREFIX_BOUND: Duration = Duration::from_millis(100);
const CSI_PARAMETER_LIMIT: usize = 64;
const STRING_BODY_LIMIT: usize = 4096;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Clone, Copy, PartialEq, Eq)]
enum StringKind {
    Osc,
    Dcs,
    Apc,
}

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi {
        parameters: Vec<u8>,
        intermediate: bool,
    },
    StringBody {
        kind: StringKind,
        length: usize,
    },
    StringEscape,
}

/// Decides how many forwarded outer-terminal bytes count as manual input.
/// It never holds bytes: forwarding happens before and independently of it.
#[derive(Default)]
pub(super) struct ReplyClassifier {
    state: State,
    pending: usize,
    last_byte: Option<Instant>,
    paste_end_matched: Option<usize>,
}

pub(super) struct Classified {
    pub(super) manual: usize,
}

impl ReplyClassifier {
    pub(super) fn feed(&mut self, bytes: &[u8], now: Instant) -> Classified {
        let manual = bytes.iter().map(|&byte| self.classify(byte, now)).sum();
        Classified { manual }
    }

    pub(super) fn expire(&mut self, now: Instant) -> Classified {
        let manual = match self.last_byte {
            Some(last) if now.saturating_duration_since(last) >= REPLY_PREFIX_BOUND => {
                self.resolve_manual()
            }
            _ => 0,
        };
        Classified { manual }
    }

    pub(super) fn finish(&mut self) -> Classified {
        Classified {
            manual: self.resolve_manual(),
        }
    }

    pub(super) fn pending(&self) -> bool {
        self.pending > 0
    }

    fn classify(&mut self, byte: u8, now: Instant) -> usize {
        let manual = match self.paste_end_matched {
            Some(matched) => {
                self.advance_paste(matched, byte);
                1
            }
            None => self.step(byte),
        };
        self.last_byte = self.pending().then_some(now);
        manual
    }

    fn advance_paste(&mut self, matched: usize, byte: u8) {
        let matched = if byte == PASTE_END[matched] {
            matched + 1
        } else if byte == ESC {
            1
        } else {
            0
        };
        self.paste_end_matched = (matched < PASTE_END.len()).then_some(matched);
    }

    fn step(&mut self, byte: u8) -> usize {
        match std::mem::take(&mut self.state) {
            State::Ground => self.ground(byte),
            State::Escape => match byte {
                b'[' => self.extend(State::Csi {
                    parameters: Vec::new(),
                    intermediate: false,
                }),
                b']' => self.open_string(StringKind::Osc),
                b'P' => self.open_string(StringKind::Dcs),
                b'_' => self.open_string(StringKind::Apc),
                _ => self.invalid(byte),
            },
            State::Csi {
                mut parameters,
                intermediate,
            } => match byte {
                0x20..=0x3f if parameters.len() == CSI_PARAMETER_LIMIT => self.invalid(byte),
                0x30..=0x3f if !intermediate => {
                    parameters.push(byte);
                    self.extend(State::Csi {
                        parameters,
                        intermediate,
                    })
                }
                0x20..=0x2f => {
                    parameters.push(byte);
                    self.extend(State::Csi {
                        parameters,
                        intermediate: true,
                    })
                }
                0x40..=0x7e => self.complete_csi(&parameters, byte),
                _ => self.invalid(byte),
            },
            State::StringBody { kind, length } => match byte {
                0x20..=0x7e | 0x80..=0xff if length == STRING_BODY_LIMIT => self.invalid(byte),
                0x20..=0x7e | 0x80..=0xff => self.extend(State::StringBody {
                    kind,
                    length: length + 1,
                }),
                BEL if kind == StringKind::Osc => self.complete_reply(),
                ESC => self.extend(State::StringEscape),
                _ => self.invalid(byte),
            },
            State::StringEscape => match byte {
                b'\\' => self.complete_reply(),
                _ => self.invalid(byte),
            },
        }
    }

    fn ground(&mut self, byte: u8) -> usize {
        if byte == ESC {
            self.state = State::Escape;
            self.pending = 1;
            0
        } else {
            1
        }
    }

    fn open_string(&mut self, kind: StringKind) -> usize {
        self.extend(State::StringBody { kind, length: 0 })
    }

    fn extend(&mut self, state: State) -> usize {
        self.state = state;
        self.pending += 1;
        0
    }

    fn invalid(&mut self, byte: u8) -> usize {
        self.resolve_manual() + self.ground(byte)
    }

    fn resolve_manual(&mut self) -> usize {
        self.state = State::Ground;
        self.last_byte = None;
        std::mem::take(&mut self.pending)
    }

    fn complete_reply(&mut self) -> usize {
        self.pending = 0;
        0
    }

    fn complete_csi(&mut self, parameters: &[u8], final_byte: u8) -> usize {
        let length = std::mem::take(&mut self.pending) + 1;
        if is_csi_reply(parameters, final_byte) {
            return 0;
        }
        if parameters == b"200" && final_byte == b'~' {
            self.paste_end_matched = Some(0);
        }
        length
    }
}

fn is_csi_reply(parameters: &[u8], final_byte: u8) -> bool {
    let (marker, rest) = match parameters.split_first() {
        Some((&marker @ (b'?' | b'>'), rest)) => (Some(marker), rest),
        _ => (None, parameters),
    };
    let split = rest
        .iter()
        .position(|byte| (0x20..=0x2f).contains(byte))
        .unwrap_or(rest.len());
    let (list, intermediates) = rest.split_at(split);
    let arity = |min, max| parameter_count(list).is_some_and(|count| (min..=max).contains(&count));
    match (marker, intermediates, final_byte) {
        (None, b"", b'R') => arity(2, 2),
        (Some(b'?'), b"", b'R') => arity(2, 3),
        (None, b"", b'n') => list == b"0" || list == b"3",
        (Some(b'?'), b"", b'n') => arity(1, 4),
        (Some(b'?'), b"", b'c') => arity(1, 32),
        (Some(b'>'), b"", b'c') => arity(1, 4),
        (Some(b'?') | None, b"$", b'y') => arity(2, 2),
        (None, b"", b't') => arity(1, 3),
        (Some(b'?'), b"", b'u') => arity(1, 1),
        (None, b"", b'I' | b'O') => list.is_empty(),
        _ => false,
    }
}

/// Counts `;`-joined digit parameters, or `None` when any parameter is empty
/// or holds a non-digit byte.
fn parameter_count(list: &[u8]) -> Option<usize> {
    list.split(|&byte| byte == b';')
        .map(|parameter| !parameter.is_empty() && parameter.iter().all(u8::is_ascii_digit))
        .try_fold(0, |count, valid| valid.then_some(count + 1))
}

#[cfg(test)]
impl ReplyClassifier {
    pub(super) fn backdate(&mut self, by: Duration) {
        let last = self
            .last_byte
            .expect("backdating requires a pending reply prefix");
        self.last_byte = Some(
            last.checked_sub(by)
                .expect("backdated reply prefix instant is representable"),
        );
    }
}

#[cfg(test)]
#[derive(Default)]
struct Callbacks {
    title: String,
}
#[cfg(test)]
impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).into_owned();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScreenSnapshot {
    #[cfg(test)]
    pub(crate) title: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) cursor_row: u16,
    pub(crate) cursor_col: u16,
    pub(crate) alternate_screen: bool,
    pub(crate) screen_seq: u64,
    pub(crate) screen: vt100::Screen,
}

pub(crate) struct TerminalState {
    #[cfg(test)]
    parser: vt100::Parser<Callbacks>,
    #[cfg(not(test))]
    parser: vt100::Parser,
    sequence: u64,
}
pub(crate) enum TerminalObservationError {
    Parser(CafleetError),
    Observer(CafleetError),
}

impl TerminalState {
    pub(crate) fn new(size: TerminalSize) -> Result<Self> {
        size.validate()?;
        let rows = size.rows.try_into().expect("validated rows");
        let cols = size.cols.try_into().expect("validated cols");
        #[cfg(test)]
        let parser = vt100::Parser::new_with_callbacks(rows, cols, 0, Callbacks::default());
        #[cfg(not(test))]
        let parser = vt100::Parser::new(rows, cols, 0);
        Ok(Self {
            parser,
            sequence: 0,
        })
    }
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.sequence = self
            .sequence
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error("terminal sequence exhausted"))?;
        self.parser.process(bytes);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn feed_observed(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> Result<()> {
        self.feed_observed_classified(bytes, observer)
            .map_err(|failure| match failure {
                TerminalObservationError::Parser(cause)
                | TerminalObservationError::Observer(cause) => cause,
            })
    }
    pub(crate) fn feed_observed_classified(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> std::result::Result<(), TerminalObservationError> {
        let mut failure = None;
        let mut start = 0;
        for (index, byte) in bytes.iter().enumerate() {
            if byte.is_ascii_control() {
                if index > start {
                    self.step(&bytes[start..index], observer, &mut failure)?;
                }
                self.step(&[*byte], observer, &mut failure)?;
                start = index + 1;
            }
        }
        if start < bytes.len() {
            self.step(&bytes[start..], observer, &mut failure)?;
        }
        match failure {
            Some(cause) => Err(TerminalObservationError::Observer(cause)),
            None => Ok(()),
        }
    }
    fn step(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
        failure: &mut Option<CafleetError>,
    ) -> std::result::Result<(), TerminalObservationError> {
        self.feed(bytes).map_err(TerminalObservationError::Parser)?;
        if let Err(cause) = observer.observe(&self.snapshot()) {
            if observer.failures_are_fatal() {
                return Err(TerminalObservationError::Observer(cause));
            }
            failure.get_or_insert(cause);
        }
        Ok(())
    }
    pub(crate) fn resize(&mut self, size: TerminalSize) -> Result<()> {
        size.validate()?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| error("terminal sequence exhausted"))?;
        self.parser.set_size(
            size.rows.try_into().expect("validated rows"),
            size.cols.try_into().expect("validated cols"),
        );
        Ok(())
    }
    pub(crate) fn snapshot(&self) -> ScreenSnapshot {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        ScreenSnapshot {
            #[cfg(test)]
            title: self.parser.callbacks().title.clone(),
            rows: rows.get(),
            cols: cols.get(),
            cursor_row,
            cursor_col,
            alternate_screen: screen.alternate_screen(),
            screen_seq: self.sequence,
            screen: screen.clone(),
        }
    }
}

pub(crate) struct RawTerminalGuard {
    terminal: File,
    original: Termios,
    restored: bool,
}
impl RawTerminalGuard {
    pub(crate) fn enter(terminal: &File) -> Result<Self> {
        let terminal = terminal.try_clone().map_err(io_error)?;
        let original = termios::tcgetattr(&terminal).map_err(io_error)?;
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&terminal, SetArg::TCSANOW, &raw).map_err(io_error)?;
        Ok(Self {
            terminal,
            original,
            restored: false,
        })
    }
    pub(crate) fn restore(&mut self) -> Result<()> {
        if !self.restored {
            termios::tcsetattr(&self.terminal, SetArg::TCSANOW, &self.original).map_err(
                |cause| {
                    error(format!(
                        "terminal restoration requires host recovery: {cause}"
                    ))
                },
            )?;
            self.restored = true;
        }
        Ok(())
    }
}
impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        if let Err(cause) = self.restore() {
            eprintln!("{cause}");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::transport::test_support::Observer;
    use crate::runtime::transport::*;

    #[test]
    fn runner_terminal_fragmented_utf8_and_ansi_reconstruct_the_same_live_screen() {
        let bytes = "\u{1b}[2J\u{1b}[H許可しますか？ ✓\r\n> prompt".as_bytes();
        let size = TerminalSize { rows: 24, cols: 80 };
        let mut whole = TerminalState::new(size).unwrap();
        whole.feed(bytes).unwrap();
        for width in 1..=bytes.len() {
            let mut fragmented = TerminalState::new(size).unwrap();
            for part in bytes.chunks(width) {
                fragmented.feed(part).unwrap();
            }
            let expected = whole.snapshot();
            let actual = fragmented.snapshot();
            assert_eq!(
                actual.screen.contents(),
                expected.screen.contents(),
                "width={width}"
            );
            assert_eq!(
                (actual.cursor_row, actual.cursor_col),
                (expected.cursor_row, expected.cursor_col)
            );
            assert_eq!(actual.title, expected.title);
        }
    }

    #[test]
    fn runner_terminal_preserves_primary_and_alternate_screens_and_fragmented_titles() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 10, cols: 40 }).unwrap();
        terminal.feed(b"primary composer").unwrap();
        let primary = terminal.snapshot();
        for byte in b"\x1b]0;Action Required\x1b\\\x1b[?1049h\x1b[2J\x1b[Halternate request" {
            terminal.feed(&[*byte]).unwrap();
        }
        let alternate = terminal.snapshot();
        assert!(alternate.alternate_screen);
        assert!(alternate.screen.contents().contains("alternate request"));
        assert!(!alternate.screen.contents().contains("primary composer"));
        assert_eq!(alternate.title, "Action Required");
        terminal.feed(b"\x1b[?1049l").unwrap();
        let restored = terminal.snapshot();
        assert!(!restored.alternate_screen);
        assert_eq!(restored.screen.contents(), primary.screen.contents());
        assert!(restored.screen_seq > alternate.screen_seq);
    }

    #[test]
    fn runner_terminal_live_snapshot_excludes_scrollback_and_keeps_prior_snapshots_immutable() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 3, cols: 40 }).unwrap();
        terminal.feed(b"old permission request").unwrap();
        let old = terminal.snapshot();
        terminal
            .feed(b"\r\nline two\r\nline three\r\nempty composer")
            .unwrap();
        let current = terminal.snapshot();
        assert!(!current.screen.contents().contains("old permission request"));
        assert!(current.screen.contents().contains("empty composer"));
        assert!(old.screen.contents().contains("old permission request"));
        assert!(current.screen_seq > old.screen_seq);
    }

    #[test]
    fn runner_terminal_cursor_updates_and_resize_remain_consistent_with_dimensions() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        terminal.feed(b"\x1b[4;7Hchoice").unwrap();
        let before = terminal.snapshot();
        assert_eq!((before.cursor_row, before.cursor_col), (3, 12));
        terminal
            .resize(TerminalSize {
                rows: 40,
                cols: 120,
            })
            .unwrap();
        let resized = terminal.snapshot();
        assert_eq!((resized.rows, resized.cols), (40, 120));
        assert!(resized.screen.contents().contains("choice"));
        assert!(resized.screen_seq >= before.screen_seq);
    }

    #[test]
    fn runner_terminal_rejects_empty_dimensions_at_creation_and_resize() {
        for size in [
            TerminalSize { rows: 0, cols: 80 },
            TerminalSize { rows: 24, cols: 0 },
        ] {
            assert!(TerminalState::new(size).is_err());
            let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            assert!(terminal.resize(size).is_err());
            let snapshot = terminal.snapshot();
            assert_eq!((snapshot.rows, snapshot.cols), (24, 80));
        }
    }
    #[test]
    fn runner_transport_visits_a_transient_screen_before_a_later_clear_in_one_fragment() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        let mut observer = Observer::default();
        terminal
            .feed_observed(b"candidate request\x1b[2J\x1b[Hcomposer", &mut observer)
            .unwrap();
        let candidate = observer
            .screens
            .iter()
            .position(|(_, screen)| screen.contains("candidate request"))
            .unwrap();
        let composer = observer
            .screens
            .iter()
            .rposition(|(_, screen)| {
                screen.contains("composer") && !screen.contains("candidate request")
            })
            .unwrap();
        assert!(candidate < composer);
        assert!(
            observer
                .screens
                .windows(2)
                .all(|pair| pair[1].0 > pair[0].0)
        );
    }

    use crate::broker::runtime as storage;
    use crate::coding_agent::ProfileCatalog;

    use crate::runtime::transport::REPLY_PREFIX_BOUND;
    use crate::runtime::writer::test_support::Clock;
    use crate::runtime::writer::test_support::Hooks;
    use crate::runtime::writer::test_support::Release;
    use crate::runtime::writer::test_support::WriterFixture;
    use crate::runtime::writer::test_support::start;
    use crate::runtime::writer::test_support::transition;
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::io;
    use std::io::ErrorKind;
    use std::panic::AssertUnwindSafe;
    use std::panic::catch_unwind;
    use std::panic::resume_unwind;
    use std::rc::Rc;
    use std::time::Duration;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Site {
        Human,
        ManualWrite,
        Child,
        DisplayWrite,
        Flush,
        Observe,
        Health,
    }

    type Trace = Rc<RefCell<Vec<Site>>>;

    #[derive(Debug)]
    enum Read {
        Data(Vec<u8>),
        Interrupted,
        Eof,
        Error(ErrorKind),
    }

    #[derive(Debug)]
    struct ReadRecord {
        capacity: usize,
        count: Option<usize>,
        entered: Instant,
        returned: Instant,
    }

    struct Reader {
        steps: VecDeque<Read>,
        fill: Option<u8>,
        width: usize,
        delay_until: Option<Instant>,
        calls: Vec<ReadRecord>,
        bytes: Vec<u8>,
        trace: Trace,
        site: Site,
    }

    impl Reader {
        fn new(site: Site, trace: &Trace, steps: impl IntoIterator<Item = Read>) -> Self {
            Self {
                steps: steps.into_iter().collect(),
                fill: None,
                width: usize::MAX,
                delay_until: None,
                calls: Vec::new(),
                bytes: Vec::new(),
                trace: trace.clone(),
                site,
            }
        }

        fn data(site: Site, trace: &Trace, bytes: &[u8], width: usize) -> Self {
            assert!(width > 0);
            Self::new(
                site,
                trace,
                bytes.chunks(width).map(|b| Read::Data(b.to_vec())),
            )
        }
    }

    impl ByteInput for Reader {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            assert!(!target.is_empty(), "an actual read has positive capacity");
            let entered = Instant::now();
            self.trace.borrow_mut().push(self.site);
            if let Some(until) = self.delay_until.take() {
                std::thread::sleep(
                    until.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                );
            }
            let result = match self.steps.pop_front() {
                Some(Read::Data(mut bytes)) => {
                    assert!(!bytes.is_empty());
                    let count = target.len().min(bytes.len()).min(self.width);
                    target[..count].copy_from_slice(&bytes[..count]);
                    bytes.drain(..count);
                    if !bytes.is_empty() {
                        self.steps.push_front(Read::Data(bytes));
                    }
                    Ok(count)
                }
                Some(Read::Interrupted) => Err(ErrorKind::Interrupted.into()),
                Some(Read::Eof) => Ok(0),
                Some(Read::Error(kind)) => Err(io::Error::new(kind, "fixture read failure")),
                None => match self.fill {
                    Some(byte) => {
                        let count = target.len().min(self.width);
                        target[..count].fill(byte);
                        Ok(count)
                    }
                    None => Err(ErrorKind::WouldBlock.into()),
                },
            };
            if let Ok(count) = result {
                self.bytes.extend_from_slice(&target[..count]);
            }
            self.calls.push(ReadRecord {
                capacity: target.len(),
                count: result.as_ref().ok().copied(),
                entered,
                returned: Instant::now(),
            });
            result
        }
    }

    struct DrainPty {
        reader: Reader,
        offered: Vec<Vec<u8>>,
        bytes: Vec<u8>,
        max_write: usize,
        write_error: Option<ErrorKind>,
        resize_error: bool,
    }

    impl DrainPty {
        fn new(trace: &Trace) -> Self {
            Self {
                reader: Reader::new(Site::Child, trace, []),
                offered: Vec::new(),
                bytes: Vec::new(),
                max_write: usize::MAX,
                write_error: None,
                resize_error: false,
            }
        }
    }

    impl ByteInput for DrainPty {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            self.reader.try_read(target)
        }
    }

    impl PtyIo for DrainPty {
        fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert!(!bytes.is_empty());
            self.reader.trace.borrow_mut().push(Site::ManualWrite);
            self.offered.push(bytes.to_vec());
            if let Some(kind) = self.write_error {
                return Err(io::Error::new(kind, "fixture PTY write failure"));
            }
            let count = bytes.len().min(self.max_write);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn resize(&mut self, _: TerminalSize) -> Result<()> {
            if self.resize_error {
                Err(error("fixture resize failure"))
            } else {
                Ok(())
            }
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    struct DrainOutput {
        trace: Trace,
        offered: Vec<Vec<u8>>,
        bytes: Vec<u8>,
        max_write: usize,
        write_error: Option<ErrorKind>,
        flush_error: Option<ErrorKind>,
        flushes: usize,
    }

    impl DrainOutput {
        fn new(trace: &Trace) -> Self {
            Self {
                trace: trace.clone(),
                offered: Vec::new(),
                bytes: Vec::new(),
                max_write: usize::MAX,
                write_error: None,
                flush_error: None,
                flushes: 0,
            }
        }
    }

    impl Display for DrainOutput {
        fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert!(!bytes.is_empty());
            self.trace.borrow_mut().push(Site::DisplayWrite);
            self.offered.push(bytes.to_vec());
            if let Some(kind) = self.write_error {
                return Err(io::Error::new(kind, "fixture display write failure"));
            }
            let count = bytes.len().min(self.max_write);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn try_flush(&mut self) -> io::Result<()> {
            self.trace.borrow_mut().push(Site::Flush);
            self.flushes += 1;
            match self.flush_error {
                Some(kind) => Err(io::Error::new(kind, "fixture flush failure")),
                None => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct Observations {
        screens: RefCell<Vec<u64>>,
        health_reads: Cell<usize>,
        last_health: RefCell<Option<ObservationHealth>>,
    }

    struct Measured<O> {
        inner: O,
        trace: Trace,
        facts: Rc<Observations>,
    }

    impl<O: ScreenObserver> ScreenObserver for Measured<O> {
        fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
            self.trace.borrow_mut().push(Site::Observe);
            self.facts.screens.borrow_mut().push(screen.screen_seq);
            self.inner.observe(screen)
        }

        fn failures_are_fatal(&self) -> bool {
            self.inner.failures_are_fatal()
        }

        fn parser_failure(&mut self) -> Result<()> {
            self.inner.parser_failure()
        }

        fn input_health(&self) -> Option<ObservationHealth> {
            self.trace.borrow_mut().push(Site::Health);
            self.facts
                .health_reads
                .set(self.facts.health_reads.get() + 1);
            let health = self.inner.input_health();
            self.facts.last_health.replace(health.clone());
            health
        }
    }

    struct Harness {
        fixture: WriterFixture,
        writer: EventWriter,
        controller: ObservationController,
        clock: Clock,
        hooks: Arc<Hooks>,
        release: Option<Release>,
        until: Instant,
        trace: Trace,
        observations: Rc<Observations>,
    }

    impl Harness {
        fn new() -> Self {
            let until = Instant::now() + Duration::from_secs(4);
            let fixture = WriterFixture::new();
            let controller = ObservationController::new(
                &fixture.fixture.run,
                ProfileCatalog::bundled(),
                storage::evidence::EvidencePolicy::metadata(),
                1..=256,
            )
            .unwrap();
            let hooks = Arc::new(Hooks::default());
            let release = Some(Release(hooks.clone()));
            let mut writer = start(&fixture, hooks.clone());
            writer.set_deadline(until);
            Self {
                fixture,
                writer,
                controller,
                clock: Clock::new(),
                hooks,
                release,
                until,
                trace: Rc::default(),
                observations: Rc::default(),
            }
        }

        fn drain(
            &mut self,
            drain_transport: &mut Transport,
            human: &mut Reader,
            pty: &mut DrainPty,
            output: &mut DrainOutput,
            until: Instant,
        ) -> Result<InputDrain> {
            drain_transport.quiesce_for_input(
                human,
                pty,
                output,
                &mut Measured {
                    inner: self.controller.observer(&self.writer, &self.clock),
                    trace: self.trace.clone(),
                    facts: self.observations.clone(),
                },
                until,
            )
        }

        fn seal(&mut self, drain_transport: &mut Transport) {
            let mut human = Reader::new(Site::Human, &self.trace, []);
            let mut pty = DrainPty::new(&self.trace);
            let mut output = DrainOutput::new(&self.trace);
            let result = self.drain(
                drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                self.until,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: drain_transport.human_generation(),
                    output_seq: drain_transport.snapshot().screen_seq,
                }
            );
            assert_eq!(human.calls.len(), 1);
            assert_eq!(pty.reader.calls.len(), 1);
        }

        fn control(&mut self) {
            let mut drain_transport = drain_transport(64, 64, 64);
            self.seal(&mut drain_transport);
            assert!(drain_transport.take_input_drain().is_some());
            assert!(drain_transport.take_input_drain().is_none());
            self.reset_observations();
        }

        fn reset_observations(&mut self) {
            self.trace.borrow_mut().clear();
            self.observations = Rc::default();
        }

        fn finish(&mut self) {
            drop(self.release.take());
            let result = self.writer.finish(self.until);
            assert!(
                self.writer.joined(),
                "writer cleanup has no confirmed join: {result:?}"
            );
            assert_eq!(result, (0, 0));
        }
    }

    fn with_harness(body: impl FnOnce(&mut Harness)) {
        let mut harness = Harness::new();
        let primary = catch_unwind(AssertUnwindSafe(|| body(&mut harness)));
        let cleanup = catch_unwind(AssertUnwindSafe(|| harness.finish()));
        if let Err(panic) = primary.and(cleanup) {
            resume_unwind(panic);
        }
    }

    fn drain_transport(bytes: usize, input: usize, display: usize) -> Transport {
        Transport::new(
            TerminalSize { rows: 24, cols: 80 },
            TransportLimits {
                input_bytes: input,
                display_bytes: display,
                bytes_per_tick: bytes,
            },
        )
        .unwrap()
    }

    fn assert_pending(result: Result<InputDrain>, drain_transport: &mut Transport) {
        assert_eq!(result.unwrap(), InputDrain::Pending);
        assert!(drain_transport.take_input_drain().is_none());
    }

    fn assert_deadline(result: Result<InputDrain>, drain_transport: &mut Transport) {
        assert!(
            matches!(result, Err(CafleetError::DeadlineExceeded)),
            "{result:?}"
        );
        assert!(drain_transport.take_input_drain().is_none());
    }

    fn assert_bounds(human: &Reader, pty: &DrainPty, output: &DrainOutput, bytes: usize) {
        assert!(human.calls.len() <= 8);
        assert!(pty.reader.calls.len() <= 8);
        assert!(pty.offered.len() <= 8);
        assert!(output.offered.len() <= 8);
        assert!(output.flushes <= 8);
        assert!(human.bytes.len() <= bytes);
        assert!(pty.reader.bytes.len() <= bytes);
        assert!(pty.bytes.len() <= bytes);
        assert!(output.bytes.len() <= bytes);
        assert!(
            human
                .calls
                .iter()
                .chain(&pty.reader.calls)
                .all(|r| r.capacity <= bytes.min(16384))
        );
    }

    #[test]
    fn drain_actual_quiet_round_seals_health_without_granting_composer_eligibility() {
        with_harness(|h| {
            let mut drain_transport = drain_transport(32, 32, 32);
            assert_eq!(drain_transport.human_generation(), 0);
            assert!(drain_transport.take_input_drain().is_none());
            h.seal(&mut drain_transport);
            assert_eq!(h.observations.health_reads.get(), 1);
            assert_eq!(
                *h.trace.borrow(),
                vec![Site::Human, Site::Child, Site::Flush, Site::Health]
            );
            let health = h.observations.last_health.borrow();
            let health = health.as_ref().expect("actual adapter supplied health");
            assert!(!health.degraded);
            assert_eq!(health.pending_count, 0);
            assert!(!health.pending_evidence);
            assert_eq!(
                health.eligibility,
                InputEligibility::Hold(GateReason::Unknown)
            );
            assert_eq!(
                serde_json::to_value(health).unwrap(),
                serde_json::to_value(h.controller.health(&h.writer)).unwrap()
            );
            assert!(drain_transport.take_input_drain().is_some());
            assert!(drain_transport.take_input_drain().is_none());
        });
    }

    #[test]
    fn drain_fragmented_reads_and_partial_writes_preserve_real_bytes_and_sequence() {
        with_harness(|h| {
            h.control();
            let human_bytes = "literal 世界\r".as_bytes();
            let child_bytes = b"child\r\n";
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, human_bytes, 4);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, child_bytes, 2);
            pty.max_write = 3;
            let mut output = DrainOutput::new(&h.trace);
            output.max_write = 3;
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: u64::try_from(human_bytes.len()).unwrap(),
                    output_seq: u64::try_from(child_bytes.len()).unwrap(),
                }
            );
            assert_eq!(pty.bytes, human_bytes);
            assert_eq!(output.bytes, child_bytes);
            assert_eq!(
                drain_transport.snapshot().screen_seq,
                u64::try_from(child_bytes.len()).unwrap()
            );
            assert!(
                drain_transport
                    .snapshot()
                    .screen
                    .contents()
                    .contains("child")
            );
            assert_eq!(drain_transport.pending_input_bytes(), 0);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            assert_eq!(h.observations.health_reads.get(), 1);
            assert!(
                h.observations
                    .screens
                    .borrow()
                    .windows(2)
                    .all(|pair| pair[0] <= pair[1])
            );
            assert_bounds(&human, &pty, &output, 64);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_interrupted_reads_require_a_later_actual_same_round_wouldblock_pair() {
        for human_interrupted in [true, false] {
            with_harness(|h| {
                h.control();
                let mut drain_transport = drain_transport(32, 32, 32);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                let mut pty = DrainPty::new(&h.trace);
                let mut output = DrainOutput::new(&h.trace);
                if human_interrupted {
                    human.steps.push_back(Read::Interrupted);
                } else {
                    pty.reader.steps.push_back(Read::Interrupted);
                }
                let result = h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until,
                );
                assert_pending(result, &mut drain_transport);
                assert_eq!(drain_transport.human_generation(), 0);
                assert_eq!(h.observations.health_reads.get(), 0);
                assert!(human.calls.len() <= 1 && pty.reader.calls.len() <= 1);
                h.seal(&mut drain_transport);
                assert!(drain_transport.take_input_drain().is_some());
            });
        }
    }

    #[test]
    fn drain_eof_is_not_a_quiet_read_and_adds_no_manual_generation() {
        for human_eof in [true, false] {
            with_harness(|h| {
                h.control();
                let mut drain_transport = drain_transport(32, 32, 32);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                let mut pty = DrainPty::new(&h.trace);
                let mut output = DrainOutput::new(&h.trace);
                if human_eof {
                    human.steps.push_back(Read::Eof);
                } else {
                    pty.reader.steps.push_back(Read::Eof);
                }
                let result = h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until,
                );
                assert_pending(result, &mut drain_transport);
                assert_eq!(drain_transport.human_generation(), 0);
                assert_eq!(h.observations.health_reads.get(), 0);
                assert_bounds(&human, &pty, &output, 32);
            });
        }
    }

    #[test]
    fn drain_round_and_separate_byte_quotas_bound_continuously_readable_sources() {
        for (bytes, width, expected_reads, expected_bytes) in [(64, 1, 8, 8), (4, usize::MAX, 1, 4)]
        {
            with_harness(|h| {
                h.control();
                let mut drain_transport = drain_transport(bytes, 128, 128);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                human.fill = Some(b'h');
                human.width = width;
                let mut pty = DrainPty::new(&h.trace);
                pty.reader.fill = Some(b'c');
                pty.reader.width = width;
                let mut output = DrainOutput::new(&h.trace);
                let result = h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until,
                );
                assert_pending(result, &mut drain_transport);
                assert_eq!(human.calls.len(), expected_reads);
                assert_eq!(pty.reader.calls.len(), expected_reads);
                assert_eq!(human.bytes.len(), expected_bytes);
                assert_eq!(pty.reader.bytes.len(), expected_bytes);
                assert_eq!(pty.bytes.len(), expected_bytes);
                assert_eq!(output.bytes.len(), expected_bytes);
                assert_eq!(h.observations.health_reads.get(), 0);
                assert_bounds(&human, &pty, &output, bytes);
                human.fill = None;
                pty.reader.fill = None;
                assert!(matches!(
                    h.drain(
                        &mut drain_transport,
                        &mut human,
                        &mut pty,
                        &mut output,
                        h.until
                    )
                    .unwrap(),
                    InputDrain::Quiescent { .. }
                ));
            });
        }
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"1234567", 1);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h
                .drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until,
                )
                .unwrap();
            assert!(matches!(
                result,
                InputDrain::Quiescent {
                    human_generation: 7,
                    ..
                }
            ));
            assert_eq!(human.calls.len(), 8);
            assert_eq!(h.observations.health_reads.get(), 1);
        });
    }

    #[test]
    fn drain_full_manual_queue_supplies_no_read_proof_and_preserves_order_on_release() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(16, 4, 32);
            let mut human = Reader::data(Site::Human, &h.trace, b"abcdefgh", 8);
            let mut pty = DrainPty::new(&h.trace);
            pty.write_error = Some(ErrorKind::WouldBlock);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(human.calls.len(), 1);
            assert_eq!(human.calls[0].capacity, 4);
            assert_eq!(drain_transport.pending_input_bytes(), 4);
            assert_eq!(drain_transport.human_generation(), 4);
            assert!(pty.bytes.is_empty());
            assert!(pty.offered.len() <= 8);
            pty.write_error = None;
            assert!(matches!(
                h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until
                )
                .unwrap(),
                InputDrain::Quiescent {
                    human_generation: 8,
                    ..
                }
            ));
            assert_eq!(pty.bytes, b"abcdefgh");
            assert_eq!(drain_transport.pending_input_bytes(), 0);
        });
    }

    #[test]
    fn drain_display_and_prefetched_child_pressure_require_real_later_drain() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(16, 32, 4);
            let mut human = Reader::data(Site::Human, &h.trace, b"manual", 6);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, b"abcdefgh", 8);
            let mut output = DrainOutput::new(&h.trace);
            output.write_error = Some(ErrorKind::WouldBlock);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(drain_transport.pending_display_bytes(), 4);
            assert_eq!(pty.bytes, b"manual");
            output.write_error = None;
            assert!(matches!(
                h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until
                )
                .unwrap(),
                InputDrain::Quiescent { .. }
            ));
            assert_eq!(output.bytes, b"abcdefgh");
        });
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(16, 32, 32);
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader.steps = [
                Read::Data(b"first".to_vec()),
                Read::Interrupted,
                Read::Data(b"rest".to_vec()),
            ]
            .into();
            let mut output = DrainOutput::new(&h.trace);
            assert_eq!(drain_transport.prefetch_child_bytes(&mut pty).unwrap(), 5);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(drain_transport.snapshot().screen_seq, 5);
            assert_eq!(output.bytes, b"first");
            let calls = h.trace.borrow();
            let observed = calls.iter().position(|s| *s == Site::Observe).unwrap();
            let second_read = calls
                .iter()
                .enumerate()
                .filter(|(_, s)| **s == Site::Child)
                .nth(1)
                .unwrap()
                .0;
            assert!(observed < second_read);
            drop(calls);
            assert!(matches!(
                h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    h.until
                )
                .unwrap(),
                InputDrain::Quiescent { .. }
            ));
            assert_eq!(output.bytes, b"firstrest");
            assert_eq!(drain_transport.snapshot().screen_seq, 9);
        });
    }

    struct LegacyObserver(usize);
    impl ScreenObserver for LegacyObserver {
        fn observe(&mut self, _: &ScreenSnapshot) -> Result<()> {
            self.0 += 1;
            Ok(())
        }
    }

    #[test]
    fn drain_absent_health_keeps_manual_observation_progress_without_a_proof() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(32, 32, 32);
            let mut human = Reader::data(Site::Human, &h.trace, b"human", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, b"output", 6);
            let mut output = DrainOutput::new(&h.trace);
            let mut observer = LegacyObserver(0);
            assert!(observer.input_health().is_none());
            let result = drain_transport.quiesce_for_input(
                &mut human,
                &mut pty,
                &mut output,
                &mut observer,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(pty.bytes, b"human");
            assert_eq!(output.bytes, b"output");
            assert_eq!(drain_transport.human_generation(), 5);
            assert!(observer.0 > 0);
            h.seal(&mut drain_transport);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_actual_controller_gap_health_prevents_quiet_proof() {
        with_harness(|h| {
            h.control();
            h.controller
                .observer_failed(ObservationFailure::Parser, &h.clock)
                .unwrap();
            let expected = h.controller.health(&h.writer);
            assert!(expected.degraded);
            assert!(expected.gap_count > 0);
            let mut drain_transport = drain_transport(32, 32, 32);
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(h.observations.health_reads.get(), 1);
            assert_eq!(
                serde_json::to_value(h.observations.last_health.borrow().as_ref().unwrap())
                    .unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        });
    }

    #[test]
    fn drain_real_pending_writer_commit_blocks_health_until_release_and_persistence() {
        with_harness(|h| {
            h.control();
            h.hooks.pause(WriterBoundary::Persist);
            h.writer.submit(transition(&h.fixture, 1)).unwrap();
            while h.hooks.calls(WriterBoundary::Persist) == 0 {
                assert!(
                    Instant::now() < h.until,
                    "actual persist boundary was not entered"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(h.writer.status().pending_count, 1);
            let mut drain_transport = drain_transport(32, 32, 32);
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(
                h.observations
                    .last_health
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .pending_count,
                1
            );
            drop(h.release.take());
            while h.writer.status().pending_count != 0 {
                assert!(
                    Instant::now() < h.until,
                    "actual event commit did not finish"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            let count: i64 = h
                .fixture
                .fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                    [h.fixture.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
            assert_eq!(h.writer.status().pending_count, 0);
            h.seal(&mut drain_transport);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_tick_and_drain_keep_actual_human_counts_before_later_errors() {
        for quiesce in [false, true] {
            for failure in ["pty", "child", "display", "flush", "observer", "zero"] {
                with_harness(|h| {
                    h.control();
                    let mut drain_transport = drain_transport(32, 32, 32);
                    h.seal(&mut drain_transport);
                    let mut human = Reader::data(Site::Human, &h.trace, "世x".as_bytes(), 4);
                    let mut pty = DrainPty::new(&h.trace);
                    pty.reader = Reader::data(Site::Child, &h.trace, b"output", 6);
                    let mut output = DrainOutput::new(&h.trace);
                    let expected = match failure {
                        "pty" => {
                            pty.write_error = Some(ErrorKind::BrokenPipe);
                            "fixture PTY write failure"
                        }
                        "child" => {
                            pty.reader.steps = [Read::Error(ErrorKind::BrokenPipe)].into();
                            "fixture read failure"
                        }
                        "display" => {
                            output.write_error = Some(ErrorKind::BrokenPipe);
                            "fixture display write failure"
                        }
                        "flush" => {
                            output.flush_error = Some(ErrorKind::BrokenPipe);
                            "fixture flush failure"
                        }
                        "observer" => {
                            h.clock.set(1);
                            h.controller
                                .observer(&h.writer, &h.clock)
                                .observe(&drain_transport.snapshot())
                                .unwrap();
                            h.clock.set(0);
                            "observation clock regressed"
                        }
                        "zero" => {
                            pty.max_write = 0;
                            "terminal write made no progress"
                        }
                        _ => unreachable!("fixed failure matrix"),
                    };
                    let result = if quiesce {
                        h.drain(
                            &mut drain_transport,
                            &mut human,
                            &mut pty,
                            &mut output,
                            h.until,
                        )
                        .map(|_| ())
                    } else {
                        drain_transport
                            .tick(
                                &mut human,
                                &mut pty,
                                &mut output,
                                &mut h.controller.observer(&h.writer, &h.clock),
                            )
                            .map(|_| ())
                    };
                    assert!(
                        result.unwrap_err().message().contains(expected),
                        "{failure}"
                    );
                    assert_eq!(human.bytes, "世x".as_bytes());
                    assert_eq!(drain_transport.human_generation(), 4);
                    assert!(drain_transport.take_input_drain().is_none());
                });
            }
        }
    }

    #[test]
    fn drain_opaque_proof_is_single_use_and_diagnostic_copies_supply_no_authority() {
        with_harness(|h| {
            let mut other = drain_transport(32, 32, 32);
            let mut drain_transport = drain_transport(32, 32, 32);
            h.seal(&mut drain_transport);
            let diagnostic = InputDrain::Quiescent {
                human_generation: 0,
                output_seq: drain_transport.snapshot().screen_seq,
            };
            let copied = diagnostic;
            assert_eq!(
                copied,
                InputDrain::Quiescent {
                    human_generation: 0,
                    output_seq: 0
                }
            );
            assert_eq!(drain_transport.human_generation(), 0);
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert!(other.take_input_drain().is_none());
            assert!(drain_transport.take_input_drain().is_some());
            assert!(drain_transport.take_input_drain().is_none());
            assert!(other.take_input_drain().is_none());
        });
    }

    #[test]
    fn drain_mutating_entrypoints_invalidate_before_success_or_later_failure() {
        for operation in [
            "tick",
            "tick-error",
            "resize",
            "resize-invalid",
            "resize-error",
            "prefetch",
            "prefetch-data",
            "flush",
            "flush-error",
            "discard",
            "drain-interrupted",
            "drain-error",
            "drain-expired",
        ] {
            with_harness(|h| {
                h.control();
                let mut drain_transport = drain_transport(32, 32, 32);
                h.seal(&mut drain_transport);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                let mut pty = DrainPty::new(&h.trace);
                let mut output = DrainOutput::new(&h.trace);
                match operation {
                    "tick" => {
                        drain_transport
                            .tick(
                                &mut human,
                                &mut pty,
                                &mut output,
                                &mut h.controller.observer(&h.writer, &h.clock),
                            )
                            .unwrap();
                    }
                    "tick-error" => {
                        human.steps.push_back(Read::Error(ErrorKind::BrokenPipe));
                        assert!(
                            drain_transport
                                .tick(
                                    &mut human,
                                    &mut pty,
                                    &mut output,
                                    &mut h.controller.observer(&h.writer, &h.clock)
                                )
                                .is_err()
                        );
                    }
                    "resize" => {
                        drain_transport
                            .resize(&mut pty, TerminalSize { rows: 25, cols: 80 })
                            .unwrap();
                    }
                    "resize-invalid" => {
                        assert!(
                            drain_transport
                                .resize(&mut pty, TerminalSize { rows: 0, cols: 80 })
                                .is_err()
                        );
                    }
                    "resize-error" => {
                        pty.resize_error = true;
                        assert!(
                            drain_transport
                                .resize(&mut pty, TerminalSize { rows: 25, cols: 80 })
                                .is_err()
                        );
                    }
                    "prefetch" => {
                        assert_eq!(drain_transport.prefetch_child_bytes(&mut pty).unwrap(), 0);
                    }
                    "prefetch-data" => {
                        pty.reader.steps.push_back(Read::Data(b"tail".to_vec()));
                        assert_eq!(drain_transport.prefetch_child_bytes(&mut pty).unwrap(), 4);
                    }
                    "flush" => {
                        assert_eq!(drain_transport.flush_display(&mut output).unwrap(), 0);
                    }
                    "flush-error" => {
                        output.flush_error = Some(ErrorKind::BrokenPipe);
                        assert!(drain_transport.flush_display(&mut output).is_err());
                    }
                    "discard" => {
                        assert_eq!(drain_transport.discard_display_tail(), 0);
                    }
                    "drain-interrupted" => {
                        human.steps.push_back(Read::Interrupted);
                        assert_pending(
                            h.drain(
                                &mut drain_transport,
                                &mut human,
                                &mut pty,
                                &mut output,
                                h.until,
                            ),
                            &mut drain_transport,
                        );
                    }
                    "drain-error" => {
                        human.steps.push_back(Read::Error(ErrorKind::BrokenPipe));
                        assert!(
                            h.drain(
                                &mut drain_transport,
                                &mut human,
                                &mut pty,
                                &mut output,
                                h.until
                            )
                            .is_err()
                        );
                    }
                    "drain-expired" => {
                        assert_deadline(
                            h.drain(
                                &mut drain_transport,
                                &mut human,
                                &mut pty,
                                &mut output,
                                Instant::now(),
                            ),
                            &mut drain_transport,
                        );
                    }
                    _ => unreachable!("fixed mutation matrix"),
                }
                assert!(drain_transport.take_input_drain().is_none(), "{operation}");
            });
        }
    }

    #[test]
    fn drain_original_expiry_prevents_later_io_and_proof_while_retaining_completed_reads() {
        with_harness(|h| {
            let mut drain_transport = drain_transport(32, 32, 32);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            assert_deadline(
                h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    Instant::now(),
                ),
                &mut drain_transport,
            );
            assert!(h.trace.borrow().is_empty());
            assert!(human.calls.is_empty() && pty.reader.calls.is_empty());
            assert!(pty.offered.is_empty() && output.offered.is_empty());
            assert_eq!(output.flushes, 0);
        });
        for fails in [false, true] {
            with_harness(|h| {
                h.control();
                let mut drain_transport = drain_transport(32, 32, 32);
                let mut human = Reader::new(
                    Site::Human,
                    &h.trace,
                    [if fails {
                        Read::Error(ErrorKind::BrokenPipe)
                    } else {
                        Read::Data(b"read".to_vec())
                    }],
                );
                let until = Instant::now() + Duration::from_millis(100);
                human.delay_until = Some(until);
                let mut pty = DrainPty::new(&h.trace);
                let mut output = DrainOutput::new(&h.trace);
                let result = h.drain(
                    &mut drain_transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    until,
                );
                let returned = Instant::now();
                assert_eq!(human.calls.len(), 1);
                let call = &human.calls[0];
                assert!(call.entered < until);
                assert!(call.returned >= until && returned >= call.returned);
                if fails {
                    assert!(
                        result
                            .unwrap_err()
                            .message()
                            .contains("fixture read failure")
                    );
                    assert_eq!(call.count, None);
                    assert_eq!(drain_transport.human_generation(), 0);
                    assert!(drain_transport.take_input_drain().is_none());
                } else {
                    assert_deadline(result, &mut drain_transport);
                    assert_eq!(call.count, Some(4));
                    assert_eq!(drain_transport.human_generation(), 4);
                    assert_eq!(drain_transport.pending_input_bytes(), 4);
                }
                assert_eq!(*h.trace.borrow(), vec![Site::Human]);
                assert!(pty.reader.calls.is_empty() && pty.offered.is_empty());
                assert!(output.offered.is_empty());
                assert_eq!(output.flushes, 0);
                assert_eq!(h.observations.health_reads.get(), 0);
            });
        }
    }

    #[derive(Clone, Copy)]
    enum CallbackFault {
        None,
        Expire,
        Fail,
        ExpireAndFail,
    }

    struct CallbackRecord {
        sequence: u64,
        contents: String,
        entered: Instant,
        returned: Instant,
    }

    struct ControlledObservation<O> {
        inner: O,
        records: Rc<RefCell<Vec<CallbackRecord>>>,
        fault: CallbackFault,
        deadline: Instant,
        fatal: bool,
    }

    impl<O: ScreenObserver> ScreenObserver for ControlledObservation<O> {
        fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
            let entered = Instant::now();
            let actual = self.inner.observe(screen);
            let targeted = self.records.borrow().len() == 1;
            if targeted
                && matches!(
                    self.fault,
                    CallbackFault::Expire | CallbackFault::ExpireAndFail
                )
            {
                std::thread::sleep(
                    self.deadline.saturating_duration_since(Instant::now())
                        + Duration::from_millis(1),
                );
            }
            self.records.borrow_mut().push(CallbackRecord {
                sequence: screen.screen_seq,
                contents: screen.screen.contents(),
                entered,
                returned: Instant::now(),
            });
            actual?;
            if targeted
                && matches!(
                    self.fault,
                    CallbackFault::Fail | CallbackFault::ExpireAndFail
                )
            {
                Err(CafleetError::App("fixture observation stopped".into()))
            } else {
                Ok(())
            }
        }

        fn failures_are_fatal(&self) -> bool {
            self.fatal
        }

        fn parser_failure(&mut self) -> Result<()> {
            self.inner.parser_failure()
        }

        fn input_health(&self) -> Option<ObservationHealth> {
            self.inner.input_health()
        }
    }

    fn assert_healthy(h: &Harness) {
        let actual = h.controller.health(&h.writer);
        assert!(!actual.degraded, "{actual:?}");
        assert_eq!(actual.pending_count, 0);
        assert!(!actual.pending_evidence);
        assert_eq!(
            actual.eligibility,
            InputEligibility::Hold(GateReason::Unknown)
        );
    }

    fn assert_incomplete_observation_stays_fenced(
        h: &mut Harness,
        drain_transport: &mut Transport,
        endpoints: (&mut Reader, &mut DrainPty, &mut DrainOutput),
        child_bytes: &[u8],
        prefix_sequence: u64,
    ) {
        let (human, pty, output) = endpoints;
        let generation = drain_transport.human_generation();
        let callbacks = h.observations.screens.borrow().len();
        assert_healthy(h);
        let first = h.drain(drain_transport, human, pty, output, h.until);
        let first_proof = drain_transport.take_input_drain().is_some();
        assert_eq!(drain_transport.snapshot().screen_seq, prefix_sequence);
        assert_eq!(h.observations.screens.borrow().len(), callbacks);
        assert_eq!(output.bytes, child_bytes);
        assert_eq!(drain_transport.pending_display_bytes(), 0);

        human.steps.push_back(Read::Data(b"manual\r".to_vec()));
        pty.reader.steps.push_back(Read::Data(b"live\r\n".to_vec()));
        let progress = drain_transport
            .tick(
                human,
                pty,
                output,
                &mut Measured {
                    inner: h.controller.observer(&h.writer, &h.clock),
                    trace: h.trace.clone(),
                    facts: h.observations.clone(),
                },
            )
            .unwrap();
        assert_eq!(progress.human_bytes, 7);
        assert_eq!(progress.child_bytes, 6);
        assert_eq!(progress.display_bytes, 6);
        assert_eq!(drain_transport.human_generation(), generation + 7);
        assert_eq!(pty.bytes, b"typedmanual\r");
        let mut displayed = child_bytes.to_vec();
        displayed.extend_from_slice(b"live\r\n");
        assert_eq!(output.bytes, displayed);
        assert_eq!(drain_transport.snapshot().screen_seq, prefix_sequence + 6);
        assert_healthy(h);
        let after_tick = h.drain(drain_transport, human, pty, output, h.until);
        let tick_proof = drain_transport.take_input_drain().is_some();

        drain_transport
            .resize(pty, TerminalSize { rows: 25, cols: 80 })
            .unwrap();
        assert_eq!(drain_transport.snapshot().screen_seq, prefix_sequence + 7);
        assert_healthy(h);
        let after_resize = h.drain(drain_transport, human, pty, output, h.until);
        let resize_proof = drain_transport.take_input_drain().is_some();
        let outcomes = [
            (first, first_proof),
            (after_tick, tick_proof),
            (after_resize, resize_proof),
        ];
        assert!(
            outcomes
                .iter()
                .all(|(result, proof)| matches!(result, Ok(InputDrain::Pending)) && !proof),
            "an incomplete observation stays fenced for this Transport lifetime: {outcomes:?}"
        );
    }

    #[test]
    fn drain_incomplete_delayed_positive_child_read_keeps_display_and_lifetime_fence() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let child_bytes = b"unseen\r\nsuffix";
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, child_bytes, child_bytes.len());
            let until = Instant::now() + Duration::from_millis(100);
            pty.reader.delay_until = Some(until);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                until,
            );
            assert_deadline(result, &mut drain_transport);
            assert_eq!(human.bytes, b"typed");
            assert_eq!(drain_transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.calls.len(), 1);
            let call = &pty.reader.calls[0];
            assert!(call.entered < until && call.returned >= until);
            assert_eq!(call.count, Some(child_bytes.len()));
            assert_eq!(pty.reader.bytes, child_bytes);
            assert_eq!(drain_transport.pending_display_bytes(), child_bytes.len());
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert_eq!(drain_transport.snapshot().screen.contents(), "");
            assert!(h.observations.screens.borrow().is_empty());
            assert_eq!(
                *h.trace.borrow(),
                vec![Site::Human, Site::ManualWrite, Site::Child]
            );
            assert!(output.bytes.is_empty() && output.offered.is_empty());
            assert_eq!(output.flushes, 0);
            assert_eq!(h.observations.health_reads.get(), 0);
            assert_incomplete_observation_stays_fenced(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                child_bytes,
                0,
            );
        });
    }

    const CONTROL_SEPARATED_CHILD: &[u8] = b"head\rTAIL\r\nsuffix";

    fn callback_drain(
        h: &mut Harness,
        drain_transport: &mut Transport,
        endpoints: (&mut Reader, &mut DrainPty, &mut DrainOutput),
        fault: CallbackFault,
        until: Instant,
        records: &Rc<RefCell<Vec<CallbackRecord>>>,
    ) -> Result<InputDrain> {
        let (human, pty, output) = endpoints;
        drain_transport.quiesce_for_input(
            human,
            pty,
            output,
            &mut ControlledObservation {
                inner: Measured {
                    inner: h.controller.observer(&h.writer, &h.clock),
                    trace: h.trace.clone(),
                    facts: h.observations.clone(),
                },
                records: records.clone(),
                fault,
                deadline: until,
                fatal: true,
            },
            until,
        )
    }

    #[test]
    fn drain_incomplete_complete_control_observes_every_fragment_and_can_seal() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(
                Site::Child,
                &h.trace,
                CONTROL_SEPARATED_CHILD,
                CONTROL_SEPARATED_CHILD.len(),
            );
            let mut output = DrainOutput::new(&h.trace);
            let records = Rc::default();
            let until = h.until;
            let result = callback_drain(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                CallbackFault::None,
                until,
                &records,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: 5,
                    output_seq: 17
                }
            );
            assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            assert_eq!(drain_transport.snapshot().screen.contents(), "TAIL\nsuffix");
            assert_eq!(drain_transport.snapshot().screen_seq, 17);
            assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
            let observed = records.borrow();
            assert_eq!(
                observed.iter().map(|r| r.sequence).collect::<Vec<_>>(),
                vec![4, 5, 9, 10, 11, 17]
            );
            assert_eq!(observed[0].contents, "head");
            assert_eq!(observed[1].contents, "head");
            assert_eq!(observed.last().unwrap().contents, "TAIL\nsuffix");
            assert!(
                observed
                    .iter()
                    .all(|r| r.entered <= r.returned && r.returned < h.until)
            );
            assert_healthy(h);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    fn incomplete_callback_case(fault: CallbackFault) {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(
                Site::Child,
                &h.trace,
                CONTROL_SEPARATED_CHILD,
                CONTROL_SEPARATED_CHILD.len(),
            );
            let mut output = DrainOutput::new(&h.trace);
            let until = Instant::now() + Duration::from_millis(100);
            let records = Rc::default();
            let result = callback_drain(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                fault,
                until,
                &records,
            );
            if matches!(fault, CallbackFault::Expire) {
                assert_deadline(result, &mut drain_transport);
            } else {
                assert_eq!(
                    format!("{:?}", result.unwrap_err()),
                    "App(\"fixture observation stopped\")"
                );
                assert!(drain_transport.take_input_drain().is_none());
            }
            assert_eq!(drain_transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(pty.reader.calls[0].count, Some(17));
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.pending_display_bytes(), 17);
            assert!(output.bytes.is_empty() && output.offered.is_empty());
            assert_eq!(output.flushes, 0);
            assert_eq!(h.observations.health_reads.get(), 0);
            assert_eq!(drain_transport.snapshot().screen_seq, 5);
            assert_eq!(drain_transport.snapshot().screen.contents(), "head");
            assert_eq!(drain_transport.snapshot().cursor_row, 0);
            assert_eq!(drain_transport.snapshot().cursor_col, 0);
            assert_eq!(*h.observations.screens.borrow(), vec![4, 5]);
            assert_eq!(
                *h.trace.borrow(),
                vec![
                    Site::Human,
                    Site::ManualWrite,
                    Site::Child,
                    Site::Observe,
                    Site::Observe
                ]
            );
            let observed = records.borrow();
            assert_eq!(observed.len(), 2);
            assert_eq!(
                observed.iter().map(|r| r.sequence).collect::<Vec<_>>(),
                vec![4, 5]
            );
            assert!(
                observed
                    .iter()
                    .all(|r| r.contents == "head" && r.entered < until)
            );
            assert!(observed[0].returned <= observed[1].entered);
            if matches!(fault, CallbackFault::Expire | CallbackFault::ExpireAndFail) {
                assert!(observed[1].returned >= until);
            } else {
                assert!(observed[1].returned < until);
            }
            drop(observed);
            assert_incomplete_observation_stays_fenced(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                5,
            );
        });
    }

    #[test]
    fn drain_incomplete_mid_observer_expiry_preserves_prefix_and_lifetime_fence() {
        incomplete_callback_case(CallbackFault::Expire);
    }

    #[test]
    fn drain_incomplete_mid_observer_error_before_expiry_keeps_primary_and_fence() {
        incomplete_callback_case(CallbackFault::Fail);
    }

    #[test]
    fn drain_incomplete_mid_observer_error_after_expiry_keeps_primary_and_fence() {
        incomplete_callback_case(CallbackFault::ExpireAndFail);
    }

    fn incomplete_tick_case(fatal: bool) {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            let mut output = DrainOutput::new(&h.trace);
            let records = Rc::new(RefCell::new(Vec::new()));
            let result = drain_transport.tick(
                &mut human,
                &mut pty,
                &mut output,
                &mut ControlledObservation {
                    inner: Measured {
                        inner: h.controller.observer(&h.writer, &h.clock),
                        trace: h.trace.clone(),
                        facts: h.observations.clone(),
                    },
                    records: records.clone(),
                    fault: CallbackFault::Fail,
                    deadline: h.until,
                    fatal,
                },
            );
            assert_eq!(drain_transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(pty.reader.calls[0].count, Some(17));
            assert!(drain_transport.take_input_drain().is_none());
            let prefix = if fatal {
                let source = match result {
                    Err(source) => source,
                    Ok(_) => panic!("fatal ordered observation returns its actual error"),
                };
                assert_eq!(
                    format!("{source:?}"),
                    "App(\"fixture observation stopped\")"
                );
                assert_eq!(*h.observations.screens.borrow(), vec![4, 5]);
                assert_eq!(records.borrow().len(), 2);
                assert_eq!(drain_transport.snapshot().screen.contents(), "head");
                assert!(output.bytes.is_empty() && output.offered.is_empty());
                assert_eq!(
                    drain_transport.pending_display_bytes(),
                    17,
                    "a completed child read remains queued after fatal observation"
                );
                5
            } else {
                let progress = result.unwrap();
                assert_eq!(progress.human_bytes, 5);
                assert_eq!(progress.child_bytes, 17);
                assert_eq!(progress.display_bytes, 17);
                assert_eq!(
                    progress.observation_error.as_deref(),
                    Some("fixture observation stopped")
                );
                assert!(matches!(
                    progress.observation_failure,
                    Some(ObservationFailure::Sink)
                ));
                assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
                assert_eq!(records.borrow().len(), 6);
                assert_eq!(drain_transport.snapshot().screen.contents(), "TAIL\nsuffix");
                assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
                assert_eq!(drain_transport.pending_display_bytes(), 0);
                17
            };
            assert_eq!(drain_transport.snapshot().screen_seq, prefix);
            assert_incomplete_observation_stays_fenced(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                prefix,
            );
        });
    }

    #[test]
    fn drain_incomplete_ordinary_fatal_observer_error_retains_chunk_and_fence() {
        incomplete_tick_case(true);
    }

    #[test]
    fn drain_incomplete_ordinary_nonfatal_observer_error_retains_reporting_and_fence() {
        incomplete_tick_case(false);
    }

    struct OneParserFailure(std::sync::atomic::AtomicBool);

    impl ObservationHooks for OneParserFailure {
        fn before(&self, _: ObservationPhase, operation: ObservationOperation) -> Result<()> {
            if matches!(operation, ObservationOperation::Parser)
                && self.0.swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                Err(CafleetError::App("fixture parser boundary stopped".into()))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn drain_incomplete_ordinary_parser_fault_cannot_be_cleared_by_later_success() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            drain_transport
                .set_observation_hooks(Arc::new(OneParserFailure(
                    std::sync::atomic::AtomicBool::new(true),
                )))
                .unwrap();
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            let mut output = DrainOutput::new(&h.trace);
            let progress = drain_transport
                .tick(
                    &mut human,
                    &mut pty,
                    &mut output,
                    &mut h.controller.observer(&h.writer, &h.clock),
                )
                .unwrap();
            assert_eq!(progress.human_bytes, 5);
            assert_eq!(progress.child_bytes, 17);
            assert_eq!(progress.display_bytes, 17);
            assert_eq!(
                progress.observation_error.as_deref(),
                Some("fixture parser boundary stopped")
            );
            assert!(matches!(
                progress.observation_failure,
                Some(ObservationFailure::Parser)
            ));
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert_eq!(drain_transport.snapshot().screen.contents(), "");
            assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.human_generation(), 5);
            assert_incomplete_observation_stays_fenced(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                0,
            );
        });
    }

    #[test]
    fn drain_incomplete_complete_observation_then_display_error_preserves_eligibility() {
        for quiesce in [false, true] {
            for discard in [false, true] {
                with_harness(|h| {
                    h.control();
                    let mut drain_transport = drain_transport(64, 64, 64);
                    let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
                    let mut pty = DrainPty::new(&h.trace);
                    pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
                    let mut output = DrainOutput::new(&h.trace);
                    output.write_error = Some(ErrorKind::BrokenPipe);
                    let result = if quiesce {
                        h.drain(
                            &mut drain_transport,
                            &mut human,
                            &mut pty,
                            &mut output,
                            h.until,
                        )
                        .map(|_| ())
                    } else {
                        drain_transport
                            .tick(
                                &mut human,
                                &mut pty,
                                &mut output,
                                &mut Measured {
                                    inner: h.controller.observer(&h.writer, &h.clock),
                                    trace: h.trace.clone(),
                                    facts: h.observations.clone(),
                                },
                            )
                            .map(|_| ())
                    };
                    assert!(
                        result
                            .unwrap_err()
                            .message()
                            .contains("fixture display write failure")
                    );
                    assert_eq!(drain_transport.human_generation(), 5);
                    assert_eq!(drain_transport.snapshot().screen_seq, 17);
                    assert_eq!(drain_transport.snapshot().screen.contents(), "TAIL\nsuffix");
                    assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
                    assert_eq!(drain_transport.pending_display_bytes(), 17);
                    assert!(output.bytes.is_empty());
                    assert!(drain_transport.take_input_drain().is_none());
                    output.write_error = None;
                    if discard {
                        assert_eq!(drain_transport.discard_display_tail(), 17);
                    }
                    assert_healthy(h);
                    let result = h.drain(
                        &mut drain_transport,
                        &mut human,
                        &mut pty,
                        &mut output,
                        h.until,
                    );
                    assert_eq!(
                        result.unwrap(),
                        InputDrain::Quiescent {
                            human_generation: 5,
                            output_seq: 17
                        }
                    );
                    assert_eq!(drain_transport.pending_display_bytes(), 0);
                    assert_eq!(
                        output.bytes,
                        if discard {
                            b"".as_slice()
                        } else {
                            CONTROL_SEPARATED_CHILD
                        }
                    );
                    assert!(drain_transport.take_input_drain().is_some());
                });
            }
        }
    }

    #[test]
    fn drain_incomplete_prefetch_consumption_control_observes_once_and_can_seal() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            let mut output = DrainOutput::new(&h.trace);
            assert_eq!(drain_transport.prefetch_child_bytes(&mut pty).unwrap(), 17);
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert!(h.observations.screens.borrow().is_empty());
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: 5,
                    output_seq: 17
                }
            );
            assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
            assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_incomplete_discarding_unobserved_prefetch_keeps_lifetime_fence() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let progress = drain_transport
                .tick(
                    &mut human,
                    &mut pty,
                    &mut output,
                    &mut h.controller.observer(&h.writer, &h.clock),
                )
                .unwrap();
            assert_eq!(progress.human_bytes, 5);
            assert_eq!(progress.child_bytes, 0);
            h.reset_observations();
            pty.reader
                .steps
                .push_back(Read::Data(CONTROL_SEPARATED_CHILD.to_vec()));
            assert_eq!(drain_transport.prefetch_child_bytes(&mut pty).unwrap(), 17);
            assert_eq!(pty.reader.calls.last().unwrap().count, Some(17));
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.discard_display_tail(), 17);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert!(h.observations.screens.borrow().is_empty());
            assert!(output.bytes.is_empty());
            assert_incomplete_observation_stays_fenced(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                b"",
                0,
            );
        });
    }

    #[test]
    fn drain_complete_terminal_reply_seals_a_proof_without_manual_generation() {
        with_harness(|h| {
            h.control();
            let reply = b"\x1b[I\x1b]11;rgb:0000/0000/0000\x07\x1b[24;80R";
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, reply, 64);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: 0,
                    output_seq: drain_transport.snapshot().screen_seq,
                }
            );
            assert_eq!(pty.bytes, reply, "the reply is forwarded unchanged");
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_pending_reply_prefix_blocks_the_proof_with_terminal_reply_pending() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"\x1b[24;", 64);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(
                drain_transport.drain_blocker(),
                Some("terminal reply pending")
            );
            assert_eq!(pty.bytes, b"\x1b[24;", "the prefix is forwarded, not held");
            assert_eq!(drain_transport.pending_input_bytes(), 0);
            assert_eq!(drain_transport.human_generation(), 0);
        });
    }

    #[test]
    fn drain_completed_reply_prefix_seals_a_proof_with_unchanged_generation() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"\x1b[24;", 64);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            human.steps.push_back(Read::Data(b"80R".to_vec()));
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: 0,
                    output_seq: drain_transport.snapshot().screen_seq,
                }
            );
            assert_eq!(drain_transport.drain_blocker(), None);
            assert_eq!(pty.bytes, b"\x1b[24;80R");
            let proof = drain_transport
                .take_input_drain()
                .expect("quiet drain seals a proof");
            assert_eq!(proof.human_generation(), 0);
            drain_transport
                .validate_observation_drain(&proof, h.until)
                .unwrap();
        });
    }

    #[test]
    fn drain_proof_taken_before_a_reply_prefix_fails_validation_while_it_is_pending() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            h.seal(&mut drain_transport);
            let proof = drain_transport
                .take_input_drain()
                .expect("quiet drain seals a proof");
            drain_transport
                .validate_observation_drain(&proof, h.until)
                .unwrap();
            let mut human = Reader::data(Site::Human, &h.trace, b"\x1b", 64);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            drain_transport
                .tick(
                    &mut human,
                    &mut pty,
                    &mut output,
                    &mut h.controller.observer(&h.writer, &h.clock),
                )
                .unwrap();
            assert_eq!(drain_transport.human_generation(), 0);
            assert!(
                drain_transport
                    .validate_observation_drain(&proof, h.until)
                    .is_err()
            );
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(
                drain_transport.drain_blocker(),
                Some("terminal reply pending")
            );
            assert!(
                drain_transport
                    .validate_observation_drain(&proof, h.until)
                    .is_err()
            );
        });
    }

    #[test]
    fn drain_backdated_lone_escape_resolves_as_manual_on_the_next_drain() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = drain_transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"\x1b", 64);
            let mut pty = DrainPty::new(&h.trace);
            let mut output = DrainOutput::new(&h.trace);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_pending(result, &mut drain_transport);
            assert_eq!(
                drain_transport.drain_blocker(),
                Some("terminal reply pending")
            );
            assert_eq!(drain_transport.human_generation(), 0);
            drain_transport.backdate_reply_prefix(REPLY_PREFIX_BOUND);
            let result = h.drain(
                &mut drain_transport,
                &mut human,
                &mut pty,
                &mut output,
                h.until,
            );
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: 1,
                    output_seq: drain_transport.snapshot().screen_seq,
                }
            );
            assert_eq!(pty.bytes, b"\x1b");
            let proof = drain_transport
                .take_input_drain()
                .expect("quiet drain seals a proof");
            assert_eq!(proof.human_generation(), 1);
        });
    }

    use std::time::Instant;

    fn classifier() -> ReplyClassifier {
        ReplyClassifier::default()
    }

    fn feed_once(bytes: &[u8]) -> (usize, bool) {
        let mut classifier = classifier();
        let manual = classifier.feed(bytes, Instant::now()).manual;
        (manual, classifier.pending())
    }

    fn assert_reply(bytes: &[u8]) {
        assert_eq!(
            feed_once(bytes),
            (0, false),
            "{:?} is a complete allowlisted reply",
            String::from_utf8_lossy(bytes)
        );
    }

    fn assert_manual(bytes: &[u8]) {
        assert_eq!(
            feed_once(bytes),
            (bytes.len(), false),
            "{:?} counts every byte as manual",
            String::from_utf8_lossy(bytes)
        );
    }

    fn joined(params: &[&str]) -> String {
        params.join(";")
    }

    mod allowlist {
        use crate::runtime::transport::tests::*;

        #[test]
        fn reply_classifier_cursor_position_reports_are_replies() {
            assert_reply(b"\x1b[24;80R");
            assert_reply(b"\x1b[1;1R");
            assert_reply(b"\x1b[?24;80R");
            assert_reply(b"\x1b[?24;80;1R");
        }

        #[test]
        fn reply_classifier_modified_f3_collides_with_a_cursor_position_report() {
            assert_reply(b"\x1b[1;5R");
        }

        #[test]
        fn reply_classifier_device_status_reports_are_replies() {
            assert_reply(b"\x1b[0n");
            assert_reply(b"\x1b[3n");
            assert_reply(b"\x1b[?1n");
            assert_reply(b"\x1b[?997;1n");
            assert_reply(b"\x1b[?1;2;3;4n");
        }

        #[test]
        fn reply_classifier_device_attribute_reports_are_replies() {
            assert_reply(b"\x1b[?1;2c");
            assert_reply(b"\x1b[?62;22c");
            assert_reply(b"\x1b[>0c");
            assert_reply(b"\x1b[>1;10;0c");
            assert_reply(b"\x1b[>41;380;0;1c");
        }

        #[test]
        fn reply_classifier_primary_device_attributes_accept_thirty_two_parameters() {
            let reply = format!("\x1b[?{}c", joined(&["1"; 32]));
            assert_eq!(
                reply.len() - 3,
                CSI_PARAMETER_LIMIT,
                "the private marker and 32 parameters fill the CSI parameter limit exactly"
            );
            assert_reply(reply.as_bytes());
        }

        #[test]
        fn reply_classifier_mode_reports_are_replies() {
            assert_reply(b"\x1b[?1004;1$y");
            assert_reply(b"\x1b[?2026;2$y");
            assert_reply(b"\x1b[4;2$y");
        }

        #[test]
        fn reply_classifier_window_reports_are_replies() {
            assert_reply(b"\x1b[1t");
            assert_reply(b"\x1b[2t");
            assert_reply(b"\x1b[4;600;800t");
            assert_reply(b"\x1b[8;24;80t");
        }

        #[test]
        fn reply_classifier_kitty_keyboard_flags_report_is_a_reply() {
            assert_reply(b"\x1b[?1u");
            assert_reply(b"\x1b[?31u");
        }

        #[test]
        fn reply_classifier_focus_events_are_replies() {
            assert_reply(b"\x1b[I");
            assert_reply(b"\x1b[O");
        }

        #[test]
        fn reply_classifier_osc_replies_terminate_with_bel_or_st() {
            assert_reply(b"\x1b]11;rgb:0000/0000/0000\x07");
            assert_reply(b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\");
            assert_reply(b"\x1b]4;1;rgb:cdcd/0000/0000\x1b\\");
        }

        #[test]
        fn reply_classifier_osc_body_accepts_bytes_at_or_above_0x80() {
            assert_reply("\x1b]2;caf\u{e9}\x07".as_bytes());
        }

        #[test]
        fn reply_classifier_dcs_replies_terminate_with_st() {
            assert_reply(b"\x1bP>|XTerm(380)\x1b\\");
            assert_reply(b"\x1bP1+r544e=787465726d\x1b\\");
            assert_reply(b"\x1bP1$r0m\x1b\\");
            assert_reply(b"\x1bP!|00000000\x1b\\");
        }

        #[test]
        fn reply_classifier_apc_replies_terminate_with_st() {
            assert_reply(b"\x1b_Gi=31;OK\x1b\\");
            assert_reply(b"\x1b_Gi=1;ENOENT:file not found\x1b\\");
        }

        #[test]
        fn reply_classifier_string_body_at_the_limit_is_a_reply() {
            let mut reply = b"\x1b]".to_vec();
            reply.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT));
            reply.push(0x07);
            assert_reply(&reply);
        }

        #[test]
        fn reply_classifier_a_burst_of_replies_counts_zero() {
            assert_reply(
                b"\x1b[I\x1b]11;rgb:0000/0000/0000\x07\x1b[24;80R\x1b[O\x1bP>|XTerm(380)\x1b\\",
            );
        }
    }

    mod arity {
        use crate::runtime::transport::tests::*;

        #[test]
        fn reply_classifier_window_report_with_four_parameters_is_manual() {
            assert_manual(b"\x1b[8;24;80;1t");
        }

        #[test]
        fn reply_classifier_cursor_position_report_with_one_parameter_is_manual() {
            assert_manual(b"\x1b[5R");
            assert_manual(b"\x1b[?5R");
            assert_manual(b"\x1b[?1;2;3;4R");
        }

        #[test]
        fn reply_classifier_empty_parameters_are_manual() {
            assert_manual(b"\x1b[8;t");
            assert_manual(b"\x1b[;24R");
            assert_manual(b"\x1b[24;;80R");
            assert_manual(b"\x1b[?n");
            assert_manual(b"\x1b[?c");
        }

        #[test]
        fn reply_classifier_device_status_outside_its_forms_is_manual() {
            assert_manual(b"\x1b[1n");
            assert_manual(b"\x1b[5n");
            assert_manual(b"\x1b[?1;2;3;4;5n");
        }

        #[test]
        fn reply_classifier_device_attributes_outside_their_arity_are_manual() {
            assert_manual(b"\x1b[>1;2;3;4;5c");
            assert_manual(b"\x1b[1c");
        }

        #[test]
        fn reply_classifier_mode_reports_outside_their_arity_are_manual() {
            assert_manual(b"\x1b[?1$y");
            assert_manual(b"\x1b[?1;2;3$y");
            assert_manual(b"\x1b[4$y");
        }

        #[test]
        fn reply_classifier_kitty_flags_with_more_than_one_parameter_are_manual() {
            assert_manual(b"\x1b[?1;2u");
        }

        #[test]
        fn reply_classifier_focus_with_parameters_or_intermediates_is_manual() {
            assert_manual(b"\x1b[1I");
            assert_manual(b"\x1b[?I");
            assert_manual(b"\x1b[ O");
        }

        #[test]
        fn reply_classifier_unlisted_intermediate_is_manual() {
            assert_manual(b"\x1b[1;1 R");
            assert_manual(b"\x1b[1$t");
        }
    }

    mod manual {
        use crate::runtime::transport::tests::*;

        #[test]
        fn reply_classifier_text_and_c0_controls_are_manual_immediately() {
            assert_manual(b"hello");
            assert_manual("caf\u{e9} \u{65e5}\u{672c}".as_bytes());
            assert_manual(b"\r");
            assert_manual(b"\t");
            assert_manual(b"\x03");
            assert_manual(b"\x7f");
            assert_manual(b"\x07");
        }

        #[test]
        fn reply_classifier_cursor_and_function_keys_are_manual() {
            assert_manual(b"\x1b[A");
            assert_manual(b"\x1b[1;5C");
            assert_manual(b"\x1b[3~");
            assert_manual(b"\x1b[15;2~");
            assert_manual(b"\x1b[Z");
        }

        #[test]
        fn reply_classifier_kitty_key_events_are_manual() {
            assert_manual(b"\x1b[97u");
            assert_manual(b"\x1b[97;5u");
            assert_manual(b"\x1b[97;5;97u");
        }

        #[test]
        fn reply_classifier_mouse_reports_are_manual() {
            assert_manual(b"\x1b[<0;10;20M");
            assert_manual(b"\x1b[<0;10;20m");
            assert_manual(b"\x1b[<64;5;5M");
            assert_manual(b"\x1b[M !!");
        }

        #[test]
        fn reply_classifier_ss3_and_alt_modified_keys_are_manual() {
            assert_manual(b"\x1bOP");
            assert_manual(b"\x1bOA");
            assert_manual(b"\x1ba");
            assert_manual(b"\x1bx");
            assert_manual(b"\x1b\r");
        }

        #[test]
        fn reply_classifier_sos_and_pm_strings_are_manual() {
            assert_manual(b"\x1bXhello\x1b\\");
            assert_manual(b"\x1b^hi\x1b\\");
        }

        #[test]
        fn reply_classifier_eight_bit_c1_bytes_are_manual() {
            assert_manual(&[0x9b]);
            assert_manual(&[0x9d]);
            assert_manual(&[0x90]);
            assert_manual(&[0x9f]);
            assert_manual(b"\x9b1;1R");
            assert_manual(b"\x9bI");
        }

        #[test]
        fn reply_classifier_overlong_csi_is_manual_at_the_byte_exceeding_the_limit() {
            let mut prefix = b"\x1b[".to_vec();
            prefix.extend(std::iter::repeat_n(b'1', CSI_PARAMETER_LIMIT));
            let mut classifier = classifier();
            let now = Instant::now();
            assert_eq!(classifier.feed(&prefix, now).manual, 0);
            assert!(classifier.pending());
            assert_eq!(
                classifier.feed(b"1", now).manual,
                prefix.len() + 1,
                "the parameter byte that exceeds the limit resolves the whole prefix"
            );
            assert!(!classifier.pending());
            assert_eq!(classifier.feed(b"R", now).manual, 1);
        }

        #[test]
        fn reply_classifier_overlong_csi_in_one_read_counts_every_byte() {
            let mut input = b"\x1b[?".to_vec();
            input.extend(joined(&["1"; 40]).bytes());
            input.push(b'c');
            assert!(input.len() - 3 > CSI_PARAMETER_LIMIT);
            assert_manual(&input);
        }

        #[test]
        fn reply_classifier_osc_body_over_the_limit_is_manual() {
            let mut prefix = b"\x1b]".to_vec();
            prefix.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT));
            let mut classifier = classifier();
            let now = Instant::now();
            assert_eq!(classifier.feed(&prefix, now).manual, 0);
            assert!(classifier.pending());
            assert_eq!(
                classifier.feed(b"x", now).manual,
                prefix.len() + 1,
                "the body byte that exceeds the limit resolves the whole prefix"
            );
            assert!(!classifier.pending());
            assert_eq!(classifier.feed(b"\x07", now).manual, 1);
        }

        #[test]
        fn reply_classifier_osc_52_clipboard_read_over_the_limit_counts_every_byte() {
            let mut input = b"\x1b]52;c;".to_vec();
            input.extend(std::iter::repeat_n(b'A', STRING_BODY_LIMIT));
            input.push(0x07);
            assert_manual(&input);
        }

        #[test]
        fn reply_classifier_dcs_and_apc_bodies_over_the_limit_count_every_byte() {
            for introducer in [b"\x1bP", b"\x1b_"] {
                let mut input = introducer.to_vec();
                input.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT + 1));
                input.extend(b"\x1b\\");
                assert_manual(&input);
            }
        }

        #[test]
        fn reply_classifier_mixed_read_counts_only_the_text_around_a_reply() {
            assert_eq!(feed_once(b"ab\x1b[Icd"), (4, false));
        }

        #[test]
        fn reply_classifier_bracketed_paste_counts_every_byte_including_reply_forms() {
            assert_manual(b"\x1b[200~\x1b[1;1R\x1b[Ipasted\x1b]11;x\x07\x1b[201~");
        }

        #[test]
        fn reply_classifier_bracketed_paste_split_across_reads_counts_every_byte() {
            let input: &[u8] = b"\x1b[200~one\x1b[1;1R\x1b[Itwo\x1b[201~";
            let mut classifier = classifier();
            let now = Instant::now();
            let manual: usize = input
                .chunks(3)
                .map(|chunk| classifier.feed(chunk, now).manual)
                .sum();
            assert_eq!(manual, input.len());
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_reply_after_bracketed_paste_counts_zero() {
            let paste: &[u8] = b"\x1b[200~text\x1b[201~";
            let mut classifier = classifier();
            let now = Instant::now();
            assert_eq!(classifier.feed(paste, now).manual, paste.len());
            assert_eq!(classifier.feed(b"\x1b[I", now).manual, 0);
            assert!(!classifier.pending());
        }
    }

    mod invalid_continuation {
        use crate::runtime::transport::tests::*;

        #[test]
        fn reply_classifier_escape_inside_csi_restarts_a_reply() {
            assert_eq!(feed_once(b"\x1b[1;\x1b[I"), (4, false));
        }

        #[test]
        fn reply_classifier_escape_before_a_reply_counts_one() {
            assert_eq!(feed_once(b"\x1b\x1b[I"), (1, false));
        }

        #[test]
        fn reply_classifier_control_bytes_inside_csi_count_the_prefix_and_the_byte() {
            for byte in [0x18, 0x1a, 0x00, 0x03, 0x07, 0x0d, 0x7f, 0x80, 0xc3, 0xff] {
                let input = [0x1b, b'[', b'1', byte];
                assert_eq!(feed_once(&input), (4, false), "byte {byte:#04x}");
            }
        }

        #[test]
        fn reply_classifier_control_byte_inside_csi_is_followed_by_classification_from_ground() {
            assert_eq!(feed_once(b"\x1b[1\x18\x1b[I"), (4, false));
        }

        #[test]
        fn reply_classifier_parameter_byte_after_an_intermediate_is_manual() {
            assert_manual(b"\x1b[1$1y");
        }

        #[test]
        fn reply_classifier_bel_ends_an_osc_reply() {
            assert_reply(b"\x1b]11;x\x07");
        }

        #[test]
        fn reply_classifier_bel_inside_dcs_or_apc_is_manual() {
            assert_manual(b"\x1bP1$r\x07");
            assert_manual(b"\x1b_Gi=1\x07");
        }

        #[test]
        fn reply_classifier_bel_inside_dcs_resolves_before_a_following_reply() {
            assert_eq!(feed_once(b"\x1bP1$r\x07\x1b[I"), (6, false));
        }

        #[test]
        fn reply_classifier_escape_without_backslash_inside_osc_is_manual() {
            assert_manual(b"\x1b]11;x\x1ba");
            assert_manual(b"\x1b]1\x1b[I");
        }

        #[test]
        fn reply_classifier_escape_without_backslash_inside_osc_reclassifies_the_next_byte() {
            assert_eq!(feed_once(b"\x1b]1\x1b\x1b[I"), (4, false));
        }

        #[test]
        fn reply_classifier_escape_without_backslash_inside_dcs_or_apc_is_manual() {
            assert_manual(b"\x1bP1$r\x1bq");
            assert_manual(b"\x1b_G\x1bq");
        }

        #[test]
        fn reply_classifier_control_byte_inside_a_string_body_is_manual() {
            assert_manual(b"\x1b]11;x\x18");
            assert_manual(b"\x1bP1\r");
        }

        #[test]
        fn reply_classifier_escape_with_an_invalid_continuation_counts_both_bytes() {
            assert_manual(b"\x1b1");
            assert_manual(b"\x1b\x7f");
        }
    }

    mod pending {
        use crate::runtime::transport::tests::*;

        const SPLIT_REPLIES: [&[u8]; 7] = [
            b"\x1b[I",
            b"\x1b[24;80R",
            b"\x1b[?62;22c",
            b"\x1b]11;rgb:0000/0000/0000\x07",
            b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
            b"\x1bP>|XTerm(380)\x1b\\",
            b"\x1b_Gi=31;OK\x1b\\",
        ];

        #[test]
        fn reply_classifier_reply_split_at_every_boundary_counts_zero() {
            let now = Instant::now();
            for reply in SPLIT_REPLIES {
                for split in 1..reply.len() {
                    let mut classifier = classifier();
                    assert_eq!(classifier.feed(&reply[..split], now).manual, 0);
                    assert!(classifier.pending(), "{reply:?} split at {split}");
                    assert_eq!(classifier.feed(&reply[split..], now).manual, 0);
                    assert!(!classifier.pending(), "{reply:?} split at {split}");
                }
            }
        }

        #[test]
        fn reply_classifier_reply_fed_one_byte_per_read_counts_zero() {
            let now = Instant::now();
            for reply in SPLIT_REPLIES {
                let mut classifier = classifier();
                for (index, byte) in reply.iter().enumerate() {
                    assert_eq!(classifier.feed(&[*byte], now).manual, 0);
                    assert_eq!(
                        classifier.pending(),
                        index + 1 < reply.len(),
                        "{reply:?} after byte {index}"
                    );
                }
            }
        }

        #[test]
        fn reply_classifier_reply_split_across_reads_at_different_instants_counts_zero() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[24", start).manual, 0);
            let later = start + REPLY_PREFIX_BOUND / 2;
            assert_eq!(classifier.expire(later).manual, 0);
            assert_eq!(classifier.feed(b";80R", later).manual, 0);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_lone_escape_resolves_as_manual_at_the_bound() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b", start).manual, 0);
            assert!(classifier.pending());
            let before = start + REPLY_PREFIX_BOUND - Duration::from_millis(1);
            assert_eq!(classifier.expire(before).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.expire(start + REPLY_PREFIX_BOUND).manual, 1);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_csi_prefix_resolves_as_manual_at_the_bound() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[12;", start).manual, 0);
            assert!(classifier.pending());
            let before = start + REPLY_PREFIX_BOUND - Duration::from_millis(1);
            assert_eq!(classifier.expire(before).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.expire(start + REPLY_PREFIX_BOUND).manual, 5);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_bound_restarts_at_each_new_prefix_byte() {
            let start = Instant::now();
            let second = start + Duration::from_millis(60);
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b", start).manual, 0);
            assert_eq!(classifier.feed(b"[", second).manual, 0);
            assert_eq!(classifier.expire(start + REPLY_PREFIX_BOUND).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.expire(second + REPLY_PREFIX_BOUND).manual, 2);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_bytes_after_an_expired_prefix_classify_from_ground() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b", start).manual, 0);
            let expired = start + REPLY_PREFIX_BOUND;
            assert_eq!(classifier.expire(expired).manual, 1);
            assert_eq!(classifier.feed(b"[I", expired).manual, 2);
            assert_eq!(classifier.feed(b"\x1b[I", expired).manual, 0);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_backdate_ages_a_pending_prefix_to_the_bound() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[", now).manual, 0);
            assert_eq!(classifier.expire(now).manual, 0);
            classifier.backdate(REPLY_PREFIX_BOUND);
            assert_eq!(classifier.expire(now).manual, 2);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_expire_without_a_pending_prefix_counts_zero() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.expire(now + REPLY_PREFIX_BOUND).manual, 0);
            assert_eq!(classifier.feed(b"\x1b[I", now).manual, 0);
            assert_eq!(classifier.expire(now + REPLY_PREFIX_BOUND * 2).manual, 0);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_resolves_a_pending_prefix_as_manual() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[1;", now).manual, 0);
            assert_eq!(classifier.finish().manual, 4);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_resolves_a_pending_string_body_as_manual() {
            let prefix: &[u8] = b"\x1b]11;rgb:00";
            let mut classifier = classifier();
            assert_eq!(classifier.feed(prefix, Instant::now()).manual, 0);
            assert_eq!(classifier.finish().manual, prefix.len());
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_resolves_a_pending_string_escape_as_manual() {
            let prefix: &[u8] = b"\x1bP1$r0m\x1b";
            let mut classifier = classifier();
            assert_eq!(classifier.feed(prefix, Instant::now()).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.finish().manual, prefix.len());
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_without_a_pending_prefix_counts_zero() {
            let mut classifier = classifier();
            assert_eq!(classifier.finish().manual, 0);
            assert_eq!(classifier.feed(b"ab", Instant::now()).manual, 2);
            assert_eq!(classifier.finish().manual, 0);
        }

        #[test]
        fn reply_classifier_text_before_a_pending_prefix_counts_in_the_same_read() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"ab\x1b[", now).manual, 2);
            assert!(classifier.pending());
            assert_eq!(classifier.feed(b"O", now).manual, 0);
            assert!(!classifier.pending());
        }
    }

    use std::collections::VecDeque;

    use crate::error::CafleetError;

    struct Bytes {
        chunks: VecDeque<Vec<u8>>,
        eof: bool,
    }

    impl Bytes {
        fn new(bytes: &[u8], width: usize) -> Self {
            assert!(width > 0);
            Self {
                chunks: bytes.chunks(width).map(<[u8]>::to_vec).collect(),
                eof: false,
            }
        }
    }

    impl ByteInput for Bytes {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            let Some(mut chunk) = self.chunks.pop_front() else {
                return if self.eof {
                    Ok(0)
                } else {
                    Err(ErrorKind::WouldBlock.into())
                };
            };
            let count = target.len().min(chunk.len());
            target[..count].copy_from_slice(&chunk[..count]);
            chunk.drain(..count);
            if !chunk.is_empty() {
                self.chunks.push_front(chunk);
            }
            Ok(count)
        }
    }

    struct Pty {
        output: Bytes,
        input: Vec<u8>,
        offers: Vec<usize>,
        blocked: bool,
        max_write: usize,
        sizes: Vec<TerminalSize>,
        closed: bool,
    }

    impl Pty {
        fn new(output: &[u8], width: usize) -> Self {
            Self {
                output: Bytes::new(output, width),
                input: Vec::new(),
                offers: Vec::new(),
                blocked: false,
                max_write: usize::MAX,
                sizes: Vec::new(),
                closed: false,
            }
        }
    }

    impl ByteInput for Pty {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            self.output.try_read(target)
        }
    }

    impl PtyIo for Pty {
        fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.blocked {
                return Err(ErrorKind::WouldBlock.into());
            }
            self.offers.push(bytes.len());
            let count = bytes.len().min(self.max_write);
            self.input.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn resize(&mut self, size: TerminalSize) -> std::result::Result<(), CafleetError> {
            self.sizes.push(size);
            Ok(())
        }
        fn close(&mut self) -> std::result::Result<(), CafleetError> {
            self.closed = true;
            Ok(())
        }
    }

    #[derive(Default)]
    struct Output {
        bytes: Vec<u8>,
        blocked: bool,
        broken: bool,
        max_write: Option<usize>,
        ordering: Option<Rc<RefCell<Vec<&'static str>>>>,
    }

    impl Display for Output {
        fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.broken {
                return Err(ErrorKind::BrokenPipe.into());
            }
            if self.blocked {
                return Err(ErrorKind::WouldBlock.into());
            }
            if let Some(ordering) = &self.ordering {
                ordering.borrow_mut().push("display");
            }
            let count = bytes.len().min(self.max_write.unwrap_or(bytes.len()));
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn try_flush(&mut self) -> io::Result<()> {
            if self.blocked {
                Err(ErrorKind::WouldBlock.into())
            } else {
                Ok(())
            }
        }
    }

    fn transport() -> Transport {
        Transport::new(
            TerminalSize { rows: 24, cols: 80 },
            TransportLimits {
                input_bytes: 32,
                display_bytes: 64,
                bytes_per_tick: 16,
            },
        )
        .unwrap()
    }

    fn service_transport() -> Transport {
        Transport::new(
            TerminalSize { rows: 24, cols: 80 },
            TransportLimits {
                input_bytes: 65536,
                display_bytes: 65536,
                bytes_per_tick: 16384,
            },
        )
        .unwrap()
    }

    fn patterned(len: usize, salt: usize) -> Vec<u8> {
        (0..len)
            .map(|index| u8::try_from((index + salt) % 251).unwrap())
            .collect()
    }

    fn reads(sizes: &[usize]) -> (Bytes, Vec<u8>) {
        let mut all = Vec::new();
        let mut chunks = VecDeque::new();
        for (salt, &size) in sizes.iter().enumerate() {
            let chunk = patterned(size, salt * 7);
            all.extend_from_slice(&chunk);
            chunks.push_back(chunk);
        }
        (Bytes { chunks, eof: false }, all)
    }

    fn unread(input: &Bytes) -> usize {
        input.chunks.iter().map(Vec::len).sum()
    }

    fn one_tick(transport: &mut Transport, human: &mut Bytes, pty: &mut Pty) -> TransportProgress {
        transport
            .tick(human, pty, &mut Output::default(), &mut Observer::default())
            .unwrap()
    }

    #[test]
    fn runner_transport_one_tick_drains_a_split_outer_reply_and_writes_it_in_one_offer() {
        let mut transport = service_transport();
        let (mut human, reply) = reads(&[4096, 3600]);
        let mut pty = Pty::new(b"", 1);
        let generation = transport.human_generation();
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            unread(&human),
            0,
            "the tick reads until the outer input would block"
        );
        assert_eq!(pty.input, reply, "all 7696 bytes reach the child in order");
        assert_eq!(
            pty.offers,
            [7696],
            "the whole queue is offered in one write"
        );
        assert_eq!(transport.pending_input_bytes(), 0);
        assert_eq!(progress.human_bytes, 7696);
        assert_eq!(progress.forwarded_bytes, 7696);
        assert!(!progress.outer_eof);
        assert_eq!(transport.human_generation(), generation + 7696);
    }

    #[test]
    fn runner_transport_outer_reads_stop_at_the_full_input_queue_and_resume_next_tick() {
        let mut transport = service_transport();
        let (mut human, payload) = reads(&[16384, 16384, 16384, 16384, 16384]);
        let mut pty = Pty::new(b"", 1);
        let first = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(first.human_bytes, 65536);
        assert_eq!(transport.human_generation(), 65536);
        assert_eq!(
            unread(&human),
            16384,
            "reads stop once the queue holds 65536 bytes"
        );
        assert_eq!(pty.input, payload[..65536]);
        let second = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            second.human_bytes, 16384,
            "the remaining reply is read on the next tick"
        );
        assert_eq!(unread(&human), 0);
        assert_eq!(pty.input, payload);
        assert_eq!(transport.human_generation(), 81920);
    }

    #[test]
    fn runner_transport_blocked_master_keeps_the_full_queue_and_reads_nothing_more() {
        let mut transport = service_transport();
        let (mut human, payload) = reads(&[16384, 16384, 16384, 16384, 16384]);
        let mut pty = Pty::new(b"", 1);
        pty.blocked = true;
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(transport.pending_input_bytes(), 65536);
        assert_eq!(unread(&human), 16384);
        let held = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(held.human_bytes, 0, "a full queue admits no further read");
        assert_eq!(
            held.forwarded_bytes, 0,
            "the blocked master accepted none of the queued input"
        );
        assert_eq!(transport.pending_input_bytes(), 65536);
        assert_eq!(unread(&human), 16384);
        pty.blocked = false;
        for _ in 0..3 {
            one_tick(&mut transport, &mut human, &mut pty);
        }
        assert_eq!(pty.input, payload);
        assert_eq!(transport.human_generation(), 81920);
    }

    #[test]
    fn runner_transport_forwards_data_read_before_outer_eof_and_reports_eof() {
        let mut transport = service_transport();
        let (mut human, reply) = reads(&[4096, 12]);
        human.eof = true;
        let mut pty = Pty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, reply);
        assert!(progress.outer_eof);
        assert_eq!(progress.human_bytes, 4108);
        assert_eq!(transport.human_generation(), 4108);
    }

    #[test]
    fn runner_transport_partial_master_write_keeps_the_remainder_queued_in_order() {
        let mut transport = service_transport();
        let (mut human, reply) = reads(&[4096, 3600]);
        let mut pty = Pty::new(b"", 1);
        pty.max_write = 5000;
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, reply[..5000]);
        assert_eq!(transport.pending_input_bytes(), 2696);
        let later = patterned(10, 99);
        human.chunks.push_back(later.clone());
        one_tick(&mut transport, &mut human, &mut pty);
        let mut expected = reply.clone();
        expected.extend_from_slice(&later);
        assert_eq!(pty.input, expected, "queued bytes precede later reads");
        assert_eq!(pty.offers, [7696, 2706]);
        assert_eq!(transport.pending_input_bytes(), 0);
    }

    #[test]
    fn runner_transport_passes_escape_sequences_unframed() {
        let mut transport = service_transport();
        let reply =
            b"\x1b]4;0;rgb:0000/0000/0000\x1b\\\x1b]11;rgb:afaf/d7d7/8787\x07\x1b[".to_vec();
        let mut human = Bytes {
            chunks: VecDeque::from([reply.clone()]),
            eof: false,
        };
        let mut pty = Pty::new(b"", 1);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            pty.input, reply,
            "an unterminated sequence is forwarded, not held"
        );
        assert_eq!(transport.pending_input_bytes(), 0);
        assert_eq!(
            transport.human_generation(),
            0,
            "complete OSC replies and a pending prefix are not yet manual"
        );
        transport.backdate_reply_prefix(REPLY_PREFIX_BOUND);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, reply);
        assert_eq!(
            transport.human_generation(),
            2,
            "the trailing ESC [ counts as manual once its bound passes"
        );
    }

    #[test]
    fn runner_transport_preserves_fragmented_child_and_human_bytes_through_partial_writes() {
        let child = "\u{1b}[2J許可 ✓\r\n\u{1b}]0;title\u{7}".as_bytes();
        let human = "\u{1b}[200~literal 世界\u{1b}[201~\r\u{3}".as_bytes();
        let mut transport = transport();
        let mut pty = Pty::new(child, 1);
        pty.max_write = 1;
        let mut human_input = Bytes::new(human, 2);
        let mut display = Output {
            max_write: Some(1),
            ..Default::default()
        };
        let mut observer = Observer::default();
        for _ in 0..1024 {
            transport
                .tick(&mut human_input, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        assert_eq!(pty.input, human);
        assert_eq!(display.bytes, child);
        assert_eq!(transport.pending_input_bytes(), 0);
        assert_eq!(transport.pending_display_bytes(), 0);
    }

    #[test]
    fn runner_transport_observes_reconstructed_output_before_displaying_it() {
        let ordering = Rc::new(RefCell::new(Vec::new()));
        let mut transport = transport();
        let mut pty = Pty::new(b"request", 7);
        let mut human = Bytes::new(b"", 1);
        let mut display = Output {
            ordering: Some(ordering.clone()),
            ..Default::default()
        };
        let mut observer = Observer {
            ordering: Some(ordering.clone()),
            ..Default::default()
        };
        for _ in 0..10 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        let calls = ordering.borrow();
        let observe = calls.iter().position(|call| *call == "observe").unwrap();
        let displayed = calls.iter().position(|call| *call == "display").unwrap();
        assert!(observe < displayed);
        assert!(
            observer
                .screens
                .iter()
                .any(|(_, screen)| screen.contains("request"))
        );
    }

    #[test]
    fn runner_transport_full_display_queue_bounds_memory_while_human_input_continues() {
        let payload = vec![b'x'; 4096];
        let mut transport = transport();
        let mut pty = Pty::new(&payload, 16);
        let mut human = Bytes::new(b"human native answer\r", 1);
        let mut display = Output {
            blocked: true,
            ..Default::default()
        };
        let mut observer = Observer::default();
        for _ in 0..128 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
            assert!(transport.pending_display_bytes() <= 64);
            assert!(transport.pending_input_bytes() <= 32);
        }
        assert_eq!(pty.input, b"human native answer\r");
        assert!(!pty.output.chunks.is_empty());
        display.blocked = false;
        for _ in 0..1024 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        assert_eq!(display.bytes, payload);
    }

    #[test]
    fn runner_transport_bounded_input_backpressure_preserves_all_pending_human_bytes() {
        let payload = vec![b'a'; 512];
        let mut transport = transport();
        let mut pty = Pty::new(b"", 1);
        pty.blocked = true;
        let mut human = Bytes::new(&payload, 16);
        let mut display = Output::default();
        let mut observer = Observer::default();
        for _ in 0..64 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
            assert!(transport.pending_input_bytes() <= 32);
        }
        assert!(!human.chunks.is_empty());
        pty.blocked = false;
        pty.max_write = 1;
        for _ in 0..2048 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        assert_eq!(pty.input, payload);
    }

    #[test]
    fn runner_transport_observation_queue_failure_reports_degradation_and_preserves_manual_io() {
        let mut transport = transport();
        let mut pty = Pty::new(b"native output", 4);
        let mut human = Bytes::new(b"yes\r", 1);
        let mut display = Output::default();
        let mut observer = Observer {
            fail: true,
            ..Default::default()
        };
        let mut reported = false;
        for _ in 0..64 {
            let progress = transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
            reported |= progress.observation_error.is_some();
        }
        assert!(reported);
        assert_eq!(pty.input, b"yes\r");
        assert_eq!(display.bytes, b"native output");
    }

    #[test]
    fn runner_transport_forwards_one_outer_terminal_query_answer_without_parser_duplicates() {
        let mut transport = transport();
        let mut pty = Pty::new(b"\x1b[6n", 1);
        let mut human = Bytes::new(b"\x1b[1;1R", 1);
        let mut display = Output::default();
        let mut observer = Observer::default();
        for _ in 0..64 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        assert_eq!(pty.input, b"\x1b[1;1R");
        assert_eq!(display.bytes, b"\x1b[6n");
        assert_eq!(
            transport.human_generation(),
            0,
            "a cursor-position report is not manual input"
        );
    }

    #[test]
    fn runner_transport_forwards_a_terminal_reply_burst_unchanged_without_manual_generation() {
        let mut transport = service_transport();
        let burst = b"\x1b[I\x1b]11;rgb:0000/0000/0000\x07\x1b[24;80R\x1b]10;rgb:ffff/ffff/ffff\x1b\\\x1b[O"
            .to_vec();
        let mut human = Bytes::new(&burst, 5);
        let mut pty = Pty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            pty.input, burst,
            "replies are forwarded byte-for-byte in order"
        );
        assert_eq!(
            progress.human_bytes,
            burst.len(),
            "pacing counts bytes read"
        );
        assert_eq!(transport.pending_input_bytes(), 0);
        assert_eq!(transport.human_generation(), 0);
    }

    #[test]
    fn runner_transport_reply_split_across_ticks_stays_out_of_manual_generation() {
        let mut transport = service_transport();
        let mut human = Bytes::new(b"\x1b[24", 16);
        let mut pty = Pty::new(b"", 1);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, b"\x1b[24", "the prefix is forwarded, not held");
        assert_eq!(transport.human_generation(), 0);
        human.chunks.push_back(b";80R".to_vec());
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, b"\x1b[24;80R");
        assert_eq!(transport.human_generation(), 0);
    }

    #[test]
    fn runner_transport_counts_only_manual_bytes_around_a_reply() {
        let mut transport = service_transport();
        let input = b"ab\x1b[Icd".to_vec();
        let mut human = Bytes::new(&input, 16);
        let mut pty = Pty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, input);
        assert_eq!(progress.human_bytes, input.len());
        assert_eq!(transport.human_generation(), 4);
    }

    #[test]
    fn runner_transport_keystrokes_still_advance_manual_generation() {
        let mut transport = service_transport();
        let input = b"yes\r\x1b[A\x03".to_vec();
        let mut human = Bytes::new(&input, 1);
        let mut pty = Pty::new(b"", 1);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, input);
        assert_eq!(
            transport.human_generation(),
            u64::try_from(input.len()).unwrap()
        );
    }

    #[test]
    fn runner_transport_outer_eof_counts_a_pending_reply_prefix_as_manual() {
        let mut transport = service_transport();
        let mut human = Bytes::new(b"ab\x1b[", 16);
        human.eof = true;
        let mut pty = Pty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert!(progress.outer_eof);
        assert_eq!(pty.input, b"ab\x1b[");
        assert_eq!(
            transport.human_generation(),
            4,
            "outer EOF resolves the pending prefix as manual before EOF is reported"
        );
    }

    #[test]
    fn runner_transport_outer_eof_after_a_complete_reply_adds_no_manual_generation() {
        let mut transport = service_transport();
        let mut human = Bytes::new(b"\x1b[I", 16);
        human.eof = true;
        let mut pty = Pty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert!(progress.outer_eof);
        assert_eq!(transport.human_generation(), 0);
    }

    #[test]
    fn runner_transport_lone_escape_counts_as_manual_once_its_prefix_bound_passes() {
        let mut transport = service_transport();
        let mut human = Bytes::new(b"\x1b", 16);
        let mut pty = Pty::new(b"", 1);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.input, b"\x1b", "a lone Escape is forwarded immediately");
        assert_eq!(transport.human_generation(), 0);
        transport.backdate_reply_prefix(REPLY_PREFIX_BOUND);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(progress.human_bytes, 0);
        assert_eq!(transport.human_generation(), 1);
        assert_eq!(pty.input, b"\x1b", "classification never synthesizes bytes");
    }

    #[test]
    fn runner_transport_resize_reaches_the_pty_and_screen_together() {
        let mut transport = transport();
        let mut pty = Pty::new(b"", 1);
        transport
            .resize(
                &mut pty,
                TerminalSize {
                    rows: 40,
                    cols: 100,
                },
            )
            .unwrap();
        assert_eq!(
            pty.sizes,
            [TerminalSize {
                rows: 40,
                cols: 100
            }]
        );
        assert_eq!(
            (transport.snapshot().rows, transport.snapshot().cols),
            (40, 100)
        );
    }

    #[test]
    fn runner_transport_reports_outer_and_child_eof_as_distinct_observations() {
        for outer in [false, true] {
            let mut transport = transport();
            let mut pty = Pty::new(b"", 1);
            pty.output.eof = !outer;
            let mut human = Bytes::new(b"", 1);
            human.eof = outer;
            let progress = transport
                .tick(
                    &mut human,
                    &mut pty,
                    &mut Output::default(),
                    &mut Observer::default(),
                )
                .unwrap();
            assert_eq!(progress.outer_eof, outer);
            assert_eq!(progress.child_eof, !outer);
        }
    }

    #[test]
    fn runner_transport_broken_display_fails_and_tail_discard_counts_only_pending_bytes() {
        let mut transport = transport();
        let mut pty = Pty::new(b"undelivered tail", 16);
        let mut human = Bytes::new(b"", 1);
        let mut display = Output {
            blocked: true,
            ..Default::default()
        };
        let mut observer = Observer::default();
        for _ in 0..4 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        let pending = transport.pending_display_bytes();
        assert!(pending > 0);
        assert_eq!(transport.discard_display_tail(), pending);
        assert_eq!(transport.discard_display_tail(), 0);
        pty.output = Bytes::new(b"broken", 6);
        display.blocked = false;
        display.broken = true;
        assert!(
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .is_err()
        );
    }

    #[test]
    fn runner_transport_requires_positive_finite_limits() {
        for field in ["input", "display", "tick"] {
            let mut limits = TransportLimits {
                input_bytes: 32,
                display_bytes: 64,
                bytes_per_tick: 16,
            };
            match field {
                "input" => limits.input_bytes = 0,
                "display" => limits.display_bytes = 0,
                "tick" => limits.bytes_per_tick = 0,
                _ => unreachable!(),
            }
            assert!(
                Transport::new(TerminalSize { rows: 24, cols: 80 }, limits).is_err(),
                "{field}"
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::error::CafleetError;
    use crate::runtime::transport::*;
    #[derive(Default)]
    pub(crate) struct Observer {
        pub(crate) screens: Vec<(u64, String)>,
        pub(crate) fail: bool,
        pub(crate) ordering: Option<Rc<RefCell<Vec<&'static str>>>>,
    }

    impl ScreenObserver for Observer {
        fn observe(&mut self, screen: &ScreenSnapshot) -> std::result::Result<(), CafleetError> {
            if let Some(ordering) = &self.ordering {
                ordering.borrow_mut().push("observe");
            }
            self.screens
                .push((screen.screen_seq, screen.screen.contents()));
            if self.fail {
                Err(CafleetError::App(
                    "fixture observation queue unavailable".into(),
                ))
            } else {
                Ok(())
            }
        }
    }
}
