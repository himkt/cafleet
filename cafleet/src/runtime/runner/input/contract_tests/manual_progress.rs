use super::super::*;
use crate::broker::runtime::{self as storage, RunStatus};
use crate::coding_agent::ProfileCatalog;
use crate::error::CafleetError;
use crate::runtime::runner::*;
use crate::runtime::runner_tests::{Fixture, owned_child::OwnedChild};
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct UnusedTokens;

impl RequestTokenSource for UnusedTokens {
    fn fill(&mut self, _: &mut [u8; 32]) -> Result<()> {
        panic!("manual progress obtains no request token")
    }
}

#[derive(Default)]
struct Human {
    queued: VecDeque<u8>,
    consumed: Vec<u8>,
}

impl ByteInput for Human {
    fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        assert!(!bytes.is_empty());
        if self.queued.is_empty() {
            return Err(ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(self.queued.len());
        for byte in &mut bytes[..count] {
            *byte = self.queued.pop_front().expect("queued human byte");
        }
        self.consumed.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
}

#[derive(Default)]
struct Pty {
    blocked: bool,
    failure: Option<ErrorKind>,
    offered: Vec<Vec<u8>>,
    written: Vec<u8>,
    reads: usize,
}

impl ByteInput for Pty {
    fn try_read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        Err(ErrorKind::WouldBlock.into())
    }
}

impl PtyIo for Pty {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(!bytes.is_empty());
        self.offered.push(bytes.to_vec());
        if let Some(kind) = self.failure {
            return Err(io::Error::new(kind, "manual progress PTY failure"));
        }
        if self.blocked {
            return Err(ErrorKind::WouldBlock.into());
        }
        self.written.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn resize(&mut self, _: TerminalSize) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

struct Output;

impl Display for Output {
    fn try_write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("the quiet child supplies no display bytes")
    }

    fn try_flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct NativeChild {
    child: OwnedChild,
    until: Instant,
}

impl NativeChild {
    fn spawn(until: Instant) -> Self {
        Self {
            child: OwnedChild::new(
                Command::new("/bin/cat")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            ),
            until,
        }
    }
}

impl Drop for NativeChild {
    fn drop(&mut self) {
        self.child.terminate_until(self.until);
    }
}

struct Harness {
    fixture: Fixture,
    _artifacts: RunArtifacts,
    lock: Arc<RunLock>,
    writer: EventWriter,
    controller: ObservationController,
    transport: Transport,
    child: NativeChild,
    until: Instant,
}

impl Harness {
    fn new() -> Self {
        let until = Instant::now() + Duration::from_secs(4);
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let lock = Arc::new(
            RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .expect("original run lock"),
        );
        lock.transfer_to_runner(
            &mut fixture.conn,
            &fixture.run.owner(),
            ProcessIdentity::current().unwrap(),
        )
        .unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        storage::reserve_event_sequences(&mut fixture.conn, &fixture.run.owner()).unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let mut controller = ObservationController::new(
            &fixture.run,
            &ProfileCatalog::bundled().unwrap(),
            storage::evidence::EvidencePolicy::metadata(),
            1..=256,
        )
        .unwrap();
        let mut writer = EventWriter::start(
            format!("sqlite:///{}", fixture.conn.path().unwrap()),
            fixture.run.clone(),
            lock.clone(),
            artifacts.paths.outbox.clone(),
        )
        .unwrap();
        writer.set_deadline(until);
        let child = NativeChild::spawn(until);
        let identity = ProcessIdentity::read(i64::from(child.child.id()))
            .unwrap()
            .expect("owned live native child");
        assert!(identity.matches().unwrap());
        fixture.conn.execute(
            "UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",
            rusqlite::params![identity.pid, identity.start_identity, crate::time::format_utc(crate::time::now_utc()), fixture.run.run_id],
        ).unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        assert_eq!(fixture.run.status, RunStatus::Running);
        let transport = Transport::new(
            TerminalSize { rows: 24, cols: 80 },
            TransportLimits {
                input_bytes: 64,
                display_bytes: 64,
                bytes_per_tick: 64,
            },
        )
        .unwrap();
        controller
            .started(
                &fixture.run,
                &transport.snapshot(),
                &writer,
                &RealObservationClock,
            )
            .unwrap();
        Self {
            fixture,
            _artifacts: artifacts,
            lock,
            writer,
            controller,
            transport,
            child,
            until,
        }
    }

