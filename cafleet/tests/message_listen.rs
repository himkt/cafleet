mod common;

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ExitStatus};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::{JoinHandle, sleep};
use std::time::{Duration, Instant};

use common::{Cli, code, stderr, stdout};
use serde_json::{Value, json};

struct ChildGuard(Option<Child>);

impl std::ops::Deref for ChildGuard {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.0.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }
}

impl ChildGuard {
    fn output(mut self) -> std::process::Output {
        let deadline = Instant::now() + Duration::from_secs(7);
        while self.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "listener failed to exit");
            sleep(Duration::from_millis(10));
        }
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                child.kill().unwrap();
            }
            child.wait().unwrap();
        }
    }
}

struct Listener {
    child: Option<Child>,
    lines: Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
    records: Vec<Value>,
}

impl Listener {
    fn start(cli: &Cli, member: i64, timeout: Option<u64>) -> Self {
        let mut args = vec!["message".into(), "listen".into(), member.to_string()];
        if let Some(timeout) = timeout {
            args.extend(["--timeout".into(), timeout.to_string()]);
        }
        let mut child = cli.spawn(&args.iter().map(String::as_str).collect::<Vec<_>>());
        let pipe = child.stdout.take().unwrap();
        let (sender, lines) = channel();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(pipe);
            loop {
                let mut line = Vec::new();
                if reader.read_until(b'\n', &mut line).unwrap() == 0 {
                    break;
                }
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child: Some(child),
            lines,
            reader: Some(reader),
            records: Vec::new(),
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    fn decode(&mut self, line: Vec<u8>) -> Value {
        assert_eq!(line.last(), Some(&b'\n'), "incomplete JSONL: {line:?}");
        let value: Value = serde_json::from_slice(&line).unwrap();
        let mut compact = serde_json::to_vec(&value).unwrap();
        compact.push(b'\n');
        assert_eq!(line, compact, "protocol records use compact UTF-8 JSONL");
        self.records.push(value.clone());
        value
    }

    fn next(&mut self) -> Value {
        match self.lines.recv_timeout(Duration::from_secs(4)) {
            Ok(line) => self.decode(line),
            Err(error) => {
                let status = self.child.as_mut().unwrap().try_wait().unwrap();
                let mut diagnostic = String::new();
                if status.is_some() {
                    self.child
                        .as_mut()
                        .unwrap()
                        .stderr
                        .as_mut()
                        .unwrap()
                        .read_to_string(&mut diagnostic)
                        .unwrap();
                }
                panic!("listener record unavailable: {error}; status={status:?}; {diagnostic}");
            }
        }
    }

    fn ready(&mut self, member: i64, fleet: i64) {
        assert_eq!(
            self.next(),
            json!({"event":"ready", "member_id":member, "fleet_id":fleet})
        );
    }

    fn message(&mut self, id: i64) -> Value {
        let envelope = self.next();
        assert_eq!(envelope["event"], "message");
        assert_eq!(envelope.as_object().unwrap().len(), 2);
        assert_eq!(envelope["message"]["message_id"], id);
        assert_eq!(envelope["message"].as_object().unwrap().len(), 7);
        envelope["message"].clone()
    }

    fn quiet(&mut self, duration: Duration) {
        assert_eq!(
            self.lines.recv_timeout(duration),
            Err(RecvTimeoutError::Timeout),
            "an idle live listener emits no protocol records"
        );
        assert!(self.child.as_mut().unwrap().try_wait().unwrap().is_none());
    }

    fn finish(mut self) -> (ExitStatus, String, Vec<Value>) {
        let deadline = Instant::now() + Duration::from_secs(7);
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "listener failed to exit");
            sleep(Duration::from_millis(10));
        }
        let output = self.child.take().unwrap().wait_with_output().unwrap();
        self.reader.take().unwrap().join().unwrap();
        for line in self.lines.try_iter().collect::<Vec<_>>() {
            self.decode(line);
        }
        (output.status, stderr(&output), self.records.clone())
    }

