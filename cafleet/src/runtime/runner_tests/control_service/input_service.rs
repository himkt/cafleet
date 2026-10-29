use super::observation_services::private_write;
use super::*;
use crate::error::CafleetError;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

mod native;
pub(super) fn native(path: &Path) {
    install_panic_record(path.parent().unwrap(), "native");
    native::run(path);
}
pub(super) const NATIVE_ENV: &str = "CAFLEET_REVIEW_SERVICE_INPUT_NATIVE";
const SCREEN: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
const PANIC_RECORD_LIMIT: usize = 16 * 1024;
const PANIC_DIRECTORY: &str = "service-input-panics";

pub(super) fn install_panic_record(directory: &Path, role: &'static str) {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    assert!(matches!(role, "native" | "runner"));
    let directory = directory.join(PANIC_DIRECTORY);
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700,
    );
    let pid = std::process::id();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(format!("service-input-panic-{role}-{pid}.txt")))
        .unwrap();
    let record = std::sync::Mutex::new(Some(file));
    let prior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        match record.lock() {
            Ok(mut slot) => {
                if let Some(mut file) = slot.take() {
                    let message = format!(
                        "role={role}; pid={pid}; location={:?}; panic={info}\n",
                        info.location(),
                    );
                    let bytes = &message.as_bytes()[..message.len().min(PANIC_RECORD_LIMIT)];
                    if let Err(cause) = file.write_all(bytes).and_then(|()| file.flush()) {
                        eprintln!("owned {role} panic record failed: {cause}");
                    }
                }
            }
            Err(cause) => eprintln!("owned {role} panic record lock failed: {cause}"),
        }
        prior(info);
    }));
}

pub(super) fn panic_records(directory: &Path) -> std::io::Result<Vec<(String, String)>> {
    use std::os::unix::fs::PermissionsExt;

    let mut records = Vec::new();
    for entry in std::fs::read_dir(directory.join(PANIC_DIRECTORY))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("service-input-panic-") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if records.len() >= 2
            || !metadata.is_file()
            || metadata.permissions().mode() & 0o777 != 0o600
        {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "owned panic records require two private regular files at most",
            ));
        }
        let mut bytes = Vec::new();
        File::open(entry.path())?
            .take(u64::try_from(PANIC_RECORD_LIMIT + 1).unwrap())
            .read_to_end(&mut bytes)?;
        if bytes.len() > PANIC_RECORD_LIMIT {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "owned panic record exceeds its original limit",
            ));
        }
        records.push((name, String::from_utf8_lossy(&bytes).into_owned()));
    }
    records.sort();
    Ok(records)
}

struct Clock;
impl InputDeadlineClock for Clock {
    fn sample(&mut self) -> Result<InputClockSample, CafleetError> {
        let before = Instant::now();
        let value = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Known(
            rustix::time::ClockId::Monotonic,
        ))
        .map_err(io_error)?;
        let after = Instant::now();
        let seconds = u64::try_from(value.tv_sec).unwrap();
        let nanos = u64::try_from(value.tv_nsec).unwrap();
        assert!(nanos < 1_000_000_000);
        Ok(InputClockSample {
            before,
            monotonic_ns: seconds
                .checked_mul(1_000_000_000)
                .unwrap()
                .checked_add(nanos)
                .unwrap(),
            after,
        })
    }
}
fn expiration(until: Instant) -> u64 {
    InputExpiration::capture(until, &mut Clock)
        .unwrap()
        .monotonic_ns()
}
fn pause(until: Instant) {
    assert!(Instant::now() < until, "original owned service deadline");
    std::thread::sleep(
        Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
    );
}

