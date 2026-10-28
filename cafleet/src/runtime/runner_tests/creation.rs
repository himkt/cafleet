use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use super::*;
use crate::broker::records::{MemberStatus, Placement};
use crate::error::CafleetError;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::params;

#[derive(Clone, Copy)]
enum PublicationFailure {
    Missing,
    Rejected,
}

struct Host {
    database_url: String,
    now: Instant,
    delay: Duration,
    calls: Vec<&'static str>,
    deadlines: Vec<(&'static str, Instant)>,
    argv: Vec<String>,
    child_argv: Vec<String>,
    spawn_count: usize,
    fail: Option<&'static str>,
    failure_is_usage: bool,
    publication_failure: Option<PublicationFailure>,
    panic_at: Option<&'static str>,
    corrupt: Option<&'static str>,
    fast_exit: bool,
    cleanup_failure: bool,
    lock: Option<RunLock>,
}

impl Host {
    fn new(fixture: &Fixture) -> Self {
        Self {
            database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
            now: Instant::now(),
            delay: Duration::ZERO,
            calls: Vec::new(),
            deadlines: Vec::new(),
            argv: Vec::new(),
            child_argv: Vec::new(),
            spawn_count: 0,
            fail: None,
            failure_is_usage: false,
            publication_failure: None,
            panic_at: None,
            corrupt: None,
            fast_exit: false,
            cleanup_failure: false,
            lock: None,
        }
    }
    fn phase(&mut self, name: &'static str, deadline: Instant) -> Result<(), CafleetError> {
        self.calls.push(name);
        self.deadlines.push((name, deadline));
        self.now += self.delay;
        if self.fail == Some(name) {
            let message = format!("fixture {name} failure");
            Err(if self.failure_is_usage {
                CafleetError::Usage(message)
            } else {
                CafleetError::App(message)
            })
        } else {
            Ok(())
        }
    }
    fn identity(run: &RunRecord) -> ControlIdentity {
        ControlIdentity {
            database_identity_json: run.database_identity_json.clone(),
            fleet_id: run.fleet_id,
            member_id: run.member_id,
            director_member_id: run.director_member_id,
            run_id: run.run_id,
            owner_epoch: run.owner_epoch,
            launch_nonce: run.launch_nonce.clone(),
        }
    }
}

impl LaunchHost for Host {
    fn now(&self) -> Instant {
        self.now
    }
    fn current_executable(&self) -> Result<PathBuf, CafleetError> {
        Ok("/absolute/cafleet".into())
    }
    fn split_runner(
        &mut self,
        argv: &[String],
        environment: &[(String, String)],
        deadline: Instant,
    ) -> Result<String, CafleetError> {
        self.phase("split", deadline)?;
        self.argv = argv.to_vec();
        assert_eq!(argv[0], "/absolute/cafleet");
        assert_eq!(argv[1], "__member-run");
        assert_eq!(argv[2], "--descriptor");
        assert!(std::path::Path::new(&argv[3]).is_absolute());
        assert_eq!(argv.len(), 4);
        for (key, value) in environment {
            if key == "CAFLEET_DATABASE_URL" {
                assert_eq!(value, &self.database_url);
            }
            assert!(!value.contains("literal assignment"));
        }
        Ok("%9".into())
    }
    fn wait_prepared(
        &mut self,
        descriptor: &LaunchDescriptor,
        deadline: Instant,
    ) -> Result<Prepared, CafleetError> {
        self.phase("prepared", deadline)?;
        assert_ne!(self.panic_at, Some("prepared"), "fixture prepared unwind");
        self.child_argv = descriptor.spawn.argv.clone();
        let mut conn = crate::db::connect(&self.database_url).unwrap();
        let before = storage::get_run(&conn, descriptor.run_id).unwrap();
        assert_eq!(before.child_pid, None);
        assert_eq!(self.spawn_count, 0);
        let lock = RunLock::acquire(&conn, &before.owner()).unwrap().unwrap();
        let process = ProcessIdentity::current().unwrap();
        let owner = lock
            .transfer_to_runner(&mut conn, &before.owner(), process.clone())
            .unwrap();
        conn.execute(
            "UPDATE member_runs SET runner_pid=?1,runner_start_identity=?2 WHERE run_id=?3",
            params![process.pid, process.start_identity, owner.run_id],
        )
        .unwrap();
        let run = storage::get_run(&conn, owner.run_id).unwrap();
        self.lock = Some(lock);
        match self.publication_failure {
            Some(PublicationFailure::Missing) => {
                conn.execute(
                    "DELETE FROM member_placements WHERE member_id=?1",
                    [run.member_id],
                )
                .unwrap();
            }
            Some(PublicationFailure::Rejected) => {
                conn.execute_batch(
                    "CREATE TRIGGER reject_pending_publication
                     BEFORE UPDATE OF mux_pane_id ON member_placements
                     BEGIN SELECT RAISE(ABORT,'fixture publication blocked'); END",
                )
                .unwrap();
            }
            None => {}
        }
        let mut identity = Self::identity(&run);
        match self.corrupt {
            Some("prepared-nonce") => identity.launch_nonce = "b".repeat(64),
            Some("prepared-epoch") => identity.owner_epoch += 1,
            Some("prepared-run") => identity.run_id += 1,
            Some("prepared-director") => identity.director_member_id += 1,
            _ => {}
        }
        Ok(Prepared {
            identity,
            runner: process,
        })
    }
    fn start(
        &mut self,
        prepared: &Prepared,
        placement: &Placement,
        deadline: Instant,
    ) -> Result<Started, CafleetError> {
        self.phase("start", deadline)?;
        let mut conn = crate::db::connect(&self.database_url).unwrap();
        let run = storage::get_run(&conn, prepared.identity.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Prepared);
        let member = crate::broker::get_member(&conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            member.placement.as_ref().unwrap().mux_pane_id.as_deref(),
            Some("%9")
        );
        assert_eq!(placement.mux_pane_id.as_deref(), Some("%9"));
        self.spawn_count += 1;
        let child = ProcessIdentity {
            pid: run.owner_pid.checked_add(1_000_000).unwrap(),
            start_identity: "fixture-child-start".into(),
        };
        conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",params![child.pid,child.start_identity,"2026-10-26T11:00:00.000000+00:00",run.run_id]).unwrap();
        assert_ne!(self.panic_at, Some("started"), "fixture started unwind");
        if self.fast_exit {
            let run = storage::get_run(&conn, run.run_id).unwrap();
            storage::reserve_event_sequences(&mut conn, &run.owner()).unwrap();
            let evidence=crate::broker::runtime::evidence::EvidencePolicy::metadata().apply(&json!({"backend":"codex","native_version":"0.163.0","profile_version":null,"rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
            storage::persist_transition(
                &mut conn,
                &run.owner(),
                &ObservedTransition {
                    run_id: run.run_id,
                    event_seq: 1,
                    episode_seq: None,
                    kind: RuntimeEventKind::RunnerExited,
                    classification: None,
                    observed_at: "2026-10-26T11:00:00.000000+00:00".into(),
                    evidence_json: evidence,
                    close_reason: None,
                    cause_json: Some(
                        json!({"kind":"native_exit","exit_code":7,"exit_signal":null}),
                    ),
                },
                PersistenceMode::Live,
            )
            .unwrap();
        }
        if self.fail == Some("started-reply") {
            return Err(CafleetError::App(
                "fixture started reply lost after spawn".into(),
            ));
        }
        let mut identity = Self::identity(&storage::get_run(&conn, run.run_id).unwrap());
        let mut reply_child = child;
        if self.corrupt == Some("started-epoch") {
            identity.owner_epoch += 1;
        }
        if self.corrupt == Some("started-child") {
            reply_child.pid += 1;
        }
        Ok(Started {
            identity,
            runner: prepared.runner.clone(),
            child: reply_child,
        })
    }
    fn shutdown_owned(
        &mut self,
        run: &RunRecord,
        clock: ShutdownClock,
    ) -> Result<ShutdownOutcome, CafleetError> {
        self.calls.push("shutdown");
        assert_eq!(clock.storage_deadline(), self.now + Duration::from_secs(4));
        let conn = crate::db::connect(&self.database_url).unwrap();
        let current = storage::get_run(&conn, run.run_id).unwrap();
        assert_eq!(current.run_id, run.run_id);
        if self.cleanup_failure {
            return Err(CafleetError::App(
                "fixture owned-child cleanup failed".into(),
            ));
        }
        self.lock.take();
        Ok(ShutdownOutcome {
            native_exit: None,
            cleanup_state: CleanupState::Complete,
            diagnostics: Vec::new(),
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        })
    }
    fn close_pane(&mut self, pane: &str, deadline: Instant) -> Result<(), CafleetError> {
        self.calls.push("close");
        self.deadlines.push(("close", deadline));
        assert_eq!(pane, "%9");
        if self.cleanup_failure {
            Err(CafleetError::App("fixture pane cleanup failed".into()))
        } else {
            Ok(())
        }
    }
}

fn inputs(fixture: &Fixture) -> (PendingMemberSpec, RunSettings) {
    let mut placement = common::placement(None);
    placement.coding_agent = "codex".into();
    (
        PendingMemberSpec {
            fleet_id: fixture.run.fleet_id,
            name: "pending-worker".into(),
            description: "pending assignment".into(),
            placement,
        },
        RunSettings {
            database_identity_json: fixture.run.database_identity_json.clone(),
            creator: ProcessIdentity::current().unwrap(),
            launch_nonce: "c".repeat(64),
            requested_mode: None,
            resolved_mode: "never".into(),
            mode_source: ModeSource::Default,
            permission_argv_json: fixture.run.permission_argv_json.clone(),
            event_evidence: EventEvidence::Metadata,
            native_version: Some("0.163.0".into()),
            profile_version: None,
            coverage: Coverage::Unknown,
        },
    )
}

fn launch(
    fixture: &mut Fixture,
    host: &mut Host,
    deadline: Instant,
) -> Result<LaunchOutcome, CafleetError> {
    let (member, settings) = inputs(fixture);
    let spec = fixture.spec();
    create_registered(
        &mut fixture.conn,
        &member,
        &settings,
        |conn, pending| {
            Ok((
                RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?,
                spec,
            ))
        },
        host,
        deadline,
    )
}

fn pending_run(fixture: &Fixture) -> RunRecord {
    let id=fixture.conn.query_row("SELECT run_id FROM member_runs WHERE member_id IN (SELECT member_id FROM members WHERE name='pending-worker')",[],|row|row.get::<_,i64>(0)).unwrap();
    storage::get_run(&fixture.conn, id).unwrap()
}

fn assert_compensated(fixture: &Fixture, run: &RunRecord) {
    let status = fixture
        .conn
        .query_row(
            "SELECT status FROM members WHERE member_id=?1",
            [run.member_id],
            |row| row.get::<_, MemberStatus>(0),
        )
        .unwrap();
    assert_eq!(status, MemberStatus::Deregistered);
    assert_eq!(
        storage::get_run(&fixture.conn, run.run_id).unwrap().status,
        RunStatus::Failed
    );
    let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id).unwrap();
    assert!(!paths.descriptor.exists());
}

fn count_registration_cleanup(conn: &Connection) -> Arc<AtomicUsize> {
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    conn.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(
            context.action,
            AuthAction::Update {
                table_name: "members",
                column_name: "status"
            }
        ) {
            observed.fetch_add(1, Ordering::SeqCst);
        }
        Authorization::Allow
    }))
    .unwrap();
    attempts
}

