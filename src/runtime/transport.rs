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
    Quiescent,
}

pub(super) struct DrainedInput {
    instance: Arc<()>,
    turn: u64,
    human_generation: u64,
    pointer_generation: u64,
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
    fn input_health(&self) -> ObservationHealth;
    fn parser_failure(&mut self) -> Result<()>;
}
#[derive(Default)]
pub(crate) struct TransportProgress {
    pub(crate) child_eof: bool,
    pub(crate) outer_eof: bool,
    pub(crate) human_bytes: usize,
    pub(crate) forwarded_bytes: usize,
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
    pointer_generation: u64,
    replies: ReplyClassifier,
    input_turn: u64,
    input_instance: Arc<()>,
    input_drain: Option<DrainedInput>,
    drain_blocker: Option<String>,
    observation_fenced: bool,
    observation_remainder: Option<Vec<u8>>,
    clock: Arc<dyn crate::time::Clock>,
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
    pub(crate) fn new(
        size: TerminalSize,
        limits: TransportLimits,
        clock: Arc<dyn crate::time::Clock>,
    ) -> Result<Self> {
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
            pointer_generation: 0,
            replies: ReplyClassifier::default(),
            input_turn: 0,
            input_instance: Arc::new(()),
            input_drain: None,
            drain_blocker: None,
            observation_fenced: false,
            observation_remainder: None,
            clock,
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
        if let Some(remainder) = self.observation_remainder.take() {
            self.observe_tick_child(&remainder, observer)?;
        }
        if let Some(bytes) = self.read_child(pty)? {
            self.child_eof = bytes.is_empty();
            self.display.extend(bytes.iter().copied());
            #[cfg(test)]
            if !bytes.is_empty()
                && let Some(hooks) = &self.observation_hooks
                && let Err(cause) =
                    hooks.before(self.observation_phase, ObservationOperation::Parser)
            {
                self.observation_fenced = true;
                observer.parser_failure()?;
                return Err(cause);
            }
            if !bytes.is_empty() {
                self.observe_tick_child(&bytes, observer)?;
            }
        }
        self.flush_display(display)?;
        progress.outer_eof = self.outer_eof;
        progress.child_eof = self.child_eof;
        Ok(progress)
    }
    fn observe_tick_child(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> Result<()> {
        match self.observe_child_bytes(bytes, observer) {
            Ok(()) => Ok(()),
            Err(TerminalObservationError::Parser(cause)) => {
                observer.parser_failure()?;
                Err(cause)
            }
            Err(TerminalObservationError::Observer { cause, .. }) => Err(cause),
        }
    }
    /// An observer failure leaves the unconsumed suffix of `bytes` as the observation
    /// remainder, which the next tick or drain feeds before any new child read; an empty
    /// remainder re-observes the current screen, so observation completes only after a
    /// successful observer callback. A parser failure has no resumable boundary and fences
    /// observation for the transport lifetime.
    fn observe_child_bytes(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> std::result::Result<(), TerminalObservationError> {
        let observed = if bytes.is_empty() {
            observer
                .observe(&self.terminal.snapshot())
                .map_err(|cause| TerminalObservationError::Observer { cause, consumed: 0 })
        } else {
            self.terminal.feed_observed_classified(bytes, observer)
        };
        match &observed {
            Ok(()) => {}
            Err(TerminalObservationError::Observer { consumed, .. }) => {
                self.retain_observation_remainder(&bytes[*consumed..]);
            }
            Err(TerminalObservationError::Parser(_)) => self.observation_fenced = true,
        }
        observed
    }
    fn retain_observation_remainder(&mut self, unobserved: &[u8]) {
        assert!(
            self.observation_remainder.is_none(),
            "an observation remainder is fed before new child output"
        );
        self.observation_remainder = Some(unobserved.to_vec());
    }
    fn observation_complete(&self) -> bool {
        !self.observation_fenced && self.observation_remainder.is_none()
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
        flush_nonblocking(display)?;
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
        let classified = self.replies.feed(&bytes, self.clock.now());
        self.count_input(classified);
        self.input.extend(bytes);
    }
    fn expire_reply_prefix(&mut self) {
        let classified = self.replies.expire(self.clock.now());
        self.count_input(classified);
    }
    fn close_outer(&mut self) {
        let classified = self.replies.finish();
        self.count_input(classified);
        self.outer_eof = true;
    }
    fn count_input(&mut self, classified: Classified) {
        self.human_generation = self
            .human_generation
            .checked_add(u64::try_from(classified.manual).expect("manual byte count fits u64"))
            .expect("human input generation fits u64");
        self.pointer_generation = self
            .pointer_generation
            .checked_add(u64::try_from(classified.pointer).expect("pointer byte count fits u64"))
            .expect("pointer input generation fits u64");
    }
    pub(super) fn human_generation(&self) -> u64 {
        self.human_generation
    }
    pub(super) fn pointer_generation(&self) -> u64 {
        self.pointer_generation
    }
    #[cfg(test)]
    pub(super) fn backdate_reply_prefix(&mut self, by: std::time::Duration) {
        self.replies.backdate(by);
    }
    pub(super) fn take_input_drain(&mut self) -> Option<DrainedInput> {
        self.input_drain.take()
    }
    /// The human generation and output sequence of the retained drain proof.
    #[cfg(test)]
    pub(super) fn drained(&self) -> Option<(u64, u64)> {
        self.input_drain
            .as_ref()
            .map(|drain| (drain.human_generation, drain.output_seq))
    }
    pub(super) fn drain_blocker(&self) -> Option<&str> {
        self.drain_blocker.as_deref()
    }
    pub(super) fn validate_input_drain(
        &self,
        proof: &DrainedInput,
        deadline: Instant,
    ) -> Result<()> {
        self.validate_ordinary(proof, deadline)?;
        ensure(
            proof.health.eligibility == InputEligibility::ComposerEmpty
                && proof.health.visibility == Visibility::NotVisible,
            "input drain is not eligible for composer input",
        )
    }

    pub(super) fn validate_ordinary(&self, proof: &DrainedInput, deadline: Instant) -> Result<()> {
        self.validate_observation_drain(proof, deadline)
    }

    pub(super) fn validate_permission(
        &self,
        proof: &DrainedInput,
        deadline: Instant,
    ) -> Result<()> {
        self.validate_ordinary(proof, deadline)?;
        ensure(
            self.pointer_generation == proof.pointer_generation,
            "input drain pointer binding changed",
        )
    }

    pub(super) fn validate_observation_drain(
        &self,
        proof: &DrainedInput,
        deadline: Instant,
    ) -> Result<()> {
        check_deadline(self.clock.now(), deadline)?;
        ensure(
            Arc::ptr_eq(&self.input_instance, &proof.instance)
                && self.input_turn == proof.turn
                && self.human_generation == proof.human_generation
                && self.terminal.snapshot().screen_seq == proof.output_seq,
            "input drain binding changed",
        )?;
        ensure(
            self.observation_complete()
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
        check_deadline(self.clock.now(), effective)?;
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
        check_deadline(self.clock.now(), effective)?;
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
    pub(crate) fn bracketed_paste(&self) -> bool {
        self.terminal.parser.screen().bracketed_paste()
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
            self.observation_fenced = true;
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
    } else {
        None
    }
}

struct DeadlineObserver<'a, O> {
    inner: &'a mut O,
    deadline: Instant,
    clock: &'a dyn crate::time::Clock,
}
impl<O: ScreenObserver> ScreenObserver for DeadlineObserver<'_, O> {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
        check_deadline(self.clock.now(), self.deadline)?;
        self.inner.observe(screen)?;
        check_deadline(self.clock.now(), self.deadline)
    }

    fn input_health(&self) -> ObservationHealth {
        self.inner.input_health()
    }

    fn parser_failure(&mut self) -> Result<()> {
        check_deadline(self.clock.now(), self.deadline)?;
        self.inner.parser_failure()?;
        check_deadline(self.clock.now(), self.deadline)
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
        check_deadline(self.clock.now(), deadline)?;
        let mut remaining = DrainBudget {
            human: self.limits.bytes_per_tick,
            child: self.limits.bytes_per_tick,
            manual: self.limits.bytes_per_tick,
            display: self.limits.bytes_per_tick,
        };
        for _ in 0..ROUND_LIMIT {
            check_deadline(self.clock.now(), deadline)?;
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
            check_deadline(self.clock.now(), deadline)?;
            if remaining.manual > 0 && !self.input.is_empty() {
                let written = write_pending(&mut self.input, remaining.manual, |bytes| {
                    pty.try_write(bytes)
                })?;
                remaining.manual -= written;
                check_deadline(self.clock.now(), deadline)?;
            }
            if let Some(remainder) = self.observation_remainder.take() {
                remaining.child = remaining
                    .child
                    .checked_sub(remainder.len())
                    .expect("an observation remainder fits a fresh drain's child quota");
                self.observe_drain_child(&remainder, observer, deadline)?;
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
            check_deadline(self.clock.now(), deadline)?;
            if remaining.display > 0 && !self.display.is_empty() {
                let written = write_pending(&mut self.display, remaining.display, |bytes| {
                    display.try_write(bytes)
                })?;
                remaining.display -= written;
                check_deadline(self.clock.now(), deadline)?;
            }
            flush_nonblocking(display)?;
            check_deadline(self.clock.now(), deadline)?;
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
                if !self.observation_complete() {
                    return Ok(self.pending("observation incomplete".into()));
                }
                let health = observer.input_health();
                check_deadline(self.clock.now(), deadline)?;
                if let Some(reason) = unsettled_health(&health) {
                    return Ok(self.pending(reason));
                }
                let output_seq = self.terminal.snapshot().screen_seq;
                check_deadline(self.clock.now(), deadline)?;
                self.input_drain = Some(DrainedInput {
                    instance: self.input_instance.clone(),
                    turn: self.input_turn,
                    human_generation: self.human_generation,
                    pointer_generation: self.pointer_generation,
                    output_seq,
                    health,
                });
                return Ok(InputDrain::Quiescent);
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
        if let Err(cause) = check_deadline(self.clock.now(), deadline) {
            self.retain_observation_remainder(bytes);
            return Err(cause);
        }
        #[cfg(test)]
        if let Some(hooks) = &self.observation_hooks {
            if let Err(cause) = hooks.before(self.observation_phase, ObservationOperation::Parser) {
                self.observation_fenced = true;
                return Err(cause);
            }
            if let Err(cause) = check_deadline(self.clock.now(), deadline) {
                self.retain_observation_remainder(bytes);
                return Err(cause);
            }
        }
        let clock = self.clock.clone();
        let mut bounded = DeadlineObserver {
            inner: observer,
            deadline,
            clock: clock.as_ref(),
        };
        match self.observe_child_bytes(bytes, &mut bounded) {
            Ok(()) => check_deadline(self.clock.now(), deadline),
            Err(TerminalObservationError::Observer { cause, .. }) => Err(cause),
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
    X10 {
        remaining: u8,
    },
    StringBody {
        kind: StringKind,
        length: usize,
    },
    StringEscape,
}

fn flush_nonblocking(display: &mut impl Display) -> Result<()> {
    match display.try_flush() {
        Err(cause) if cause.kind() != ErrorKind::WouldBlock => Err(io_error(cause)),
        _ => Ok(()),
    }
}

#[derive(Default)]
pub(super) struct ReplyClassifier {
    state: State,
    pending: usize,
    last_byte: Option<Instant>,
    paste_end_matched: Option<usize>,
}

#[derive(Default)]
pub(super) struct Classified {
    pub(super) manual: usize,
    pub(super) pointer: usize,
}

impl Classified {
    fn manual(count: usize) -> Self {
        Self {
            manual: count,
            pointer: 0,
        }
    }

    fn pointer(count: usize) -> Self {
        Self {
            manual: 0,
            pointer: count,
        }
    }
}

impl ReplyClassifier {
    pub(super) fn feed(&mut self, bytes: &[u8], now: Instant) -> Classified {
        let mut total = Classified::default();
        for &byte in bytes {
            let classified = self.classify(byte, now);
            total.manual += classified.manual;
            total.pointer += classified.pointer;
        }
        total
    }

    pub(super) fn expire(&mut self, now: Instant) -> Classified {
        let manual = match self.last_byte {
            Some(last) if now.saturating_duration_since(last) >= REPLY_PREFIX_BOUND => {
                self.resolve_manual()
            }
            _ => 0,
        };
        Classified::manual(manual)
    }

    pub(super) fn finish(&mut self) -> Classified {
        Classified::manual(self.resolve_manual())
    }

    pub(super) fn pending(&self) -> bool {
        self.pending > 0
    }

    fn classify(&mut self, byte: u8, now: Instant) -> Classified {
        let classified = match self.paste_end_matched {
            Some(matched) => {
                self.advance_paste(matched, byte);
                Classified::manual(1)
            }
            None => self.step(byte),
        };
        self.last_byte = self.pending().then_some(now);
        classified
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

    fn step(&mut self, byte: u8) -> Classified {
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
            State::X10 { remaining } => match byte {
                0x20..=0xff if remaining == 1 => {
                    Classified::pointer(std::mem::take(&mut self.pending) + 1)
                }
                0x20..=0xff => self.extend(State::X10 {
                    remaining: remaining.checked_sub(1).expect("X10 payload remains"),
                }),
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

    fn ground(&mut self, byte: u8) -> Classified {
        if byte == ESC {
            self.state = State::Escape;
            self.pending = 1;
            Classified::default()
        } else {
            Classified::manual(1)
        }
    }

    fn open_string(&mut self, kind: StringKind) -> Classified {
        self.extend(State::StringBody { kind, length: 0 })
    }

    fn extend(&mut self, state: State) -> Classified {
        self.state = state;
        self.pending += 1;
        Classified::default()
    }

    fn invalid(&mut self, byte: u8) -> Classified {
        let manual = self.resolve_manual();
        let mut classified = self.ground(byte);
        classified.manual += manual;
        classified
    }

    fn resolve_manual(&mut self) -> usize {
        self.state = State::Ground;
        self.last_byte = None;
        std::mem::take(&mut self.pending)
    }

    fn complete_reply(&mut self) -> Classified {
        self.pending = 0;
        Classified::default()
    }

    fn complete_csi(&mut self, parameters: &[u8], final_byte: u8) -> Classified {
        if parameters.is_empty() && final_byte == b'M' {
            return self.extend(State::X10 { remaining: 3 });
        }
        let length = std::mem::take(&mut self.pending) + 1;
        if matches!(final_byte, b'M' | b'm')
            && parameters
                .strip_prefix(b"<")
                .is_some_and(|list| parameter_count(list) == Some(3))
        {
            return Classified::pointer(length);
        }
        if is_csi_reply(parameters, final_byte) {
            return Classified::default();
        }
        if parameters == b"200" && final_byte == b'~' {
            self.paste_end_matched = Some(0);
        }
        Classified::manual(length)
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
    /// `(row, col)`; `None` for a capture whose source exposes no cursor.
    pub(crate) cursor: Option<(u16, u16)>,
    /// `None` for a capture whose source exposes no screen mode.
    pub(crate) alternate_screen: Option<bool>,
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
    Observer {
        cause: CafleetError,
        consumed: usize,
    },
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
    fn advance_sequence(&mut self, by: u64) -> Result<()> {
        self.sequence = self
            .sequence
            .checked_add(by)
            .ok_or_else(|| error("terminal sequence exhausted"))?;
        Ok(())
    }
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.advance_sequence(bytes.len() as u64)?;
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
                | TerminalObservationError::Observer { cause, .. } => cause,
            })
    }
    pub(crate) fn feed_observed_classified(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> std::result::Result<(), TerminalObservationError> {
        let mut start = 0;
        for (index, byte) in bytes.iter().enumerate() {
            if byte.is_ascii_control() {
                if index > start {
                    self.step(bytes, start..index, observer)?;
                }
                self.step(bytes, index..index + 1, observer)?;
                start = index + 1;
            }
        }
        if start < bytes.len() {
            self.step(bytes, start..bytes.len(), observer)?;
        }
        Ok(())
    }
    /// An observer failure reports the step's end as consumed: its bytes reached the parser.
    fn step(
        &mut self,
        bytes: &[u8],
        range: std::ops::Range<usize>,
        observer: &mut impl ScreenObserver,
    ) -> std::result::Result<(), TerminalObservationError> {
        let consumed = range.end;
        self.feed(&bytes[range])
            .map_err(TerminalObservationError::Parser)?;
        observer
            .observe(&self.snapshot())
            .map_err(|cause| TerminalObservationError::Observer { cause, consumed })
    }
    pub(crate) fn resize(&mut self, size: TerminalSize) -> Result<()> {
        size.validate()?;
        self.advance_sequence(1)?;
        self.parser.set_size(
            size.rows.try_into().expect("validated rows"),
            size.cols.try_into().expect("validated cols"),
        );
        Ok(())
    }
    /// A fresh terminal of the capture's size with each captured row written at its row
    /// position, carrying exactly the cursor and screen mode the capture reports.
    pub(crate) fn replay(capture: &crate::multiplexer::ScreenCapture) -> Result<ScreenSnapshot> {
        let mut terminal = Self::new(TerminalSize {
            rows: capture.rows,
            cols: capture.cols,
        })?;
        let mut bytes = Vec::new();
        if capture.alternate_screen == Some(true) {
            bytes.extend_from_slice(b"\x1b[?1049h");
        }
        let body = capture.body.strip_suffix('\n').unwrap_or(&capture.body);
        for (row, line) in body.split('\n').take(usize::from(capture.rows)).enumerate() {
            bytes.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
            bytes.extend_from_slice(line.strip_suffix('\r').unwrap_or(line).as_bytes());
            bytes.extend_from_slice(b"\x1b[0m");
        }
        if let Some((row, col)) = capture.cursor {
            bytes.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
        }
        terminal.feed(&bytes)?;
        Ok(ScreenSnapshot {
            cursor: capture.cursor,
            alternate_screen: capture.alternate_screen,
            ..terminal.snapshot()
        })
    }

    pub(crate) fn snapshot(&self) -> ScreenSnapshot {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        ScreenSnapshot {
            #[cfg(test)]
            title: self.parser.callbacks().title.clone(),
            rows: rows.get(),
            cols: cols.get(),
            cursor: Some(screen.cursor_position()),
            alternate_screen: Some(screen.alternate_screen()),
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
    use crate::runtime::transport::test_support::FakeDisplay;
    use crate::runtime::transport::test_support::FakeInput;
    use crate::runtime::transport::test_support::FakePty;
    use crate::runtime::transport::test_support::Observer;
    use crate::runtime::transport::*;

    #[test]
    fn runner_transport_bracketed_paste_selection_follows_the_child_mode_2004() {
        let mut transport = transport();
        let mut pty = FakePty::new(b"", 1);
        assert!(!transport.bracketed_paste());
        pty.output.chunks.push_back(b"\x1b[?2004h".to_vec());
        one_tick(&mut transport, &mut FakeInput::default(), &mut pty);
        assert!(transport.bracketed_paste());
        pty.output.chunks.push_back(b"\x1b[?2004l".to_vec());
        one_tick(&mut transport, &mut FakeInput::default(), &mut pty);
        assert!(!transport.bracketed_paste());
    }

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
            assert_eq!(actual.cursor, expected.cursor);
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
        assert_eq!(alternate.alternate_screen, Some(true));
        assert!(alternate.screen.contents().contains("alternate request"));
        assert!(!alternate.screen.contents().contains("primary composer"));
        assert_eq!(alternate.title, "Action Required");
        terminal.feed(b"\x1b[?1049l").unwrap();
        let restored = terminal.snapshot();
        assert_eq!(restored.alternate_screen, Some(false));
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
        assert_eq!(before.cursor, Some((3, 12)));
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

    mod capture_replay {
        use crate::multiplexer::ScreenCapture;
        use crate::runtime::transport::*;

        fn capture(
            (rows, cols): (u16, u16),
            cursor: Option<(u16, u16)>,
            alternate_screen: Option<bool>,
            body: &str,
        ) -> ScreenCapture {
            ScreenCapture {
                rows,
                cols,
                cursor,
                alternate_screen,
                body: body.to_owned(),
            }
        }

        fn rows_of(snapshot: &ScreenSnapshot) -> Vec<String> {
            snapshot.screen.rows(0, snapshot.cols).collect()
        }

        #[test]
        fn a_tmux_capture_replays_every_row_at_its_position_with_its_cursor_and_screen_mode() {
            for alternate in [false, true] {
                let snapshot = TerminalState::replay(&capture(
                    (4, 20),
                    Some((3, 8)),
                    Some(alternate),
                    "row one\nrow two\n\nlast row\n",
                ))
                .unwrap();
                assert_eq!((snapshot.rows, snapshot.cols), (4, 20), "{alternate}");
                assert_eq!(
                    rows_of(&snapshot),
                    ["row one", "row two", "", "last row"],
                    "{alternate}"
                );
                assert_eq!(snapshot.cursor, Some((3, 8)), "{alternate}");
                assert_eq!(snapshot.alternate_screen, Some(alternate));
                assert_eq!(
                    snapshot.screen.alternate_screen(),
                    alternate,
                    "the replay enters the alternate screen exactly when the capture is on it"
                );
            }
        }

        #[test]
        fn a_full_width_row_leaves_the_next_row_at_its_own_position() {
            let full = "0123456789";
            let snapshot = TerminalState::replay(&capture(
                (3, 10),
                Some((2, 0)),
                Some(false),
                &format!("{full}\nnext\nlast\n"),
            ))
            .unwrap();
            assert_eq!(rows_of(&snapshot), [full, "next", "last"]);
        }

        #[test]
        fn a_herdr_capture_replays_crlf_styled_rows_without_cursor_or_screen_mode() {
            let snapshot = TerminalState::replay(&capture(
                (3, 30),
                None,
                None,
                "› \x1b[2mAsk anything\x1b[0m\r\n\r\nstatus line\r\n",
            ))
            .unwrap();
            assert_eq!((snapshot.rows, snapshot.cols), (3, 30));
            assert_eq!(rows_of(&snapshot), ["› Ask anything", "", "status line"]);
            assert!(
                snapshot.screen.cell(0, 2).unwrap().dim(),
                "the replay keeps the captured cell styles"
            );
            assert!(!snapshot.screen.cell(2, 0).unwrap().dim());
            assert_eq!(snapshot.cursor, None);
            assert_eq!(snapshot.alternate_screen, None);
            assert!(!snapshot.screen.alternate_screen());
        }

        #[test]
        fn every_runner_snapshot_carries_its_cursor_and_screen_mode() {
            let mut terminal = TerminalState::new(TerminalSize { rows: 5, cols: 20 }).unwrap();
            terminal.feed(b"\x1b[2;4Hx").unwrap();
            let primary = terminal.snapshot();
            assert_eq!(primary.cursor, Some((1, 4)));
            assert_eq!(primary.alternate_screen, Some(false));
            terminal.feed(b"\x1b[?1049h").unwrap();
            assert_eq!(terminal.snapshot().alternate_screen, Some(true));
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

    use crate::coding_agent::ProfileCatalog;

    use crate::runtime::observation::test_support::controller::fixture_clock;
    use crate::runtime::transport::REPLY_PREFIX_BOUND;
    use crate::runtime::writer::test_support::Hooks;
    use crate::runtime::writer::test_support::Release;
    use crate::runtime::writer::test_support::WriterFixture;
    use crate::runtime::writer::test_support::committed;
    use crate::runtime::writer::test_support::start;
    use crate::runtime::writer::test_support::transition;
    use crate::time::Clock as _;
    use crate::time::test_support::TestClock;
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

    /// The harness's endpoint call trace and the virtual clock every fake endpoint reads.
    struct Recorder {
        sites: RefCell<Vec<Site>>,
        clock: Arc<TestClock>,
    }

    impl std::ops::Deref for Recorder {
        type Target = RefCell<Vec<Site>>;

        fn deref(&self) -> &Self::Target {
            &self.sites
        }
    }

    impl Recorder {
        /// Advances the virtual clock just past `deadline`, as a call blocking that long would.
        fn block_past(&self, deadline: Instant) {
            self.clock.advance(
                deadline.saturating_duration_since(self.clock.now()) + Duration::from_millis(1),
            );
        }
    }

    type Trace = Rc<Recorder>;

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
            let entered = self.trace.clock.now();
            self.trace.borrow_mut().push(self.site);
            if let Some(until) = self.delay_until.take() {
                self.trace.block_past(until);
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
                returned: self.trace.clock.now(),
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

        fn parser_failure(&mut self) -> Result<()> {
            self.inner.parser_failure()
        }

        fn input_health(&self) -> ObservationHealth {
            self.trace.borrow_mut().push(Site::Health);
            self.facts
                .health_reads
                .set(self.facts.health_reads.get() + 1);
            let health = self.inner.input_health();
            self.facts.last_health.replace(Some(health.clone()));
            health
        }
    }

    struct Harness {
        fixture: WriterFixture,
        writer: EventWriter,
        controller: ObservationController,
        clock: TestClock,
        hooks: Arc<Hooks>,
        release: Option<Release>,
        until: Instant,
        trace: Trace,
        observations: Rc<Observations>,
    }

    impl Harness {
        fn new() -> Self {
            let time = Arc::new(TestClock::starting_at(Instant::now()));
            let until = time.now() + Duration::from_secs(4);
            let fixture = WriterFixture::new();
            let controller =
                ObservationController::new(&fixture.fixture.run, ProfileCatalog::bundled())
                    .unwrap();
            let hooks = Arc::new(Hooks::default());
            let release = Some(Release(hooks.clone()));
            let mut writer = start(&fixture, hooks.clone());
            writer.set_deadline(until);
            Self {
                fixture,
                writer,
                controller,
                clock: fixture_clock(),
                hooks,
                release,
                until,
                trace: Rc::new(Recorder {
                    sites: RefCell::default(),
                    clock: time,
                }),
                observations: Rc::default(),
            }
        }

        fn time(&self) -> &Arc<TestClock> {
            &self.trace.clock
        }

        /// A transport with these limits on the harness's virtual clock.
        fn transport(&self, bytes: usize, input: usize, display: usize) -> Transport {
            Transport::new(
                TerminalSize { rows: 24, cols: 80 },
                TransportLimits {
                    input_bytes: input,
                    display_bytes: display,
                    bytes_per_tick: bytes,
                },
                self.time().clone(),
            )
            .unwrap()
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

        fn drain_with(
            &mut self,
            transport: &mut Transport,
            io: &mut Endpoints,
        ) -> Result<InputDrain> {
            let until = self.until;
            self.drain(transport, &mut io.human, &mut io.pty, &mut io.output, until)
        }

        fn tick_with(
            &mut self,
            transport: &mut Transport,
            io: &mut Endpoints,
        ) -> Result<TransportProgress> {
            transport.tick(
                &mut io.human,
                &mut io.pty,
                &mut io.output,
                &mut self.controller.observer(&self.writer, &self.clock),
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
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(
                drain_transport.drained(),
                Some((
                    drain_transport.human_generation(),
                    drain_transport.snapshot().screen_seq
                ))
            );
            assert_eq!(human.calls.len(), 1);
            assert_eq!(pty.reader.calls.len(), 1);
        }

        fn control(&mut self) {
            let mut drain_transport = self.transport(64, 64, 64);
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

    struct Endpoints {
        human: Reader,
        pty: DrainPty,
        output: DrainOutput,
    }

    impl Endpoints {
        fn new(trace: &Trace, human: Reader) -> Self {
            Self {
                human,
                pty: DrainPty::new(trace),
                output: DrainOutput::new(trace),
            }
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
            let mut drain_transport = h.transport(32, 32, 32);
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
            let human_bytes = "literal 世界\r".as_bytes();
            let child_bytes = b"child\r\n";
            let mut drain_transport = h.transport(64, 64, 64);
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
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(
                drain_transport.drained(),
                Some((
                    u64::try_from(human_bytes.len()).unwrap(),
                    u64::try_from(child_bytes.len()).unwrap()
                ))
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
        for (step, on_human) in [
            (Read::Interrupted, true),
            (Read::Interrupted, false),
            (Read::Eof, true),
            (Read::Eof, false),
        ] {
            with_harness(|h| {
                let mut drain_transport = h.transport(32, 32, 32);
                let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
                let interrupted = matches!(step, Read::Interrupted);
                if on_human {
                    io.human.steps.push_back(step);
                } else {
                    io.pty.reader.steps.push_back(step);
                }
                let result = h.drain_with(&mut drain_transport, &mut io);
                assert_pending(result, &mut drain_transport);
                assert_eq!(drain_transport.human_generation(), 0);
                assert_eq!(h.observations.health_reads.get(), 0);
                assert!(io.human.calls.len() <= 1 && io.pty.reader.calls.len() <= 1);
                assert_bounds(&io.human, &io.pty, &io.output, 32);
                if interrupted {
                    h.seal(&mut drain_transport);
                    assert!(drain_transport.take_input_drain().is_some());
                }
            });
        }
    }

    #[test]
    fn drain_round_and_separate_byte_quotas_bound_continuously_readable_sources() {
        for (bytes, width, expected_reads, expected_bytes) in [(64, 1, 8, 8), (4, usize::MAX, 1, 4)]
        {
            with_harness(|h| {
                h.control();
                let mut drain_transport = h.transport(bytes, 128, 128);
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
                    InputDrain::Quiescent
                ));
            });
        }
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(64, 64, 64);
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
            assert_eq!(result, InputDrain::Quiescent);
            assert_eq!(drain_transport.drained().map(|(human, _)| human), Some(7));
            assert_eq!(human.calls.len(), 8);
            assert_eq!(h.observations.health_reads.get(), 1);
        });
    }

    #[test]
    fn drain_full_manual_queue_supplies_no_read_proof_and_preserves_order_on_release() {
        with_harness(|h| {
            let mut drain_transport = h.transport(16, 4, 32);
            let mut io = Endpoints::new(
                &h.trace,
                Reader::data(Site::Human, &h.trace, b"abcdefgh", 8),
            );
            io.pty.write_error = Some(ErrorKind::WouldBlock);
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_pending(result, &mut drain_transport);
            assert_eq!(io.human.calls.len(), 1);
            assert_eq!(io.human.calls[0].capacity, 4);
            assert_eq!(drain_transport.pending_input_bytes(), 4);
            assert_eq!(drain_transport.human_generation(), 4);
            assert!(io.pty.bytes.is_empty());
            assert!(io.pty.offered.len() <= 8);
            io.pty.write_error = None;
            assert_eq!(
                h.drain_with(&mut drain_transport, &mut io).unwrap(),
                InputDrain::Quiescent
            );
            assert_eq!(drain_transport.drained().map(|(human, _)| human), Some(8));
            assert_eq!(io.pty.bytes, b"abcdefgh");
            assert_eq!(drain_transport.pending_input_bytes(), 0);
        });
    }

    #[test]
    fn drain_display_and_prefetched_child_pressure_require_real_later_drain() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(16, 32, 4);
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
                InputDrain::Quiescent
            ));
            assert_eq!(output.bytes, b"abcdefgh");
        });
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(16, 32, 32);
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
                InputDrain::Quiescent
            ));
            assert_eq!(output.bytes, b"firstrest");
            assert_eq!(drain_transport.snapshot().screen_seq, 9);
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
            let mut drain_transport = h.transport(32, 32, 32);
            let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
            let result = h.drain_with(&mut drain_transport, &mut io);
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
            h.hooks.pause(WriterBoundary::Persist);
            h.writer.submit(transition(&h.fixture, 1)).unwrap();
            h.hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(h.writer.status().pending_count, 1);
            let mut drain_transport = h.transport(32, 32, 32);
            let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
            let result = h.drain_with(&mut drain_transport, &mut io);
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
            crate::runtime::writer::test_support::yield_until(|| {
                h.writer.status().pending_count == 0
            });
            assert_eq!(committed(&h.fixture), 1);
            h.seal(&mut drain_transport);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_tick_and_drain_keep_actual_human_counts_before_later_errors() {
        for quiesce in [false, true] {
            for failure in ["pty", "child", "display", "flush", "observer", "zero"] {
                with_harness(|h| {
                    let mut drain_transport = h.transport(32, 32, 32);
                    h.seal(&mut drain_transport);
                    let mut io = Endpoints::new(
                        &h.trace,
                        Reader::data(Site::Human, &h.trace, "世x".as_bytes(), 4),
                    );
                    io.pty.reader = Reader::data(Site::Child, &h.trace, b"output", 6);
                    let expected = match failure {
                        "pty" => {
                            io.pty.write_error = Some(ErrorKind::BrokenPipe);
                            "fixture PTY write failure"
                        }
                        "child" => {
                            io.pty.reader.steps = [Read::Error(ErrorKind::BrokenPipe)].into();
                            "fixture read failure"
                        }
                        "display" => {
                            io.output.write_error = Some(ErrorKind::BrokenPipe);
                            "fixture display write failure"
                        }
                        "flush" => {
                            io.output.flush_error = Some(ErrorKind::BrokenPipe);
                            "fixture flush failure"
                        }
                        "observer" => {
                            h.clock.set_elapsed(Duration::from_millis(1));
                            h.controller
                                .observer(&h.writer, &h.clock)
                                .observe(&drain_transport.snapshot())
                                .unwrap();
                            h.clock.set_elapsed(Duration::from_millis(0));
                            "observation clock regressed"
                        }
                        "zero" => {
                            io.pty.max_write = 0;
                            "terminal write made no progress"
                        }
                        _ => unreachable!("fixed failure matrix"),
                    };
                    let result = if quiesce {
                        h.drain_with(&mut drain_transport, &mut io).map(|_| ())
                    } else {
                        h.tick_with(&mut drain_transport, &mut io).map(|_| ())
                    };
                    assert!(
                        result.unwrap_err().message().contains(expected),
                        "{failure}"
                    );
                    assert_eq!(io.human.bytes, "世x".as_bytes());
                    assert_eq!(drain_transport.human_generation(), 4);
                    assert!(drain_transport.take_input_drain().is_none());
                });
            }
        }
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
                let mut drain_transport = h.transport(32, 32, 32);
                h.seal(&mut drain_transport);
                let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
                match operation {
                    "tick" => {
                        h.tick_with(&mut drain_transport, &mut io).unwrap();
                    }
                    "tick-error" => {
                        io.human.steps.push_back(Read::Error(ErrorKind::BrokenPipe));
                        assert!(h.tick_with(&mut drain_transport, &mut io).is_err());
                    }
                    "resize" => {
                        drain_transport
                            .resize(&mut io.pty, TerminalSize { rows: 25, cols: 80 })
                            .unwrap();
                    }
                    "resize-invalid" => {
                        assert!(
                            drain_transport
                                .resize(&mut io.pty, TerminalSize { rows: 0, cols: 80 })
                                .is_err()
                        );
                    }
                    "resize-error" => {
                        io.pty.resize_error = true;
                        assert!(
                            drain_transport
                                .resize(&mut io.pty, TerminalSize { rows: 25, cols: 80 })
                                .is_err()
                        );
                    }
                    "prefetch" => {
                        assert_eq!(
                            drain_transport.prefetch_child_bytes(&mut io.pty).unwrap(),
                            0
                        );
                    }
                    "prefetch-data" => {
                        io.pty.reader.steps.push_back(Read::Data(b"tail".to_vec()));
                        assert_eq!(
                            drain_transport.prefetch_child_bytes(&mut io.pty).unwrap(),
                            4
                        );
                    }
                    "flush" => {
                        assert_eq!(drain_transport.flush_display(&mut io.output).unwrap(), 0);
                    }
                    "flush-error" => {
                        io.output.flush_error = Some(ErrorKind::BrokenPipe);
                        assert!(drain_transport.flush_display(&mut io.output).is_err());
                    }
                    "discard" => {
                        assert_eq!(drain_transport.discard_display_tail(), 0);
                    }
                    "drain-interrupted" => {
                        io.human.steps.push_back(Read::Interrupted);
                        assert_pending(
                            h.drain_with(&mut drain_transport, &mut io),
                            &mut drain_transport,
                        );
                    }
                    "drain-error" => {
                        io.human.steps.push_back(Read::Error(ErrorKind::BrokenPipe));
                        assert!(h.drain_with(&mut drain_transport, &mut io).is_err());
                    }
                    "drain-expired" => {
                        assert_deadline(
                            h.drain(
                                &mut drain_transport,
                                &mut io.human,
                                &mut io.pty,
                                &mut io.output,
                                h.time().now(),
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
            let mut drain_transport = h.transport(32, 32, 32);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
            assert_deadline(
                h.drain(
                    &mut drain_transport,
                    &mut io.human,
                    &mut io.pty,
                    &mut io.output,
                    h.time().now(),
                ),
                &mut drain_transport,
            );
            assert!(h.trace.borrow().is_empty());
            assert!(io.human.calls.is_empty() && io.pty.reader.calls.is_empty());
            assert!(io.pty.offered.is_empty() && io.output.offered.is_empty());
            assert_eq!(io.output.flushes, 0);
        });
        for fails in [false, true] {
            with_harness(|h| {
                let mut drain_transport = h.transport(32, 32, 32);
                let human = Reader::new(
                    Site::Human,
                    &h.trace,
                    [if fails {
                        Read::Error(ErrorKind::BrokenPipe)
                    } else {
                        Read::Data(b"read".to_vec())
                    }],
                );
                let mut io = Endpoints::new(&h.trace, human);
                let until = h.time().now() + Duration::from_millis(100);
                io.human.delay_until = Some(until);
                let result = h.drain(
                    &mut drain_transport,
                    &mut io.human,
                    &mut io.pty,
                    &mut io.output,
                    until,
                );
                let returned = h.time().now();
                assert_eq!(io.human.calls.len(), 1);
                let call = &io.human.calls[0];
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
                assert!(io.pty.reader.calls.is_empty() && io.pty.offered.is_empty());
                assert!(io.output.offered.is_empty());
                assert_eq!(io.output.flushes, 0);
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
        trace: Trace,
    }

    impl<O: ScreenObserver> ScreenObserver for ControlledObservation<O> {
        fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
            let entered = self.trace.clock.now();
            let actual = self.inner.observe(screen);
            let targeted = self.records.borrow().len() == 1;
            if targeted
                && matches!(
                    self.fault,
                    CallbackFault::Expire | CallbackFault::ExpireAndFail
                )
            {
                self.trace.block_past(self.deadline);
            }
            self.records.borrow_mut().push(CallbackRecord {
                sequence: screen.screen_seq,
                contents: screen.screen.contents(),
                entered,
                returned: self.trace.clock.now(),
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

        fn parser_failure(&mut self) -> Result<()> {
            self.inner.parser_failure()
        }

        fn input_health(&self) -> ObservationHealth {
            self.inner.input_health()
        }
    }

    fn assert_healthy(h: &Harness) {
        let actual = h.controller.health(&h.writer);
        assert!(!actual.degraded, "{actual:?}");
        assert_eq!(actual.pending_count, 0);
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

    const RESUMED_CHILD: &[u8] = b"live\r\n";

    fn reference_screen(bytes: &[u8]) -> ScreenSnapshot {
        let mut reference = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        reference.feed(bytes).unwrap();
        reference.snapshot()
    }

    fn observes_before_first_child_read(trace: &[Site]) -> usize {
        trace
            .iter()
            .take_while(|site| **site != Site::Child)
            .filter(|site| **site == Site::Observe)
            .count()
    }

    /// The next drain feeds the retained remainder of the interrupted chunk before any newly
    /// read child output, so the screen holds every byte and the drain seals again.
    fn assert_retained_remainder_is_observed_and_seals(
        h: &mut Harness,
        drain_transport: &mut Transport,
        endpoints: (&mut Reader, &mut DrainPty, &mut DrainOutput),
        child_bytes: &[u8],
        remainder_screens: &[u64],
    ) {
        let (human, pty, output) = endpoints;
        let generation = drain_transport.human_generation();
        let callbacks = h.observations.screens.borrow().len();
        let traced = h.trace.borrow().len();
        assert_healthy(h);
        pty.reader
            .steps
            .push_back(Read::Data(RESUMED_CHILD.to_vec()));
        let resumed = h.drain(drain_transport, human, pty, output, h.until);
        assert_eq!(resumed.unwrap(), InputDrain::Quiescent);
        let mut all = child_bytes.to_vec();
        all.extend_from_slice(RESUMED_CHILD);
        let full = all.len() as u64;
        assert_eq!(drain_transport.drained(), Some((generation, full)));
        assert!(drain_transport.take_input_drain().is_some());
        let mut expected = remainder_screens.to_vec();
        expected.extend([full - 2, full - 1, full]);
        assert_eq!(h.observations.screens.borrow()[callbacks..], expected[..]);
        assert_eq!(
            observes_before_first_child_read(&h.trace.borrow()[traced..]),
            remainder_screens.len(),
            "the retained remainder is observed before any newly read child output"
        );
        let reference = reference_screen(&all);
        assert_eq!(drain_transport.snapshot().screen_seq, reference.screen_seq);
        assert_eq!(
            drain_transport.snapshot().screen.contents(),
            reference.screen.contents()
        );
        assert_eq!(output.bytes, all);
        assert_eq!(drain_transport.pending_display_bytes(), 0);

        human.steps.push_back(Read::Data(b"manual\r".to_vec()));
        pty.reader.steps.push_back(Read::Data(b"more\r\n".to_vec()));
        drain_transport
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
        assert_eq!(drain_transport.human_generation(), generation + 7);
        let after_tick = h.drain(drain_transport, human, pty, output, h.until);
        assert_eq!(after_tick.unwrap(), InputDrain::Quiescent);
        assert!(
            drain_transport.take_input_drain().is_some(),
            "a fed remainder leaves no fence behind"
        );
    }

    #[test]
    fn drain_incomplete_delayed_positive_child_read_feeds_the_retained_chunk_on_the_next_drain() {
        with_harness(|h| {
            let mut drain_transport = h.transport(64, 64, 64);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let child_bytes = b"unseen\r\nsuffix";
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"typed", 5));
            io.pty.reader = Reader::data(Site::Child, &h.trace, child_bytes, child_bytes.len());
            let until = h.time().now() + Duration::from_millis(100);
            io.pty.reader.delay_until = Some(until);
            let result = h.drain(
                &mut drain_transport,
                &mut io.human,
                &mut io.pty,
                &mut io.output,
                until,
            );
            assert_deadline(result, &mut drain_transport);
            assert_eq!(io.human.bytes, b"typed");
            assert_eq!(drain_transport.human_generation(), 5);
            assert_eq!(io.pty.bytes, b"typed");
            assert_eq!(io.pty.reader.calls.len(), 1);
            let call = &io.pty.reader.calls[0];
            assert!(call.entered < until && call.returned >= until);
            assert_eq!(call.count, Some(child_bytes.len()));
            assert_eq!(io.pty.reader.bytes, child_bytes);
            assert_eq!(drain_transport.pending_display_bytes(), child_bytes.len());
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert_eq!(drain_transport.snapshot().screen.contents(), "");
            assert!(h.observations.screens.borrow().is_empty());
            assert_eq!(
                *h.trace.borrow(),
                vec![Site::Human, Site::ManualWrite, Site::Child]
            );
            assert!(io.output.bytes.is_empty() && io.output.offered.is_empty());
            assert_eq!(io.output.flushes, 0);
            assert_eq!(h.observations.health_reads.get(), 0);
            assert_retained_remainder_is_observed_and_seals(
                h,
                &mut drain_transport,
                (&mut io.human, &mut io.pty, &mut io.output),
                child_bytes,
                &[6, 7, 8, 14],
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
                trace: h.trace.clone(),
            },
            until,
        )
    }

    #[test]
    fn drain_incomplete_complete_control_observes_every_fragment_and_can_seal() {
        with_harness(|h| {
            let mut drain_transport = h.transport(64, 64, 64);
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"typed", 5));
            io.pty.reader = Reader::data(
                Site::Child,
                &h.trace,
                CONTROL_SEPARATED_CHILD,
                CONTROL_SEPARATED_CHILD.len(),
            );
            let records = Rc::default();
            let until = h.until;
            let result = callback_drain(
                h,
                &mut drain_transport,
                (&mut io.human, &mut io.pty, &mut io.output),
                CallbackFault::None,
                until,
                &records,
            );
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(drain_transport.drained(), Some((5, 17)));
            assert_eq!(io.output.bytes, CONTROL_SEPARATED_CHILD);
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
            let mut drain_transport = h.transport(64, 64, 64);
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
            let until = h.time().now() + Duration::from_millis(100);
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
            assert_eq!(drain_transport.snapshot().cursor, Some((0, 0)));
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
            assert_retained_remainder_is_observed_and_seals(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                &[9, 10, 11, 17],
            );
        });
    }

    #[test]
    fn drain_incomplete_mid_observer_expiry_feeds_the_retained_remainder_on_the_next_drain() {
        incomplete_callback_case(CallbackFault::Expire);
    }

    #[test]
    fn drain_incomplete_mid_observer_error_before_expiry_keeps_primary_and_feeds_the_remainder() {
        incomplete_callback_case(CallbackFault::Fail);
    }

    #[test]
    fn drain_incomplete_mid_observer_error_after_expiry_keeps_primary_and_feeds_the_remainder() {
        incomplete_callback_case(CallbackFault::ExpireAndFail);
    }

    #[test]
    fn drain_incomplete_remainder_feeds_on_the_next_tick_before_new_child_output() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(64, 64, 64);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
            io.pty.reader = Reader::data(
                Site::Child,
                &h.trace,
                CONTROL_SEPARATED_CHILD,
                CONTROL_SEPARATED_CHILD.len(),
            );
            let until = h.time().now() + Duration::from_millis(100);
            let records = Rc::default();
            let result = callback_drain(
                h,
                &mut drain_transport,
                (&mut io.human, &mut io.pty, &mut io.output),
                CallbackFault::Expire,
                until,
                &records,
            );
            assert_deadline(result, &mut drain_transport);
            assert_eq!(drain_transport.snapshot().screen_seq, 5);
            let traced = h.trace.borrow().len();
            io.pty
                .reader
                .steps
                .push_back(Read::Data(RESUMED_CHILD.to_vec()));
            drain_transport
                .tick(
                    &mut io.human,
                    &mut io.pty,
                    &mut io.output,
                    &mut Measured {
                        inner: h.controller.observer(&h.writer, &h.clock),
                        trace: h.trace.clone(),
                        facts: h.observations.clone(),
                    },
                )
                .unwrap();
            assert_eq!(
                *h.observations.screens.borrow(),
                vec![4, 5, 9, 10, 11, 17, 21, 22, 23]
            );
            assert_eq!(
                observes_before_first_child_read(&h.trace.borrow()[traced..]),
                4,
                "the tick feeds the retained remainder before its own child read"
            );
            let mut all = CONTROL_SEPARATED_CHILD.to_vec();
            all.extend_from_slice(RESUMED_CHILD);
            let reference = reference_screen(&all);
            assert_eq!(drain_transport.snapshot().screen_seq, reference.screen_seq);
            assert_eq!(
                drain_transport.snapshot().screen.contents(),
                reference.screen.contents()
            );
            assert_eq!(io.output.bytes, all);
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(drain_transport.drained(), Some((0, all.len() as u64)));
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_incomplete_fatal_observer_error_feeds_the_retained_remainder_on_the_next_drain() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(64, 64, 64);
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
                    trace: h.trace.clone(),
                },
            );
            assert_eq!(drain_transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(pty.reader.calls[0].count, Some(17));
            assert!(drain_transport.take_input_drain().is_none());
            let source = match result {
                Err(source) => source,
                Ok(_) => panic!("ordered observation returns its actual error"),
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
                "a completed child read remains queued after a failed observation"
            );
            assert_eq!(drain_transport.snapshot().screen_seq, 5);
            assert_retained_remainder_is_observed_and_seals(
                h,
                &mut drain_transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                &[9, 10, 11, 17],
            );
        });
    }

    #[derive(Clone, Copy, Debug)]
    enum Refusal {
        /// What the deadline guard returns when the deadline passed before the callback: the
        /// inner observer never sees the screen.
        Expire,
        Fail,
    }

    /// Refuses the `refuse_at`-th observer callback without passing its screen on, then
    /// forwards every later callback.
    struct RefusingObservation<O> {
        inner: O,
        calls: usize,
        refuse_at: Option<usize>,
        refusal: Refusal,
        deadline: Instant,
        trace: Trace,
    }

    impl<O: ScreenObserver> ScreenObserver for RefusingObservation<O> {
        fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
            let call = self.calls;
            self.calls += 1;
            if self.refuse_at == Some(call) {
                return match self.refusal {
                    Refusal::Expire => {
                        self.trace.block_past(self.deadline);
                        Err(CafleetError::DeadlineExceeded)
                    }
                    Refusal::Fail => Err(CafleetError::App("fixture observation refused".into())),
                };
            }
            self.inner.observe(screen)
        }

        fn parser_failure(&mut self) -> Result<()> {
            self.inner.parser_failure()
        }

        fn input_health(&self) -> ObservationHealth {
            self.inner.input_health()
        }
    }

    /// Two observed steps, `head` then the carriage return, so the second callback is the
    /// chunk's final step.
    const TWO_STEP_CHILD: &[u8] = b"head\r";

    fn refused_drain(
        h: &mut Harness,
        drain_transport: &mut Transport,
        io: &mut Endpoints,
        refuse_at: Option<usize>,
        refusal: Refusal,
        until: Instant,
    ) -> Result<InputDrain> {
        drain_transport.quiesce_for_input(
            &mut io.human,
            &mut io.pty,
            &mut io.output,
            &mut RefusingObservation {
                inner: Measured {
                    inner: h.controller.observer(&h.writer, &h.clock),
                    trace: h.trace.clone(),
                    facts: h.observations.clone(),
                },
                calls: 0,
                refuse_at,
                refusal,
                deadline: until,
                trace: h.trace.clone(),
            },
            until,
        )
    }

    fn final_step_fault_case(refusal: Refusal) {
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(64, 64, 64);
            h.seal(&mut drain_transport);
            h.reset_observations();
            let mut io = Endpoints::new(&h.trace, Reader::new(Site::Human, &h.trace, []));
            io.pty.reader =
                Reader::data(Site::Child, &h.trace, TWO_STEP_CHILD, TWO_STEP_CHILD.len());
            let until = h.time().now() + Duration::from_millis(100);
            let result = refused_drain(h, &mut drain_transport, &mut io, Some(1), refusal, until);
            match refusal {
                Refusal::Expire => assert_deadline(result, &mut drain_transport),
                Refusal::Fail => {
                    assert_eq!(
                        format!("{:?}", result.unwrap_err()),
                        "App(\"fixture observation refused\")"
                    );
                    assert!(drain_transport.take_input_drain().is_none());
                }
            }
            assert_eq!(
                drain_transport.snapshot().screen_seq,
                5,
                "the final step reached the screen"
            );
            assert_eq!(
                *h.observations.screens.borrow(),
                vec![4],
                "no observer callback succeeded for the final snapshot"
            );

            let refused_again = refused_drain(
                h,
                &mut drain_transport,
                &mut io,
                Some(0),
                Refusal::Fail,
                h.until,
            );
            assert!(
                !matches!(refused_again, Ok(InputDrain::Quiescent)),
                "{refusal:?}: observation stays incomplete while the final snapshot is unobserved: {refused_again:?}"
            );
            assert!(drain_transport.take_input_drain().is_none());
            assert_eq!(*h.observations.screens.borrow(), vec![4]);

            assert_healthy(h);
            let resumed = refused_drain(h, &mut drain_transport, &mut io, None, refusal, h.until);
            assert_eq!(resumed.unwrap(), InputDrain::Quiescent, "{refusal:?}");
            assert_eq!(
                *h.observations.screens.borrow(),
                vec![4, 5],
                "{refusal:?}: the drain observes the final snapshot before it seals"
            );
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_incomplete_final_step_deadline_guard_expiry_reobserves_the_snapshot_before_sealing() {
        final_step_fault_case(Refusal::Expire);
    }

    #[test]
    fn drain_incomplete_final_step_observer_error_reobserves_the_snapshot_before_sealing() {
        final_step_fault_case(Refusal::Fail);
    }

    #[test]
    fn drain_incomplete_complete_observation_then_display_error_preserves_eligibility() {
        for quiesce in [false, true] {
            for discard in [false, true] {
                with_harness(|h| {
                    let mut drain_transport = h.transport(64, 64, 64);
                    let mut io =
                        Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"typed", 5));
                    io.pty.reader =
                        Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
                    io.output.write_error = Some(ErrorKind::BrokenPipe);
                    let result = if quiesce {
                        h.drain_with(&mut drain_transport, &mut io).map(|_| ())
                    } else {
                        drain_transport
                            .tick(
                                &mut io.human,
                                &mut io.pty,
                                &mut io.output,
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
                    assert!(io.output.bytes.is_empty());
                    assert!(drain_transport.take_input_drain().is_none());
                    io.output.write_error = None;
                    if discard {
                        assert_eq!(drain_transport.discard_display_tail(), 17);
                    }
                    assert_healthy(h);
                    let result = h.drain_with(&mut drain_transport, &mut io);
                    assert_eq!(result.unwrap(), InputDrain::Quiescent);
                    assert_eq!(drain_transport.drained(), Some((5, 17)));
                    assert_eq!(drain_transport.pending_display_bytes(), 0);
                    assert_eq!(
                        io.output.bytes,
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
            let mut drain_transport = h.transport(64, 64, 64);
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"typed", 5));
            io.pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            assert_eq!(
                drain_transport.prefetch_child_bytes(&mut io.pty).unwrap(),
                17
            );
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert!(h.observations.screens.borrow().is_empty());
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(drain_transport.drained(), Some((5, 17)));
            assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
            assert_eq!(io.output.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_incomplete_discarding_unobserved_prefetch_keeps_lifetime_fence() {
        with_harness(|h| {
            h.control();
            let mut drain_transport = h.transport(64, 64, 64);
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"typed", 5));
            let progress = h.tick_with(&mut drain_transport, &mut io).unwrap();
            assert_eq!(progress.human_bytes, 5);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            h.reset_observations();
            io.pty
                .reader
                .steps
                .push_back(Read::Data(CONTROL_SEPARATED_CHILD.to_vec()));
            assert_eq!(
                drain_transport.prefetch_child_bytes(&mut io.pty).unwrap(),
                17
            );
            assert_eq!(io.pty.reader.calls.last().unwrap().count, Some(17));
            assert_eq!(io.pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(drain_transport.discard_display_tail(), 17);
            assert_eq!(drain_transport.pending_display_bytes(), 0);
            assert_eq!(drain_transport.snapshot().screen_seq, 0);
            assert!(h.observations.screens.borrow().is_empty());
            assert!(io.output.bytes.is_empty());
            assert_incomplete_observation_stays_fenced(
                h,
                &mut drain_transport,
                (&mut io.human, &mut io.pty, &mut io.output),
                b"",
                0,
            );
        });
    }

    #[test]
    fn drain_complete_terminal_reply_seals_a_proof_without_manual_generation() {
        with_harness(|h| {
            let reply = b"\x1b[I\x1b]11;rgb:0000/0000/0000\x07\x1b[24;80R";
            let mut drain_transport = h.transport(64, 64, 64);
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, reply, 64));
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(
                drain_transport.drained(),
                Some((0, drain_transport.snapshot().screen_seq))
            );
            assert_eq!(io.pty.bytes, reply, "the reply is forwarded unchanged");
            assert!(drain_transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn drain_completed_reply_prefix_seals_a_proof_with_unchanged_generation() {
        with_harness(|h| {
            let mut drain_transport = h.transport(64, 64, 64);
            let mut io = Endpoints::new(
                &h.trace,
                Reader::data(Site::Human, &h.trace, b"\x1b[24;", 64),
            );
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_pending(result, &mut drain_transport);
            assert_eq!(
                drain_transport.drain_blocker(),
                Some("terminal reply pending")
            );
            assert_eq!(
                io.pty.bytes, b"\x1b[24;",
                "the prefix is forwarded, not held"
            );
            assert_eq!(drain_transport.pending_input_bytes(), 0);
            assert_eq!(drain_transport.human_generation(), 0);
            io.human.steps.push_back(Read::Data(b"80R".to_vec()));
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(
                drain_transport.drained(),
                Some((0, drain_transport.snapshot().screen_seq))
            );
            assert_eq!(drain_transport.drain_blocker(), None);
            assert_eq!(io.pty.bytes, b"\x1b[24;80R");
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
            let mut drain_transport = h.transport(64, 64, 64);
            h.seal(&mut drain_transport);
            let proof = drain_transport
                .take_input_drain()
                .expect("quiet drain seals a proof");
            drain_transport
                .validate_observation_drain(&proof, h.until)
                .unwrap();
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"\x1b", 64));
            h.tick_with(&mut drain_transport, &mut io).unwrap();
            assert_eq!(drain_transport.human_generation(), 0);
            assert!(
                drain_transport
                    .validate_observation_drain(&proof, h.until)
                    .is_err()
            );
            let result = h.drain_with(&mut drain_transport, &mut io);
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
            let mut drain_transport = h.transport(64, 64, 64);
            let mut io = Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"\x1b", 64));
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_pending(result, &mut drain_transport);
            assert_eq!(
                drain_transport.drain_blocker(),
                Some("terminal reply pending")
            );
            assert_eq!(drain_transport.human_generation(), 0);
            drain_transport.backdate_reply_prefix(REPLY_PREFIX_BOUND);
            let result = h.drain_with(&mut drain_transport, &mut io);
            assert_eq!(result.unwrap(), InputDrain::Quiescent);
            assert_eq!(
                drain_transport.drained(),
                Some((1, drain_transport.snapshot().screen_seq))
            );
            assert_eq!(io.pty.bytes, b"\x1b");
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

    const POINTER_REPORTS: &[&[u8]] = &[
        b"\x1b[<0;10;20M",
        b"\x1b[<0;10;20m",
        b"\x1b[<35;120;40M",
        b"\x1b[<64;5;5M",
        b"\x1b[<65;5;5M",
        b"\x1b[M !!",
        b"\x1b[M\xff\x80\x7f",
    ];

    mod pointer {
        use crate::runtime::transport::tests::*;

        #[test]
        fn pointer_reports_split_at_every_boundary_classify_only_when_complete() {
            let start = Instant::now();
            for report in POINTER_REPORTS {
                for split in 1..report.len() {
                    let mut classifier = classifier();
                    let prefix = classifier.feed(&report[..split], start);
                    assert_eq!((prefix.manual, prefix.pointer), (0, 0));
                    assert!(classifier.pending(), "{report:?} split={split}");
                    let suffix = classifier.feed(&report[split..], start);
                    assert_eq!(
                        (suffix.manual, suffix.pointer, classifier.pending()),
                        (0, report.len(), false),
                        "{report:?} split={split}"
                    );
                }
            }
        }

        #[test]
        fn pointer_x10_accepts_each_payload_byte_from_space_through_ff() {
            let now = Instant::now();
            for position in 3..6 {
                for byte in 0x20..=0xff {
                    let mut report = *b"\x1b[M !!";
                    report[position] = byte;
                    let mut classifier = classifier();
                    let classified = classifier.feed(&report, now);
                    assert_eq!(
                        (classified.manual, classified.pointer, classifier.pending()),
                        (0, report.len(), false),
                        "position={position} byte={byte:#04x}"
                    );
                }
            }
        }

        #[test]
        fn pointer_x10_low_payload_bytes_resolve_the_report_as_manual() {
            let now = Instant::now();
            for position in 3..6 {
                for byte in 0..0x20 {
                    let mut report = *b"\x1b[M !!";
                    report[position] = byte;
                    let mut classifier = classifier();
                    let classified = classifier.feed(&report, now);
                    let remaining = classifier.finish();
                    assert_eq!(
                        (
                            classified.manual + remaining.manual,
                            classified.pointer + remaining.pointer,
                            classifier.pending()
                        ),
                        (report.len(), 0, false),
                        "position={position} byte={byte:#04x}"
                    );
                }
            }
        }

        #[test]
        fn pointer_x10_prefix_expires_as_manual_at_the_bound() {
            let report = b"\x1b[M !!";
            let start = Instant::now();
            for split in 1..report.len() {
                let mut classifier = classifier();
                let prefix = classifier.feed(&report[..split], start);
                assert_eq!((prefix.manual, prefix.pointer), (0, 0));
                let before =
                    classifier.expire(start + REPLY_PREFIX_BOUND - Duration::from_nanos(1));
                assert_eq!((before.manual, before.pointer), (0, 0));
                assert!(classifier.pending());
                let expired = classifier.expire(start + REPLY_PREFIX_BOUND);
                assert_eq!((expired.manual, expired.pointer), (split, 0));
                assert!(!classifier.pending());
                let suffix = classifier.feed(&report[split..], start + REPLY_PREFIX_BOUND);
                assert_eq!((suffix.manual, suffix.pointer), (report.len() - split, 0));
                assert!(!classifier.pending());
            }
        }

        #[test]
        fn pointer_x10_outer_eof_resolves_each_incomplete_prefix_as_manual() {
            let report = b"\x1b[M !!";
            for split in 1..report.len() {
                let mut classifier = classifier();
                let prefix = classifier.feed(&report[..split], Instant::now());
                assert_eq!((prefix.manual, prefix.pointer), (0, 0));
                let finished = classifier.finish();
                assert_eq!((finished.manual, finished.pointer), (split, 0));
                assert!(!classifier.pending());
                let repeated = classifier.finish();
                assert_eq!((repeated.manual, repeated.pointer), (0, 0));
            }
        }

        #[test]
        fn pointer_reports_inside_a_fragmented_human_paste_remain_manual() {
            let paste = b"\x1b[200~\x1b[<0;10;20M\x1b[M !!\x1b[<35;10;20m\x1b[201~";
            let now = Instant::now();
            for width in 1..=paste.len() {
                let mut classifier = classifier();
                let mut manual = 0;
                for bytes in paste.chunks(width) {
                    let classified = classifier.feed(bytes, now);
                    manual += classified.manual;
                    assert_eq!(classified.pointer, 0, "width={width}");
                }
                assert_eq!(manual, paste.len(), "width={width}");
                assert!(!classifier.pending());
                let after = classifier.feed(b"\x1b[M !!", now);
                assert_eq!((after.manual, after.pointer), (0, 6));
                assert!(!classifier.pending());
            }
        }

        #[test]
        fn pointer_drain_forwards_reports_and_seals_both_generations() {
            with_harness(|h| {
                for report in POINTER_REPORTS {
                    let mut transport = h.transport(64, 64, 64);
                    let mut io =
                        Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, report, 64));
                    assert_eq!(
                        h.drain_with(&mut transport, &mut io).unwrap(),
                        InputDrain::Quiescent
                    );
                    assert_eq!(io.pty.bytes, *report);
                    assert_eq!(io.human.bytes, *report);
                    assert_eq!(transport.human_generation(), 0);
                    assert_eq!(transport.pointer_generation(), report.len() as u64);
                    let proof = transport
                        .take_input_drain()
                        .expect("a complete pointer report drains");
                    assert_eq!(proof.human_generation(), 0);
                    assert_eq!(proof.pointer_generation, report.len() as u64);
                    transport.validate_ordinary(&proof, h.until).unwrap();
                    transport.validate_permission(&proof, h.until).unwrap();
                }
            });
        }

        #[test]
        fn pointer_drain_split_x10_waits_for_the_complete_report_before_counting() {
            with_harness(|h| {
                let report = b"\x1b[M\xff\x80\x7f";
                for split in 1..report.len() {
                    let mut transport = h.transport(64, 64, 64);
                    let mut io = Endpoints::new(
                        &h.trace,
                        Reader::data(Site::Human, &h.trace, &report[..split], 64),
                    );
                    let pending = h.drain_with(&mut transport, &mut io);
                    assert_pending(pending, &mut transport);
                    assert_eq!(transport.human_generation(), 0);
                    assert_eq!(transport.pointer_generation(), 0);
                    assert_eq!(io.pty.bytes, report[..split]);
                    io.human
                        .steps
                        .push_back(Read::Data(report[split..].to_vec()));
                    assert_eq!(
                        h.drain_with(&mut transport, &mut io).unwrap(),
                        InputDrain::Quiescent
                    );
                    assert_eq!(transport.human_generation(), 0);
                    assert_eq!(transport.pointer_generation(), report.len() as u64);
                    assert_eq!(io.pty.bytes, report);
                }
            });
        }

        #[test]
        fn pointer_drain_expired_x10_prefix_counts_manual_bytes_once() {
            with_harness(|h| {
                let prefix = b"\x1b[M !";
                let mut transport = h.transport(64, 64, 64);
                let mut io =
                    Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, prefix, 64));
                let pending = h.drain_with(&mut transport, &mut io);
                assert_pending(pending, &mut transport);
                assert_eq!(transport.human_generation(), 0);
                assert_eq!(transport.pointer_generation(), 0);
                transport.backdate_reply_prefix(REPLY_PREFIX_BOUND);
                assert_eq!(
                    h.drain_with(&mut transport, &mut io).unwrap(),
                    InputDrain::Quiescent
                );
                assert_eq!(transport.human_generation(), prefix.len() as u64);
                assert_eq!(transport.pointer_generation(), 0);
                io.human.steps.push_back(Read::Data(b"!".to_vec()));
                assert_eq!(
                    h.drain_with(&mut transport, &mut io).unwrap(),
                    InputDrain::Quiescent
                );
                assert_eq!(transport.human_generation(), (prefix.len() + 1) as u64);
                assert_eq!(transport.pointer_generation(), 0);
                assert_eq!(io.pty.bytes, b"\x1b[M !!");
            });
        }

        #[test]
        fn pointer_drain_ordinary_proof_ignores_only_a_stale_pointer_generation() {
            with_harness(|h| {
                let report = b"\x1b[<0;10;20M";
                let mut transport = h.transport(64, 64, 64);
                let mut io =
                    Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, report, 64));
                assert_eq!(
                    h.drain_with(&mut transport, &mut io).unwrap(),
                    InputDrain::Quiescent
                );
                let mut proof = transport
                    .take_input_drain()
                    .expect("the actual drain sealed a proof");
                assert_eq!(proof.pointer_generation, report.len() as u64);
                proof.pointer_generation = 0;
                transport.validate_ordinary(&proof, h.until).unwrap();
                assert!(transport.validate_permission(&proof, h.until).is_err());
            });
        }

        #[test]
        fn pointer_drain_both_proofs_reject_a_stale_keyboard_generation() {
            with_harness(|h| {
                let mut transport = h.transport(64, 64, 64);
                let mut io =
                    Endpoints::new(&h.trace, Reader::data(Site::Human, &h.trace, b"x", 64));
                assert_eq!(
                    h.drain_with(&mut transport, &mut io).unwrap(),
                    InputDrain::Quiescent
                );
                let mut proof = transport
                    .take_input_drain()
                    .expect("the actual drain sealed a proof");
                assert_eq!(proof.human_generation(), 1);
                transport.validate_ordinary(&proof, h.until).unwrap();
                transport.validate_permission(&proof, h.until).unwrap();
                proof.human_generation = 0;
                assert!(transport.validate_ordinary(&proof, h.until).is_err());
                assert!(transport.validate_permission(&proof, h.until).is_err());
            });
        }

        #[test]
        fn pointer_drain_both_proofs_retain_the_turn_binding() {
            with_harness(|h| {
                let mut transport = h.transport(64, 64, 64);
                h.seal(&mut transport);
                let old = transport
                    .take_input_drain()
                    .expect("the initial drain sealed a proof");
                transport.validate_ordinary(&old, h.until).unwrap();
                transport.validate_permission(&old, h.until).unwrap();
                let mut io = Endpoints::new(
                    &h.trace,
                    Reader::data(Site::Human, &h.trace, b"\x1b[M !!", 64),
                );
                assert_eq!(
                    h.drain_with(&mut transport, &mut io).unwrap(),
                    InputDrain::Quiescent
                );
                assert_eq!(transport.human_generation(), old.human_generation());
                assert!(transport.validate_ordinary(&old, h.until).is_err());
                assert!(transport.validate_permission(&old, h.until).is_err());
                let fresh = transport
                    .take_input_drain()
                    .expect("the new turn sealed a fresh proof");
                transport.validate_ordinary(&fresh, h.until).unwrap();
                transport.validate_permission(&fresh, h.until).unwrap();
            });
        }

        #[test]
        fn pointer_drain_both_proofs_retain_the_deadline_check() {
            with_harness(|h| {
                let mut transport = h.transport(64, 64, 64);
                h.seal(&mut transport);
                let proof = transport
                    .take_input_drain()
                    .expect("the actual drain sealed a proof");
                let expired = h.time().now();
                assert!(transport.validate_ordinary(&proof, expired).is_err());
                assert!(transport.validate_permission(&proof, expired).is_err());
            });
        }

        #[test]
        fn pointer_transport_pacing_counts_all_outer_bytes() {
            let mut transport = transport();
            let report = b"\x1b[M !!";
            let bytes = report.repeat(8);
            let mut human = FakeInput::new(&bytes, bytes.len());
            let mut pty = FakePty::new(b"", 1);
            for (read, consumed, pointer) in [(32, 32, 30), (16, 48, 48)] {
                let progress = one_tick(&mut transport, &mut human, &mut pty);
                assert_eq!(progress.human_bytes, read);
                assert_eq!(human.consumed.len(), consumed);
                assert_eq!(pty.written, bytes[..consumed]);
                assert_eq!(transport.human_generation(), 0);
                assert_eq!(transport.pointer_generation(), pointer);
            }
            assert_eq!(unread(&human), 0);
        }
    }

    mod classification {
        use crate::runtime::transport::tests::*;

        /// What one read of an input classifies.
        #[derive(Clone, Copy, Debug)]
        enum Expected {
            /// Every byte is an allowlisted reply.
            Reply,
            /// Every byte counts as manual input.
            Manual,
            /// Every byte is pointer input.
            Pointer,
            Counts {
                manual: usize,
                pointer: usize,
            },
        }

        use Expected::Counts;
        use Expected::Manual;
        use Expected::Pointer;
        use Expected::Reply;

        /// Inputs that one read classifies alike, leaving no prefix pending.
        struct OneRead {
            name: &'static str,
            inputs: &'static [&'static [u8]],
            classified: Expected,
        }

        const ONE_READS: &[OneRead] = &[
            OneRead {
                name: "cursor position reports are replies",
                inputs: &[
                    b"\x1b[24;80R",
                    b"\x1b[1;1R",
                    b"\x1b[?24;80R",
                    b"\x1b[?24;80;1R",
                ],
                classified: Reply,
            },
            OneRead {
                name: "a modified F3 collides with a cursor position report",
                inputs: &[b"\x1b[1;5R"],
                classified: Reply,
            },
            OneRead {
                name: "device status reports are replies",
                inputs: &[
                    b"\x1b[0n",
                    b"\x1b[3n",
                    b"\x1b[?1n",
                    b"\x1b[?997;1n",
                    b"\x1b[?1;2;3;4n",
                ],
                classified: Reply,
            },
            OneRead {
                name: "device attribute reports are replies",
                inputs: &[
                    b"\x1b[?1;2c",
                    b"\x1b[?62;22c",
                    b"\x1b[>0c",
                    b"\x1b[>1;10;0c",
                    b"\x1b[>41;380;0;1c",
                ],
                classified: Reply,
            },
            OneRead {
                name: "mode reports are replies",
                inputs: &[b"\x1b[?1004;1$y", b"\x1b[?2026;2$y", b"\x1b[4;2$y"],
                classified: Reply,
            },
            OneRead {
                name: "window reports are replies",
                inputs: &[b"\x1b[1t", b"\x1b[2t", b"\x1b[4;600;800t", b"\x1b[8;24;80t"],
                classified: Reply,
            },
            OneRead {
                name: "the kitty keyboard flags report is a reply",
                inputs: &[b"\x1b[?1u", b"\x1b[?31u"],
                classified: Reply,
            },
            OneRead {
                name: "focus events are replies",
                inputs: &[b"\x1b[I", b"\x1b[O"],
                classified: Reply,
            },
            OneRead {
                name: "OSC replies terminate with BEL or ST",
                inputs: &[
                    b"\x1b]11;rgb:0000/0000/0000\x07",
                    b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
                    b"\x1b]4;1;rgb:cdcd/0000/0000\x1b\\",
                ],
                classified: Reply,
            },
            OneRead {
                name: "an OSC body accepts bytes at or above 0x80",
                inputs: &["\x1b]2;caf\u{e9}\x07".as_bytes()],
                classified: Reply,
            },
            OneRead {
                name: "DCS replies terminate with ST",
                inputs: &[
                    b"\x1bP>|XTerm(380)\x1b\\",
                    b"\x1bP1+r544e=787465726d\x1b\\",
                    b"\x1bP1$r0m\x1b\\",
                    b"\x1bP!|00000000\x1b\\",
                ],
                classified: Reply,
            },
            OneRead {
                name: "APC replies terminate with ST",
                inputs: &[
                    b"\x1b_Gi=31;OK\x1b\\",
                    b"\x1b_Gi=1;ENOENT:file not found\x1b\\",
                ],
                classified: Reply,
            },
            OneRead {
                name: "a burst of replies counts zero",
                inputs: &[
                    b"\x1b[I\x1b]11;rgb:0000/0000/0000\x07\x1b[24;80R\x1b[O\x1bP>|XTerm(380)\x1b\\",
                ],
                classified: Reply,
            },
            OneRead {
                name: "a window report with four parameters is manual",
                inputs: &[b"\x1b[8;24;80;1t"],
                classified: Manual,
            },
            OneRead {
                name: "a cursor position report with one parameter is manual",
                inputs: &[b"\x1b[5R", b"\x1b[?5R", b"\x1b[?1;2;3;4R"],
                classified: Manual,
            },
            OneRead {
                name: "empty parameters are manual",
                inputs: &[
                    b"\x1b[8;t",
                    b"\x1b[;24R",
                    b"\x1b[24;;80R",
                    b"\x1b[?n",
                    b"\x1b[?c",
                ],
                classified: Manual,
            },
            OneRead {
                name: "device status outside its forms is manual",
                inputs: &[b"\x1b[1n", b"\x1b[5n", b"\x1b[?1;2;3;4;5n"],
                classified: Manual,
            },
            OneRead {
                name: "device attributes outside their arity are manual",
                inputs: &[b"\x1b[>1;2;3;4;5c", b"\x1b[1c"],
                classified: Manual,
            },
            OneRead {
                name: "mode reports outside their arity are manual",
                inputs: &[b"\x1b[?1$y", b"\x1b[?1;2;3$y", b"\x1b[4$y"],
                classified: Manual,
            },
            OneRead {
                name: "kitty flags with more than one parameter are manual",
                inputs: &[b"\x1b[?1;2u"],
                classified: Manual,
            },
            OneRead {
                name: "focus with parameters or intermediates is manual",
                inputs: &[b"\x1b[1I", b"\x1b[?I", b"\x1b[ O"],
                classified: Manual,
            },
            OneRead {
                name: "an unlisted intermediate is manual",
                inputs: &[b"\x1b[1;1 R", b"\x1b[1$t"],
                classified: Manual,
            },
            OneRead {
                name: "text and C0 controls are manual immediately",
                inputs: &[
                    b"hello",
                    "caf\u{e9} \u{65e5}\u{672c}".as_bytes(),
                    b"\r",
                    b"\t",
                    b"\x03",
                    b"\x7f",
                    b"\x07",
                ],
                classified: Manual,
            },
            OneRead {
                name: "cursor and function keys are manual",
                inputs: &[
                    b"\x1b[A",
                    b"\x1b[1;5C",
                    b"\x1b[3~",
                    b"\x1b[15;2~",
                    b"\x1b[Z",
                ],
                classified: Manual,
            },
            OneRead {
                name: "kitty key events are manual",
                inputs: &[b"\x1b[97u", b"\x1b[97;5u", b"\x1b[97;5;97u"],
                classified: Manual,
            },
            OneRead {
                name: "SS3 and alt-modified keys are manual",
                inputs: &[b"\x1bOP", b"\x1bOA", b"\x1ba", b"\x1bx", b"\x1b\r"],
                classified: Manual,
            },
            OneRead {
                name: "SOS and PM strings are manual",
                inputs: &[b"\x1bXhello\x1b\\", b"\x1b^hi\x1b\\"],
                classified: Manual,
            },
            OneRead {
                name: "eight-bit C1 bytes are manual",
                inputs: &[&[0x9b], &[0x9d], &[0x90], &[0x9f], b"\x9b1;1R", b"\x9bI"],
                classified: Manual,
            },
            OneRead {
                name: "a mixed read counts only the text around a reply",
                inputs: &[b"ab\x1b[Icd"],
                classified: Counts {
                    manual: 4,
                    pointer: 0,
                },
            },
            OneRead {
                name: "a bracketed paste counts every byte including reply forms",
                inputs: &[b"\x1b[200~\x1b[1;1R\x1b[Ipasted\x1b]11;x\x07\x1b[201~"],
                classified: Manual,
            },
            OneRead {
                name: "an escape inside CSI restarts a reply",
                inputs: &[b"\x1b[1;\x1b[I"],
                classified: Counts {
                    manual: 4,
                    pointer: 0,
                },
            },
            OneRead {
                name: "an escape before a reply counts one",
                inputs: &[b"\x1b\x1b[I"],
                classified: Counts {
                    manual: 1,
                    pointer: 0,
                },
            },
            OneRead {
                name: "a control byte inside CSI counts the prefix and the byte",
                inputs: &[
                    &[0x1b, b'[', b'1', 0x18],
                    &[0x1b, b'[', b'1', 0x1a],
                    &[0x1b, b'[', b'1', 0x00],
                    &[0x1b, b'[', b'1', 0x03],
                    &[0x1b, b'[', b'1', 0x07],
                    &[0x1b, b'[', b'1', 0x0d],
                    &[0x1b, b'[', b'1', 0x7f],
                    &[0x1b, b'[', b'1', 0x80],
                    &[0x1b, b'[', b'1', 0xc3],
                    &[0x1b, b'[', b'1', 0xff],
                ],
                classified: Counts {
                    manual: 4,
                    pointer: 0,
                },
            },
            OneRead {
                name: "a control byte inside CSI is followed by classification from ground",
                inputs: &[b"\x1b[1\x18\x1b[I"],
                classified: Counts {
                    manual: 4,
                    pointer: 0,
                },
            },
            OneRead {
                name: "a parameter byte after an intermediate is manual",
                inputs: &[b"\x1b[1$1y"],
                classified: Manual,
            },
            OneRead {
                name: "BEL inside DCS or APC is manual",
                inputs: &[b"\x1bP1$r\x07", b"\x1b_Gi=1\x07"],
                classified: Manual,
            },
            OneRead {
                name: "BEL inside DCS resolves before a following reply",
                inputs: &[b"\x1bP1$r\x07\x1b[I"],
                classified: Counts {
                    manual: 6,
                    pointer: 0,
                },
            },
            OneRead {
                name: "an escape without backslash inside OSC is manual",
                inputs: &[b"\x1b]11;x\x1ba", b"\x1b]1\x1b[I"],
                classified: Manual,
            },
            OneRead {
                name: "an escape without backslash inside OSC reclassifies the next byte",
                inputs: &[b"\x1b]1\x1b\x1b[I"],
                classified: Counts {
                    manual: 4,
                    pointer: 0,
                },
            },
            OneRead {
                name: "an escape without backslash inside DCS or APC is manual",
                inputs: &[b"\x1bP1$r\x1bq", b"\x1b_G\x1bq"],
                classified: Manual,
            },
            OneRead {
                name: "a control byte inside a string body is manual",
                inputs: &[b"\x1b]11;x\x18", b"\x1bP1\r"],
                classified: Manual,
            },
            OneRead {
                name: "an escape with an invalid continuation counts both bytes",
                inputs: &[b"\x1b1", b"\x1b\x7f"],
                classified: Manual,
            },
            OneRead {
                name: "pointer reports classify every byte as pointer input",
                inputs: POINTER_REPORTS,
                classified: Pointer,
            },
            OneRead {
                name: "an X10 report consumes exactly three payload bytes",
                inputs: &[b"\x1b[M !!xy\x1b[I"],
                classified: Counts {
                    manual: 2,
                    pointer: 6,
                },
            },
            OneRead {
                name: "an SGR report requires exactly three numeric parameters",
                inputs: &[
                    b"\x1b[<0;10M",
                    b"\x1b[<0;10;20;30M",
                    b"\x1b[<;10;20M",
                    b"\x1b[<0;;20m",
                    b"\x1b[<0;10;m",
                    b"\x1b[<0:10:20M",
                    b"\x1b[<0;10;20 M",
                    b"\x1b[?0;10;20M",
                    b"\x1b[0;10;20M",
                    b"\x1b[<0;10;20R",
                ],
                classified: Manual,
            },
            OneRead {
                name: "an X10 report requires a parameterless CSI without intermediates",
                inputs: &[b"\x1b[1M !!", b"\x1b[?M !!", b"\x1b[ M !!", b"\x1b[1 M !!"],
                classified: Manual,
            },
            OneRead {
                name: "an invalid escape inside an X10 report restarts the reply classifier",
                inputs: &[b"\x1b[M \x1b[I"],
                classified: Counts {
                    manual: 4,
                    pointer: 0,
                },
            },
            OneRead {
                name: "mixed keyboard, reports and replies keep independent counts",
                inputs: &[b"ab\x1b[<0;10;20M\x1b[I\x1b[M !!cd\x1b[24;80R"],
                classified: Counts {
                    manual: 4,
                    pointer: 17,
                },
            },
        ];

        /// A generated input at a classifier limit, which one read classifies.
        struct BuiltRead {
            name: &'static str,
            input: fn() -> Vec<u8>,
            classified: Expected,
        }

        const BUILT_READS: &[BuiltRead] = &[
            BuiltRead {
                name: "primary device attributes accept thirty-two parameters",
                input: || {
                    let reply = format!("\x1b[?{}c", ["1"; 32].join(";"));
                    assert_eq!(
                        reply.len() - 3,
                        CSI_PARAMETER_LIMIT,
                        "the private marker and 32 parameters fill the CSI parameter limit exactly"
                    );
                    reply.into_bytes()
                },
                classified: Reply,
            },
            BuiltRead {
                name: "a string body at the limit is a reply",
                input: || {
                    let mut reply = b"\x1b]".to_vec();
                    reply.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT));
                    reply.push(0x07);
                    reply
                },
                classified: Reply,
            },
            BuiltRead {
                name: "an OSC 52 clipboard read over the limit counts every byte",
                input: || {
                    let mut input = b"\x1b]52;c;".to_vec();
                    input.extend(std::iter::repeat_n(b'A', STRING_BODY_LIMIT));
                    input.push(0x07);
                    input
                },
                classified: Manual,
            },
            BuiltRead {
                name: "a DCS body over the limit counts every byte",
                input: || {
                    let mut input = b"\x1bP".to_vec();
                    input.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT + 1));
                    input.extend(b"\x1b\\");
                    input
                },
                classified: Manual,
            },
            BuiltRead {
                name: "an APC body over the limit counts every byte",
                input: || {
                    let mut input = b"\x1b_".to_vec();
                    input.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT + 1));
                    input.extend(b"\x1b\\");
                    input
                },
                classified: Manual,
            },
        ];

        fn assert_one_read(name: &str, input: &[u8], classified: Expected) {
            let expected = match classified {
                Reply => (0, 0),
                Manual => (input.len(), 0),
                Pointer => (0, input.len()),
                Counts { manual, pointer } => (manual, pointer),
            };
            let mut classifier = classifier();
            let result = classifier.feed(input, Instant::now());
            assert_eq!(
                (result.manual, result.pointer, classifier.pending()),
                (expected.0, expected.1, false),
                "{name}: {:?}",
                String::from_utf8_lossy(input)
            );
        }

        #[test]
        fn reply_classifier_one_read_classifies_every_byte_and_leaves_nothing_pending() {
            for case in ONE_READS {
                for input in case.inputs {
                    assert_one_read(case.name, input, case.classified);
                }
            }
            for case in BUILT_READS {
                assert_one_read(case.name, &(case.input)(), case.classified);
            }
        }

        /// One classifier call at an offset from the case's start instant.
        #[derive(Clone, Copy)]
        enum Op {
            Feed(&'static [u8], Duration),
            Expire(Duration),
            Finish,
        }

        /// A call, the manual bytes it resolves and whether a prefix stays pending after it;
        /// no step in these cases yields pointer input.
        struct Step {
            op: Op,
            manual: usize,
            pending: bool,
        }

        const fn step(op: Op, manual: usize, pending: bool) -> Step {
            Step {
                op,
                manual,
                pending,
            }
        }

        const START: Duration = Duration::ZERO;
        const BOUND: Duration = REPLY_PREFIX_BOUND;
        const JUST_BEFORE_BOUND: Duration = BOUND.saturating_sub(Duration::from_millis(1));
        const HALF_BOUND: Duration = Duration::from_nanos(BOUND.as_nanos() as u64 / 2);
        const TWICE_BOUND: Duration = BOUND.saturating_add(BOUND);
        const SECOND_BYTE: Duration = Duration::from_millis(60);
        const THIRD_BYTE: Duration = Duration::from_millis(120);

        use Op::Expire;
        use Op::Feed;
        use Op::Finish;

        struct Sequence {
            name: &'static str,
            steps: &'static [Step],
        }

        const SEQUENCES: &[Sequence] = &[
            Sequence {
                name: "a reply split across reads at different instants counts zero",
                steps: &[
                    step(Feed(b"\x1b[24", START), 0, true),
                    step(Expire(HALF_BOUND), 0, true),
                    step(Feed(b";80R", HALF_BOUND), 0, false),
                ],
            },
            Sequence {
                name: "a lone escape resolves as manual at the bound",
                steps: &[
                    step(Feed(b"\x1b", START), 0, true),
                    step(Expire(JUST_BEFORE_BOUND), 0, true),
                    step(Expire(BOUND), 1, false),
                ],
            },
            Sequence {
                name: "a CSI prefix resolves as manual at the bound",
                steps: &[
                    step(Feed(b"\x1b[12;", START), 0, true),
                    step(Expire(JUST_BEFORE_BOUND), 0, true),
                    step(Expire(BOUND), 5, false),
                ],
            },
            Sequence {
                name: "the bound restarts at each new prefix byte",
                steps: &[
                    step(Feed(b"\x1b", START), 0, true),
                    step(Feed(b"[", SECOND_BYTE), 0, true),
                    step(Expire(BOUND), 0, true),
                    step(Expire(SECOND_BYTE.saturating_add(BOUND)), 2, false),
                ],
            },
            Sequence {
                name: "bytes after an expired prefix classify from ground",
                steps: &[
                    step(Feed(b"\x1b", START), 0, true),
                    step(Expire(BOUND), 1, false),
                    step(Feed(b"[I", BOUND), 2, false),
                    step(Feed(b"\x1b[I", BOUND), 0, false),
                ],
            },
            Sequence {
                name: "expiry without a pending prefix counts zero",
                steps: &[
                    step(Expire(BOUND), 0, false),
                    step(Feed(b"\x1b[I", START), 0, false),
                    step(Expire(TWICE_BOUND), 0, false),
                ],
            },
            Sequence {
                name: "finish resolves a pending prefix as manual",
                steps: &[
                    step(Feed(b"\x1b[1;", START), 0, true),
                    step(Finish, 4, false),
                ],
            },
            Sequence {
                name: "finish resolves a pending string body as manual",
                steps: &[
                    step(Feed(b"\x1b]11;rgb:00", START), 0, true),
                    step(Finish, 11, false),
                ],
            },
            Sequence {
                name: "finish resolves a pending string escape as manual",
                steps: &[
                    step(Feed(b"\x1bP1$r0m\x1b", START), 0, true),
                    step(Finish, 8, false),
                ],
            },
            Sequence {
                name: "finish without a pending prefix counts zero",
                steps: &[
                    step(Finish, 0, false),
                    step(Feed(b"ab", START), 2, false),
                    step(Finish, 0, false),
                ],
            },
            Sequence {
                name: "text before a pending prefix counts in the same read",
                steps: &[
                    step(Feed(b"ab\x1b[", START), 2, true),
                    step(Feed(b"O", START), 0, false),
                ],
            },
            Sequence {
                name: "a reply after a bracketed paste counts zero",
                steps: &[
                    step(Feed(b"\x1b[200~text\x1b[201~", START), 16, false),
                    step(Feed(b"\x1b[I", START), 0, false),
                ],
            },
            Sequence {
                name: "the X10 pending bound restarts at each payload byte",
                steps: &[
                    step(Feed(b"\x1b[M", START), 0, true),
                    step(Feed(b" ", SECOND_BYTE), 0, true),
                    step(Expire(BOUND), 0, true),
                    step(Feed(b"!", THIRD_BYTE), 0, true),
                    step(Expire(SECOND_BYTE.saturating_add(BOUND)), 0, true),
                    step(Expire(THIRD_BYTE.saturating_add(BOUND)), 5, false),
                ],
            },
        ];

        #[test]
        fn reply_classifier_sequences_resolve_prefixes_at_the_bound_or_at_finish() {
            for case in SEQUENCES {
                let name = case.name;
                let start = Instant::now();
                let mut classifier = classifier();
                for (index, step) in case.steps.iter().enumerate() {
                    let result = match step.op {
                        Feed(bytes, at) => classifier.feed(bytes, start + at),
                        Expire(at) => classifier.expire(start + at),
                        Finish => classifier.finish(),
                    };
                    assert_eq!(
                        (result.manual, result.pointer, classifier.pending()),
                        (step.manual, 0, step.pending),
                        "{name}: step {index}"
                    );
                }
            }
        }

        /// A pending prefix filled to a limit; one more body byte resolves it.
        struct LimitOverflow {
            name: &'static str,
            prefix: fn() -> Vec<u8>,
            overflow: &'static [u8],
            terminator: &'static [u8],
        }

        const LIMIT_OVERFLOWS: &[LimitOverflow] = &[
            LimitOverflow {
                name: "an overlong CSI is manual at the parameter byte exceeding the limit",
                prefix: || {
                    let mut prefix = b"\x1b[".to_vec();
                    prefix.extend(std::iter::repeat_n(b'1', CSI_PARAMETER_LIMIT));
                    prefix
                },
                overflow: b"1",
                terminator: b"R",
            },
            LimitOverflow {
                name: "an OSC body over the limit is manual at the body byte exceeding the limit",
                prefix: || {
                    let mut prefix = b"\x1b]".to_vec();
                    prefix.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT));
                    prefix
                },
                overflow: b"x",
                terminator: b"\x07",
            },
        ];