fn configure(fixture: &mut Fixture, spec: &mut SpawnSpec, until: Instant) {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture._dir.path().join(PANIC_DIRECTORY))
        .unwrap();
    let mut catalog: Value =
        serde_json::from_str(include_str!("../fixtures/synthetic-profile.json")).unwrap();
    let mut executable = catalog["fixtures"][0].clone();
    executable["id"] = json!("synthetic-service-submission");
    catalog["fixtures"].as_array_mut().unwrap().push(executable);
    let rule = &mut catalog["profiles"][0]["rules"][0];
    rule["id"] = json!("synthetic-service-composer");
    rule["observation"] = json!("composer_empty");
    rule["region"]["start_pattern"] = json!("^Synthetic state$");
    rule["region"]["end_pattern"] = json!("^State end$");
    rule["matchers"] = json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
    rule["extract"] = json!({"operation":null,"choices":[],"native_request_id":null});
    rule["verified_noninteraction"] = json!(true);
    catalog["profiles"][0]["capabilities"] = json!({"detection":null,"annotations":null,"answers":[],"composer":{
        "rule_ids":["synthetic-service-composer"],"fixture_ids":[catalog["fixtures"][0]["id"]],
        "submission":{
            "prompt":{"prefix_hex":"","submit_hex":"0d","fixture_ids":["synthetic-service-submission"]},
            "exec":{"prefix_hex":"2120","submit_hex":"0d","fixture_ids":["synthetic-service-submission"]}
        }
    }});
    fixture.conn.execute("UPDATE member_runs SET native_version='synthetic-0.0.1',profile_version='synthetic-profile-1',profile_match='exact',coverage='unknown' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    let config = fixture._dir.path().join("service-input-native.json");
    private_write(
        &config,
        &serde_json::to_vec(&json!({
            "database_url":format!("sqlite:///{}",fixture.conn.path().unwrap()),
            "run_id":fixture.run.run_id,"expires_at_mono_ns":expiration(until)
        }))
        .unwrap(),
    );
    spec.argv = vec![
        std::env::current_exe().unwrap().to_str().unwrap().into(),
        "--exact".into(),
        CHILD_TEST.into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ];
    spec.environment
        .push((NATIVE_ENV.into(), config.to_str().unwrap().into()));
    private_write(
        &fixture._dir.path().join("observation-services.json"),
        &serde_json::to_vec(&json!({"catalog":catalog})).unwrap(),
    );
}