fn reject_registration_cleanup(conn: &Connection) {
    conn.execute_batch(
        "CREATE TRIGGER reject_registration_cleanup BEFORE UPDATE OF status ON members
         BEGIN SELECT RAISE(ABORT,'fixture database cleanup blocked'); END",
    )
    .unwrap();
}

fn require_launch_error(result: Result<LaunchOutcome, CafleetError>) -> CafleetError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("fixture creation must fail"),
    }
}

#[test]
fn runner_creation_allocates_member_and_run_in_one_transaction_or_rolls_both_back() {
    let mut fixture = Fixture::new();
    let (member, settings) = inputs(&fixture);
    let before = fixture
        .conn
        .query_row("SELECT COUNT(*) FROM members", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    fixture.conn.execute_batch("CREATE TRIGGER fail_pending_run BEFORE INSERT ON member_runs BEGIN SELECT RAISE(ABORT,'pending run insert failure'); END").unwrap();
    assert!(
        register_pending_run(
            &mut fixture.conn,
            &member,
            &settings,
            Instant::now() + Duration::from_secs(10)
        )
        .is_err()
    );
    assert_eq!(
        fixture
            .conn
            .query_row("SELECT COUNT(*) FROM members", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        fixture
            .conn
            .query_row("SELECT COUNT(*) FROM member_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn runner_creation_guard_covers_preparation_failure_before_artifacts_after_allocation_and_after_descriptor()
 {
    for stage in ["before-artifacts", "after-artifacts", "after-descriptor"] {
        let mut fixture = Fixture::new();
        let (member, settings) = inputs(&fixture);
        let spec = fixture.spec();
        let mut host = Host::new(&fixture);
        let deadline = host.now + Duration::from_secs(10);
        let result = create_registered(
            &mut fixture.conn,
            &member,
            &settings,
            |conn, pending| {
                if stage == "before-artifacts" {
                    return Err(CafleetError::App("fixture preparation failure".into()));
                }
                let artifacts =
                    RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
                if stage == "after-descriptor" {
                    artifacts.write_descriptor(&LaunchDescriptor::new(
                        &storage::get_run(conn, pending.run.run_id)?,
                        spec,
                    )?)?;
                }
                Err(CafleetError::App("fixture preparation failure".into()))
            },
            &mut host,
            deadline,
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{stage} must compensate"),
        };
        assert!(error.message().contains("fixture preparation failure"));
        assert_compensated(&fixture, &pending_run(&fixture));
        assert!(host.calls.is_empty());
        assert_eq!(host.spawn_count, 0);
    }
}

#[test]
fn runner_creation_publishes_prepared_placement_before_one_start_and_unlinks_the_descriptor() {
    let mut fixture = Fixture::new();
    let cleanup_attempts = count_registration_cleanup(&fixture.conn);
    let mut host = Host::new(&fixture);
    let deadline = host.now + Duration::from_secs(10);
    let outcome = launch(&mut fixture, &mut host, deadline).unwrap();
    assert_eq!(host.calls, ["split", "prepared", "start"]);
    assert_eq!(host.spawn_count, 1);
    assert!(
        host.deadlines
            .iter()
            .all(|(_, recorded)| *recorded == deadline)
    );
    assert_eq!(outcome.run.status, RunStatus::Running);
    assert_eq!(outcome.run.owner_kind, OwnerKind::Runner);
    assert_eq!(outcome.run.owner_epoch, 2);
    assert!(
        RunLock::acquire(&fixture.conn, &outcome.run.owner())
            .unwrap()
            .is_none()
    );
    assert_eq!(outcome.placement.mux_pane_id.as_deref(), Some("%9"));
    assert_eq!(outcome.run.child_pid, Some(outcome.started.child.pid));
    assert!(
        !RuntimePaths::for_run(&outcome.run.database_identity_json, outcome.run.run_id)
            .unwrap()
            .descriptor
            .exists()
    );
    assert!(
        crate::broker::get_member(
            &fixture.conn,
            outcome.member.member_id,
            outcome.run.fleet_id
        )
        .unwrap()
        .is_some()
    );
    let member_id = outcome.member.member_id;
    let fleet_id = outcome.run.fleet_id;
    drop(outcome);
    assert!(
        crate::broker::get_member(&fixture.conn, member_id, fleet_id)
            .unwrap()
            .is_some()
    );
    assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 0);
    assert_eq!(host.calls, ["split", "prepared", "start"]);
}

#[test]
fn runner_creation_missing_or_rejected_publication_compensates_the_owned_pane_once() {
    for failure in [PublicationFailure::Missing, PublicationFailure::Rejected] {
        let mut fixture = Fixture::new();
        let cleanup_attempts = count_registration_cleanup(&fixture.conn);
        let mut host = Host::new(&fixture);
        host.publication_failure = Some(failure);
        let deadline = host.now + Duration::from_secs(10);
        let error = require_launch_error(launch(&mut fixture, &mut host, deadline));
        assert_eq!(error.exit_code(), 1);
        let expected = match failure {
            PublicationFailure::Missing => "placement",
            PublicationFailure::Rejected => "fixture publication blocked",
        };
        assert!(error.message().contains(expected), "{error}");
        assert_eq!(host.calls, ["split", "prepared", "shutdown", "close"]);
        assert_eq!(host.spawn_count, 0);
        assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
        assert_compensated(&fixture, &pending_run(&fixture));
    }
}

#[test]
fn runner_creation_preserves_usage_exit_through_preparation_and_launch_cleanup() {
    for phase in ["preparation", "start"] {
        for cleanup_fails in [false, true] {
            let mut fixture = Fixture::new();
            let cleanup_attempts = count_registration_cleanup(&fixture.conn);
            if cleanup_fails {
                reject_registration_cleanup(&fixture.conn);
            }
            let mut host = Host::new(&fixture);
            let deadline = host.now + Duration::from_secs(10);
            let result = if phase == "preparation" {
                let (member, settings) = inputs(&fixture);
                create_registered(
                    &mut fixture.conn,
                    &member,
                    &settings,
                    |_, _| {
                        Err(CafleetError::Usage(
                            "fixture preparation usage failure".into(),
                        ))
                    },
                    &mut host,
                    deadline,
                )
            } else {
                host.fail = Some("start");
                host.failure_is_usage = true;
                launch(&mut fixture, &mut host, deadline)
            };
            let error = require_launch_error(result);
            assert_eq!(error.exit_code(), 2, "{error}");
            assert!(
                error.message().starts_with(&format!("fixture {phase}")),
                "{error}"
            );
            assert_eq!(
                error
                    .message()
                    .matches("fixture database cleanup blocked")
                    .count(),
                usize::from(cleanup_fails)
            );
            assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
            assert_eq!(host.spawn_count, 0);
            let run = pending_run(&fixture);
            if cleanup_fails {
                assert!(
                    crate::broker::get_member(&fixture.conn, run.member_id, run.fleet_id)
                        .unwrap()
                        .is_some()
                );
            } else {
                assert_compensated(&fixture, &run);
            }
            assert_eq!(
                host.calls.iter().filter(|call| **call == "close").count(),
                usize::from(phase == "start")
            );
            assert_eq!(
                host.calls
                    .iter()
                    .filter(|call| **call == "shutdown")
                    .count(),
                usize::from(phase == "start")
            );
        }
    }
}

#[test]
fn runner_creation_preserves_publication_failure_and_combined_cleanup_errors_once() {
    let mut fixture = Fixture::new();
    let cleanup_attempts = count_registration_cleanup(&fixture.conn);
    reject_registration_cleanup(&fixture.conn);
    let mut host = Host::new(&fixture);
    host.publication_failure = Some(PublicationFailure::Rejected);
    host.cleanup_failure = true;
    let deadline = host.now + Duration::from_secs(10);
    let error = require_launch_error(launch(&mut fixture, &mut host, deadline));
    assert_eq!(error.exit_code(), 1);
    let text = error.message();
    assert!(
        text.lines()
            .next()
            .unwrap()
            .contains("fixture publication blocked"),
        "{text}"
    );
    for diagnostic in [
        "fixture publication blocked",
        "fixture owned-child cleanup failed",
        "fixture pane cleanup failed",
        "fixture database cleanup blocked",
    ] {
        assert_eq!(text.matches(diagnostic).count(), 1, "{text}");
    }
    assert_eq!(host.calls, ["split", "prepared", "shutdown", "close"]);
    assert_eq!(host.spawn_count, 0);
    assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
    let run = pending_run(&fixture);
    assert!(
        crate::broker::get_member(&fixture.conn, run.member_id, run.fleet_id)
            .unwrap()
            .is_some()
    );
    assert!(
        !RuntimePaths::for_run(&run.database_identity_json, run.run_id)
            .unwrap()
            .descriptor
            .exists()
    );
}

#[test]
fn runner_creation_unwind_during_preparation_compensates_registration_and_descriptor_once() {
    for stage in ["before-artifacts", "after-artifacts", "after-descriptor"] {
        let mut fixture = Fixture::new();
        let cleanup_attempts = count_registration_cleanup(&fixture.conn);
        let (member, settings) = inputs(&fixture);
        let spec = fixture.spec();
        let mut host = Host::new(&fixture);
        let deadline = host.now + Duration::from_secs(10);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            create_registered(
                &mut fixture.conn,
                &member,
                &settings,
                |conn, pending| {
                    assert_ne!(stage, "before-artifacts", "fixture preparation unwind");
                    let artifacts =
                        RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
                    if stage == "after-descriptor" {
                        artifacts.write_descriptor(&LaunchDescriptor::new(
                            &storage::get_run(conn, pending.run.run_id)?,
                            spec,
                        )?)?;
                    }
                    panic!("fixture preparation unwind");
                },
                &mut host,
                deadline,
            )
        }));
        assert!(unwound.is_err(), "{stage}");
        assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
        assert_compensated(&fixture, &pending_run(&fixture));
        assert!(host.calls.is_empty());
        assert_eq!(host.spawn_count, 0);
    }
}

fn assert_launch_unwind_compensates(phase: &'static str) {
    let mut fixture = Fixture::new();
    let cleanup_attempts = count_registration_cleanup(&fixture.conn);
    let mut host = Host::new(&fixture);
    host.panic_at = Some(phase);
    let deadline = host.now + Duration::from_secs(10);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        launch(&mut fixture, &mut host, deadline)
    }));
    assert!(unwound.is_err());
    let started = phase == "started";
    assert_eq!(host.spawn_count, usize::from(started));
    assert_eq!(
        host.calls
            .iter()
            .filter(|call| **call == "shutdown")
            .count(),
        usize::from(started)
    );
    assert_eq!(
        host.calls.iter().filter(|call| **call == "close").count(),
        1
    );
    assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
    assert_compensated(&fixture, &pending_run(&fixture));
}

