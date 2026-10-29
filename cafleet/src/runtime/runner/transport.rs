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
    pub forwarded_bytes: usize,
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
        let mut forwarded = 0;
        while !self.outer_eof {
            let capacity =
                (self.limits.input_bytes - self.input.len()).min(self.limits.bytes_per_tick);
            let Some(bytes) = read_available(human, capacity).map_err(io_error)? else {
                break;
            };
            self.outer_eof = bytes.is_empty();
            forwarded += bytes.len();
            self.retain_human(bytes);
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

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::error::CafleetError;
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
                .push((screen.screen_seq, screen.contents.clone()));
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::{self, ErrorKind};
    use std::rc::Rc;

    use super::test_support::*;
    use super::*;
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
