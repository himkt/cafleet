use super::*;
use crate::coding_agent::ProfileCatalog;
use crate::runtime::runner_tests::owned_child::OwnedChild;
use std::process::{Command, Stdio};

pub(super) const VERSION: &str = "synthetic-0.0.1";
pub(super) const PROFILE: &str = "synthetic-profile-1";
pub(super) const NOW: &str = "2026-10-26T16:40:00.000000+00:00";
pub(super) fn definitions() -> Value {
    serde_json::from_str(include_str!(
        "../../../runner_tests/fixtures/synthetic-profile.json"
    ))
    .unwrap()
}
pub(super) fn catalog() -> ProfileCatalog {
    ProfileCatalog::from_definitions(&definitions()).unwrap()
}
pub(super) fn configure(fixture: &mut WriterFixture) {
    fixture.fixture.conn.execute("UPDATE member_runs SET native_version=?1,profile_version=?2,coverage='tested',runner_pid=owner_pid,runner_start_identity=owner_start_identity WHERE run_id=?3",rusqlite::params![VERSION,PROFILE,fixture.fixture.run.run_id]).unwrap();
    fixture.fixture.run =
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
}
pub(super) struct Clock {
    origin: Instant,
    millis: AtomicU64,
}
impl Clock {
    pub(super) fn new() -> Self {
        Self {
            origin: Instant::now(),
            millis: AtomicU64::new(0),
        }
    }
    pub(super) fn set(&self, millis: u64) {
        self.millis.store(millis, Ordering::Release);
    }
}
impl ObservationClock for Clock {
    fn sample(&self) -> ObservationMoment {
        ObservationMoment {
            monotonic: self.origin + Duration::from_millis(self.millis.load(Ordering::Acquire)),
            observed_at: NOW.into(),
        }
    }
    fn kind(&self) -> ObservationClockKind {
        ObservationClockKind::Deterministic
    }
}
pub(super) struct ChildGuard {
    pub child: OwnedChild,
    pub identity: ProcessIdentity,
}
impl ChildGuard {
    pub(super) fn spawn(executable: &std::path::Path, arguments: &[String]) -> Self {
        let child = OwnedChild::new(
            Command::new(executable)
                .args(arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let identity = ProcessIdentity::read(i64::from(child.id()))
            .unwrap()
            .expect("owned child generation");
        Self { child, identity }
    }
}
pub(super) fn running(fixture: &mut WriterFixture, child: &ProcessIdentity) {
    fixture.fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![child.pid,child.start_identity,NOW,fixture.fixture.run.run_id]).unwrap();
    fixture.fixture.run =
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
}
pub(super) fn form(operation: &str) -> Vec<u8> {
    format!("\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: {operation}\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ").into_bytes()
}