#[test]
fn runner_creation_unwind_after_split_closes_the_owned_pane_once() {
    assert_launch_unwind_compensates("prepared");
}

#[test]
fn runner_creation_unwind_after_spawn_stops_the_owned_child_and_closes_the_pane_once() {
    assert_launch_unwind_compensates("started");
}

#[test]
fn runner_creation_compensates_split_prepare_and_spawn_failures_without_leaking_registration() {
    for phase in ["split", "prepared", "start"] {
        let mut fixture = Fixture::new();
        let mut host = Host::new(&fixture);
        host.fail = Some(phase);
        let deadline = host.now + Duration::from_secs(10);
        assert!(
            launch(&mut fixture, &mut host, deadline).is_err(),
            "{phase}"
        );
        assert_compensated(&fixture, &pending_run(&fixture));
        assert_eq!(host.spawn_count, 0);
        assert!(host.calls.iter().filter(|call| **call == "close").count() <= 1);
        assert!(
            host.calls
                .iter()
                .filter(|call| **call == "shutdown")
                .count()
                <= 1
        );
    }
}

#[test]
fn runner_creation_rejects_each_stale_prepared_identity_before_starting_a_child() {
    for corrupt in [
        "prepared-nonce",
        "prepared-epoch",
        "prepared-run",
        "prepared-director",
    ] {
        let mut fixture = Fixture::new();
        let mut host = Host::new(&fixture);
        host.corrupt = Some(corrupt);
        let deadline = host.now + Duration::from_secs(10);
        assert!(
            launch(&mut fixture, &mut host, deadline).is_err(),
            "{corrupt}"
        );
        assert_eq!(host.spawn_count, 0);
        assert_compensated(&fixture, &pending_run(&fixture));
    }
}