struct Peer {
    stream: UnixStream,
    decoder: ControlDecoder,
}
impl Peer {
    fn connect(fixture: &ServiceFixture) -> Self {
        let stream = UnixStream::connect(&fixture._artifacts.paths.control_socket).unwrap();
        stream.set_nonblocking(true).unwrap();
        Self {
            stream,
            decoder: ControlDecoder::new(),
        }
    }
    fn send(&mut self, bytes: &[u8], until: Instant) {
        let mut offset = 0;
        while offset < bytes.len() {
            assert!(Instant::now() < until);
            match self.stream.write(&bytes[offset..]) {
                Ok(0) => panic!("owned control writer disconnected"),
                Ok(count) => offset += count,
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    pause(until)
                }
                Err(cause) => panic!("owned control write: {cause}"),
            }
        }
    }
    fn reply(&mut self, until: Instant) -> Value {
        loop {
            assert!(Instant::now() < until, "original reply observation bound");
            let mut bytes = [0; 4096];
            match self.stream.read(&mut bytes) {
                Ok(0) => {
                    self.decoder.finish().unwrap();
                    panic!("owned control disconnected before a complete reply");
                }
                Ok(count) => {
                    let mut frames = self.decoder.feed(&bytes[..count]).unwrap();
                    if !frames.is_empty() {
                        assert_eq!(frames.len(), 1);
                        self.decoder.finish().unwrap();
                        return frames.remove(0);
                    }
                }
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    pause(until)
                }
                Err(cause) => panic!("owned control read: {cause}"),
            }
        }
    }
}
fn request(
    fixture: &ServiceFixture,
    key: &str,
    literal: &str,
    kind: InputKind,
    message: Option<i64>,
    until: Instant,
) -> Value {
    json!({"protocol_version":1,"kind":if kind==InputKind::Exec {"submit_exec"} else {"submit_prompt"},
        "identity":fixture.prepared.as_ref().unwrap(),"expires_at_mono_ns":expiration(until),
        "operation_key":key,"input_kind":kind,"message_id":message,"literal":literal})
}
fn submission(reply: Value) -> SubmissionResult {
    assert!(
        reply.get("error").is_none(),
        "actual service submission: {reply}"
    );
    assert_eq!(reply["result"]["kind"], "submission", "{reply}");
    let result: SubmissionResult =
        serde_json::from_value(reply["result"]["submission"].clone()).unwrap();
    assert_eq!(result.status, SubmissionStatus::Accepted);
    assert_eq!(result.stage, SubmissionStage::Submitted);
    result
}
fn exchange(fixture: &ServiceFixture, value: &Value, until: Instant) -> Value {
    let mut peer = Peer::connect(fixture);
    peer.send(&encode_control_frame(value).unwrap(), until);
    peer.reply(until)
}
fn manual(fixture: &mut ServiceFixture, line: &[u8], until: Instant) {
    let mut offset = 0;
    while offset < line.len() {
        assert!(Instant::now() < until);
        match fixture.master.write(&line[offset..]) {
            Ok(0) => panic!("owned outer PTY closed"),
            Ok(count) => offset += count,
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                pause(until)
            }
            Err(cause) => panic!("owned manual write: {cause}"),
        }
    }
}
pub(crate) fn records(fixture: &ServiceFixture) -> Vec<Value> {
    let bytes = std::fs::read(
        fixture
            .fixture
            ._dir
            .path()
            .join("service-input-lines.jsonl"),
    )
    .unwrap();
    assert!(bytes.len() <= 65536);
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| line.ends_with(b"\n"))
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}
fn failure_diagnostic(fixture: &mut ServiceFixture, stage: &str) -> String {
    let flags = nix::fcntl::OFlag::from_bits_retain(
        nix::fcntl::fcntl(&fixture.master, nix::fcntl::FcntlArg::F_GETFL).unwrap(),
    );
    assert!(flags.contains(nix::fcntl::OFlag::O_NONBLOCK));
    let mut bytes = Vec::new();
    let outer_read = (&mut fixture.master)
        .take(64 * 1024)
        .read_to_end(&mut bytes);
    let prior = String::from_utf8_lossy(&fixture.output[..fixture.output.len().min(64 * 1024)])
        .into_owned();
    let connection = fixture.connection_diagnostic(stage);
    format!(
        "{connection}; outer_read={outer_read:?}; outer_bytes={:?}; previous_outer_bytes={prior:?}",
        String::from_utf8_lossy(&bytes),
    )
}
pub(crate) fn wait_record(fixture: &mut ServiceFixture, line: &str, until: Instant) {
    while !records(fixture).iter().any(|record| record["line"] == line) {
        assert!(
            fixture.child.try_wait().unwrap().is_none(),
            "runner remains live while awaiting its native journal: {}",
            failure_diagnostic(fixture, "native journal wait"),
        );
        pause(until);
    }
}
pub(crate) fn composer(fixture: &mut ServiceFixture, until: Instant) -> HealthSnapshot {
    loop {
        assert!(Instant::now() < until);
        let health = fixture.health();
        if health.observation.as_ref().is_some_and(|observation| {
            observation.eligibility == InputEligibility::ComposerEmpty && !observation.degraded
        }) {
            return health;
        }
        pause(until);
    }
}
pub(crate) fn barrier(fixture: &mut ServiceFixture, until: Instant) {
    let before = fixture.health().output_seq;
    manual(fixture, b"barrier\r", until);
    wait_record(fixture, "barrier", until);
    while composer(fixture, until).output_seq <= before {
        pause(until);
    }
}
pub(crate) fn with_service(body: impl FnOnce(&mut ServiceFixture, Instant)) {
    let until = Instant::now() + Duration::from_secs(10);
    let mut fixture = ServiceFixture::configured_until(false, until, |fixture, spec| {
        configure(fixture, spec, until)
    });
    fixture.cleanup_deadline = Some(until);
    let primary = catch_unwind(AssertUnwindSafe(|| {
        fixture.start_running();
        composer(&mut fixture, until);
        body(&mut fixture, until);
    }));
    let cleanup = catch_unwind(AssertUnwindSafe(|| {
        if fixture.stopped {
            fixture.resume();
        }
        manual(&mut fixture, b"finish\r", until);
        let status = fixture.child.poll_until(until).unwrap();
        if status.is_none() {
            fixture.child.terminate_until(until);
        }
        assert_eq!(
            status
                .expect("owned service reaped under original deadline")
                .code(),
            Some(0),
            "owned service cleanup retains its original exit expectation: {}",
            failure_diagnostic(&mut fixture, "service cleanup"),
        );
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Exited);
        assert_eq!(
            run.cleanup_state,
            CleanupState::Complete,
            "{:?}",
            run.cleanup_diagnostics_json
        );
    }));
    if let Err(primary) = primary {
        if let Err(cleanup) = &cleanup {
            eprintln!("service input cleanup also failed: {cleanup:?}");
        }
        resume_unwind(primary);
    }
    if let Err(cleanup) = cleanup {
        resume_unwind(cleanup);
    }
}

