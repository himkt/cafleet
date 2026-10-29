use super::*;
use crate::input::{Deadline, PaneLock, delete_fleet_with_host};
use crate::multiplexer::PostCloseLayout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};

#[derive(Clone, Copy)]
enum Mutation {
    None,
    BeforeRecovery,
    AfterRecovery,
    AfterClose,
    OtherPlacement(i64),
}
struct DeletionHost {
    recovery: Host,
    settings: Settings,
    mutation: Mutation,
    calls: Vec<&'static str>,
    connection: usize,
    recovered: Option<RunRecord>,
}
impl DeletionHost {
    fn new(case: &Case, deadline: Deadline) -> Self {
        Self {
            recovery: Host::new(deadline.instant()),
            settings: case.settings.clone(),
            mutation: Mutation::None,
            calls: Vec::new(),
            connection: std::ptr::from_ref(&case.fixture.conn).addr(),
            recovered: None,
        }
    }
    fn assert_pane(&self, placement: &Placement) {
        assert!(
            PaneLock::try_acquire(&self.settings.database_url, placement)
                .unwrap()
                .is_none()
        );
    }
    fn assert_connection(&self, conn: &mut Connection) {
        assert_eq!(std::ptr::from_ref(conn).addr(), self.connection);
    }
}
impl crate::input::DeletionHost for DeletionHost {
    fn shutdown(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        deadline: Deadline,
    ) -> TestResult<ShutdownOutcome> {
        self.assert_connection(conn);
        assert_eq!(deadline.instant(), self.recovery.deadline);
        let placement = crate::broker::get_member(conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        self.assert_pane(&placement);
        self.calls.push("shutdown");
        conn.execute(
            "UPDATE member_runs SET cleanup_state='complete' WHERE run_id=?1",
            [run.run_id],
        )
        .unwrap();
        Ok(ShutdownOutcome {
            native_exit: None,
            cleanup_state: CleanupState::Complete,
            diagnostics: Vec::new(),
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        })
    }
    fn recover(
        &mut self,
        conn: &mut Connection,
        settings: &Settings,
        member_id: i64,
        expected_run: &RunRecord,
        expected_placement: &Placement,
        held: &PaneLock,
        deadline: Deadline,
    ) -> TestResult<RecoveryCompletion> {
        self.assert_connection(conn);
        assert_eq!(deadline.instant(), self.recovery.deadline);
        assert_eq!(expected_run.member_id, member_id);
        self.assert_pane(expected_placement);
        held.validate_binding(&settings.database_url, expected_placement)
            .unwrap();
        self.calls.push("recover");
        if matches!(self.mutation, Mutation::BeforeRecovery) {
            conn.execute(
                "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [expected_run.run_id],
            )
            .unwrap();
        }
        let completion = recover_member_with_held_pane_lock(
            conn,
            settings,
            member_id,
            expected_run,
            expected_placement,
            None,
            deadline.instant(),
            held,
            &mut self.recovery,
        )?;
        assert_eq!(
            completion.run,
            storage::get_run(conn, expected_run.run_id).unwrap()
        );
        self.recovered = Some(completion.run.clone());
        match self.mutation {
            Mutation::AfterRecovery => {
                conn.execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [expected_run.run_id],
                )
                .unwrap();
            }
            Mutation::OtherPlacement(member) => {
                conn.execute(
                    "UPDATE member_placements SET mux_pane_id='%foreign' WHERE member_id=?1",
                    [member],
                )
                .unwrap();
            }
            _ => {}
        }
        Ok(completion)
    }
    fn close_pane(
        &mut self,
        placement: &Placement,
        deadline: Deadline,
    ) -> TestResult<PostCloseLayout> {
        assert_eq!(deadline.instant(), self.recovery.deadline);
        self.assert_pane(placement);
        self.calls.push("close");
        if matches!(self.mutation, Mutation::AfterClose) {
            let run = self
                .recovered
                .as_ref()
                .expect("close follows proven recovery");
            let foreign =
                Connection::open(crate::db::database_path(&self.settings.database_url).unwrap())
                    .unwrap();
            foreign
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [run.run_id],
                )
                .unwrap();
        }
        Ok(PostCloseLayout::None)
    }
    fn restore_layout(&mut self, _: PostCloseLayout) -> TestResult<()> {
        Ok(())
    }
}
fn active(case: &Case) {
    assert!(
        crate::broker::get_fleet(&case.fixture.conn, case.fixture.run.fleet_id)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_none()
    );
    assert!(
        crate::broker::get_member(
            &case.fixture.conn,
            case.fixture.run.member_id,
            case.fixture.run.fleet_id
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn runner_recovery_deletion_uses_real_shared_outbox_open_scan_append_and_compact_under_outer_budget()
 {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let original = case.fixture.run.clone();
    case.release();
    let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
    let mut host = DeletionHost::new(&case, deadline);
    case.fixture
        .conn
        .busy_timeout(Duration::from_millis(137))
        .unwrap();
    delete_fleet_with_host(
        &mut case.fixture.conn,
        &case.settings,
        original.fleet_id,
        deadline,
        &mut host,
    )
    .unwrap();
    assert_eq!(host.calls, vec!["shutdown", "recover", "close"]);
    let recovered = host.recovered.as_ref().unwrap();
    assert_eq!(recovered.owner_epoch, original.owner_epoch + 1);
    assert_eq!(recovered.status, RunStatus::Lost);
    let events = events(&case);
    assert_eq!(
        events.iter().map(|row| row.event.kind).collect::<Vec<_>>(),
        vec![
            RuntimeEventKind::InteractionOpened,
            RuntimeEventKind::InteractionClosed,
            RuntimeEventKind::RunnerLost
        ]
    );
    assert!(events.iter().all(
        |row| row.event.notification_state == NotificationState::Inbox
            && row.event.message_id.is_some()
    ));
    assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
    assert!(
        crate::broker::get_fleet(&case.fixture.conn, original.fleet_id)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_some()
    );
    let (status, deregistered_at): (String, Option<String>) = case
        .fixture
        .conn
        .query_row(
            "SELECT status,deregistered_at FROM members WHERE member_id=?1 AND fleet_id=?2",
            rusqlite::params![original.member_id, original.fleet_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "deregistered");
    assert!(deregistered_at.is_some());
    assert_eq!(
        case.fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        137
    );
    for boundary in ["before_replay", "after_commit", "before_compact"] {
        assert!(host.recovery.calls.iter().any(|name| name == boundary));
    }
}

#[test]
fn runner_recovery_deletion_rejects_foreign_pre_entry_post_recovery_and_post_close_owner_changes() {
    for mutation in [
        Mutation::BeforeRecovery,
        Mutation::AfterRecovery,
        Mutation::AfterClose,
    ] {
        let mut case = Case::new();
        case.seed(&[opening(&case.fixture.run, 1, 1)]);
        let epoch = case.fixture.run.owner_epoch;
        let original_bytes = case.spool_bytes().unwrap();
        case.release();
        let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
        let mut host = DeletionHost::new(&case, deadline);
        host.mutation = mutation;
        assert!(
            delete_fleet_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.fleet_id,
                deadline,
                &mut host
            )
            .is_err()
        );
        active(&case);
        let current = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
        match mutation {
            Mutation::BeforeRecovery => {
                assert_eq!(host.calls, vec!["shutdown", "recover"]);
                assert!(events(&case).is_empty());
                assert_eq!(current.owner_epoch, epoch + 1);
                assert_eq!(case.spool_bytes().unwrap(), original_bytes);
            }
            Mutation::AfterRecovery => {
                assert_eq!(host.calls, vec!["shutdown", "recover"]);
                assert_eq!(events(&case).len(), 3);
                assert_eq!(current.owner_epoch, epoch + 2);
            }
            Mutation::AfterClose => {
                assert_eq!(host.calls, vec!["shutdown", "recover", "close"]);
                assert_eq!(events(&case).len(), 3);
                assert_eq!(current.owner_epoch, epoch + 2);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn runner_recovery_deletion_updates_only_its_proven_target_and_keeps_other_placements_pinned() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let other = common::register(
        &mut case.fixture.conn,
        case.fixture.run.fleet_id,
        "other-deletion-target",
        None,
    );
    case.release();
    let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
    let mut host = DeletionHost::new(&case, deadline);
    host.mutation = Mutation::OtherPlacement(other);
    assert!(
        delete_fleet_with_host(
            &mut case.fixture.conn,
            &case.settings,
            case.fixture.run.fleet_id,
            deadline,
            &mut host
        )
        .is_err()
    );
    active(&case);
    assert!(host.recovered.is_some());
    assert_eq!(host.calls, vec!["shutdown", "recover"]);
    assert_eq!(
        crate::broker::get_member(&case.fixture.conn, other, case.fixture.run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap()
            .mux_pane_id
            .as_deref(),
        Some("%foreign")
    );
}

#[test]
fn runner_recovery_deletion_exhausted_deadline_retains_pending_namespace_and_active_registry() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let before = rows(&case.fixture.conn);
    let bytes = case.spool_bytes().unwrap();
    case.release();
    let deadline = Deadline::until(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
    );
    let mut host = DeletionHost::new(&case, deadline);
    assert!(
        delete_fleet_with_host(
            &mut case.fixture.conn,
            &case.settings,
            case.fixture.run.fleet_id,
            deadline,
            &mut host
        )
        .is_err()
    );
    assert_eq!(rows(&case.fixture.conn), before);
    assert_eq!(case.spool_bytes().unwrap(), bytes);
    active(&case);
    assert!(host.calls.is_empty());
}

const LONG_SQL: &str = "WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<1000000000) SELECT sum(n) FROM numbers";

struct EmergencyInterrupt {
    cancellation: Option<mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    fired: Arc<AtomicBool>,
}
impl EmergencyInterrupt {
    fn arm(conn: &Connection, deadline: Instant) -> Self {
        let interrupt = conn.get_interrupt_handle();
        let fired = Arc::new(AtomicBool::new(false));
        let observed = fired.clone();
        let (cancellation, cancelled) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            match cancelled.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    observed.store(true, Ordering::Release);
                    interrupt.interrupt();
                }
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
            }
        });
        Self {
            cancellation: Some(cancellation),
            worker: Some(worker),
            fired,
        }
    }
    fn finish(mut self) -> bool {
        drop(self.cancellation.take());
        self.worker.take().unwrap().join().unwrap();
        self.fired.load(Ordering::Acquire)
    }
}
impl Drop for EmergencyInterrupt {
    fn drop(&mut self) {
        drop(self.cancellation.take());
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            eprintln!("owned emergency SQL interruption worker panicked");
        }
    }
}

#[test]
fn runner_recovery_deletion_success_error_and_unwind_restore_the_still_live_outer_scope() {
    for exit in ["success", "error", "unwind"] {
        let mut case = Case::new();
        case.seed(&[opening(&case.fixture.run, 1, 1)]);
        case.release();
        let start = Instant::now();
        let inner = start + Duration::from_millis(300);
        let outer = start + Duration::from_millis(700);
        let emergency = EmergencyInterrupt::arm(&case.fixture.conn, start + Duration::from_secs(2));
        let deadline = Deadline::until(inner);
        let mut host = DeletionHost::new(&case, deadline);
        if exit != "success" {
            host.recovery.action = Some(Box::new(move |boundary, _, _| {
                if boundary == "before_compact" {
                    if exit == "unwind" {
                        panic!("owned injected recovery unwind");
                    }
                    return Err(CafleetError::App("owned injected recovery error".into()));
                }
                Ok(())
            }));
        }
        case.fixture
            .conn
            .busy_timeout(Duration::from_millis(137))
            .unwrap();
        let fleet = case.fixture.run.fleet_id;
        let result = with_database_deadline(&mut case.fixture.conn, outer, |conn| {
            let deletion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                delete_fleet_with_host(conn, &case.settings, fleet, deadline, &mut host)
            }));
            match exit {
                "success" => assert!(matches!(deletion, Ok(Ok(_)))),
                "error" => assert!(matches!(deletion, Ok(Err(_)))),
                "unwind" => assert!(deletion.is_err()),
                _ => unreachable!(),
            }
            assert!(
                host.recovery
                    .calls
                    .iter()
                    .any(|name| name == "before_compact")
            );
            std::thread::sleep(
                (inner + Duration::from_millis(20)).saturating_duration_since(Instant::now()),
            );
            assert!(Instant::now() > inner && Instant::now() < outer);
            assert_eq!(
                conn.query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                42
            );
            conn.query_row(LONG_SQL, [], |row| row.get::<_, i64>(0))
                .map_err(crate::broker::members::db_err)
        });
        let emergency_fired = emergency.finish();
        eprintln!("recovery nested SQL {exit}: emergency_fired={emergency_fired}");
        assert!(!emergency_fired, "the original outer deadline cancels SQL");
        assert!(result.is_err());
        assert!(
            Instant::now() >= outer,
            "the retired inner deadline cannot cancel outer work: {exit}"
        );
        assert!(start.elapsed() < Duration::from_millis(1400));
        assert_eq!(
            case.fixture
                .conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            137
        );
        assert_eq!(
            case.fixture
                .conn
                .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
    }
}

