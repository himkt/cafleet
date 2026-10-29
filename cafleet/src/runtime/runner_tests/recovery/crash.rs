use super::super::owned_child::OwnedChild;
use super::*;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

fn write_json(path: &Path, value: &Value) {
    let temporary = path.with_extension("provisional");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .unwrap();
    file.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
    file.sync_all().unwrap();
    std::fs::rename(temporary, path).unwrap();
}

struct Task {
    child: OwnedChild,
    log: PathBuf,
    deadline: Instant,
}
impl Task {
    fn spawn(case: &Case, mode: &str, marker: &Path, release: &Path, result: &Path) -> Self {
        let config = case.fixture._dir.path().join(format!("{mode}-config.json"));
        write_json(
            &config,
            &json!({"mode":mode,"database_url":case.settings.database_url,"run_id":case.fixture.run.run_id,"member_id":case.fixture.run.member_id,"expected_epoch":case.fixture.run.owner_epoch,"marker":marker,"release":release,"result":result}),
        );
        let log = case.fixture._dir.path().join(format!("{mode}-child.log"));
        let output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&log)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        let child = OwnedChild::new(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    runner_recovery_owned_process_child_path(),
                    "--nocapture",
                ])
                .env("CAFLEET_TEST_RECOVERY_CONFIG", config)
                .stdin(Stdio::null())
                .stdout(Stdio::from(output.try_clone().unwrap()))
                .stderr(Stdio::from(output))
                .spawn()
                .unwrap(),
        );
        Self {
            child,
            log,
            deadline,
        }
    }
    fn await_json(&mut self, path: &Path) -> Value {
        loop {
            match std::fs::read(path) {
                Ok(bytes) => return serde_json::from_slice(&bytes).unwrap(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("owned marker read failed: {error}"),
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "owned helper exited before {}: {}",
                path.display(),
                std::fs::read_to_string(&self.log).unwrap()
            );
            assert!(
                Instant::now() < self.deadline - Duration::from_millis(500),
                "owned marker timeout: {}",
                std::fs::read_to_string(&self.log).unwrap()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn finish(&mut self) {
        let status = self
            .child
            .poll_until(self.deadline - Duration::from_millis(500))
            .unwrap();
        if status.is_none() {
            self.child.terminate_until(self.deadline);
        }
        assert!(
            status.is_some_and(|status| status.success()),
            "owned helper: {status:?}: {}",
            std::fs::read_to_string(&self.log).unwrap()
        );
    }
    fn kill_and_reap(&mut self) {
        let process = ProcessIdentity::read(i64::from(self.child.id()))
            .unwrap()
            .expect("the selected owned crash helper remains alive");
        assert!(process.matches().unwrap());
        self.child.terminate_until(self.deadline);
        assert!(self.child.try_wait().unwrap().is_some());
        assert!(!process.matches().unwrap());
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.child.terminate_until(self.deadline);
    }
}

struct Release(PathBuf);
impl Release {
    fn release(&self) {
        std::fs::write(&self.0, b"release").unwrap();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        if let Err(error) = std::fs::write(&self.0, b"release") {
            eprintln!("owned recovery release failed: {error}");
        }
    }
}

#[test]
fn runner_recovery_hard_kill_after_actual_writer_fsync_replays_the_original_once() {
    let mut case = Case::creator();
    case.release();
    let marker = case.fixture._dir.path().join("fsynced.json");
    let release = case.fixture._dir.path().join("writer-release");
    let result = case.fixture._dir.path().join("unused-result.json");
    let mut task = Task::spawn(&case, "writer", &marker, &release, &result);
    let snapshot = task.await_json(&marker);
    case.refresh();
    assert_eq!(snapshot["run_id"], case.fixture.run.run_id);
    assert_eq!(snapshot["owner_epoch"], case.fixture.run.owner_epoch);
    assert_eq!(snapshot["owner_pid"], i64::from(task.child.id()));
    assert_eq!(
        snapshot["owner_start_identity"],
        case.fixture.run.owner_start_identity
    );
    assert_eq!(snapshot["event_seq"], 1);
    assert!(snapshot["spool_bytes"].as_u64().unwrap() > 36);
    assert!(events(&case).is_empty());
    task.kill_and_reap();
    let report = case.recover(&mut new_host()).unwrap();
    assert_eq!(counts(&report), (1, 3, 0));
    assert_eq!(report.status, RunStatus::Lost);
    let committed = events(&case);
    assert_eq!(committed.len(), 3);
    assert_eq!(committed[0].event.kind, RuntimeEventKind::InteractionOpened);
    assert_eq!(committed[0].event.event_seq, 1);
    assert_eq!(
        sha256_hex(&frame(
            &storage::original_transition(&committed[0]).unwrap()
        )),
        snapshot["spool_digest"].as_str().unwrap()
    );
    assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
    assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
    assert_eq!(events(&case), committed);
}

#[test]
fn runner_recovery_hard_kill_before_writer_append_records_loss_without_inventing_an_observation() {
    let mut case = Case::creator();
    case.release();
    let marker = case.fixture._dir.path().join("before-append.json");
    let release = case.fixture._dir.path().join("before-append-release");
    let result = case.fixture._dir.path().join("before-append-unused.json");
    let mut task = Task::spawn(&case, "unsynced", &marker, &release, &result);
    let snapshot = task.await_json(&marker);
    assert_eq!(snapshot["spool_bytes"], 0);
    assert!(events(&case).is_empty());
    case.refresh();
    task.kill_and_reap();
    let report = case.recover(&mut new_host()).unwrap();
    assert_eq!(counts(&report), (0, 1, 0));
    let committed = events(&case);
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].event.kind, RuntimeEventKind::RunnerLost);
    assert!(committed[0].interaction.is_none());
}

#[test]
fn runner_recovery_concurrent_owned_processes_contend_on_the_actual_original_lock_and_epoch() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    case.release();
    let marker = case.fixture._dir.path().join("takeover.json");
    let release = case.fixture._dir.path().join("takeover-release");
    let first_result = case.fixture._dir.path().join("first-result.json");
    let second_result = case.fixture._dir.path().join("second-result.json");
    let mut first = Task::spawn(&case, "hold", &marker, &release, &first_result);
    let gate = Release(release.clone());
    first.await_json(&marker);
    let mut second = Task::spawn(
        &case,
        "recover",
        &case.fixture._dir.path().join("second-marker.json"),
        &case.fixture._dir.path().join("second-release"),
        &second_result,
    );
    let deferred = second.await_json(&second_result);
    second.finish();
    assert_eq!(deferred["report"]["outcome"], "live_or_unreachable");
    assert_eq!(deferred["report"]["replayed_count"], 0);
    assert_eq!(deferred["spool_state"], "not_owner");
    gate.release();
    let recovered = first.await_json(&first_result);
    first.finish();
    assert_eq!(recovered["report"]["outcome"], "recovered");
    assert_eq!(recovered["report"]["replayed_count"], 1);
    assert_eq!(recovered["report"]["inbox_count"], 3);
    assert_eq!(recovered["spool_state"], "cleared");
    let events = events(&case);
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
            .count(),
        1
    );
    let mut message_ids = events
        .iter()
        .map(|row| row.event.message_id.unwrap())
        .collect::<Vec<_>>();
    message_ids.sort();
    message_ids.dedup();
    assert_eq!(message_ids.len(), 3);
}