#[test]
fn runner_creation_lost_started_reply_or_mismatched_child_compensates_one_uncertain_spawn() {
    for failure in ["started-reply", "started-epoch", "started-child"] {
        let mut fixture = Fixture::new();
        let mut host = Host::new(&fixture);
        if failure == "started-reply" {
            host.fail = Some(failure);
        } else {
            host.corrupt = Some(failure);
        }
        let deadline = host.now + Duration::from_secs(10);
        assert!(
            launch(&mut fixture, &mut host, deadline).is_err(),
            "{failure}"
        );
        assert_eq!(host.spawn_count, 1);
        assert_eq!(
            host.calls
                .iter()
                .filter(|call| **call == "shutdown")
                .count(),
            1
        );
        assert_eq!(
            host.calls.iter().filter(|call| **call == "close").count(),
            1
        );
        assert_compensated(&fixture, &pending_run(&fixture));
    }
}

#[test]
fn runner_creation_uses_one_handshake_deadline_and_compensates_a_post_start_timeout() {
    let mut fixture = Fixture::new();
    let mut host = Host::new(&fixture);
    host.delay = Duration::from_secs(4);
    let deadline = host.now + Duration::from_secs(10);
    assert!(launch(&mut fixture, &mut host, deadline).is_err());
    assert_eq!(host.spawn_count, 1);
    assert!(
        host.deadlines
            .iter()
            .filter(|(phase, _)| *phase != "close")
            .all(|(_, recorded)| *recorded == deadline)
    );
    assert_compensated(&fixture, &pending_run(&fixture));
}

