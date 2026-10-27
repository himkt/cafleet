use super::*;
use crate::broker::runtime::*;
use crate::runtime::runner_tests::Fixture;
mod controller;
mod input_drain;
mod outbox;
mod refresh;
mod support;
mod worker;

struct WriterFixture {
    fixture: Fixture,
    artifacts: RunArtifacts,
    lock: Arc<RunLock>,
}
impl WriterFixture {
    fn new() -> Self {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let lock = Arc::new(
            RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap(),
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
        Self {
            fixture,
            artifacts,
            lock,
        }
    }
    fn start(&self, path: PathBuf) -> Result<EventWriter> {
        EventWriter::start(
            format!("sqlite:///{}", self.fixture.conn.path().unwrap()),
            self.fixture.run.clone(),
            self.lock.clone(),
            path,
        )
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

fn transition(fixture: &WriterFixture, sequence: i64) -> ObservedTransition {
    let evidence=storage::evidence::EvidencePolicy::metadata().apply(&serde_json::json!({"backend":fixture.fixture.run.backend,"native_version":fixture.fixture.run.native_version,"profile_version":fixture.fixture.run.profile_version,"rule_id":"runner-lifecycle","screen_seq":sequence,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
    ObservedTransition {
        run_id: fixture.fixture.run.run_id,
        event_seq: sequence,
        episode_seq: None,
        kind: RuntimeEventKind::CoverageUnknown,
        classification: None,
        observed_at: "2026-10-26T16:40:00.000000+00:00".into(),
        evidence_json: evidence,
        close_reason: None,
        cause_json: Some(serde_json::json!({"kind":"coverage_unknown"})),
    }
}

fn frame(transition: &ObservedTransition) -> Vec<u8> {
    let body = serde_json::to_vec(transition).unwrap();
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(&Sha256::digest(&body));
    bytes.extend(body);
    bytes
}

fn independent_frames(mut bytes: &[u8]) -> Vec<ObservedTransition> {
    let mut values = Vec::new();
    while !bytes.is_empty() {
        assert!(bytes.len() >= 36);
        let length = usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap();
        assert!(length > 0 && length <= 16 * 1024 * 1024);
        assert!(bytes.len() >= 36 + length);
        let body = &bytes[36..36 + length];
        assert_eq!(&bytes[4..36], &Sha256::digest(body)[..]);
        values.push(serde_json::from_slice(body).unwrap());
        bytes = &bytes[36 + length..];
    }
    values
}

fn assert_start_rejected(fixture: &WriterFixture, path: PathBuf) {
    let result = fixture.start(path);
    let rejected = result.is_err();
    if let Ok(mut writer) = result {
        writer.finish(Instant::now() + Duration::from_millis(100));
    }
    assert!(
        rejected,
        "writer startup validates the recorded owned outbox before accepting work"
    );
}

#[test]
fn runner_batch2_writer_starts_with_the_exact_private_recorded_outbox() {
    let fixture = WriterFixture::new();
    let mut writer = fixture
        .start(fixture.artifacts.paths.outbox.clone())
        .unwrap();
    assert_eq!(
        writer.finish(Instant::now() + Duration::from_secs(1)),
        (0, 0)
    );
}

#[test]
fn runner_batch2_writer_foreign_outbox_path_fails_before_worker_start() {
    let fixture = WriterFixture::new();
    let foreign = fixture.fixture._dir.path().join("foreign-outbox");
    std::fs::write(&foreign, []).unwrap();
    assert_start_rejected(&fixture, foreign.clone());
    assert_eq!(std::fs::read(foreign).unwrap(), b"");
}

#[test]
fn runner_batch2_writer_missing_or_symlinked_outbox_fails_before_worker_start() {
    for symlink in [false, true] {
        let fixture = WriterFixture::new();
        let path = &fixture.artifacts.paths.outbox;
        std::fs::remove_file(path).unwrap();
        let target = fixture.fixture._dir.path().join("untouched-target");
        std::fs::write(&target, b"original target").unwrap();
        if symlink {
            std::os::unix::fs::symlink(&target, path).unwrap();
        }
        assert_start_rejected(&fixture, path.clone());
        assert_eq!(std::fs::read(target).unwrap(), b"original target");
    }
}

#[test]
fn runner_batch2_writer_stale_owner_is_rejected_before_worker_start() {
    let mut fixture = WriterFixture::new();
    fixture.fixture.run.owner_epoch += 1;
    assert_start_rejected(&fixture, fixture.artifacts.paths.outbox.clone());
}

#[test]
fn runner_batch2_writer_spool_sync_is_pending_until_the_actual_sqlite_commit() {
    let fixture = WriterFixture::new();
    let blocking = crate::db::connect(&format!(
        "sqlite:///{}",
        fixture.fixture.conn.path().unwrap()
    ))
    .unwrap();
    blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
    let value = transition(&fixture, 1);
    let mut writer = fixture
        .start(fixture.artifacts.paths.outbox.clone())
        .unwrap();
    writer.submit(value.clone()).unwrap();
    let until = deadline();
    let bytes = loop {
        let bytes = std::fs::read(&fixture.artifacts.paths.outbox).unwrap();
        if bytes.len() == frame(&value).len() {
            break bytes;
        }
        assert!(
            Instant::now() < until,
            "complete pending frame was not written: {:?}",
            writer.diagnostic()
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(independent_frames(&bytes), vec![value]);
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    blocking.execute_batch("ROLLBACK").unwrap();
    let until = deadline();
    loop {
        let count = fixture
            .fixture
            .conn
            .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        if count == 1 {
            break;
        }
        assert!(
            Instant::now() < until,
            "event did not commit: {:?}",
            writer.diagnostic()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(writer.finish(deadline()), (0, 0));
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
