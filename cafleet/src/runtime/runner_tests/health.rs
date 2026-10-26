use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

mod observation_contract;
use super::*;
use crate::error::CafleetError;
use rusqlite::params;

fn activate(fixture: &mut Fixture) -> (RunArtifacts, RunLock) {
    let artifacts = fixture.artifacts();
    let held = RunLock::acquire(&fixture.conn, &fixture.run.owner())
        .unwrap()
        .unwrap();
    let process = ProcessIdentity::current().unwrap();
    let next = held
        .transfer_to_runner(&mut fixture.conn, &fixture.run.owner(), process.clone())
        .unwrap();
    fixture.conn.execute("UPDATE member_runs SET status='running',runner_pid=?1,runner_start_identity=?2,child_pid=?1,child_start_identity=?2 WHERE run_id=?3",params![process.pid,process.start_identity,next.run_id]).unwrap();
    fixture.run = storage::get_run(&fixture.conn, next.run_id).unwrap();
    (artifacts, held)
}

struct Probe {
    change: Option<&'static str>,
    calls: usize,
    deadline: Instant,
}

impl HealthProbe for Probe {
    fn probe(
        &mut self,
        run: &RunRecord,
        deadline: Instant,
    ) -> Result<HealthSnapshot, CafleetError> {
        self.calls += 1;
        assert_eq!(deadline, self.deadline);
        if self.change == Some("unreachable") {
            return Err(CafleetError::App("fixture endpoint unreachable".into()));
        }
        let mut reply = HealthSnapshot {
            identity: ControlIdentity {
                database_identity_json: run.database_identity_json.clone(),
                fleet_id: run.fleet_id,
                member_id: run.member_id,
                director_member_id: run.director_member_id,
                run_id: run.run_id,
                owner_epoch: run.owner_epoch,
                launch_nonce: run.launch_nonce.clone(),
            },
            runner: ProcessIdentity {
                pid: run.runner_pid.unwrap(),
                start_identity: run.runner_start_identity.clone().unwrap(),
            },
            child: Some(ProcessIdentity {
                pid: run.child_pid.unwrap(),
                start_identity: run.child_start_identity.clone().unwrap(),
            }),
            output_seq: 120,
            observation: None,
        };
        match self.change {
            Some("run") => reply.identity.run_id += 1,
            Some("epoch") => reply.identity.owner_epoch += 1,
            Some("nonce") => reply.identity.launch_nonce = "b".repeat(64),
            Some("database") => {
                reply.identity.database_identity_json["instance_id"] = json!("0".repeat(32))
            }
            Some("runner") => reply.runner.start_identity = "different-generation".into(),
            Some("child") => {
                reply.child.as_mut().unwrap().start_identity = "different-child-generation".into()
            }
            Some("director") => reply.identity.director_member_id += 1,
            None => {}
            Some(other) => panic!("unknown health fixture variant: {other}"),
        }
        Ok(reply)
    }
}