#[test]
fn runner_creation_immediate_nonzero_native_exit_is_a_successful_spawn_with_one_inbox_event() {
    let mut fixture = Fixture::new();
    let mut host = Host::new(&fixture);
    host.fast_exit = true;
    let deadline = host.now + Duration::from_secs(10);
    let outcome = launch(&mut fixture, &mut host, deadline).unwrap();
    assert_eq!(host.spawn_count, 1);
    assert!(!host.calls.contains(&"shutdown"));
    assert_eq!(outcome.run.status, RunStatus::Exited);
    assert_eq!(outcome.run.exit_code, Some(7));
    assert_eq!(outcome.run.resolved_mode, "never");
    let messages =
        crate::broker::poll_messages(&fixture.conn, outcome.run.director_member_id).unwrap();
    assert_eq!(messages.len(), 1);
    let payload: serde_json::Value = serde_json::from_str(&messages[0].text).unwrap();
    assert_eq!(payload["kind"], "runner_exited");
    assert_eq!(payload["cause"]["exit_code"], 7);
}

#[test]
fn runner_creation_retains_the_primary_failure_and_all_owned_cleanup_diagnostics() {
    let mut fixture = Fixture::new();
    let mut host = Host::new(&fixture);
    host.fail = Some("started-reply");
    host.cleanup_failure = true;
    let deadline = host.now + Duration::from_secs(10);
    let error = match launch(&mut fixture, &mut host, deadline) {
        Err(error) => error,
        Ok(_) => panic!("lost reply must fail"),
    };
    let text = error.message();
    assert!(text.contains("started reply lost"));
    assert!(text.contains("owned-child cleanup failed"));
    assert!(text.contains("pane cleanup failed"));
    assert_eq!(host.spawn_count, 1);
    assert_eq!(
        pending_run(&fixture).cleanup_state,
        CleanupState::Incomplete
    );
}