        #[test]
        fn reply_classifier_the_byte_exceeding_a_limit_resolves_the_whole_prefix() {
            for case in LIMIT_OVERFLOWS {
                let name = case.name;
                let prefix = (case.prefix)();
                let mut classifier = classifier();
                let now = Instant::now();
                assert_eq!(classifier.feed(&prefix, now).manual, 0, "{name}");
                assert!(classifier.pending(), "{name}");
                assert_eq!(
                    classifier.feed(case.overflow, now).manual,
                    prefix.len() + 1,
                    "{name}"
                );
                assert!(!classifier.pending(), "{name}");
                assert_eq!(classifier.feed(case.terminator, now).manual, 1, "{name}");
            }
        }

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
                    assert_eq!(
                        classifier.feed(&reply[..split], now).manual,
                        0,
                        "{reply:?} split at {split}"
                    );
                    assert!(classifier.pending(), "{reply:?} split at {split}");
                    assert_eq!(
                        classifier.feed(&reply[split..], now).manual,
                        0,
                        "{reply:?} split at {split}"
                    );
                    assert!(!classifier.pending(), "{reply:?} split at {split}");
                }
            }
        }
    }

    use std::collections::VecDeque;

    use crate::error::CafleetError;

    fn transport() -> Transport {
        Transport::new(
            TerminalSize { rows: 24, cols: 80 },
            TransportLimits {
                input_bytes: 32,
                display_bytes: 64,
                bytes_per_tick: 16,
            },
            Arc::new(TestClock::starting_at(Instant::now())),
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
            Arc::new(TestClock::starting_at(Instant::now())),
        )
        .unwrap()
    }

    fn patterned(len: usize, salt: usize) -> Vec<u8> {
        (0..len)
            .map(|index| u8::try_from((index + salt) % 251).unwrap())
            .collect()
    }

    fn reads(sizes: &[usize]) -> (FakeInput, Vec<u8>) {
        let mut all = Vec::new();
        let mut chunks = VecDeque::new();
        for (salt, &size) in sizes.iter().enumerate() {
            let chunk = patterned(size, salt * 7);
            all.extend_from_slice(&chunk);
            chunks.push_back(chunk);
        }
        (
            FakeInput {
                chunks,
                ..FakeInput::default()
            },
            all,
        )
    }

    fn unread(input: &FakeInput) -> usize {
        input.chunks.iter().map(Vec::len).sum()
    }

    fn one_tick(
        transport: &mut Transport,
        human: &mut FakeInput,
        pty: &mut FakePty,
    ) -> TransportProgress {
        transport
            .tick(
                human,
                pty,
                &mut FakeDisplay::default(),
                &mut Observer::default(),
            )
            .unwrap()
    }

    #[test]
    fn runner_transport_one_tick_drains_a_split_outer_reply_and_writes_it_in_one_offer() {
        let mut transport = service_transport();
        let (mut human, reply) = reads(&[4096, 3600]);
        let mut pty = FakePty::new(b"", 1);
        let generation = transport.human_generation();
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            unread(&human),
            0,
            "the tick reads until the outer input would block"
        );
        assert_eq!(
            pty.written, reply,
            "all 7696 bytes reach the child in order"
        );
        assert_eq!(
            pty.offers.iter().map(Vec::len).collect::<Vec<_>>(),
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
        let mut pty = FakePty::new(b"", 1);
        let first = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(first.human_bytes, 65536);
        assert_eq!(transport.human_generation(), 65536);
        assert_eq!(
            unread(&human),
            16384,
            "reads stop once the queue holds 65536 bytes"
        );
        assert_eq!(pty.written, payload[..65536]);
        let second = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            second.human_bytes, 16384,
            "the remaining reply is read on the next tick"
        );
        assert_eq!(unread(&human), 0);
        assert_eq!(pty.written, payload);
        assert_eq!(transport.human_generation(), 81920);
    }

    #[test]
    fn runner_transport_blocked_master_keeps_the_full_queue_and_reads_nothing_more() {
        let mut transport = service_transport();
        let (mut human, payload) = reads(&[16384, 16384, 16384, 16384, 16384]);
        let mut pty = FakePty::new(b"", 1);
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
        assert_eq!(pty.written, payload);
        assert_eq!(transport.human_generation(), 81920);
    }

    #[test]
    fn runner_transport_forwards_data_read_before_outer_eof_and_reports_eof() {
        let mut transport = service_transport();
        let (mut human, reply) = reads(&[4096, 12]);
        human.eof = true;
        let mut pty = FakePty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.written, reply);
        assert!(progress.outer_eof);
        assert_eq!(progress.human_bytes, 4108);
        assert_eq!(transport.human_generation(), 4108);
    }

    #[test]
    fn runner_transport_partial_master_write_keeps_the_remainder_queued_in_order() {
        let mut transport = service_transport();
        let (mut human, reply) = reads(&[4096, 3600]);
        let mut pty = FakePty::new(b"", 1);
        pty.max_write = 5000;
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.written, reply[..5000]);
        assert_eq!(transport.pending_input_bytes(), 2696);
        let later = patterned(10, 99);
        human.chunks.push_back(later.clone());
        one_tick(&mut transport, &mut human, &mut pty);
        let mut expected = reply.clone();
        expected.extend_from_slice(&later);
        assert_eq!(pty.written, expected, "queued bytes precede later reads");
        assert_eq!(
            pty.offers.iter().map(Vec::len).collect::<Vec<_>>(),
            [7696, 2706]
        );
        assert_eq!(transport.pending_input_bytes(), 0);
    }

    #[test]
    fn runner_transport_passes_escape_sequences_unframed() {
        let mut transport = service_transport();
        let reply =
            b"\x1b]4;0;rgb:0000/0000/0000\x1b\\\x1b]11;rgb:afaf/d7d7/8787\x07\x1b[".to_vec();
        let mut human = FakeInput {
            chunks: VecDeque::from([reply.clone()]),
            ..FakeInput::default()
        };
        let mut pty = FakePty::new(b"", 1);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            pty.written, reply,
            "an unterminated sequence is forwarded, not held"
        );
        assert_eq!(transport.pending_input_bytes(), 0);
        assert_eq!(
            transport.human_generation(),
            0,
            "complete OSC replies and a pending prefix are not yet manual"
        );
        transport.backdate_reply_prefix(REPLY_PREFIX_BOUND);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(progress.human_bytes, 0);
        assert_eq!(pty.written, reply, "classification never synthesizes bytes");
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
        let mut pty = FakePty::new(child, 1);
        pty.max_write = 1;
        let mut human_input = FakeInput::new(human, 2);
        let mut display = FakeDisplay {
            max_write: Some(1),
            ..Default::default()
        };
        let mut observer = Observer::default();
        for _ in 0..1024 {
            transport
                .tick(&mut human_input, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        assert_eq!(pty.written, human);
        assert_eq!(display.bytes, child);
        assert_eq!(transport.pending_input_bytes(), 0);
        assert_eq!(transport.pending_display_bytes(), 0);
    }

    #[test]
    fn runner_transport_observes_reconstructed_output_before_displaying_it() {
        let ordering = Rc::new(RefCell::new(Vec::new()));
        let mut transport = transport();
        let mut pty = FakePty::new(b"request", 7);
        let mut human = FakeInput::new(b"", 1);
        let mut display = FakeDisplay {
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
        let mut pty = FakePty::new(&payload, 16);
        let mut human = FakeInput::new(b"human native answer\r", 1);
        let mut display = FakeDisplay {
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
        assert_eq!(pty.written, b"human native answer\r");
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
        let mut pty = FakePty::new(b"", 1);
        pty.blocked = true;
        let mut human = FakeInput::new(&payload, 16);
        let mut display = FakeDisplay::default();
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
        assert_eq!(pty.written, payload);
    }

    #[test]
    fn runner_transport_forwards_one_outer_terminal_query_answer_without_parser_duplicates() {
        let mut transport = transport();
        let mut pty = FakePty::new(b"\x1b[6n", 1);
        let mut human = FakeInput::new(b"\x1b[1;1R", 1);
        let mut display = FakeDisplay::default();
        let mut observer = Observer::default();
        for _ in 0..64 {
            transport
                .tick(&mut human, &mut pty, &mut display, &mut observer)
                .unwrap();
        }
        assert_eq!(pty.written, b"\x1b[1;1R");
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
        let mut human = FakeInput::new(&burst, 5);
        let mut pty = FakePty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(
            pty.written, burst,
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
        let mut human = FakeInput::new(b"\x1b[24", 16);
        let mut pty = FakePty::new(b"", 1);
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.written, b"\x1b[24", "the prefix is forwarded, not held");
        assert_eq!(transport.human_generation(), 0);
        human.chunks.push_back(b";80R".to_vec());
        one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.written, b"\x1b[24;80R");
        assert_eq!(transport.human_generation(), 0);
    }

    #[test]
    fn runner_transport_counts_only_manual_bytes_around_a_reply() {
        let mut transport = service_transport();
        let input = b"ab\x1b[Icd".to_vec();
        let mut human = FakeInput::new(&input, 16);
        let mut pty = FakePty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert_eq!(pty.written, input);
        assert_eq!(progress.human_bytes, input.len());
        assert_eq!(transport.human_generation(), 4);
    }

    #[test]
    fn runner_transport_outer_eof_counts_a_pending_reply_prefix_as_manual() {
        let mut transport = service_transport();
        let mut human = FakeInput::new(b"ab\x1b[", 16);
        human.eof = true;
        let mut pty = FakePty::new(b"", 1);
        let progress = one_tick(&mut transport, &mut human, &mut pty);
        assert!(progress.outer_eof);
        assert_eq!(pty.written, b"ab\x1b[");
        assert_eq!(
            transport.human_generation(),
            4,
            "outer EOF resolves the pending prefix as manual before EOF is reported"
        );
    }

    #[test]
    fn runner_transport_resize_reaches_the_pty_and_screen_together() {
        let mut transport = transport();
        let mut pty = FakePty::new(b"", 1);
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
            let mut pty = FakePty::new(b"", 1);
            pty.output.eof = !outer;
            let mut human = FakeInput::new(b"", 1);
            human.eof = outer;
            let progress = transport
                .tick(
                    &mut human,
                    &mut pty,
                    &mut FakeDisplay::default(),
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
        let mut pty = FakePty::new(b"undelivered tail", 16);
        let mut human = FakeInput::new(b"", 1);
        let mut display = FakeDisplay {
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
        pty.output = FakeInput::new(b"broken", 6);
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
                Transport::new(
                    TerminalSize { rows: 24, cols: 80 },
                    limits,
                    Arc::new(TestClock::starting_at(Instant::now()))
                )
                .is_err(),
                "{field}"
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;
    use std::io::ErrorKind;
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
        fn input_health(&self) -> crate::runtime::observation::ObservationHealth {
            unreachable!("the transport fixture observer is never asked to quiesce input")
        }
        fn parser_failure(&mut self) -> std::result::Result<(), CafleetError> {
            Ok(())
        }
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

    /// Terminal input that first fails `interruptions` reads with `EINTR`, then yields its
    /// queued chunks one per read, then `WouldBlock`, or end of file once `eof` is set.
    #[derive(Default)]
    pub(crate) struct FakeInput {
        pub(crate) chunks: VecDeque<Vec<u8>>,
        pub(crate) eof: bool,
        pub(crate) consumed: Vec<u8>,
        pub(crate) interruptions: usize,
    }

    impl FakeInput {
        pub(crate) fn new(bytes: &[u8], width: usize) -> Self {
            assert!(width > 0);
            Self {
                chunks: bytes.chunks(width).map(<[u8]>::to_vec).collect(),
                ..Self::default()
            }
        }
    }

    impl ByteInput for FakeInput {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            if self.interruptions > 0 {
                self.interruptions -= 1;
                return Err(ErrorKind::Interrupted.into());
            }
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
            self.consumed.extend_from_slice(&target[..count]);
            Ok(count)
        }
    }

    /// A PTY whose child side reads from `output` and whose master side records every offer.
    /// `failure` refuses every write, `fault` only the next one.
    pub(crate) struct FakePty {
        pub(crate) output: FakeInput,
        pub(crate) reads: usize,
        pub(crate) written: Vec<u8>,
        pub(crate) offers: Vec<Vec<u8>>,
        pub(crate) blocked: bool,
        pub(crate) failure: Option<ErrorKind>,
        pub(crate) fault: Option<ErrorKind>,
        pub(crate) max_write: usize,
        pub(crate) sizes: Vec<TerminalSize>,
        pub(crate) closed: bool,
    }

    impl FakePty {
        pub(crate) fn new(output: &[u8], width: usize) -> Self {
            Self {
                output: FakeInput::new(output, width),
                reads: 0,
                written: Vec::new(),
                offers: Vec::new(),
                blocked: false,
                failure: None,
                fault: None,
                max_write: usize::MAX,
                sizes: Vec::new(),
                closed: false,
            }
        }
    }

    impl ByteInput for FakePty {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            self.output.try_read(target)
        }
    }

    impl PtyIo for FakePty {
        fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert!(!bytes.is_empty());
            if self.blocked {
                return Err(ErrorKind::WouldBlock.into());
            }
            self.offers.push(bytes.to_vec());
            if let Some(kind) = self.fault.take().or(self.failure) {
                return Err(io::Error::new(kind, "fixture PTY write failure"));
            }
            let count = bytes.len().min(self.max_write);
            self.written.extend_from_slice(&bytes[..count]);
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

    /// A display that records accepted bytes; `ordering` logs each write as `display`.
    #[derive(Default)]
    pub(crate) struct FakeDisplay {
        pub(crate) bytes: Vec<u8>,
        pub(crate) blocked: bool,
        pub(crate) broken: bool,
        pub(crate) max_write: Option<usize>,
        pub(crate) ordering: Option<Rc<RefCell<Vec<&'static str>>>>,
    }

    impl Display for FakeDisplay {
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
}