struct SilentServer {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SilentServer {
    fn start(path: &std::path::Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let observed = stop.clone();
        let thread = std::thread::spawn(move || {
            let end = Instant::now() + Duration::from_secs(2);
            let mut connections = Vec::new();
            while !observed.load(Ordering::SeqCst) && Instant::now() < end {
                match listener.accept() {
                    Ok((connection, _)) => connections.push(connection),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("fixture socket accept failed: {error}"),
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for SilentServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn counts(conn: &Connection) -> (i64, i64, i64) {
    let scalar = |table| {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    (
        scalar("member_interactions"),
        scalar("member_runtime_events"),
        scalar("messages"),
    )
}

#[test]
fn runner_health_matching_identity_refreshes_reachability_without_claiming_composer_eligibility() {
    let mut fixture = Fixture::new();
    let (_artifacts, _held) = activate(&mut fixture);
    fixture.conn.execute("UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z',reachability='suspect' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut probe = Probe {
        change: None,
        calls: 0,
        deadline,
    };
    let before = counts(&fixture.conn);
    let check =
        check_input_health_with_probe(&mut fixture.conn, fixture.run.run_id, deadline, &mut probe)
            .unwrap();
    assert!(check.matched);
    assert_eq!(probe.calls, 1);
    let run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    assert_eq!(run.reachability, Reachability::Reachable);
    assert_ne!(run.last_health_at.as_deref(), Some("2000-01-01T00:00:00Z"));
    assert_eq!(run.coverage, Coverage::Unknown);
    assert_eq!(run.event_seq_reserved_through, 0);
    assert_eq!(counts(&fixture.conn), before);
    assert_eq!(run.status, RunStatus::Running);
}

#[test]
fn runner_health_unreachable_or_stale_generation_holds_input_without_closing_or_allocating_events()
{
    for change in [
        "unreachable",
        "run",
        "epoch",
        "nonce",
        "database",
        "runner",
        "child",
        "director",
    ] {
        let mut fixture = Fixture::new();
        let (_artifacts, _held) = activate(&mut fixture);
        storage::reserve_event_sequences(&mut fixture.conn, &fixture.run.owner()).unwrap();
        let evidence=crate::broker::runtime::evidence::EvidencePolicy::metadata().apply(&json!({"backend":"codex","native_version":"0.163.0","profile_version":null,"rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
        storage::persist_transition(
            &mut fixture.conn,
            &fixture.run.owner(),
            &ObservedTransition {
                run_id: fixture.run.run_id,
                event_seq: 1,
                episode_seq: Some(1),
                kind: RuntimeEventKind::InteractionOpened,
                classification: Some(InteractionClass::Question),
                observed_at: "2026-10-26T11:00:00Z".into(),
                evidence_json: evidence,
                close_reason: None,
                cause_json: None,
            },
            PersistenceMode::Live,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut probe = Probe {
            change: Some(change),
            calls: 0,
            deadline,
        };
        let before = counts(&fixture.conn);
        let check = check_input_health_with_probe(
            &mut fixture.conn,
            fixture.run.run_id,
            deadline,
            &mut probe,
        )
        .unwrap();
        assert!(!check.matched, "{change}");
        let run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        assert_eq!(run.reachability, Reachability::Suspect, "{change}");
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.event_seq_reserved_through, 256);
        assert_eq!(counts(&fixture.conn), before);
        assert_eq!(
            fixture
                .conn
                .query_row(
                    "SELECT closed_at FROM member_interactions WHERE run_id=?1",
                    [run.run_id],
                    |row| row.get::<_, Option<String>>(0)
                )
                .unwrap(),
            None
        );
    }
}

#[test]
fn runner_health_expired_budget_performs_no_probe_or_registry_mutation() {
    let mut fixture = Fixture::new();
    let (_artifacts, _held) = activate(&mut fixture);
    let deadline = Instant::now() - Duration::from_secs(1);
    let mut probe = Probe {
        change: None,
        calls: 0,
        deadline,
    };
    let before = std::fs::read(fixture.conn.path().unwrap()).unwrap();
    assert!(
        check_input_health_with_probe(&mut fixture.conn, fixture.run.run_id, deadline, &mut probe)
            .is_err()
    );
    assert_eq!(probe.calls, 0);
    assert_eq!(std::fs::read(fixture.conn.path().unwrap()).unwrap(), before);
}

#[test]
fn runner_health_real_silent_endpoint_is_bounded_and_probe_is_read_only() {
    let mut fixture = Fixture::new();
    let (artifacts, _held) = activate(&mut fixture);
    let _server = SilentServer::start(&artifacts.paths.control_socket);
    let before = std::fs::read(fixture.conn.path().unwrap()).unwrap();
    let start = Instant::now();
    assert!(probe_health(&fixture.run, start + Duration::from_millis(80)).is_err());
    assert!(start.elapsed() < Duration::from_millis(750));
    assert_eq!(std::fs::read(fixture.conn.path().unwrap()).unwrap(), before);
}

#[test]
fn runner_doctor_runtime_diagnosis_reports_leftovers_and_preserves_registry_and_artifact_bytes() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    artifacts
        .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
        .unwrap();
    fixture
        .conn
        .execute(
            "UPDATE member_runs SET status='failed' WHERE run_id=?1",
            [fixture.run.run_id],
        )
        .unwrap();
    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    let files = [
        std::path::PathBuf::from(fixture.conn.path().unwrap()),
        artifacts.paths.namespace_manifest.clone(),
        artifacts.paths.descriptor.clone(),
        artifacts.paths.owner_lock.clone(),
    ];
    let before = files
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect::<Vec<_>>();
    let start = Instant::now();
    let diagnosis = diagnose_runtime(
        &fixture.conn,
        fixture.run.run_id,
        start + Duration::from_millis(80),
    )
    .unwrap();
    assert!(start.elapsed() < Duration::from_millis(750));
    assert!(
        diagnosis
            .diagnostics
            .iter()
            .any(|line| line.contains("descriptor"))
    );
    assert!(
        diagnosis
            .diagnostics
            .iter()
            .any(|line| line.contains("recover"))
    );
    for (path, bytes) in files.iter().zip(before) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn runner_registry_show_returns_historical_health_without_contacting_a_live_endpoint() {
    let mut fixture = Fixture::new();
    let (artifacts, _held) = activate(&mut fixture);
    let _server = SilentServer::start(&artifacts.paths.control_socket);
    fixture
        .conn
        .execute(
            "UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1",
            [fixture.run.run_id],
        )
        .unwrap();
    let before = counts(&fixture.conn);
    let start = Instant::now();
    let member =
        crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
            .unwrap()
            .unwrap();
    assert!(start.elapsed() < Duration::from_millis(750));
    assert_eq!(
        crate::presentation::member(&member)["runtime"]["last_health_at"],
        "2000-01-01T00:00:00Z"
    );
    assert_eq!(counts(&fixture.conn), before);
    assert_eq!(
        storage::get_run(&fixture.conn, fixture.run.run_id)
            .unwrap()
            .status,
        RunStatus::Running
    );
}