#[test]
fn step5_service_input_socket_submission_persists_typed_outcome_and_matching_key_never_replays() {
    with_service(|fixture, until| {
        let key = "prompt:service:single";
        let literal = "socket prompt λ";
        let value = request(
            fixture,
            key,
            literal,
            InputKind::Prompt,
            None,
            until.min(Instant::now() + Duration::from_secs(3)),
        );
        let result = submission(exchange(fixture, &value, until));
        let id = result.input_id.expect("actual durable service input ID");
        let row = storage::get_input(&fixture.fixture.conn, id).unwrap();
        assert_eq!(row.state, InputState::Accepted);
        assert_eq!(row.operation_key, key);
        wait_record(fixture, literal, until);
        let replay = submission(exchange(fixture, &value, until));
        assert_eq!(replay.input_id, Some(id));
        barrier(fixture, until);
        assert_eq!(
            records(fixture)
                .iter()
                .filter(|record| record["line"] == literal)
                .count(),
            1
        );
        assert_eq!(storage::get_input(&fixture.fixture.conn, id).unwrap(), row);
        let received = records(fixture)
            .into_iter()
            .find(|record| record["line"] == literal)
            .unwrap();
        assert!(
            received["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|input| input["input_id"] == id)
        );
    });
}

#[test]
fn step5_service_input_queued_expiration_keeps_manual_progress_and_uses_fresh_output_for_exec() {
    with_service(|fixture, until| {
        let before = composer(fixture, until).output_seq;
        let expires = Instant::now() + Duration::from_millis(800);
        let value = request(
            fixture,
            "exec:service:expired",
            "expired command",
            InputKind::Exec,
            None,
            expires,
        );
        let frame = encode_control_frame(&value).unwrap();
        let mut queued = Peer::connect(fixture);
        queued.send(&frame[..4], until);
        manual(fixture, b"manual progress\r", until);
        wait_record(fixture, "manual progress", expires.min(until));
        let observed = loop {
            let sample = composer(fixture, expires.min(until));
            if sample.output_seq > before {
                break sample;
            }
            pause(expires.min(until));
        };
        while Instant::now() <= expires {
            pause(until);
        }
        queued.send(&frame[4..], until);
        let rejected = queued.reply(until);
        assert!(
            rejected["error"]
                .as_str()
                .is_some_and(|message| message.contains("deadline")),
            "{rejected}"
        );
        let repeated = exchange(fixture, &value, until);
        assert!(
            repeated["error"]
                .as_str()
                .is_some_and(|message| message.contains("deadline")),
            "{repeated}"
        );
        assert!(
            storage::find_input(
                &fixture.fixture.conn,
                fixture.fixture.run.run_id,
                "exec:service:expired"
            )
            .unwrap()
            .is_none()
        );
        let fresh = request(
            fixture,
            "exec:service:fresh",
            "queued command λ",
            InputKind::Exec,
            None,
            until.min(Instant::now() + Duration::from_secs(3)),
        );
        let reply = submission(exchange(fixture, &fresh, until));
        assert!(reply.output_seq.unwrap() >= observed.output_seq);
        wait_record(fixture, "! queued command λ", until);
        barrier(fixture, until);
        assert_eq!(
            records(fixture)
                .iter()
                .map(|record| record["line"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            ["manual progress", "! queued command λ", "barrier"]
        );
    });
}

#[test]
fn step5_service_input_disconnected_preview_consumer_retains_outcome_without_automatic_retyping() {
    with_service(|fixture, until| {
        let message = crate::broker::send_message(
            &mut fixture.fixture.conn,
            fixture.fixture.run.director_member_id,
            fixture.fixture.run.member_id,
            "disconnected preview λ",
        )
        .unwrap();
        let key = format!(
            "preview:{}:{}",
            fixture.fixture.run.run_id, message.message_id
        );
        let accepted_until = until.min(Instant::now() + Duration::from_secs(3));
        let value = request(
            fixture,
            &key,
            &message.text,
            InputKind::Preview,
            Some(message.message_id),
            accepted_until,
        );
        let mut abandoned = Peer::connect(fixture);
        abandoned.send(&encode_control_frame(&value).unwrap(), until);
        abandoned.stream.shutdown(Shutdown::Read).unwrap();
        let row = loop {
            let current =
                storage::find_input(&fixture.fixture.conn, fixture.fixture.run.run_id, &key)
                    .unwrap();
            if let Some(row) = current.as_ref()
                && row.state == InputState::Accepted
            {
                break row.clone();
            }
            assert!(
                Instant::now() < accepted_until,
                "disconnected consumer's durable accepted outcome is absent: row={current:?}; native_records={:?}",
                records(fixture),
            );
            assert!(fixture.child.try_wait().unwrap().is_none());
            pause(accepted_until);
        };
        drop(abandoned);
        wait_record(fixture, &message.text, until);
        let retained = submission(exchange(fixture, &value, until));
        assert_eq!(retained.input_id, Some(row.input_id));
        barrier(fixture, until);
        assert_eq!(
            records(fixture)
                .iter()
                .filter(|record| record["line"] == message.text)
                .count(),
            1
        );
        assert_eq!(
            storage::get_input(&fixture.fixture.conn, row.input_id).unwrap(),
            row
        );
        assert_eq!(row.message_id, Some(message.message_id));
        let notified: Option<String> = fixture
            .fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(notified.is_none());
    });
}

#[test]
fn step5_service_input_expired_unreserved_attempt_releases_pending_work_for_a_fresh_key() {
    const EXPIRATION_TOLERANCE: Duration = Duration::from_millis(100);

    with_service(|fixture, until| {
        composer(fixture, until);
        let journal: String = fixture
            .fixture
            .conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
        let blocker = crate::db::connect(&format!(
            "sqlite:///{}",
            fixture.fixture.conn.path().unwrap()
        ))
        .unwrap();
        blocker.busy_timeout(Duration::ZERO).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(!blocker.is_autocommit());
        let expired_key = "prompt:service:reservation-expired";
        let expires = Instant::now() + Duration::from_millis(400);
        assert!(expires < until);
        let expired = request(
            fixture,
            expired_key,
            "expired reservation must stay unwritten",
            InputKind::Prompt,
            None,
            expires,
        );
        let started = Instant::now();
        let failure = exchange(fixture, &expired, until);
        let returned = Instant::now();
        let elapsed = returned.duration_since(started);
        assert!(!blocker.is_autocommit());
        assert!(
            returned >= expires - EXPIRATION_TOLERANCE
                && returned <= expires + EXPIRATION_TOLERANCE,
            "original expiration bounds the SQL wait with conservative conversion and scheduling tolerance: elapsed={elapsed:?}; returned={returned:?}; expires={expires:?}"
        );
        assert!(
            failure["error"]
                .as_str()
                .is_some_and(|message| message.contains("deadline")
                    || message.contains("database is locked")),
            "{failure}"
        );
        assert!(
            storage::find_input(
                &fixture.fixture.conn,
                fixture.fixture.run.run_id,
                expired_key
            )
            .unwrap()
            .is_none()
        );
        assert!(records(fixture).is_empty());
        blocker.execute_batch("ROLLBACK").unwrap();
        assert!(blocker.is_autocommit());
        composer(fixture, until);
        let fresh_key = "prompt:service:after-expired-reservation";
        let literal = "fresh after expired reservation";
        let fresh = request(
            fixture,
            fresh_key,
            literal,
            InputKind::Prompt,
            None,
            until.min(Instant::now() + Duration::from_secs(3)),
        );
        let reply = exchange(fixture, &fresh, until);
        eprintln!(
            "reservation expiry under actual {journal} writer lock: elapsed={elapsed:?}; expired_reply={failure}; fresh_reply={reply}; native_records={:?}",
            records(fixture)
        );
        let accepted = submission(reply);
        let row = storage::get_input(&fixture.fixture.conn, accepted.input_id.unwrap()).unwrap();
        assert_eq!(row.operation_key, fresh_key);
        assert_eq!(row.state, InputState::Accepted);
        wait_record(fixture, literal, until);
        barrier(fixture, until);
        assert_eq!(
            records(fixture)
                .iter()
                .map(|record| record["line"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            [literal, "barrier"]
        );
        assert!(
            storage::find_input(
                &fixture.fixture.conn,
                fixture.fixture.run.run_id,
                expired_key
            )
            .unwrap()
            .is_none()
        );
    });
}

#[test]
fn step5_service_input_post_write_failure_preserves_known_identity_and_original_history() {
    with_service(|fixture, until| {
        let key = "prompt:service:known-input-error";
        let literal = "SERVICE_R2_PRIVATE_LITERAL_λ";
        fixture.fixture.conn.execute_batch(&format!(
            "CREATE TRIGGER owned_input_accept_failure BEFORE UPDATE OF state ON member_input_operations
             WHEN OLD.operation_key='{key}' AND OLD.state='in_progress' AND NEW.state='accepted'
             BEGIN SELECT RAISE(FAIL,'owned accepted update failure'); END"
        )).unwrap();
        let value = request(
            fixture,
            key,
            literal,
            InputKind::Prompt,
            None,
            until.min(Instant::now() + Duration::from_secs(3)),
        );
        let attempt = catch_unwind(AssertUnwindSafe(|| exchange(fixture, &value, until)));
        let removed = fixture
            .fixture
            .conn
            .execute_batch("DROP TRIGGER owned_input_accept_failure");
        let reply = match attempt {
            Ok(reply) => {
                removed.unwrap();
                reply
            }
            Err(primary) => {
                if let Err(cleanup) = removed {
                    eprintln!("owned SQL trigger removal also failed: {cleanup}");
                }
                resume_unwind(primary);
            }
        };
        let cause = reply["error"]
            .as_str()
            .expect("actual Accepted-persistence failure envelope");
        assert!(cause.contains("owned accepted update failure"), "{reply}");
        assert!(
            !cause.contains(literal),
            "safe identities replace literal payloads in errors"
        );
        let original = storage::find_input(&fixture.fixture.conn, fixture.fixture.run.run_id, key)
            .unwrap()
            .expect("independent durable reservation after the actual write");
        assert_eq!(original.run_id, fixture.fixture.run.run_id);
        assert_eq!(original.operation_key, key);
        assert_eq!(original.state, InputState::InProgress);
        wait_record(fixture, literal, until);
        let history = exchange(fixture, &value, until);
        assert!(history.get("error").is_none(), "{history}");
        assert_eq!(history["result"]["kind"], "submission");
        let retained: SubmissionResult =
            serde_json::from_value(history["result"]["submission"].clone()).unwrap();
        assert_eq!(retained.input_id, Some(original.input_id));
        assert_eq!(retained.status, SubmissionStatus::Uncertain);
        assert_eq!(retained.stage, SubmissionStage::Submitted);
        barrier(fixture, until);
        let count = records(fixture)
            .iter()
            .filter(|record| record["line"] == literal)
            .count();
        assert_eq!(count, 1);
        assert_eq!(
            storage::get_input(&fixture.fixture.conn, original.input_id).unwrap(),
            original
        );
        eprintln!(
            "actual post-write persistence error: cause={cause:?}; input_id={}; run_id={}; key={:?}; state={:?}; child_line_count={count}",
            original.input_id, original.run_id, original.operation_key, original.state
        );
        assert!(
            cause.contains(&format!("input_id={}", original.input_id))
                && cause.contains(&format!("run_id={}", original.run_id))
                && cause.contains(&format!(
                    "operation_key={}",
                    serde_json::to_string(&original.operation_key).unwrap()
                )),
            "actual bookkeeping failure retains the independently observed safe input identities: {cause}"
        );
    });
}
