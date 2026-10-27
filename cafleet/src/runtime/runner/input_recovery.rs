#[cfg(test)]
pub(super) mod tests {
    use super::super::*;
    use crate::broker::records::Placement;
    use crate::broker::runtime::{self as storage, *};
    use crate::config::Settings;
    use crate::input::{Deadline, PaneLock, with_database_budget};
    use crate::runtime::runner_tests::Fixture;
    use rusqlite::Connection;
    use serde_json::json;
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, Instant};

    pub(in crate::runtime::runner) enum Termination {
        Terminated,
        Alive,
        Unknown,
    }

    type BoundaryAction = Box<dyn FnMut(RecoveryBoundary, &RunRecord, Instant) -> Result<()>>;

    pub(in crate::runtime::runner) struct TypedHost {
        pub(in crate::runtime::runner) until: Instant,
        pub(in crate::runtime::runner) owner: Termination,
        pub(in crate::runtime::runner) startup: Termination,
        pub(in crate::runtime::runner) matching_health: bool,
        pub(in crate::runtime::runner) calls: Vec<&'static str>,
        pub(in crate::runtime::runner) processes: Vec<ProcessIdentity>,
        pub(in crate::runtime::runner) startups: Vec<(RunRecord, Option<Placement>)>,
        pub(in crate::runtime::runner) action: Option<BoundaryAction>,
    }

    impl TypedHost {
        pub(in crate::runtime::runner) fn new(until: Instant) -> Self {
            Self {
                until,
                owner: Termination::Terminated,
                startup: Termination::Terminated,
                matching_health: false,
                calls: Vec::new(),
                processes: Vec::new(),
                startups: Vec::new(),
                action: None,
            }
        }

        fn check(&self, until: Instant) -> Result<()> {
            assert_eq!(
                until, self.until,
                "evidence uses the original effective bound"
            );
            Deadline::until(until).remaining().map(|_| ())
        }

        fn evidence(value: &Termination) -> ProcessEvidence {
            match value {
                Termination::Terminated => ProcessEvidence::Terminated,
                Termination::Alive => ProcessEvidence::Alive,
                Termination::Unknown => {
                    ProcessEvidence::Unknown("owned process-set uncertainty".into())
                }
            }
        }
    }

    impl RecoveryHost for TypedHost {
        fn current_process(&mut self) -> Result<ProcessIdentity> {
            self.calls.push("current");
            self.check(self.until)?;
            ProcessIdentity::current()
        }

        fn process_evidence(
            &mut self,
            process: &ProcessIdentity,
            until: Instant,
        ) -> Result<ProcessEvidence> {
            self.check(until)?;
            self.calls.push("owner");
            self.processes.push(process.clone());
            Ok(Self::evidence(&self.owner))
        }

        fn startup_process_evidence(
            &mut self,
            run: &RunRecord,
            placement: Option<&Placement>,
            until: Instant,
        ) -> Result<ProcessEvidence> {
            self.check(until)?;
            self.calls.push("startup");
            self.startups.push((run.clone(), placement.cloned()));
            Ok(Self::evidence(&self.startup))
        }

        fn health(&mut self, run: &RunRecord, until: Instant) -> Result<RecoveryHealth> {
            self.check(until)?;
            self.calls.push("health");
            if self.matching_health {
                Ok(RecoveryHealth::Matching(HealthSnapshot {
                    identity: ControlIdentity::from_run(run),
                    runner: ProcessIdentity {
                        pid: run
                            .runner_pid
                            .expect("fixture has a real registered runner identity"),
                        start_identity: run.runner_start_identity.clone().unwrap(),
                    },
                    child: None,
                    output_seq: 1,
                    observation: None,
                }))
            } else {
                Ok(RecoveryHealth::Unavailable(
                    "owned receipt endpoint unavailable".into(),
                ))
            }
        }

        fn cleanup_owned(&mut self, _: &RunRecord, _: ShutdownClock) -> Result<ShutdownOutcome> {
            panic!("receipt-only settlement has no child cleanup capability")
        }

        fn unlink_descriptor(&mut self, _: &std::path::Path, _: Instant) -> Result<()> {
            panic!("receipt-only settlement preserves its descriptor")
        }

        fn checkpoint(
            &mut self,
            boundary: RecoveryBoundary,
            run: &RunRecord,
            until: Instant,
        ) -> Result<()> {
            self.check(until)?;
            let name = match boundary {
                RecoveryBoundary::AfterSnapshot => "snapshot",
                RecoveryBoundary::BeforeTakeover => "before",
                RecoveryBoundary::AfterTakeover => "after",
                _ => panic!("receipt-only settlement uses no replay/cleanup boundary"),
            };
            self.calls.push(name);
            if let Some(action) = &mut self.action {
                action(boundary, run, until)?;
            }
            Ok(())
        }
    }

    struct ReceiptCase {
        fixture: Fixture,
        artifacts: RunArtifacts,
        settings: Settings,
        placement: Placement,
        input: InputOperation,
    }

    impl ReceiptCase {
        fn new() -> Self {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
            let owner = lock
                .transfer_to_runner(
                    &mut fixture.conn,
                    &fixture.run.owner(),
                    ProcessIdentity::current().unwrap(),
                )
                .unwrap();
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.run.director_member_id,
                fixture.run.member_id,
                "owned exclusive receipt preview",
            )
            .unwrap();
            let input = storage::reserve_input(
                &mut fixture.conn,
                &owner,
                &NewInput {
                    operation_key: format!("preview:{}:{}", owner.run_id, message.message_id),
                    kind: InputKind::Preview,
                    payload_digest: "d".repeat(64),
                    message_id: Some(message.message_id),
                },
            )
            .unwrap();
            let input = storage::record_input_state(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputState::Uncertain,
            )
            .unwrap();
            storage::reserve_event_sequences(&mut fixture.conn, &owner).unwrap();
            fixture.conn.execute(
                "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=7,ended_at='2026-10-27T18:13:00Z',cleanup_diagnostics_json=?1 WHERE run_id=?2",
                rusqlite::params![json!([{"kind":"owned_terminal_history"}]).to_string(), owner.run_id],
            ).unwrap();
            fixture.run = storage::get_run(&fixture.conn, owner.run_id).unwrap();
            lock.validate(&fixture.conn, &fixture.run.owner()).unwrap();
            std::fs::write(&artifacts.paths.outbox, [0, 0]).unwrap();
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
            let settings = Settings {
                database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
                max_text_len: 200,
                multiplexer: None,
            };
            drop(lock);
            Self {
                fixture,
                artifacts,
                settings,
                placement,
                input,
            }
        }

        fn invoke(
            &mut self,
            held: Option<&PaneLock>,
            host: &mut TypedHost,
        ) -> std::result::Result<InputOperation, InputFailure> {
            super::reconcile_input_with_host(
                &mut self.fixture.conn,
                &self.settings,
                &self.input,
                &self.fixture.run,
                held.map(|_| &self.placement),
                held,
                InputDisposition::Submitted,
                "fixture:exclusive-receipt",
                host.until,
                host,
            )
        }

        fn assert_original_lock_available(&self) {
            let run = storage::get_run(&self.fixture.conn, self.input.run_id).unwrap();
            let lock = RunLock::acquire(&self.fixture.conn, &run.owner())
                .unwrap()
                .expect("receipt-only ownership releases its original namespace on return");
            lock.validate(&self.fixture.conn, &run.owner()).unwrap();
        }

        fn spool(&self) -> (Vec<u8>, u64, u64) {
            let metadata = std::fs::symlink_metadata(&self.artifacts.paths.outbox).unwrap();
            (
                std::fs::read(&self.artifacts.paths.outbox).unwrap(),
                metadata.dev(),
                metadata.ino(),
            )
        }
    }

    type DatabaseRows = Vec<(String, Vec<Vec<rusqlite::types::Value>>)>;

    fn rows(conn: &Connection, excluded: &[&str]) -> DatabaseRows {
        let names = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        names
            .into_iter()
            .filter(|name| !excluded.contains(&name.as_str()))
            .map(|name| {
                let quoted = name.replace('"', "\"\"");
                let mut statement = conn
                    .prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY rowid"))
                    .unwrap();
                let width = statement.column_count();
                let values = statement
                    .query_map([], |row| {
                        (0..width)
                            .map(|index| row.get(index))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (name, values)
            })
            .collect()
    }

    fn assert_owner_only(before: &RunRecord, after: &RunRecord) {
        let process = ProcessIdentity::current().unwrap();
        let mut expected = before.clone();
        expected.owner_epoch += 1;
        expected.owner_kind = OwnerKind::Reconciler;
        expected.owner_pid = process.pid;
        expected.owner_start_identity = process.start_identity;
        assert_eq!(*after, expected);
    }

    #[test]
    fn exclusive_receipt_terminal_preview_commits_only_owner_disposition_and_stamp() {
        let mut case = ReceiptCase::new();
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        let before = case.fixture.run.clone();
        let history = rows(
            &case.fixture.conn,
            &["member_runs", "member_input_operations", "messages"],
        );
        let message =
            crate::broker::get_message(&case.fixture.conn, case.input.message_id.unwrap()).unwrap();
        let stamp: Option<String> = case
            .fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stamp.is_none());
        let spool = case.spool();
        let mut host = TypedHost::new(Instant::now() + Duration::from_secs(2));
        let settled = case.invoke(Some(&held), &mut host).unwrap();
        assert_eq!(settled.input_id, case.input.input_id);
        assert_eq!(settled.state, InputState::Settled);
        assert_eq!(settled.disposition, Some(InputDisposition::Submitted));
        assert_eq!(
            settled.evidence_ref.as_deref(),
            Some("fixture:exclusive-receipt")
        );
        let mut expected = case.input.clone();
        expected.state = settled.state;
        expected.disposition = settled.disposition;
        expected.evidence_ref = settled.evidence_ref.clone();
        expected.updated_at = settled.updated_at.clone();
        assert_eq!(settled, expected);
        assert_owner_only(
            &before,
            &storage::get_run(&case.fixture.conn, before.run_id).unwrap(),
        );
        let stamped = crate::broker::get_message(&case.fixture.conn, message.message_id).unwrap();
        let stamp: Option<String> = case
            .fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stamp.is_some());
        assert_eq!(stamped, message);
        assert_eq!(
            rows(
                &case.fixture.conn,
                &["member_runs", "member_input_operations", "messages"]
            ),
            history
        );
        assert_eq!(case.spool(), spool);
        for boundary in ["snapshot", "owner", "startup", "health", "before", "after"] {
            assert!(host.calls.contains(&boundary), "{boundary}");
        }
        assert!(host.processes.contains(&ProcessIdentity {
            pid: before.owner_pid,
            start_identity: before.owner_start_identity.clone()
        }));
        assert!(
            host.startups
                .iter()
                .any(|(run, placement)| *run == before
                    && placement.as_ref() == Some(&case.placement))
        );
        held.validate_binding(&case.settings.database_url, &case.placement)
            .unwrap();
        assert!(
            PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .is_none()
        );
        case.assert_original_lock_available();
    }

    #[test]
    fn exclusive_receipt_absent_placement_requires_real_inactive_history_and_a_matched_pair() {
        for deleted_fleet in [false, true] {
            let mut case = ReceiptCase::new();
            if deleted_fleet {
                crate::broker::fleets::delete_fleet_rows(
                    &case.fixture.conn,
                    case.fixture.run.fleet_id,
                )
                .unwrap();
            } else {
                assert!(
                    crate::broker::deregister_member(
                        &mut case.fixture.conn,
                        case.fixture.run.member_id
                    )
                    .unwrap()
                );
            }
            let count: i64 = case
                .fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM member_placements WHERE member_id=?1",
                    [case.fixture.run.member_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            let history = rows(
                &case.fixture.conn,
                &["member_runs", "member_input_operations", "messages"],
            );
            let spool = case.spool();
            let mut host = TypedHost::new(Instant::now() + Duration::from_secs(2));
            case.invoke(None, &mut host).unwrap();
            assert!(
                host.startups
                    .iter()
                    .any(|(run, placement)| { run == &case.fixture.run && placement.is_none() })
            );
            assert_owner_only(
                &case.fixture.run,
                &storage::get_run(&case.fixture.conn, case.input.run_id).unwrap(),
            );
            assert_eq!(
                rows(
                    &case.fixture.conn,
                    &["member_runs", "member_input_operations", "messages"]
                ),
                history
            );
            assert_eq!(case.spool(), spool);
            case.assert_original_lock_available();
        }

        let mut case = ReceiptCase::new();
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        let before = rows(&case.fixture.conn, &[]);
        for (placement, guard) in [(Some(&case.placement), None), (None, Some(&held))] {
            let mut host = TypedHost::new(Instant::now() + Duration::from_secs(1));
            assert!(
                super::reconcile_input_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    &case.input,
                    &case.fixture.run,
                    placement,
                    guard,
                    InputDisposition::Submitted,
                    "fixture:invalid-pair",
                    host.until,
                    &mut host,
                )
                .is_err()
            );
            assert!(host.calls.is_empty());
            assert_eq!(rows(&case.fixture.conn, &[]), before);
        }
        case.fixture
            .conn
            .execute(
                "DELETE FROM member_placements WHERE member_id=?1",
                [case.fixture.run.member_id],
            )
            .unwrap();
        let before = rows(&case.fixture.conn, &[]);
        let mut host = TypedHost::new(Instant::now() + Duration::from_secs(1));
        assert!(case.invoke(None, &mut host).is_err());
        assert_eq!(rows(&case.fixture.conn, &[]), before);
        assert!(host.calls.is_empty());
    }

    #[test]
    fn exclusive_receipt_transaction_failure_rolls_back_owner_stamp_and_input_before_retry() {
        for after_cas in [false, true] {
            let mut case = ReceiptCase::new();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            let before = rows(&case.fixture.conn, &[]);
            let spool = case.spool();
            let mut host = TypedHost::new(Instant::now() + Duration::from_secs(2));
            if after_cas {
                let epoch = case.fixture.run.owner_epoch;
                host.action = Some(Box::new(move |boundary, run, _| {
                    if matches!(boundary, RecoveryBoundary::AfterTakeover) {
                        assert_eq!(run.owner_epoch, epoch + 1);
                        return Err(error("owned after-CAS receipt failure"));
                    }
                    Ok(())
                }));
            } else {
                case.fixture.conn.execute_batch("CREATE TRIGGER exclusive_receipt_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.state='settled' BEGIN SELECT RAISE(FAIL,'owned exclusive settlement failure'); END").unwrap();
            }
            let result = case.invoke(Some(&held), &mut host);
            let removed = if after_cas {
                Ok(())
            } else {
                case.fixture
                    .conn
                    .execute_batch("DROP TRIGGER exclusive_receipt_failure")
            };
            assert!(
                removed.is_ok(),
                "primary={:?}; trigger_cleanup={removed:?}",
                result.as_ref().err()
            );
            let failure = result.unwrap_err();
            assert!(failure.source.to_string().contains(if after_cas {
                "owned after-CAS receipt failure"
            } else {
                "owned exclusive settlement failure"
            }));
            assert_eq!(failure.input_id, Some(case.input.input_id));
            assert_eq!(failure.run_id, Some(case.input.run_id));
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(case.input.operation_key.as_str())
            );
            assert!(host.calls.contains(&"after"));
            assert_eq!(rows(&case.fixture.conn, &[]), before);
            assert_eq!(case.spool(), spool);
            case.assert_original_lock_available();
            let mut retry = TypedHost::new(host.until);
            case.invoke(Some(&held), &mut retry).unwrap();
            assert_owner_only(
                &case.fixture.run,
                &storage::get_run(&case.fixture.conn, case.input.run_id).unwrap(),
            );
            case.assert_original_lock_available();
        }
    }

    #[test]
    fn exclusive_receipt_requires_original_lock_and_positive_owner_startup_and_health_exclusion() {
        use std::os::unix::fs::OpenOptionsExt;

        for condition in [
            "held_lock",
            "missing_lock",
            "replaced_lock",
            "live_reconciler",
            "unknown_owner",
            "unknown_startup",
            "live_startup",
            "matching_health",
        ] {
            let mut case = ReceiptCase::new();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            let mut host = TypedHost::new(Instant::now() + Duration::from_secs(2));
            let mut original_lock = None;
            match condition {
                "held_lock" => {
                    original_lock =
                        RunLock::acquire(&case.fixture.conn, &case.fixture.run.owner()).unwrap()
                }
                "missing_lock" => std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap(),
                "replaced_lock" => {
                    std::fs::rename(
                        &case.artifacts.paths.owner_lock,
                        case.fixture._dir.path().join("original-owner.lock"),
                    )
                    .unwrap();
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&case.artifacts.paths.owner_lock)
                        .unwrap();
                }
                "live_reconciler" | "unknown_startup" => {
                    case.fixture
                        .conn
                        .execute(
                            "UPDATE member_runs SET owner_kind='reconciler' WHERE run_id=?1",
                            [case.fixture.run.run_id],
                        )
                        .unwrap();
                    case.fixture.run =
                        storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
                    if condition == "live_reconciler" {
                        host.owner = Termination::Alive;
                    } else {
                        host.startup = Termination::Unknown;
                    }
                }
                "unknown_owner" => host.owner = Termination::Unknown,
                "live_startup" => host.startup = Termination::Alive,
                "matching_health" => host.matching_health = true,
                _ => unreachable!(),
            }
            if condition == "held_lock" {
                assert!(original_lock.is_some());
            }
            let before = rows(&case.fixture.conn, &[]);
            let spool = case.spool();
            let failure = case.invoke(Some(&held), &mut host).unwrap_err();
            assert_eq!(failure.input_id, Some(case.input.input_id), "{condition}");
            assert_eq!(failure.run_id, Some(case.input.run_id), "{condition}");
            assert_eq!(rows(&case.fixture.conn, &[]), before, "{condition}");
            assert_eq!(case.spool(), spool, "{condition}");
            assert!(!host.calls.contains(&"after"), "{condition}");
            if matches!(condition, "unknown_startup" | "live_startup") {
                assert!(
                    host.calls.contains(&"startup"),
                    "noncreator history still requires startup exclusion"
                );
            }
            held.validate_binding(&case.settings.database_url, &case.placement)
                .unwrap();
            if let Some(lock) = original_lock {
                lock.validate(&case.fixture.conn, &case.fixture.run.owner())
                    .unwrap();
            }
        }
    }

    #[test]
    fn exclusive_receipt_revalidates_original_input_placement_and_budget_after_real_sql_entry() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex, mpsc};

        for change in ["digest", "state", "placement", "appearance", "deadline"] {
            let mut case = ReceiptCase::new();
            let journal: String = case
                .fixture
                .conn
                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "wal");
            let placement_row = case.fixture.conn.query_row(
                "SELECT mux_session,mux_window_id,mux_pane_id,backend,coding_agent,created_at FROM member_placements WHERE member_id=?1",
                [case.fixture.run.member_id], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, Option<String>>(2)?,row.get::<_, String>(3)?,row.get::<_, String>(4)?,row.get::<_, String>(5)?)),
            ).unwrap();
            let held = if change == "appearance" {
                crate::broker::deregister_member(
                    &mut case.fixture.conn,
                    case.fixture.run.member_id,
                )
                .unwrap();
                None
            } else {
                Some(
                    PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                        .unwrap()
                        .unwrap(),
                )
            };
            let before = rows(&case.fixture.conn, &[]);
            let spool = case.spool();
            let started = Instant::now();
            let cap = started
                + if change == "deadline" {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(2)
                };
            let mut host = TypedHost::new(cap);
            let (start, started_writer) = mpsc::sync_channel(1);
            let (locked, writer_locked) = mpsc::sync_channel(1);
            let (begin, began) = mpsc::sync_channel(1);
            let armed = Arc::new(AtomicBool::new(false));
            let observed = Arc::new(AtomicBool::new(false));
            let budgets = Arc::new(Mutex::new(Vec::new()));
            let enabled = armed.clone();
            let observed_begin = observed.clone();
            let sql_budgets = budgets.clone();
            let mut begin = Some(begin);
            case.fixture
                .conn
                .authorizer(Some(move |context: AuthContext<'_>| {
                    sql_budgets
                        .lock()
                        .unwrap()
                        .push(Deadline::inherited_or_new().instant());
                    if enabled.load(Ordering::Acquire)
                        && matches!(
                            context.action,
                            AuthAction::Transaction {
                                operation: TransactionOperation::Begin
                            }
                        )
                        && let Some(begin) = begin.take()
                    {
                        observed_begin.store(true, Ordering::Release);
                        begin.send(()).unwrap();
                    }
                    Authorization::Allow
                }))
                .unwrap();
            let mut start = Some(start);
            host.action = Some(Box::new(move |boundary, _, until| {
                if matches!(boundary, RecoveryBoundary::BeforeTakeover)
                    && let Some(start) = start.take()
                {
                    start.send(()).unwrap();
                    writer_locked
                        .recv_timeout(until.saturating_duration_since(Instant::now()))
                        .unwrap();
                    armed.store(true, Ordering::Release);
                }
                Ok(())
            }));
            let path = case.fixture.conn.path().unwrap().to_owned();
            let input_id = case.input.input_id;
            let member_id = case.fixture.run.member_id;
            let expected = Arc::new(Mutex::new(None));
            let published = expected.clone();
            let result = std::thread::scope(|scope| {
                let writer = scope.spawn(move || {
                    started_writer.recv_timeout(cap.saturating_duration_since(Instant::now())).unwrap();
                    let conn = Connection::open(path).unwrap();
                    conn.busy_timeout(Duration::ZERO).unwrap();
                    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
                    match change {
                        "digest" => { conn.execute("UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2", rusqlite::params!["e".repeat(64), input_id]).unwrap(); }
                        "state" => { conn.execute("UPDATE member_input_operations SET state='accepted' WHERE input_id=?1", [input_id]).unwrap(); }
                        "placement" => { conn.execute("DELETE FROM member_placements WHERE member_id=?1", [member_id]).unwrap(); }
                        "appearance" => { conn.execute(
                            "INSERT INTO member_placements(member_id,mux_session,mux_window_id,mux_pane_id,backend,coding_agent,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                            rusqlite::params![member_id,placement_row.0,placement_row.1,placement_row.2,placement_row.3,placement_row.4,placement_row.5],
                        ).unwrap(); }
                        "deadline" => {}
                        _ => unreachable!(),
                    }
                    *published.lock().unwrap() = Some(rows(&conn, &[]));
                    locked.send(()).unwrap();
                    began.recv_timeout(cap.saturating_duration_since(Instant::now())).unwrap();
                    let release = if change == "deadline" { cap } else { (Instant::now() + Duration::from_millis(40)).min(cap) };
                    std::thread::sleep(release.saturating_duration_since(Instant::now()));
                    conn.execute_batch("COMMIT").unwrap();
                });
                let result =
                    with_database_budget(&mut case.fixture.conn, Deadline::until(cap), |conn| {
                        Ok(super::reconcile_input_with_host(
                            conn,
                            &case.settings,
                            &case.input,
                            &case.fixture.run,
                            held.as_ref().map(|_| &case.placement),
                            held.as_ref(),
                            InputDisposition::Submitted,
                            "fixture:fixed-after-wait",
                            cap + Duration::from_secs(1),
                            &mut host,
                        ))
                    });
                let joined = writer.join();
                (result, joined)
            });
            let removed = case
                .fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            assert!(
                removed.is_ok(),
                "primary={:?}; writer={:?}; hook_cleanup={removed:?}",
                result.0.as_ref().map(|value| value.as_ref().err()),
                result.1.as_ref().err()
            );
            assert!(
                result.1.is_ok(),
                "owned SQL writer completed; primary={:?}",
                result.0.as_ref().map(|value| value.as_ref().err())
            );
            let failure = result.0.unwrap().unwrap_err();
            assert_eq!(failure.input_id, Some(case.input.input_id));
            assert!(
                observed.load(Ordering::Acquire),
                "{change}: real BEGIN is attempted while the separate writer owns Immediate"
            );
            assert!(!host.calls.contains(&"after"), "{change}");
            assert_eq!(
                rows(&case.fixture.conn, &[]),
                *expected.lock().unwrap().as_ref().unwrap(),
                "{change}"
            );
            if change == "deadline" {
                assert_eq!(rows(&case.fixture.conn, &[]), before);
                assert!(Instant::now() >= cap);
                assert!(
                    started.elapsed() < Duration::from_millis(750),
                    "the 200ms original bound has a fixed 550ms scheduling tolerance"
                );
            }
            let budgets = budgets.lock().unwrap();
            assert!(!budgets.is_empty());
            assert!(budgets.iter().all(|until| *until <= cap));
            assert_eq!(case.spool(), spool);
            case.assert_original_lock_available();
        }
    }
}
