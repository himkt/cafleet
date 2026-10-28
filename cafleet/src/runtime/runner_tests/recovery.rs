use crate::broker::records::Placement;
use crate::config::Settings;
use crate::error::CafleetError;
use serde_json::Value;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;

mod crash;
mod deletion;
mod foundations;
mod history;
mod identity;
mod missing_paths;
mod replay;
mod review_clock;
mod review_completion;
mod review_descriptor;
mod review_history;
mod review_relationships;
mod terminal_tail;

type TestResult<T> = std::result::Result<T, CafleetError>;
const NOW: &str = "2026-10-26T20:00:00.000000+00:00";

struct Case {
    fixture: Fixture,
    artifacts: RunArtifacts,
    held: Option<Arc<RunLock>>,
    settings: Settings,
    placement: Placement,
}

impl Case {
    fn new() -> Self {
        let mut case = Self::creator();
        case.held
            .as_ref()
            .unwrap()
            .transfer_to_runner(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
        case.fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2",
                rusqlite::params![NOW, case.fixture.run.run_id],
            )
            .unwrap();
        case.refresh();
        storage::reserve_event_sequences(&mut case.fixture.conn, &case.fixture.run.owner())
            .unwrap();
        case.refresh();
        case
    }

    fn creator() -> Self {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let held = Arc::new(
            RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap(),
        );
        let settings = Settings {
            database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
            max_text_len: 200,
            multiplexer: None,
        };
        let placement =
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
        Self {
            fixture,
            artifacts,
            held: Some(held),
            settings,
            placement,
        }
    }

    fn seed(&self, transitions: &[ObservedTransition]) {
        let held = self
            .held
            .as_ref()
            .expect("the original owner holds its runtime lock while seeding");
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut spool = Outbox::open_owned(
            &self.fixture.conn,
            &self.fixture.run,
            held.clone(),
            deadline,
        )
        .unwrap();
        for transition in transitions {
            spool.append_synced(transition, deadline).unwrap();
        }
    }

    fn replace_owned_bytes(&self, bytes: &[u8]) {
        self.held
            .as_ref()
            .expect("owned spool mutation requires its original lock")
            .validate(&self.fixture.conn, &self.fixture.run.owner())
            .unwrap();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.artifacts.paths.outbox)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }

    fn release(&mut self) {
        self.held.take();
    }

    fn refresh(&mut self) {
        self.fixture.run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
    }

    fn recover(&mut self, host: &mut Host) -> TestResult<RecoveryReport> {
        self.release();
        recover_member_with_host(
            &mut self.fixture.conn,
            &self.settings,
            self.fixture.run.member_id,
            None,
            host.deadline,
            host,
        )
    }

    fn spool_bytes(&self) -> Option<Vec<u8>> {
        let run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
        let current = ProcessIdentity::current().unwrap();
        assert_eq!(
            run.owner_pid, current.pid,
            "the current process owns the spool"
        );
        assert_eq!(run.owner_start_identity, current.start_identity);
        let acquired;
        let held = match &self.held {
            Some(held) => held.as_ref(),
            None => {
                acquired = RunLock::acquire(&self.fixture.conn, &run.owner())
                    .unwrap()
                    .expect("the actual current owner reacquires its original matching lock");
                &acquired
            }
        };
        held.validate(&self.fixture.conn, &run.owner()).unwrap();
        match std::fs::read(&self.artifacts.paths.outbox) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("owned spool snapshot failed: {error}"),
        }
    }
}

#[derive(Clone, Copy)]
enum Evidence {
    Alive,
    Terminated,
    Unknown,
}
impl Evidence {
    fn value(self) -> ProcessEvidence {
        match self {
            Self::Alive => ProcessEvidence::Alive,
            Self::Terminated => ProcessEvidence::Terminated,
            Self::Unknown => {
                ProcessEvidence::Unknown("bounded injected identity uncertainty".into())
            }
        }
    }
}