#[test]
fn runner_recovery_owned_hard_kill_inside_exceptional_transaction_rolls_back_every_record() {
    let mut case = exceptional_case_with_concurrent_reads();
    let value = opening(&case.fixture.run, 1, 1);
    storage::persist_transition(
        &mut case.fixture.conn,
        &case.fixture.run.owner(),
        &value,
        PersistenceMode::Live,
    )
    .unwrap();
    case.release();
    std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
    let before = rows(&case.fixture.conn);
    let marker = case.fixture._dir.path().join("transaction-open.json");
    let release = case.fixture._dir.path().join("transaction-release");
    let result = case.fixture._dir.path().join("transaction-result.json");
    let mut task = Task::spawn(&case, "transaction", &marker, &release, &result);
    let pending = task.await_json(&marker);
    eprintln!("exceptional parent commit marker: {pending}");
    assert_eq!(pending["journal_mode"], "wal");
    assert_eq!(pending["event_rows"], 2);
    assert_eq!(pending["run_rows"], 1);
    assert_eq!(pending["interaction_rows"], 1);
    let read_started = Instant::now();
    let read = with_database_deadline(
        &mut case.fixture.conn,
        task.deadline - Duration::from_millis(500),
        |conn| {
            conn.query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(crate::broker::members::db_err)
        },
    );
    eprintln!(
        "exceptional parent read elapsed={:?}, result={read:?}, child_log={}",
        read_started.elapsed(),
        std::fs::read_to_string(&task.log).unwrap()
    );
    assert_eq!(read.unwrap(), 1);
    assert_eq!(
        rows(&case.fixture.conn),
        before,
        "the open exceptional transaction has no visible partial commit"
    );
    task.kill_and_reap();
    assert_eq!(
        rows(&case.fixture.conn),
        before,
        "SQLite rolls back the killed owner's actual transaction"
    );
    let attestation = OwnerTerminationAttestation {
        expected_run_id: case.fixture.run.run_id,
        expected_owner_epoch: case.fixture.run.owner_epoch,
        evidence_ref: "audit:owned-transaction-fixture".into(),
    };
    let mut host = new_host();
    host.process = Evidence::Unknown;
    let report = recover_member_with_host(
        &mut case.fixture.conn,
        &case.settings,
        case.fixture.run.member_id,
        Some(&attestation),
        host.deadline,
        &mut host,
    )
    .unwrap();
    assert_eq!(counts(&report), (0, 2, 0));
    assert_eq!(events(&case).len(), 3);
}