    fn closed(self, member: i64, reason: &str) -> Vec<Value> {
        let (status, diagnostic, records) = self.finish();
        assert!(status.success(), "{status:?}: {diagnostic}");
        assert_eq!(diagnostic, "");
        assert_eq!(
            records.last().unwrap(),
            &json!({"event":"closed", "member_id":member, "reason":reason})
        );
        records
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().unwrap().is_none() {
                child.kill().unwrap();
            }
            child.wait().unwrap();
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

fn fixture() -> (Cli, i64, i64, i64) {
    let cli = Cli::new();
    let (fleet, director) = cli.seeded_fleet();
    let worker = cli.seed_member(fleet, "worker");
    (cli, fleet, director, worker)
}

fn ack(cli: &Cli, message: i64) {
    let output = cli.run(&["message", "ack", &message.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
}

fn stored_messages(cli: &Cli) -> Vec<Vec<rusqlite::types::Value>> {
    let conn = cli.sqlite();
    let mut statement = conn
        .prepare("SELECT * FROM messages ORDER BY message_id")
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn listen_usage_has_a_positional_subject_an_optional_lease_and_jsonl_output() {
    let _resource = common::process_test_resource_guard();
    let cli = Cli::new();
    let output = cli.run(&["message", "listen", "--help"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let help = stdout(&output);
    assert!(help.contains("listen [OPTIONS] <MEMBER_ID>"), "{help}");
    assert!(help.contains("--timeout <SECONDS>"), "{help}");
    assert!(!help.contains("--json"), "{help}");
    for args in [
        vec!["message", "listen"],
        vec!["message", "listen", "not-an-id"],
        vec!["message", "listen", "1", "--timeout", "0"],
        vec!["message", "listen", "1", "--timeout", "1801"],
        vec!["message", "listen", "1", "--timeout", "-1"],
        vec!["message", "listen", "1", "--timeout", "1.5"],
        vec!["message", "listen", "1", "--json"],
    ] {
        let rejected = cli.run(&args);
        assert_eq!(code(&rejected), 2, "{args:?}: {}", stderr(&rejected));
    }
}

#[test]
fn ready_precedes_oldest_first_replay_with_timestamp_ties_and_multiple_senders() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    let other = cli.seed_member(fleet, "other");
    let later = cli.seed_message(worker, director, "later timestamp");
    let first = cli.seed_message(other, director, "earlier timestamp");
    let tied = cli.seed_message(worker, director, "same earlier timestamp");
    cli.sqlite().execute(
        "UPDATE messages SET created_at=CASE message_id WHEN ?1 THEN '2026-10-25T02:00:00+00:00' ELSE '2026-10-25T01:00:00+00:00' END",
        [later],
    ).unwrap();
    let mut listener = Listener::start(&cli, director, Some(1));
    listener.ready(director, fleet);
    for id in [first, tied, later] {
        let record = listener.message(id);
        let shown = cli.run(&["message", "show", &id.to_string(), "--json"]);
        assert_eq!(code(&shown), 0, "{}", stderr(&shown));
        let shown: Value = serde_json::from_str(&stdout(&shown)).unwrap();
        assert_eq!(record, shown["message"]);
    }
    assert_eq!(listener.closed(director, "timeout").len(), 5);
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn unicode_multiline_and_control_bytes_exceeding_the_pane_limit_remain_exact() {
    let _resource = common::process_test_resource_guard();
    let (mut cli, fleet, director, worker) = fixture();
    cli.set_env("CAFLEET_MAX_TEXT_LEN", "1");
    let body = format!(
        "{}\nnext\r\nlast\t\u{0000}\u{001b}\u{007f}🙂",
        "界".repeat(5000)
    );
    let id = cli.seed_message(worker, director, &body);
    let mut listener = Listener::start(&cli, director, Some(1));
    listener.ready(director, fleet);
    assert_eq!(listener.message(id)["text"], body);
    assert_eq!(listener.closed(director, "timeout").len(), 3);
}

#[test]
fn one_live_process_suppresses_unacked_ids_and_receives_equal_and_backdated_commits() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    let first = cli.seed_message(worker, director, "A remains unACKed");
    cli.sqlite()
        .execute(
            "UPDATE messages SET created_at='2026-10-25T01:00:00+00:00' WHERE message_id=?1",
            [first],
        )
        .unwrap();
    let started = Instant::now();
    let mut listener = Listener::start(&cli, director, Some(4));
    let pid = listener.pid();
    listener.ready(director, fleet);
    listener.message(first);
    listener.quiet(Duration::from_millis(1200));
    let mut conn = cli.sqlite();
    let tx = conn.transaction().unwrap();
    tx.execute("INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text) VALUES(?1, ?2, '2026-10-25T01:00:00+00:00', 'input_required', '2026-10-25T01:00:00+00:00', 'B')", [worker, director]).unwrap();
    let tied = tx.last_insert_rowid();
    tx.execute("INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text) VALUES(?1, ?2, '1900-01-01T00:00:00+00:00', 'input_required', '2026-10-25T01:00:00+00:00', 'C')", [worker, director]).unwrap();
    let backdated = tx.last_insert_rowid();
    tx.commit().unwrap();
    listener.message(backdated);
    listener.message(tied);
    assert_eq!(listener.pid(), pid);
    let records = listener.closed(director, "timeout");
    assert!(
        started.elapsed() < Duration::from_millis(4800),
        "activity leaves the original four-second lease unchanged"
    );
    assert_eq!(records.len(), 5);
    assert_eq!(stored_messages(&cli).len(), 3);
}

#[test]
fn restart_replays_only_still_pending_messages_and_keeps_strict_ack_semantics() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    let pending = cli.seed_message(worker, director, "replay me");
    let consumed = cli.seed_message(worker, director, "consume before the first read");
    ack(&cli, consumed);
    for _ in 0..2 {
        let mut listener = Listener::start(&cli, director, Some(1));
        listener.ready(director, fleet);
        listener.message(pending);
        assert_eq!(listener.closed(director, "timeout").len(), 3);
    }
    ack(&cli, pending);
    let again = cli.run(&["message", "ack", &pending.to_string(), "--json"]);
    assert_eq!(code(&again), 1);
    assert!(stderr(&again).contains("Cannot ACK message in state completed"));
    let mut listener = Listener::start(&cli, director, Some(1));
    listener.ready(director, fleet);
    assert_eq!(listener.closed(director, "timeout").len(), 2);
}

#[test]
fn listening_preserves_preview_and_consumption_state_including_a_stamped_self_send() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    let self_send = cli.seed_message(director, director, "self note");
    let stamped = cli.seed_message(worker, director, "already notified");
    let claimed = cli.seed_message(worker, director, "uncertain old preview");
    cli.sqlite()
        .execute(
            "UPDATE messages SET notified_at=created_at WHERE message_id=?1",
            [stamped],
        )
        .unwrap();
    cli.sqlite()
        .execute(
            "UPDATE messages SET preview_started_at=created_at WHERE message_id=?1",
            [claimed],
        )
        .unwrap();
    let before = stored_messages(&cli);
    let mut listener = Listener::start(&cli, director, Some(1));
    listener.ready(director, fleet);
    for id in [self_send, stamped, claimed] {
        listener.message(id);
    }
    listener.closed(director, "timeout");
    assert_eq!(stored_messages(&cli), before);
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn an_ordinary_recipient_needs_no_placement_and_receives_only_its_own_fleet_inbox() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    let assignment = cli.seed_message(director, worker, "local assignment");
    cli.seed_message(worker, director, "different local inbox");
    let (foreign, foreign_director) = cli.seed_fleet("foreign");
    let foreign_worker = cli.seed_member(foreign, "foreign-worker");
    cli.seed_message(foreign_worker, foreign_director, "foreign inbox");
    cli.sqlite()
        .execute("DELETE FROM member_placements WHERE member_id=?1", [worker])
        .unwrap();
    std::fs::remove_file(cli.shim_dir.join("tmux")).unwrap();
    let mut listener = Listener::start(&cli, worker, Some(1));
    listener.ready(worker, fleet);
    listener.message(assignment);
    assert_eq!(listener.closed(worker, "timeout").len(), 3);
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn an_idle_listener_emits_only_ready_and_closed_and_never_retries_previews() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    cli.seed_message(director, worker, "ordinary preview stays pending");
    let before = stored_messages(&cli);
    let mut listener = Listener::start(&cli, director, Some(2));
    listener.ready(director, fleet);
    listener.quiet(Duration::from_millis(1100));
    assert_eq!(listener.closed(director, "timeout").len(), 2);
    assert_eq!(stored_messages(&cli), before);
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn separate_read_only_observers_can_receive_the_same_pending_record() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, worker) = fixture();
    let pending = cli.seed_message(worker, director, "observed twice");
    let mut first = Listener::start(&cli, director, Some(2));
    let mut second = Listener::start(&cli, director, Some(2));
    first.ready(director, fleet);
    second.ready(director, fleet);
    assert_eq!(first.message(pending), second.message(pending));
    first.closed(director, "timeout");
    second.closed(director, "timeout");
    let inbox = cli.run(&["message", "poll", &director.to_string(), "--json"]);
    let inbox: Value = serde_json::from_str(&stdout(&inbox)).unwrap();
    assert_eq!(inbox[0]["message_id"], pending);
}

#[test]
fn unknown_or_inactive_startup_subjects_fail_without_a_ready_record() {
    let _resource = common::process_test_resource_guard();
    let (cli, _, _, worker) = fixture();
    cli.sqlite()
        .execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [worker],
        )
        .unwrap();
    for (member, lease) in [(worker, "1"), (99999, "1"), (99999, "1800")] {
        let output = cli.run(&["message", "listen", &member.to_string(), "--timeout", lease]);
        assert_eq!(code(&output), 1, "{}", stderr(&output));
        assert_eq!(stdout(&output), "");
        assert!(stderr(&output).contains(&format!("Member {member} not found")));
    }
}

#[test]
fn a_missing_root_placement_fails_at_startup_and_after_ready() {
    let _resource = common::process_test_resource_guard();
    for after_ready in [false, true] {
        let (cli, fleet, director, worker) = fixture();
        let mut listener = after_ready.then(|| Listener::start(&cli, worker, Some(4)));
        if let Some(listener) = &mut listener {
            listener.ready(worker, fleet);
        }
        cli.sqlite()
            .execute(
                "DELETE FROM member_placements WHERE member_id=?1",
                [director],
            )
            .unwrap();
        if let Some(listener) = listener {
            let (status, diagnostic, records) = listener.finish();
            assert_eq!(status.code(), Some(1));
            assert!(!diagnostic.is_empty());
            assert_eq!(records.len(), 1);
        } else {
            let output = cli.run(&["message", "listen", &worker.to_string(), "--timeout", "1"]);
            assert_eq!(code(&output), 1, "{}", stderr(&output));
            assert_eq!(stdout(&output), "");
            assert!(!stderr(&output).is_empty());
        }
        assert!(cli.shim_calls().is_empty());
    }
}

#[test]
fn normal_member_deregistration_closes_a_live_stream() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, _, worker) = fixture();
    let mut listener = Listener::start(&cli, worker, None);
    listener.ready(worker, fleet);
    cli.sqlite()
        .execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [worker],
        )
        .unwrap();
    assert_eq!(listener.closed(worker, "member_inactive").len(), 2);
}