type CheckpointAction = Box<dyn FnMut(&str, &RunRecord, Instant) -> TestResult<()>>;
struct Host {
    deadline: Instant,
    process: Evidence,
    startup: Evidence,
    matching_health: bool,
    cleanup: CleanupState,
    calls: Vec<String>,
    action: Option<CheckpointAction>,
}
impl Host {
    fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            process: Evidence::Terminated,
            startup: Evidence::Terminated,
            matching_health: false,
            cleanup: CleanupState::Complete,
            calls: Vec::new(),
            action: None,
        }
    }
    fn check(&self, deadline: Instant) -> TestResult<()> {
        assert_eq!(
            deadline, self.deadline,
            "all evidence/checkpoints share one original deadline"
        );
        if Instant::now() >= deadline {
            Err(CafleetError::DeadlineExceeded)
        } else {
            Ok(())
        }
    }
}
impl RecoveryHost for Host {
    fn current_process(&mut self) -> TestResult<ProcessIdentity> {
        self.calls.push("current_process".into());
        ProcessIdentity::current()
    }
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        deadline: Instant,
    ) -> TestResult<ProcessEvidence> {
        self.check(deadline)?;
        self.calls.push(format!(
            "process:{}:{}",
            process.pid, process.start_identity
        ));
        Ok(self.process.value())
    }
    fn startup_process_evidence(
        &mut self,
        run: &RunRecord,
        placement: Option<&Placement>,
        deadline: Instant,
    ) -> TestResult<ProcessEvidence> {
        self.check(deadline)?;
        assert_eq!(run.owner_kind, OwnerKind::Creator);
        assert!(placement.is_some());
        self.calls.push("startup".into());
        Ok(self.startup.value())
    }
    fn health(&mut self, run: &RunRecord, deadline: Instant) -> TestResult<RecoveryHealth> {
        self.check(deadline)?;
        self.calls.push("health".into());
        if self.matching_health {
            Ok(RecoveryHealth::Matching(HealthSnapshot {
                identity: ControlIdentity::from_run(run),
                runner: ProcessIdentity {
                    pid: run.runner_pid.unwrap(),
                    start_identity: run.runner_start_identity.clone().unwrap(),
                },
                child: None,
                output_seq: 1,
                observation: None,
            }))
        } else {
            Ok(RecoveryHealth::Unavailable(
                "bounded injected endpoint unavailable".into(),
            ))
        }
    }
    fn cleanup_owned(
        &mut self,
        _: &RunRecord,
        clock: ShutdownClock,
    ) -> TestResult<ShutdownOutcome> {
        assert!(clock.child_deadline() <= self.deadline);
        assert!(clock.display_deadline() <= self.deadline);
        assert!(clock.storage_deadline() <= self.deadline);
        self.calls.push("cleanup".into());
        Ok(ShutdownOutcome {
            native_exit: None,
            cleanup_state: self.cleanup,
            diagnostics: Vec::new(),
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        })
    }
    fn checkpoint(
        &mut self,
        boundary: RecoveryBoundary,
        run: &RunRecord,
        deadline: Instant,
    ) -> TestResult<()> {
        self.check(deadline)?;
        let name = match boundary {
            RecoveryBoundary::AfterSnapshot => "after_snapshot",
            RecoveryBoundary::BeforeTakeover => "before_takeover",
            RecoveryBoundary::AfterTakeover => "after_takeover",
            RecoveryBoundary::BeforeReplay => "before_replay",
            RecoveryBoundary::AfterCommit => "after_commit",
            RecoveryBoundary::BeforeCompact => "before_compact",
            RecoveryBoundary::BeforeFinish => "before_finish",
        };
        self.calls.push(name.into());
        assert!(
            self.calls.len() < 1024,
            "a bounded recovery pass makes finite progress"
        );
        if let Some(action) = &mut self.action {
            action(name, run, deadline)?;
        }
        Ok(())
    }
}

fn evidence(run: &RunRecord, sequence: i64) -> Value {
    storage::evidence::EvidencePolicy::metadata().apply(&json!({"backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,"rule_id":"recovery-fixture","screen_seq":sequence,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap()
}
fn opening(run: &RunRecord, sequence: i64, episode: i64) -> ObservedTransition {
    ObservedTransition {
        run_id: run.run_id,
        event_seq: sequence,
        episode_seq: Some(episode),
        kind: RuntimeEventKind::InteractionOpened,
        classification: Some(InteractionClass::PermissionRequest),
        observed_at: NOW.into(),
        evidence_json: evidence(run, sequence),
        close_reason: None,
        cause_json: None,
    }
}
fn obscured(run: &RunRecord, sequence: i64, episode: i64) -> ObservedTransition {
    ObservedTransition {
        kind: RuntimeEventKind::InteractionObscured,
        ..opening(run, sequence, episode)
    }
}
fn exited(
    run: &RunRecord,
    sequence: i64,
    code: Option<i64>,
    signal: Option<i64>,
) -> ObservedTransition {
    ObservedTransition {
        run_id: run.run_id,
        event_seq: sequence,
        episode_seq: None,
        kind: RuntimeEventKind::RunnerExited,
        classification: None,
        observed_at: NOW.into(),
        evidence_json: evidence(run, sequence),
        close_reason: None,
        cause_json: Some(json!({"kind":"native_exit","exit_code":code,"exit_signal":signal})),
    }
}
fn frame(transition: &ObservedTransition) -> Vec<u8> {
    let body = serde_json::to_vec(transition).unwrap();
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(&Sha256::digest(&body));
    bytes.extend(body);
    bytes
}
fn events(case: &Case) -> Vec<RuntimeEventView> {
    let ids = case
        .fixture
        .conn
        .prepare("SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq")
        .unwrap()
        .query_map([case.fixture.run.run_id], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    ids.into_iter()
        .map(|id| storage::get_event(&case.fixture.conn, id).unwrap())
        .collect()
}
fn counts(report: &RecoveryReport) -> (u64, u64, u64) {
    (
        report.replayed_count,
        report.inbox_count,
        report.archived_count,
    )
}
fn new_host() -> Host {
    Host::new(Instant::now() + Duration::from_secs(2))
}

#[derive(Debug, PartialEq)]
struct TableSnapshot {
    name: String,
    rows: Vec<Vec<rusqlite::types::Value>>,
}
fn rows(conn: &Connection) -> Vec<TableSnapshot> {
    let names = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let mut statement = conn
                .prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY rowid"))
                .unwrap();
            let width = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..width)
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            TableSnapshot { name, rows }
        })
        .collect()
}