#[test]
fn runner_recovery_concurrent_exceptional_processes_commit_one_gap_and_reject_the_original_epoch() {
    let mut case = exceptional_case_with_concurrent_reads();
    let value = opening(&case.fixture.run, 1, 1);
    storage::persist_transition(
        &mut case.fixture.conn,
        &case.fixture.run.owner(),
        &value,
        PersistenceMode::Live,
    )
    .unwrap();
    case.release();
    std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
    let old_epoch = case.fixture.run.owner_epoch;
    let old_gap = case.fixture.run.observation_gap_count;
    let marker = case
        .fixture
        ._dir
        .path()
        .join("exceptional-before-commit.json");
    let release = case.fixture._dir.path().join("exceptional-release");
    let first_result = case.fixture._dir.path().join("exceptional-first.json");
    let mut first = Task::spawn(&case, "exceptional_first", &marker, &release, &first_result);
    let gate = Release(release.clone());
    let pending = first.await_json(&marker);
    eprintln!("exceptional first commit marker: {pending}");
    assert_eq!(pending["journal_mode"], "wal");
    assert_eq!(pending["event_rows"], 2);
    let second_marker = case
        .fixture
        ._dir
        .path()
        .join("exceptional-second-snapshot.json");
    let second_result = case.fixture._dir.path().join("exceptional-second.json");
    let mut second = Task::spawn(
        &case,
        "exceptional_stale",
        &second_marker,
        &case.fixture._dir.path().join("unused-exceptional-release"),
        &second_result,
    );
    let second_read_started = Instant::now();
    let captured = second.await_json(&second_marker);
    eprintln!(
        "exceptional second snapshot wait={:?}, snapshot={captured}, first_log={}, second_log={}",
        second_read_started.elapsed(),
        std::fs::read_to_string(&first.log).unwrap(),
        std::fs::read_to_string(&second.log).unwrap()
    );
    assert_eq!(captured["epoch"], old_epoch);
    assert_eq!(captured["journal_mode"], "wal");
    gate.release();
    let accepted = first.await_json(&first_result);
    first.finish();
    eprintln!(
        "exceptional first completion: {}",
        std::fs::read_to_string(&first.log).unwrap()
    );
    let rejected = second.await_json(&second_result);
    second.finish();
    assert_eq!(accepted["report"]["replayed_count"], 0);
    assert_eq!(accepted["report"]["inbox_count"], 2);
    assert_eq!(accepted["spool_state"], "database_only");
    assert!(!rejected["error"].as_str().unwrap().is_empty());
    assert_eq!(rejected["expected_epoch"], old_epoch);
    assert_eq!(rejected["current_epoch"], old_epoch + 1);
    let current = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
    assert_eq!(current.owner_epoch, old_epoch + 1);
    assert_eq!(current.observation_gap_count, old_gap + 1);
    assert_eq!(events(&case).len(), 3);
}