#[test]
fn fleet_deletion_takes_precedence_over_member_deregistration_in_one_transaction() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, _) = fixture();
    let mut listener = Listener::start(&cli, director, None);
    listener.ready(director, fleet);
    let mut conn = cli.sqlite();
    let tx = conn.transaction().unwrap();
    tx.execute(
        "UPDATE fleets SET deleted_at=created_at WHERE fleet_id=?1",
        [fleet],
    )
    .unwrap();
    tx.execute(
        "UPDATE members SET status='deregistered' WHERE fleet_id=?1",
        [fleet],
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(listener.closed(director, "fleet_deleted").len(), 2);
}

#[test]
fn corruption_or_changed_identity_after_ready_ends_with_a_visible_error() {
    let _resource = common::process_test_resource_guard();
    for mutation in [
        "member-missing",
        "fleet-missing",
        "changed-fleet",
        "bad-status",
        "bad-root",
    ] {
        let (cli, fleet, director, worker) = fixture();
        let (foreign, _) = cli.seed_fleet("other");
        let mut listener = Listener::start(&cli, worker, Some(4));
        listener.ready(worker, fleet);
        let conn = cli.sqlite();
        conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON")
            .unwrap();
        match mutation {
            "member-missing" => {
                conn.execute("DELETE FROM members WHERE member_id=?1", [worker])
                    .unwrap();
            }
            "fleet-missing" => {
                conn.execute("DELETE FROM fleets WHERE fleet_id=?1", [fleet])
                    .unwrap();
            }
            "changed-fleet" => {
                conn.execute(
                    "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
                    [foreign, worker],
                )
                .unwrap();
            }
            "bad-status" => {
                conn.execute(
                    "UPDATE members SET status='corrupt' WHERE member_id=?1",
                    [worker],
                )
                .unwrap();
            }
            "bad-root" => {
                conn.execute(
                    "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
                    [99999, fleet],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let (status, diagnostic, records) = listener.finish();
        assert_eq!(status.code(), Some(1), "{mutation}: {diagnostic}");
        assert!(!diagnostic.is_empty(), "{mutation}");
        assert_eq!(records.len(), 1, "{mutation}: {records:?}");
        assert!(cli.shim_calls().is_empty(), "{mutation}: {director}");
    }
}

#[test]
fn startup_respects_the_existing_schema_and_asset_guards() {
    let _resource = common::process_test_resource_guard();
    let (cli, _, director, _) = fixture();
    cli.seed_asset_row("claude", "0.0.0");
    let stale = cli.run(&["message", "listen", &director.to_string(), "--timeout", "1"]);
    assert_eq!(code(&stale), 1);
    assert_eq!(stdout(&stale), "");
    assert!(!stderr(&stale).is_empty());
    let fresh = Cli::new();
    fresh.seed_incompatible_database();
    let incompatible = fresh.run(&["message", "listen", "1", "--timeout", "1"]);
    assert_eq!(code(&incompatible), 1);
    assert_eq!(stdout(&incompatible), "");
    assert!(!stderr(&incompatible).is_empty());
}

#[test]
fn a_broken_stdout_pipe_is_a_failure_and_keeps_messages_pending() {
    let _resource = common::process_test_resource_guard();
    let (cli, _, director, worker) = fixture();
    cli.seed_message(worker, director, "still pending after broken output");
    let before = stored_messages(&cli);
    let mut child = ChildGuard(Some(cli.spawn(&[
        "message",
        "listen",
        &director.to_string(),
        "--timeout",
        "1",
    ])));
    drop(child.stdout.take());
    let output = child.output();
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(!stderr(&output).is_empty());
    assert_eq!(stored_messages(&cli), before);
}

#[test]
fn output_backpressure_releases_the_snapshot_and_ends_as_failure_within_the_lease() {
    let _resource = common::process_test_resource_guard();
    let (cli, _, director, worker) = fixture();
    cli.seed_message(worker, director, &"x".repeat(2 * 1024 * 1024));
    let before = stored_messages(&cli);
    let started = Instant::now();
    let mut child = ChildGuard(Some(cli.spawn(&[
        "message",
        "listen",
        &director.to_string(),
        "--timeout",
        "1",
    ])));
    let mut pipe = child.stdout.take().unwrap();
    let mut ready = Vec::new();
    loop {
        let mut byte = [0];
        pipe.read_exact(&mut byte).unwrap();
        ready.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&ready).unwrap()["event"],
        "ready"
    );
    sleep(Duration::from_millis(150));
    let conn = cli.sqlite();
    conn.busy_timeout(Duration::ZERO).unwrap();
    conn.execute_batch("BEGIN EXCLUSIVE; ROLLBACK")
        .expect("stdout backpressure owns no read transaction");
    let deadline = Instant::now() + Duration::from_secs(4);
    while child.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "stalled stdout exceeded its deadline"
        );
        sleep(Duration::from_millis(10));
    }
    let output = child.output();
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(!stderr(&output).is_empty());
    assert!(started.elapsed() < Duration::from_secs(4));
    let mut partial = Vec::new();
    pipe.read_to_end(&mut partial).unwrap();
    assert!(
        !partial.ends_with(b"\n"),
        "an incomplete record stays unconsumed"
    );
    assert_eq!(stored_messages(&cli), before);
}

