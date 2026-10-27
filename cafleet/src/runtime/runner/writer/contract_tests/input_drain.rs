use super::support::Clock;
use super::worker::{Hooks, Release};
use super::*;
use crate::coding_agent::ProfileCatalog;
use std::cell::{Cell, RefCell};
use std::io::{self, ErrorKind};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

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
        let mut writer = super::worker::start(&fixture, hooks.clone());
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
            self.writer.thread.is_none(),
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
    for (bytes, width, expected_reads, expected_bytes) in [(64, 1, 8, 8), (4, usize::MAX, 1, 4)] {
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
        let result =
            transport.quiesce_for_input(&mut human, &mut pty, &mut output, &mut observer, h.until);
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
            serde_json::to_value(h.observations.last_health.borrow().as_ref().unwrap()).unwrap(),
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
fn step5_drain_d14_original_expiry_prevents_later_io_and_proof_while_retaining_completed_reads() {
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