fn exceptional_case_with_concurrent_reads() -> Case {
    let case = Case::new();
    let mode: String = case
        .fixture
        .conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    case
}

struct PersistGate {
    run: RunRecord,
    lock: Arc<RunLock>,
    paths: RuntimePaths,
    marker: PathBuf,
    release: PathBuf,
    written: AtomicBool,
    boundary: WriterBoundary,
}
impl WriterHooks for PersistGate {
    fn before(&self, boundary: WriterBoundary, deadline: Instant) -> TestResult<()> {
        if std::mem::discriminant(&boundary) != std::mem::discriminant(&self.boundary) {
            return Ok(());
        }
        if !self.written.swap(true, Ordering::AcqRel) {
            let observed = crate::db::connect_read_only(&format!(
                "sqlite:///{}",
                self.run.database_identity_json["canonical_path"]
                    .as_str()
                    .unwrap()
            ))?;
            self.lock.validate(&observed, &self.run.owner())?;
            let bytes = std::fs::read(&self.paths.outbox).unwrap();
            write_json(
                &self.marker,
                &json!({"run_id":self.run.run_id,"owner_epoch":self.run.owner_epoch,"owner_pid":self.run.owner_pid,"owner_start_identity":self.run.owner_start_identity,"event_seq":1,"spool_bytes":bytes.len(),"spool_digest":sha256_hex(&bytes)}),
            );
        }
        while Instant::now() < deadline {
            if self.release.exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(CafleetError::DeadlineExceeded)
    }
}

pub(crate) fn runner_recovery_owned_process_child_path() -> &'static str {
    test_path!(runner_recovery_owned_process_child)
}