#[test]
fn normal_stdout_completion_restores_the_shared_descriptor_flags() {
    use nix::fcntl::{FcntlArg, fcntl};
    use std::io::{Seek, SeekFrom};
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, _) = fixture();
    let mut file = tempfile::tempfile().unwrap();
    let before = fcntl(&file, FcntlArg::F_GETFL).unwrap();
    let child = ChildGuard(Some(cli.spawn_with_stdout(
        &["message", "listen", &director.to_string(), "--timeout", "1"],
        std::process::Stdio::from(file.try_clone().unwrap()),
    )));
    let output = child.output();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    assert_eq!(fcntl(&file, FcntlArg::F_GETFL).unwrap(), before);
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut records = String::new();
    file.read_to_string(&mut records).unwrap();
    let records: Vec<Value> = records
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records,
        [
            json!({"event":"ready", "member_id":director, "fleet_id":fleet}),
            json!({"event":"closed", "member_id":director, "reason":"timeout"})
        ]
    );
}

#[test]
fn snapshot_lock_budget_exhaustion_is_a_visible_failure_after_ready() {
    let _resource = common::process_test_resource_guard();
    let (cli, fleet, director, _) = fixture();
    let mut listener = Listener::start(&cli, director, Some(3));
    listener.ready(director, fleet);
    let conn = cli.sqlite();
    conn.busy_timeout(Duration::from_millis(500)).unwrap();
    conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (status, diagnostic, records) = listener.finish();
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(status.code(), Some(1), "{diagnostic}");
    assert!(!diagnostic.is_empty());
    assert_eq!(
        records,
        [json!({"event":"ready", "member_id":director, "fleet_id":fleet})]
    );
}

#[test]
fn an_unbounded_listener_is_cancelled_promptly_and_its_child_is_collected() {
    let _resource = common::process_test_resource_guard();
    use std::os::unix::process::ExitStatusExt;
    let (cli, fleet, director, _) = fixture();
    let mut listener = Listener::start(&cli, director, None);
    listener.ready(director, fleet);
    listener.quiet(Duration::from_millis(100));
    let started = Instant::now();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(listener.pid()).unwrap()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let (status, diagnostic, records) = listener.finish();
    assert_eq!(
        status.signal(),
        Some(nix::sys::signal::Signal::SIGTERM as i32)
    );
    assert_eq!(diagnostic, "");
    assert_eq!(records.len(), 1);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(cli.shim_calls().is_empty());
}