#[test]
fn runner_recovery_deletion_actual_replay_sql_is_cancelled_within_the_supplied_remaining_budget() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    case.fixture.conn.execute_batch(&format!("CREATE TABLE recovery_sql_probe (entered INTEGER NOT NULL); CREATE TRIGGER slow_replay BEFORE INSERT ON member_runtime_events BEGIN INSERT INTO recovery_sql_probe VALUES (1); SELECT ({LONG_SQL}); END")).unwrap();
    let entered = Arc::new(Mutex::new(None));
    let observed = entered.clone();
    case.fixture
        .conn
        .update_hook(Some(
            move |_: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if table == "recovery_sql_probe" {
                    observed
                        .lock()
                        .expect("owned replay SQL entry observation")
                        .get_or_insert_with(Instant::now);
                }
            },
        ))
        .unwrap();
    let bytes = case.spool_bytes().unwrap();
    case.release();
    case.fixture
        .conn
        .busy_timeout(Duration::from_millis(137))
        .unwrap();
    let start = Instant::now();
    let deadline = Deadline::until(start + Duration::from_millis(150));
    let emergency = EmergencyInterrupt::arm(&case.fixture.conn, start + Duration::from_secs(1));
    let mut host = DeletionHost::new(&case, deadline);
    let result = delete_fleet_with_host(
        &mut case.fixture.conn,
        &case.settings,
        case.fixture.run.fleet_id,
        deadline,
        &mut host,
    );
    let returned = Instant::now();
    let emergency_fired = emergency.finish();
    case.fixture
        .conn
        .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
        .unwrap();
    let entered = *entered.lock().expect("owned replay SQL entry observation");
    eprintln!(
        "recovery replay SQL: entered={:?}, returned={:?}, emergency_fired={emergency_fired}",
        entered.map(|at| at.duration_since(start)),
        returned.duration_since(start)
    );
    assert!(!emergency_fired, "the original replay deadline cancels SQL");
    let entered = entered.expect("the actual replay trigger entered before SQL cancellation");
    assert!(entered >= start && entered < deadline.instant());
    assert!(returned >= deadline.instant());
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_millis(900));
    assert!(
        host.recovery
            .calls
            .iter()
            .any(|name| name == "before_replay")
    );
    assert!(!host.calls.contains(&"close"));
    assert!(events(&case).is_empty());
    assert_eq!(
        case.fixture
            .conn
            .query_row("SELECT COUNT(*) FROM recovery_sql_probe", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "canceled replay rolls back its actual trigger write"
    );
    assert_eq!(case.spool_bytes().unwrap(), bytes);
    active(&case);
    case.fixture
        .conn
        .execute_batch("DROP TRIGGER slow_replay")
        .unwrap();
    assert_eq!(
        case.fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        137
    );
    assert_eq!(
        case.fixture
            .conn
            .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        42
    );
}