#[test]
#[ignore = "owned isolated recovery process helper"]
fn runner_recovery_owned_process_child() {
    let config: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("CAFLEET_TEST_RECOVERY_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let mode = config["mode"].as_str().unwrap();
    let database_url = config["database_url"].as_str().unwrap();
    let run_id = config["run_id"].as_i64().unwrap();
    let member_id = config["member_id"].as_i64().unwrap();
    let marker = PathBuf::from(config["marker"].as_str().unwrap());
    let release = PathBuf::from(config["release"].as_str().unwrap());
    let result = PathBuf::from(config["result"].as_str().unwrap());
    let mut conn = crate::db::connect(database_url).unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(4);
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    eprintln!(
        "owned recovery helper mode={mode}, journal_mode={journal_mode}, elapsed={:?}",
        started.elapsed()
    );
    if matches!(mode, "writer" | "unsynced") {
        let original = storage::get_run(&conn, run_id).unwrap();
        let lock = Arc::new(RunLock::acquire(&conn, &original.owner()).unwrap().unwrap());
        let owner = lock
            .transfer_to_runner(
                &mut conn,
                &original.owner(),
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
        conn.execute(
            "UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2",
            rusqlite::params![NOW, run_id],
        )
        .unwrap();
        storage::reserve_event_sequences(&mut conn, &owner).unwrap();
        let run = storage::get_run(&conn, run_id).unwrap();
        let paths = validate_run_paths(&conn, &run).unwrap();
        let hooks = Arc::new(PersistGate {
            run: run.clone(),
            lock: lock.clone(),
            paths: paths.clone(),
            marker,
            release,
            written: AtomicBool::new(false),
            boundary: if mode == "writer" {
                WriterBoundary::Persist
            } else {
                WriterBoundary::Append
            },
        });
        let mut writer = EventWriter::start_with_hooks(
            database_url.into(),
            run.clone(),
            lock,
            paths.outbox,
            hooks,
        )
        .unwrap();
        writer.submit(opening(&run, 1, 1)).unwrap();
        let status = writer.finish(deadline);
        panic!("the selected writer should be killed after fsync: {status:?}");
    }
    let settings = Settings::for_database(
        std::path::Path::new(crate::db::database_path(database_url).unwrap()),
        None,
    );
    let mut host = Host::new(deadline);
    let claim;
    let attestation = if matches!(
        mode,
        "transaction" | "exceptional_first" | "exceptional_stale"
    ) {
        let marker = marker.clone();
        let release = release.clone();
        if mode != "exceptional_stale" {
            let changed = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
                String,
                std::collections::BTreeSet<i64>,
            >::new()));
            let updates = changed.clone();
            conn.update_hook(Some(
                move |_: rusqlite::hooks::Action, _: &str, table: &str, rowid: i64| {
                    updates
                        .lock()
                        .expect("owned transaction update trace")
                        .entry(table.into())
                        .or_default()
                        .insert(rowid);
                },
            ))
            .unwrap();
            let scenario = mode.to_owned();
            let hook_journal_mode = journal_mode.clone();
            conn.commit_hook(Some(move || {
            let trace=changed.lock().expect("owned transaction commit trace");
            let count=|table:&str|trace.get(table).map_or(0,std::collections::BTreeSet::len);
            let state=json!({"transaction":"before-exceptional-commit","event_rows":count("member_runtime_events"),"run_rows":count("member_runs"),"interaction_rows":count("member_interactions"),"journal_mode":hook_journal_mode,"helper_elapsed_ms":started.elapsed().as_millis(),"remaining_ms":deadline.saturating_duration_since(Instant::now()).as_millis()});
            drop(trace);
            eprintln!("owned recovery commit hook mode={scenario}, entry={state}");
            write_json(&marker,&state);
            while Instant::now()<deadline && !release.exists(){std::thread::sleep(Duration::from_millis(1));}
            let released=release.exists();
            eprintln!("owned recovery commit hook mode={scenario}, released={released}, expired={}, elapsed={:?}", Instant::now()>=deadline, started.elapsed());
            !released
        })).unwrap();
        }
        claim = OwnerTerminationAttestation {
            expected_run_id: run_id,
            expected_owner_epoch: config["expected_epoch"].as_i64().unwrap(),
            evidence_ref: "audit:owned-transaction-fixture".into(),
        };
        host.process = Evidence::Unknown;
        Some(&claim)
    } else {
        assert!(matches!(mode, "hold" | "recover"));
        None
    };
    if mode == "hold" {
        host.action = Some(Box::new(move |boundary, run, until| {
            if boundary == "after_takeover" {
                write_json(&marker, &json!({"epoch":run.owner_epoch}));
                while Instant::now() < until && !release.exists() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                if !release.exists() {
                    return Err(CafleetError::DeadlineExceeded);
                }
            }
            Ok(())
        }));
    } else if mode == "exceptional_stale" {
        let mut written = false;
        host.action = Some(Box::new(move |boundary, run, _| {
            if boundary == "after_snapshot" && !written {
                write_json(
                    &marker,
                    &json!({"epoch":run.owner_epoch,"journal_mode":journal_mode,"helper_elapsed_ms":started.elapsed().as_millis()}),
                );
                written = true;
            }
            Ok(())
        }));
    }
    let recovery = recover_member_with_host(
        &mut conn,
        &settings,
        member_id,
        attestation,
        deadline,
        &mut host,
    );
    if mode == "exceptional_stale" {
        let error = match recovery {
            Ok(_) => {
                panic!("the original exceptional epoch becomes stale after the competing commit")
            }
            Err(error) => error,
        };
        let current = storage::get_run(&conn, run_id).unwrap();
        write_json(
            &result,
            &json!({"error":error.message(),"expected_epoch":config["expected_epoch"],"current_epoch":current.owner_epoch}),
        );
        return;
    }
    let report = recovery.unwrap();
    let run = storage::get_run(&conn, run_id).unwrap();
    let actual = ProcessIdentity::current().unwrap();
    let spool_state = if matches!(mode, "transaction" | "exceptional_first") {
        "database_only"
    } else if run.owner_pid == actual.pid && run.owner_start_identity == actual.start_identity {
        let held = RunLock::acquire_released(&conn, &run.owner())
            .unwrap()
            .unwrap();
        held.validate(&conn, &run.owner()).unwrap();
        let paths = validate_run_paths(&conn, &run).unwrap();
        match std::fs::read(paths.outbox) {
            Ok(bytes) => {
                assert!(bytes.is_empty());
                "cleared"
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "cleared",
            Err(error) => panic!("owned final spool read failed: {error}"),
        }
    } else {
        "not_owner"
    };
    write_json(
        &result,
        &json!({"report":crate::presentation::runtime_recovery(&report),"spool_state":spool_state}),
    );
}
