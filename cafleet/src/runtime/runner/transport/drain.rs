use super::*;
use std::time::Instant;

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

fn before_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(CafleetError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

struct DeadlineObserver<'a, O> {
    inner: &'a mut O,
    deadline: Instant,
}
impl<O: ScreenObserver> ScreenObserver for DeadlineObserver<'_, O> {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
        before_deadline(self.deadline)?;
        self.inner.observe(screen)?;
        before_deadline(self.deadline)
    }

    fn failures_are_fatal(&self) -> bool {
        true
    }

    fn parser_failure(&mut self) -> Result<()> {
        before_deadline(self.deadline)?;
        self.inner.parser_failure()?;
        before_deadline(self.deadline)
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
        before_deadline(deadline)?;
        let mut remaining = DrainBudget {
            human: self.limits.bytes_per_tick,
            child: self.limits.bytes_per_tick,
            manual: self.limits.bytes_per_tick,
            display: self.limits.bytes_per_tick,
        };
        for _ in 0..ROUND_LIMIT {
            before_deadline(deadline)?;
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
                AvailableRead::Eof => self.outer_eof = true,
                _ => {}
            }
            before_deadline(deadline)?;
            if remaining.manual > 0 && !self.input.is_empty() {
                let written = write_pending(&mut self.input, remaining.manual, |bytes| {
                    pty.try_write(bytes)
                })?;
                remaining.manual -= written;
                before_deadline(deadline)?;
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
            before_deadline(deadline)?;
            if remaining.display > 0 && !self.display.is_empty() {
                let written = write_pending(&mut self.display, remaining.display, |bytes| {
                    display.try_write(bytes)
                })?;
                remaining.display -= written;
                before_deadline(deadline)?;
            }
            match display.try_flush() {
                Ok(()) => {}
                Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                Err(cause) => return Err(io_error(cause)),
            }
            before_deadline(deadline)?;
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
                if !self.observation_complete {
                    return Ok(self.pending("observation incomplete".into()));
                }
                let health = observer.input_health();
                before_deadline(deadline)?;
                let Some(health) = health else {
                    return Ok(self.pending("observation health unavailable".into()));
                };
                if let Some(reason) = unsettled_health(&health) {
                    return Ok(self.pending(reason));
                }
                let output_seq = self.terminal.snapshot().screen_seq;
                before_deadline(deadline)?;
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
        before_deadline(deadline)?;
        #[cfg(test)]
        if let Some(hooks) = &self.observation_hooks {
            hooks.before(self.observation_phase, ObservationOperation::Parser)?;
            before_deadline(deadline)?;
        }
        let mut bounded = DeadlineObserver {
            inner: observer,
            deadline,
        };
        match self.terminal.feed_observed_classified(bytes, &mut bounded) {
            Ok(()) => {
                self.observation_complete = observation_complete_before;
                before_deadline(deadline)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::runtime as storage;
    use crate::coding_agent::ProfileCatalog;
    use crate::runtime::runner::writer::test_support::{
        Clock, Hooks, Release, WriterFixture, start, transition,
    };
    use std::cell::{Cell, RefCell};
    use std::io::{self, ErrorKind};
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
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

    struct Pty {
        reader: Reader,
        offered: Vec<Vec<u8>>,
        bytes: Vec<u8>,
        max_write: usize,
        write_error: Option<ErrorKind>,
        resize_error: bool,
    }

    impl Pty {
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

    impl ByteInput for Pty {
        fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            self.reader.try_read(target)
        }
    }

    impl PtyIo for Pty {
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

    struct Output {
        trace: Trace,
        offered: Vec<Vec<u8>>,
        bytes: Vec<u8>,
        max_write: usize,
        write_error: Option<ErrorKind>,
        flush_error: Option<ErrorKind>,
        flushes: usize,
    }

    impl Output {
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

    impl Display for Output {
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
                &ProfileCatalog::bundled().unwrap(),
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
            transport: &mut Transport,
            human: &mut Reader,
            pty: &mut Pty,
            output: &mut Output,
            until: Instant,
        ) -> Result<InputDrain> {
            transport.quiesce_for_input(
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

        fn seal(&mut self, transport: &mut Transport) {
            let mut human = Reader::new(Site::Human, &self.trace, []);
            let mut pty = Pty::new(&self.trace);
            let mut output = Output::new(&self.trace);
            let result = self.drain(transport, &mut human, &mut pty, &mut output, self.until);
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: transport.human_generation(),
                    output_seq: transport.snapshot().screen_seq,
                }
            );
            assert_eq!(human.calls.len(), 1);
            assert_eq!(pty.reader.calls.len(), 1);
        }

        fn control(&mut self) {
            let mut transport = transport(64, 64, 64);
            self.seal(&mut transport);
            assert!(transport.take_input_drain().is_some());
            assert!(transport.take_input_drain().is_none());
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
        if let Err(primary) = primary {
            if cleanup.is_err() {
                eprintln!("input-drain fixture cleanup also failed; primary failure is retained");
            }
            resume_unwind(primary);
        }
        if let Err(cleanup) = cleanup {
            resume_unwind(cleanup);
        }
    }

    fn transport(bytes: usize, input: usize, display: usize) -> Transport {
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

    fn assert_pending(result: Result<InputDrain>, transport: &mut Transport) {
        assert_eq!(result.unwrap(), InputDrain::Pending);
        assert!(transport.take_input_drain().is_none());
    }

    fn assert_deadline(result: Result<InputDrain>, transport: &mut Transport) {
        assert!(
            matches!(result, Err(CafleetError::DeadlineExceeded)),
            "{result:?}"
        );
        assert!(transport.take_input_drain().is_none());
    }

    fn assert_bounds(human: &Reader, pty: &Pty, output: &Output, bytes: usize) {
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
    fn step5_drain_d1_actual_quiet_round_seals_health_without_granting_composer_eligibility() {
        with_harness(|h| {
            let mut transport = transport(32, 32, 32);
            assert_eq!(transport.human_generation(), 0);
            assert!(transport.take_input_drain().is_none());
            h.seal(&mut transport);
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
            assert!(transport.take_input_drain().is_some());
            assert!(transport.take_input_drain().is_none());
        });
    }

    #[test]
    fn step5_drain_d2_fragmented_reads_and_partial_writes_preserve_real_bytes_and_sequence() {
        with_harness(|h| {
            h.control();
            let human_bytes = "literal 世界\r".as_bytes();
            let child_bytes = b"child\r\n";
            let mut transport = transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, human_bytes, 4);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, child_bytes, 2);
            pty.max_write = 3;
            let mut output = Output::new(&h.trace);
            output.max_write = 3;
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
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
                transport.snapshot().screen_seq,
                u64::try_from(child_bytes.len()).unwrap()
            );
            assert!(transport.snapshot().contents.contains("child"));
            assert_eq!(transport.pending_input_bytes(), 0);
            assert_eq!(transport.pending_display_bytes(), 0);
            assert_eq!(h.observations.health_reads.get(), 1);
            assert!(
                h.observations
                    .screens
                    .borrow()
                    .windows(2)
                    .all(|pair| pair[0] <= pair[1])
            );
            assert_bounds(&human, &pty, &output, 64);
            assert!(transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn step5_drain_d3_interrupted_reads_require_a_later_actual_same_round_wouldblock_pair() {
        for human_interrupted in [true, false] {
            with_harness(|h| {
                h.control();
                let mut transport = transport(32, 32, 32);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                let mut pty = Pty::new(&h.trace);
                let mut output = Output::new(&h.trace);
                if human_interrupted {
                    human.steps.push_back(Read::Interrupted);
                } else {
                    pty.reader.steps.push_back(Read::Interrupted);
                }
                let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
                assert_pending(result, &mut transport);
                assert_eq!(transport.human_generation(), 0);
                assert_eq!(h.observations.health_reads.get(), 0);
                assert!(human.calls.len() <= 1 && pty.reader.calls.len() <= 1);
                h.seal(&mut transport);
                assert!(transport.take_input_drain().is_some());
            });
        }
    }

    #[test]
    fn step5_drain_d4_eof_is_not_a_quiet_read_and_adds_no_manual_generation() {
        for human_eof in [true, false] {
            with_harness(|h| {
                h.control();
                let mut transport = transport(32, 32, 32);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                let mut pty = Pty::new(&h.trace);
                let mut output = Output::new(&h.trace);
                if human_eof {
                    human.steps.push_back(Read::Eof);
                } else {
                    pty.reader.steps.push_back(Read::Eof);
                }
                let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
                assert_pending(result, &mut transport);
                assert_eq!(transport.human_generation(), 0);
                assert_eq!(h.observations.health_reads.get(), 0);
                assert_bounds(&human, &pty, &output, 32);
            });
        }
    }

    #[test]
    fn step5_drain_d5_round_and_separate_byte_quotas_bound_continuously_readable_sources() {
        for (bytes, width, expected_reads, expected_bytes) in [(64, 1, 8, 8), (4, usize::MAX, 1, 4)]
        {
            with_harness(|h| {
                h.control();
                let mut transport = transport(bytes, 128, 128);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                human.fill = Some(b'h');
                human.width = width;
                let mut pty = Pty::new(&h.trace);
                pty.reader.fill = Some(b'c');
                pty.reader.width = width;
                let mut output = Output::new(&h.trace);
                let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
                assert_pending(result, &mut transport);
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
                    h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                        .unwrap(),
                    InputDrain::Quiescent { .. }
                ));
            });
        }
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"1234567", 1);
            let mut pty = Pty::new(&h.trace);
            let mut output = Output::new(&h.trace);
            let result = h
                .drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
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
    fn step5_drain_d6_full_manual_queue_supplies_no_read_proof_and_preserves_order_on_release() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(16, 4, 32);
            let mut human = Reader::data(Site::Human, &h.trace, b"abcdefgh", 8);
            let mut pty = Pty::new(&h.trace);
            pty.write_error = Some(ErrorKind::WouldBlock);
            let mut output = Output::new(&h.trace);
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
            assert_pending(result, &mut transport);
            assert_eq!(human.calls.len(), 1);
            assert_eq!(human.calls[0].capacity, 4);
            assert_eq!(transport.pending_input_bytes(), 4);
            assert_eq!(transport.human_generation(), 4);
            assert!(pty.bytes.is_empty());
            assert!(pty.offered.len() <= 8);
            pty.write_error = None;
            assert!(matches!(
                h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                    .unwrap(),
                InputDrain::Quiescent {
                    human_generation: 8,
                    ..
                }
            ));
            assert_eq!(pty.bytes, b"abcdefgh");
            assert_eq!(transport.pending_input_bytes(), 0);
        });
    }

    #[test]
    fn step5_drain_d7_display_and_prefetched_child_pressure_require_real_later_drain() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(16, 32, 4);
            let mut human = Reader::data(Site::Human, &h.trace, b"manual", 6);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, b"abcdefgh", 8);
            let mut output = Output::new(&h.trace);
            output.write_error = Some(ErrorKind::WouldBlock);
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
            assert_pending(result, &mut transport);
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(transport.pending_display_bytes(), 4);
            assert_eq!(pty.bytes, b"manual");
            output.write_error = None;
            assert!(matches!(
                h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                    .unwrap(),
                InputDrain::Quiescent { .. }
            ));
            assert_eq!(output.bytes, b"abcdefgh");
        });
        with_harness(|h| {
            h.control();
            let mut transport = transport(16, 32, 32);
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = Pty::new(&h.trace);
            pty.reader.steps = [
                Read::Data(b"first".to_vec()),
                Read::Interrupted,
                Read::Data(b"rest".to_vec()),
            ]
            .into();
            let mut output = Output::new(&h.trace);
            assert_eq!(transport.prefetch_child_bytes(&mut pty).unwrap(), 5);
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
            assert_pending(result, &mut transport);
            assert_eq!(transport.snapshot().screen_seq, 5);
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
                h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                    .unwrap(),
                InputDrain::Quiescent { .. }
            ));
            assert_eq!(output.bytes, b"firstrest");
            assert_eq!(transport.snapshot().screen_seq, 9);
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
    fn step5_drain_d8_absent_health_keeps_manual_observation_progress_without_a_proof() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(32, 32, 32);
            let mut human = Reader::data(Site::Human, &h.trace, b"human", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, b"output", 6);
            let mut output = Output::new(&h.trace);
            let mut observer = LegacyObserver(0);
            assert!(observer.input_health().is_none());
            let result = transport.quiesce_for_input(
                &mut human,
                &mut pty,
                &mut output,
                &mut observer,
                h.until,
            );
            assert_pending(result, &mut transport);
            assert_eq!(pty.bytes, b"human");
            assert_eq!(output.bytes, b"output");
            assert_eq!(transport.human_generation(), 5);
            assert!(observer.0 > 0);
            h.seal(&mut transport);
            assert!(transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn step5_drain_d9_actual_controller_gap_health_prevents_quiet_proof() {
        with_harness(|h| {
            h.control();
            h.controller
                .observer_failed(ObservationFailure::Parser, &h.clock)
                .unwrap();
            let expected = h.controller.health(&h.writer);
            assert!(expected.degraded);
            assert!(expected.gap_count > 0);
            let mut transport = transport(32, 32, 32);
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = Pty::new(&h.trace);
            let mut output = Output::new(&h.trace);
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
            assert_pending(result, &mut transport);
            assert_eq!(h.observations.health_reads.get(), 1);
            assert_eq!(
                serde_json::to_value(h.observations.last_health.borrow().as_ref().unwrap())
                    .unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        });
    }

    #[test]
    fn step5_drain_d10_real_pending_writer_commit_blocks_health_until_release_and_persistence() {
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
            let mut transport = transport(32, 32, 32);
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = Pty::new(&h.trace);
            let mut output = Output::new(&h.trace);
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
            assert_pending(result, &mut transport);
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
            h.seal(&mut transport);
            assert!(transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn step5_drain_d11_tick_and_drain_keep_actual_human_counts_before_later_errors() {
        for quiesce in [false, true] {
            for failure in ["pty", "child", "display", "flush", "observer", "zero"] {
                with_harness(|h| {
                    h.control();
                    let mut transport = transport(32, 32, 32);
                    h.seal(&mut transport);
                    let mut human = Reader::data(Site::Human, &h.trace, "世x".as_bytes(), 4);
                    let mut pty = Pty::new(&h.trace);
                    pty.reader = Reader::data(Site::Child, &h.trace, b"output", 6);
                    let mut output = Output::new(&h.trace);
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
                                .observe(&transport.snapshot())
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
                        h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                            .map(|_| ())
                    } else {
                        transport
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
                    assert_eq!(transport.human_generation(), 4);
                    assert!(transport.take_input_drain().is_none());
                });
            }
        }
    }

    #[test]
    fn step5_drain_d12_opaque_proof_is_single_use_and_diagnostic_copies_supply_no_authority() {
        with_harness(|h| {
            let mut other = transport(32, 32, 32);
            let mut transport = transport(32, 32, 32);
            h.seal(&mut transport);
            let diagnostic = InputDrain::Quiescent {
                human_generation: 0,
                output_seq: transport.snapshot().screen_seq,
            };
            let copied = diagnostic;
            assert_eq!(
                copied,
                InputDrain::Quiescent {
                    human_generation: 0,
                    output_seq: 0
                }
            );
            assert_eq!(transport.human_generation(), 0);
            assert_eq!(transport.snapshot().screen_seq, 0);
            assert!(other.take_input_drain().is_none());
            assert!(transport.take_input_drain().is_some());
            assert!(transport.take_input_drain().is_none());
            assert!(other.take_input_drain().is_none());
        });
    }

    #[test]
    fn step5_drain_d13_mutating_entrypoints_invalidate_before_success_or_later_failure() {
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
                let mut transport = transport(32, 32, 32);
                h.seal(&mut transport);
                let mut human = Reader::new(Site::Human, &h.trace, []);
                let mut pty = Pty::new(&h.trace);
                let mut output = Output::new(&h.trace);
                match operation {
                    "tick" => {
                        transport
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
                            transport
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
                        transport
                            .resize(&mut pty, TerminalSize { rows: 25, cols: 80 })
                            .unwrap();
                    }
                    "resize-invalid" => {
                        assert!(
                            transport
                                .resize(&mut pty, TerminalSize { rows: 0, cols: 80 })
                                .is_err()
                        );
                    }
                    "resize-error" => {
                        pty.resize_error = true;
                        assert!(
                            transport
                                .resize(&mut pty, TerminalSize { rows: 25, cols: 80 })
                                .is_err()
                        );
                    }
                    "prefetch" => {
                        assert_eq!(transport.prefetch_child_bytes(&mut pty).unwrap(), 0);
                    }
                    "prefetch-data" => {
                        pty.reader.steps.push_back(Read::Data(b"tail".to_vec()));
                        assert_eq!(transport.prefetch_child_bytes(&mut pty).unwrap(), 4);
                    }
                    "flush" => {
                        assert_eq!(transport.flush_display(&mut output).unwrap(), 0);
                    }
                    "flush-error" => {
                        output.flush_error = Some(ErrorKind::BrokenPipe);
                        assert!(transport.flush_display(&mut output).is_err());
                    }
                    "discard" => {
                        assert_eq!(transport.discard_display_tail(), 0);
                    }
                    "drain-interrupted" => {
                        human.steps.push_back(Read::Interrupted);
                        assert_pending(
                            h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until),
                            &mut transport,
                        );
                    }
                    "drain-error" => {
                        human.steps.push_back(Read::Error(ErrorKind::BrokenPipe));
                        assert!(
                            h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                                .is_err()
                        );
                    }
                    "drain-expired" => {
                        assert_deadline(
                            h.drain(
                                &mut transport,
                                &mut human,
                                &mut pty,
                                &mut output,
                                Instant::now(),
                            ),
                            &mut transport,
                        );
                    }
                    _ => unreachable!("fixed mutation matrix"),
                }
                assert!(transport.take_input_drain().is_none(), "{operation}");
            });
        }
    }

    #[test]
    fn step5_drain_d14_original_expiry_prevents_later_io_and_proof_while_retaining_completed_reads()
    {
        with_harness(|h| {
            let mut transport = transport(32, 32, 32);
            h.seal(&mut transport);
            h.reset_observations();
            let mut human = Reader::new(Site::Human, &h.trace, []);
            let mut pty = Pty::new(&h.trace);
            let mut output = Output::new(&h.trace);
            assert_deadline(
                h.drain(
                    &mut transport,
                    &mut human,
                    &mut pty,
                    &mut output,
                    Instant::now(),
                ),
                &mut transport,
            );
            assert!(h.trace.borrow().is_empty());
            assert!(human.calls.is_empty() && pty.reader.calls.is_empty());
            assert!(pty.offered.is_empty() && output.offered.is_empty());
            assert_eq!(output.flushes, 0);
        });
        for fails in [false, true] {
            with_harness(|h| {
                h.control();
                let mut transport = transport(32, 32, 32);
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
                let mut pty = Pty::new(&h.trace);
                let mut output = Output::new(&h.trace);
                let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, until);
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
                    assert_eq!(transport.human_generation(), 0);
                    assert!(transport.take_input_drain().is_none());
                } else {
                    assert_deadline(result, &mut transport);
                    assert_eq!(call.count, Some(4));
                    assert_eq!(transport.human_generation(), 4);
                    assert_eq!(transport.pending_input_bytes(), 4);
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
                contents: screen.contents.clone(),
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
                Err(CafleetError::Value("fixture observation stopped".into()))
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
        transport: &mut Transport,
        endpoints: (&mut Reader, &mut Pty, &mut Output),
        child_bytes: &[u8],
        prefix_sequence: u64,
    ) {
        let (human, pty, output) = endpoints;
        let generation = transport.human_generation();
        let callbacks = h.observations.screens.borrow().len();
        assert_healthy(h);
        let first = h.drain(transport, human, pty, output, h.until);
        let first_proof = transport.take_input_drain().is_some();
        assert_eq!(transport.snapshot().screen_seq, prefix_sequence);
        assert_eq!(h.observations.screens.borrow().len(), callbacks);
        assert_eq!(output.bytes, child_bytes);
        assert_eq!(transport.pending_display_bytes(), 0);

        human.steps.push_back(Read::Data(b"manual\r".to_vec()));
        pty.reader.steps.push_back(Read::Data(b"live\r\n".to_vec()));
        let progress = transport
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
        assert_eq!(transport.human_generation(), generation + 7);
        assert_eq!(pty.bytes, b"typedmanual\r");
        let mut displayed = child_bytes.to_vec();
        displayed.extend_from_slice(b"live\r\n");
        assert_eq!(output.bytes, displayed);
        assert_eq!(transport.snapshot().screen_seq, prefix_sequence + 6);
        assert_healthy(h);
        let after_tick = h.drain(transport, human, pty, output, h.until);
        let tick_proof = transport.take_input_drain().is_some();

        transport
            .resize(pty, TerminalSize { rows: 25, cols: 80 })
            .unwrap();
        assert_eq!(transport.snapshot().screen_seq, prefix_sequence + 7);
        assert_healthy(h);
        let after_resize = h.drain(transport, human, pty, output, h.until);
        let resize_proof = transport.take_input_drain().is_some();
        let outcomes = [
            (first, first_proof),
            (after_tick, tick_proof),
            (after_resize, resize_proof),
        ];
        eprintln!("observation-incomplete later quiet/tick/resize outcomes: {outcomes:?}");
        assert!(
            outcomes
                .iter()
                .all(|(result, proof)| matches!(result, Ok(InputDrain::Pending)) && !proof),
            "an incomplete observation stays fenced for this Transport lifetime: {outcomes:?}"
        );
    }

    #[test]
    fn step5_drain_incomplete_delayed_positive_child_read_keeps_display_and_lifetime_fence() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            h.seal(&mut transport);
            h.reset_observations();
            let child_bytes = b"unseen\r\nsuffix";
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, child_bytes, child_bytes.len());
            let until = Instant::now() + Duration::from_millis(100);
            pty.reader.delay_until = Some(until);
            let mut output = Output::new(&h.trace);
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, until);
            assert_deadline(result, &mut transport);
            assert_eq!(human.bytes, b"typed");
            assert_eq!(transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.calls.len(), 1);
            let call = &pty.reader.calls[0];
            assert!(call.entered < until && call.returned >= until);
            assert_eq!(call.count, Some(child_bytes.len()));
            assert_eq!(pty.reader.bytes, child_bytes);
            assert_eq!(transport.pending_display_bytes(), child_bytes.len());
            assert_eq!(transport.snapshot().screen_seq, 0);
            assert_eq!(transport.snapshot().contents, "");
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
                &mut transport,
                (&mut human, &mut pty, &mut output),
                child_bytes,
                0,
            );
        });
    }

    const CONTROL_SEPARATED_CHILD: &[u8] = b"head\rTAIL\r\nsuffix";

    fn callback_drain(
        h: &mut Harness,
        transport: &mut Transport,
        endpoints: (&mut Reader, &mut Pty, &mut Output),
        fault: CallbackFault,
        until: Instant,
        records: &Rc<RefCell<Vec<CallbackRecord>>>,
    ) -> Result<InputDrain> {
        let (human, pty, output) = endpoints;
        transport.quiesce_for_input(
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
    fn step5_drain_incomplete_complete_control_observes_every_fragment_and_can_seal() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(
                Site::Child,
                &h.trace,
                CONTROL_SEPARATED_CHILD,
                CONTROL_SEPARATED_CHILD.len(),
            );
            let mut output = Output::new(&h.trace);
            let records = Rc::default();
            let until = h.until;
            let result = callback_drain(
                h,
                &mut transport,
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
            assert_eq!(transport.pending_display_bytes(), 0);
            assert_eq!(transport.snapshot().contents, "TAIL\nsuffix");
            assert_eq!(transport.snapshot().screen_seq, 17);
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
            assert!(transport.take_input_drain().is_some());
        });
    }

    fn incomplete_callback_case(fault: CallbackFault) {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            h.seal(&mut transport);
            h.reset_observations();
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(
                Site::Child,
                &h.trace,
                CONTROL_SEPARATED_CHILD,
                CONTROL_SEPARATED_CHILD.len(),
            );
            let mut output = Output::new(&h.trace);
            let until = Instant::now() + Duration::from_millis(100);
            let records = Rc::default();
            let result = callback_drain(
                h,
                &mut transport,
                (&mut human, &mut pty, &mut output),
                fault,
                until,
                &records,
            );
            if matches!(fault, CallbackFault::Expire) {
                assert_deadline(result, &mut transport);
            } else {
                assert_eq!(
                    format!("{:?}", result.unwrap_err()),
                    "Value(\"fixture observation stopped\")"
                );
                assert!(transport.take_input_drain().is_none());
            }
            assert_eq!(transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(pty.reader.calls[0].count, Some(17));
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(transport.pending_display_bytes(), 17);
            assert!(output.bytes.is_empty() && output.offered.is_empty());
            assert_eq!(output.flushes, 0);
            assert_eq!(h.observations.health_reads.get(), 0);
            assert_eq!(transport.snapshot().screen_seq, 5);
            assert_eq!(transport.snapshot().contents, "head");
            assert_eq!(transport.snapshot().cursor_row, 0);
            assert_eq!(transport.snapshot().cursor_col, 0);
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
                &mut transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                5,
            );
        });
    }

    #[test]
    fn step5_drain_incomplete_mid_observer_expiry_preserves_prefix_and_lifetime_fence() {
        incomplete_callback_case(CallbackFault::Expire);
    }

    #[test]
    fn step5_drain_incomplete_mid_observer_error_before_expiry_keeps_primary_and_fence() {
        incomplete_callback_case(CallbackFault::Fail);
    }

    #[test]
    fn step5_drain_incomplete_mid_observer_error_after_expiry_keeps_primary_and_fence() {
        incomplete_callback_case(CallbackFault::ExpireAndFail);
    }

    fn incomplete_tick_case(fatal: bool) {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            let mut output = Output::new(&h.trace);
            let records = Rc::new(RefCell::new(Vec::new()));
            let result = transport.tick(
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
            assert_eq!(transport.human_generation(), 5);
            assert_eq!(pty.bytes, b"typed");
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(pty.reader.calls.len(), 1);
            assert_eq!(pty.reader.calls[0].count, Some(17));
            assert!(transport.take_input_drain().is_none());
            let prefix = if fatal {
                let source = match result {
                    Err(source) => source,
                    Ok(_) => panic!("fatal ordered observation returns its actual error"),
                };
                assert_eq!(
                    format!("{source:?}"),
                    "Value(\"fixture observation stopped\")"
                );
                assert_eq!(*h.observations.screens.borrow(), vec![4, 5]);
                assert_eq!(records.borrow().len(), 2);
                assert_eq!(transport.snapshot().contents, "head");
                assert!(output.bytes.is_empty() && output.offered.is_empty());
                assert_eq!(
                    transport.pending_display_bytes(),
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
                assert_eq!(transport.snapshot().contents, "TAIL\nsuffix");
                assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
                assert_eq!(transport.pending_display_bytes(), 0);
                17
            };
            assert_eq!(transport.snapshot().screen_seq, prefix);
            assert_incomplete_observation_stays_fenced(
                h,
                &mut transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                prefix,
            );
        });
    }

    #[test]
    fn step5_drain_incomplete_ordinary_fatal_observer_error_retains_chunk_and_fence() {
        incomplete_tick_case(true);
    }

    #[test]
    fn step5_drain_incomplete_ordinary_nonfatal_observer_error_retains_reporting_and_fence() {
        incomplete_tick_case(false);
    }

    struct OneParserFailure(std::sync::atomic::AtomicBool);

    impl ObservationHooks for OneParserFailure {
        fn before(&self, _: ObservationSite, operation: ObservationOperation) -> Result<()> {
            if matches!(operation, ObservationOperation::Parser)
                && self.0.swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                Err(CafleetError::Value(
                    "fixture parser boundary stopped".into(),
                ))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn step5_drain_incomplete_ordinary_parser_fault_cannot_be_cleared_by_later_success() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            transport
                .set_observation_hooks(Arc::new(OneParserFailure(
                    std::sync::atomic::AtomicBool::new(true),
                )))
                .unwrap();
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            let mut output = Output::new(&h.trace);
            let progress = transport
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
            assert_eq!(transport.snapshot().screen_seq, 0);
            assert_eq!(transport.snapshot().contents, "");
            assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(transport.human_generation(), 5);
            assert_incomplete_observation_stays_fenced(
                h,
                &mut transport,
                (&mut human, &mut pty, &mut output),
                CONTROL_SEPARATED_CHILD,
                0,
            );
        });
    }

    #[test]
    fn step5_drain_incomplete_complete_observation_then_display_error_preserves_eligibility() {
        for quiesce in [false, true] {
            for discard in [false, true] {
                with_harness(|h| {
                    h.control();
                    let mut transport = transport(64, 64, 64);
                    let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
                    let mut pty = Pty::new(&h.trace);
                    pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
                    let mut output = Output::new(&h.trace);
                    output.write_error = Some(ErrorKind::BrokenPipe);
                    let result = if quiesce {
                        h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until)
                            .map(|_| ())
                    } else {
                        transport
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
                    assert_eq!(transport.human_generation(), 5);
                    assert_eq!(transport.snapshot().screen_seq, 17);
                    assert_eq!(transport.snapshot().contents, "TAIL\nsuffix");
                    assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
                    assert_eq!(transport.pending_display_bytes(), 17);
                    assert!(output.bytes.is_empty());
                    assert!(transport.take_input_drain().is_none());
                    output.write_error = None;
                    if discard {
                        assert_eq!(transport.discard_display_tail(), 17);
                    }
                    assert_healthy(h);
                    let result =
                        h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
                    assert_eq!(
                        result.unwrap(),
                        InputDrain::Quiescent {
                            human_generation: 5,
                            output_seq: 17
                        }
                    );
                    assert_eq!(transport.pending_display_bytes(), 0);
                    assert_eq!(
                        output.bytes,
                        if discard {
                            b"".as_slice()
                        } else {
                            CONTROL_SEPARATED_CHILD
                        }
                    );
                    assert!(transport.take_input_drain().is_some());
                });
            }
        }
    }

    #[test]
    fn step5_drain_incomplete_prefetch_consumption_control_observes_once_and_can_seal() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            pty.reader = Reader::data(Site::Child, &h.trace, CONTROL_SEPARATED_CHILD, 17);
            let mut output = Output::new(&h.trace);
            assert_eq!(transport.prefetch_child_bytes(&mut pty).unwrap(), 17);
            assert_eq!(transport.snapshot().screen_seq, 0);
            assert!(h.observations.screens.borrow().is_empty());
            let result = h.drain(&mut transport, &mut human, &mut pty, &mut output, h.until);
            assert_eq!(
                result.unwrap(),
                InputDrain::Quiescent {
                    human_generation: 5,
                    output_seq: 17
                }
            );
            assert_eq!(*h.observations.screens.borrow(), vec![4, 5, 9, 10, 11, 17]);
            assert_eq!(output.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(transport.pending_display_bytes(), 0);
            assert!(transport.take_input_drain().is_some());
        });
    }

    #[test]
    fn step5_drain_incomplete_discarding_unobserved_prefetch_keeps_lifetime_fence() {
        with_harness(|h| {
            h.control();
            let mut transport = transport(64, 64, 64);
            let mut human = Reader::data(Site::Human, &h.trace, b"typed", 5);
            let mut pty = Pty::new(&h.trace);
            let mut output = Output::new(&h.trace);
            let progress = transport
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
            assert_eq!(transport.prefetch_child_bytes(&mut pty).unwrap(), 17);
            assert_eq!(pty.reader.calls.last().unwrap().count, Some(17));
            assert_eq!(pty.reader.bytes, CONTROL_SEPARATED_CHILD);
            assert_eq!(transport.discard_display_tail(), 17);
            assert_eq!(transport.pending_display_bytes(), 0);
            assert_eq!(transport.snapshot().screen_seq, 0);
            assert!(h.observations.screens.borrow().is_empty());
            assert!(output.bytes.is_empty());
            assert_incomplete_observation_stays_fenced(
                h,
                &mut transport,
                (&mut human, &mut pty, &mut output),
                b"",
                0,
            );
        });
    }
}
