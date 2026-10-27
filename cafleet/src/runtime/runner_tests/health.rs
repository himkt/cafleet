use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

mod observation_contract;
mod recovery_entry;
use super::*;
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