    fn coordinator(&mut self) -> InputCoordinator {
        InputCoordinator::new(
            &mut self.fixture.conn,
            &self.fixture.run,
            self.lock.clone(),
            InputServices {
                catalog: ProfileCatalog::bundled().unwrap(),
                token_source: Box::new(UnusedTokens),
                hooks: Arc::new(InertInputHooks),
            },
            self.until,
        )
        .unwrap()
    }

    fn quiet_control(&mut self) {
        loop {
            self.controller
                .poll(
                    &self.transport.snapshot(),
                    &self.writer,
                    &RealObservationClock,
                )
                .unwrap();
            let health = self.controller.health(&self.writer);
            if !health.degraded && health.pending_count == 0 && !health.pending_evidence {
                break;
            }
            assert!(Instant::now() < self.until, "writer health: {health:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            self.transport
                .quiesce_for_input(
                    &mut Human::default(),
                    &mut Pty::default(),
                    &mut Output,
                    &mut self
                        .controller
                        .observer(&self.writer, &RealObservationClock),
                    self.until,
                )
                .unwrap(),
            InputDrain::Quiescent {
                human_generation: 0,
                output_seq: 0,
            }
        );
    }

    fn finish(&mut self) {
        let counts = self.writer.finish(self.until);
        let status = self.writer.status();
        self.child.child.terminate_until(self.until);
        let reaped = self.child.child.try_wait();
        assert_eq!(counts, (0, 0), "writer cleanup: {status:?}");
        assert!(
            !status.degraded && status.diagnostic.is_none(),
            "writer finish confirms join or reports degradation: {status:?}"
        );
        assert!(reaped.unwrap().is_some(), "owned native child was reaped");
    }
}

fn with_harness(body: impl FnOnce(&mut Harness)) {
    let mut harness = Harness::new();
    let primary = catch_unwind(AssertUnwindSafe(|| body(&mut harness)));
    let cleanup = catch_unwind(AssertUnwindSafe(|| harness.finish()));
    if let Err(primary) = primary {
        if let Err(cleanup) = &cleanup {
            eprintln!("manual-progress cleanup also failed: {cleanup:?}");
        }
        resume_unwind(primary);
    }
    if let Err(cleanup) = cleanup {
        resume_unwind(cleanup);
    }
}

#[test]
fn step5_manual_progress_counts_only_new_reads_across_pending_zero_and_successive_steps() {
    with_harness(|h| {
        h.quiet_control();
        let mut coordinator = h.coordinator();
        assert_eq!(coordinator.manual_generation, 0);
        let mut human = Human::default();
        human.queued.extend(b"first");
        let mut pty = Pty {
            blocked: true,
            ..Pty::default()
        };
        let mut calls = 0;
        let result = coordinator
            .progress_with_manual(&mut h.transport, |transport| {
                calls += 1;
                transport.quiesce_for_input(
                    &mut human,
                    &mut pty,
                    &mut Output,
                    &mut h.controller.observer(&h.writer, &RealObservationClock),
                    h.until,
                )
            })
            .unwrap();
        assert_eq!(result, InputDrain::Pending);
        assert_eq!(calls, 1);
        assert_eq!(human.consumed, b"first");
        assert_eq!(h.transport.human_generation(), 5);
        assert_eq!(coordinator.manual_generation, 5);
        assert_eq!(h.transport.pending_input_bytes(), 5);
        assert!(pty.written.is_empty());

        let zero = coordinator
            .progress_with_manual(&mut h.transport, |transport| {
                calls += 1;
                transport.tick(
                    &mut human,
                    &mut pty,
                    &mut Output,
                    &mut h.controller.observer(&h.writer, &RealObservationClock),
                )
            })
            .unwrap();
        assert_eq!(zero.human_bytes, 0);
        assert_eq!(calls, 2);
        assert_eq!(h.transport.human_generation(), 5);
        assert_eq!(coordinator.manual_generation, 5);
        assert_eq!(h.transport.pending_input_bytes(), 5);

        human.queued.extend(b"+next");
        pty.blocked = false;
        let next = coordinator
            .progress_with_manual(&mut h.transport, |transport| {
                calls += 1;
                transport.tick(
                    &mut human,
                    &mut pty,
                    &mut Output,
                    &mut h.controller.observer(&h.writer, &RealObservationClock),
                )
            })
            .unwrap();
        assert_eq!(next.human_bytes, 5);
        assert_eq!(calls, 3);
        assert_eq!(human.consumed, b"first+next");
        assert_eq!(h.transport.human_generation(), 10);
        assert_eq!(coordinator.manual_generation, 10);
        assert_eq!(h.transport.pending_input_bytes(), 0);
        assert_eq!(pty.written, b"first+next");
    });
}

#[test]
fn step5_manual_progress_accounts_real_human_read_before_returning_the_pty_failure() {
    with_harness(|h| {
        h.quiet_control();
        let mut coordinator = h.coordinator();
        let mut human = Human::default();
        human.queued.extend(b"typed");
        let mut pty = Pty {
            failure: Some(ErrorKind::BrokenPipe),
            ..Pty::default()
        };
        let mut calls = 0;
        let result = coordinator.progress_with_manual(&mut h.transport, |transport| {
            calls += 1;
            transport.tick(
                &mut human,
                &mut pty,
                &mut Output,
                &mut h.controller.observer(&h.writer, &RealObservationClock),
            )
        });
        let failure = match result {
            Ok(_) => panic!("actual PTY failure remains the progress result"),
            Err(failure) => failure,
        };
        assert_eq!(calls, 1);
        assert_eq!(human.consumed, b"typed");
        assert_eq!(h.transport.human_generation(), 5);
        assert_eq!(coordinator.manual_generation, 5);
        assert_eq!(h.transport.pending_input_bytes(), 5);
        assert_eq!(pty.offered, vec![b"typed".to_vec()]);
        assert!(pty.written.is_empty());
        assert_eq!(pty.reads, 0);
        assert!(matches!(&failure.source, CafleetError::App(message)
            if message == "member runtime: manual progress PTY failure"));
        assert_eq!(failure.source.exit_code(), 1);
        assert_eq!(failure.run_id, Some(h.fixture.run.run_id));
        assert_eq!(failure.interaction_id, None);
        assert_eq!(failure.input_id, None);
        assert_eq!(failure.operation_key, None);
    });
}

#[test]
fn step5_manual_progress_composes_only_the_constructed_accounting_error_branches() {
    let only = combine_manual_progress(
        Ok(17),
        Err(CafleetError::DatabaseBusy("accounting only".into())),
    )
    .expect_err("accounting error remains visible after successful progress");
    assert!(matches!(&only, CafleetError::DatabaseBusy(message) if message == "accounting only"));
    assert_eq!(only.exit_code(), 1);
    let both = combine_manual_progress::<()>(
        Err(CafleetError::Usage("primary progress".into())),
        Err(CafleetError::DatabaseBusy("secondary accounting".into())),
    )
    .expect_err("both errors remain visible");
    assert_eq!(both.exit_code(), 2);
    match both {
        CafleetError::WithDiagnostics { cause, diagnostics } => {
            assert!(
                matches!(*cause, CafleetError::Usage(ref message) if message == "primary progress")
            );
            assert_eq!(diagnostics, vec!["secondary accounting"]);
        }
        other => {
            panic!("dual errors retain the primary cause and accounting diagnostic: {other:?}")
        }
    }
}