#[test]
fn runner_creation_invalid_spawn_construction_under_the_guard_keeps_assignment_out_of_failed_rows()
{
    let mut fixture = Fixture::new();
    let (member, settings) = inputs(&fixture);
    let mut spec = fixture.spec();
    let private = "fixture-private-assignment-194";
    spec.argv.push(private.into());
    spec.cwd = "relative".into();
    let mut host = Host::new(&fixture);
    let deadline = host.now + Duration::from_secs(10);
    assert!(
        create_registered(
            &mut fixture.conn,
            &member,
            &settings,
            |conn, pending| {
                let artifacts =
                    RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
                let run = storage::get_run(conn, pending.run.run_id)?;
                LaunchDescriptor::new(&run, spec.clone())?;
                Ok((artifacts, spec))
            },
            &mut host,
            deadline
        )
        .is_err()
    );
    let run = pending_run(&fixture);
    assert_compensated(&fixture, &run);
    let serialized=fixture.conn.query_row("SELECT permission_argv_json || COALESCE(cleanup_diagnostics_json,'') FROM member_runs WHERE run_id=?1",[run.run_id],|row|row.get::<_,String>(0)).unwrap();
    assert!(!serialized.contains(private));
    assert!(host.calls.is_empty());
}

#[test]
fn runner_creation_substitutes_once_and_keeps_the_assignment_in_one_literal_native_argument() {
    let mut fixture = Fixture::new();
    let (member, settings) = inputs(&fixture);
    let mut spec = fixture.spec();
    let mut host = Host::new(&fixture);
    let deadline = host.now + Duration::from_secs(10);
    let template = "member={member_id}; literal={{member_id}}; '$x' `text` 世界";
    let outcome = create_registered(
        &mut fixture.conn,
        &member,
        &settings,
        |conn, pending| {
            let assignment = crate::spawn_prompt::substitute_spawn_placeholders(
                template,
                pending.run.fleet_id,
                pending.member.member_id,
                pending.run.director_member_id,
                "codex",
            )?;
            spec.argv = vec!["/fixture/codex".into(), assignment];
            Ok((
                RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?,
                spec,
            ))
        },
        &mut host,
        deadline,
    )
    .unwrap();
    assert_eq!(
        host.child_argv,
        [
            "/fixture/codex".to_owned(),
            format!(
                "member={}; literal={{member_id}}; '$x' `text` 世界",
                outcome.member.member_id
            )
        ]
    );
    assert!(!host.argv.iter().any(|arg| arg.contains("literal=")));
}
