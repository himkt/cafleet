mod common;

use std::time::{Duration, Instant};

use common::{Cli, code, stderr, stdout};

const CLAUSES: &str = "[cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

struct Fixture {
    cli: Cli,
    fleet: i64,
    director: i64,
    worker: i64,
    pane: String,
    // Every case races the five-second pane-input budget, so cases run one at a time.
    _test_resource: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new() -> Self {
        let test_resource = common::process_test_resource_guard();
        let cli = Cli::new();
        let (fleet, director) = cli.seeded_fleet();
        let worker = cli.seed_member(fleet, "worker");
        let pane = format!("%{}", worker + 4);
        Self {
            cli,
            fleet,
            director,
            worker,
            pane,
            _test_resource: test_resource,
        }
    }

    fn send_args(&self, body: &str) -> Vec<String> {
        vec![
            "message".to_owned(),
            "send".to_owned(),
            "--from-member-id".to_owned(),
            self.director.to_string(),
            "--to-member-id".to_owned(),
            self.worker.to_string(),
            body.to_owned(),
            "--json".to_owned(),
        ]
    }

    fn send(&self, body: &str) -> std::process::Output {
        let args = self.send_args(body);
        self.cli
            .run(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn spawn_send(&self, body: &str) -> std::process::Child {
        let args = self.send_args(body);
        self.cli
            .spawn(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn stamp(&self, id: i64) -> Option<String> {
        self.cli
            .sqlite()
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn claimed(&self, id: i64) -> bool {
        self.cli
            .sqlite()
            .query_row(
                "SELECT preview_started_at IS NOT NULL FROM messages WHERE message_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn state(&self, id: i64) -> String {
        self.cli
            .sqlite()
            .query_row(
                "SELECT status_state FROM messages WHERE message_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn id_of(&self, body: &str) -> i64 {
        self.cli
            .sqlite()
            .query_row(
                "SELECT message_id FROM messages WHERE text=?1",
                [body],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn wait_for_id_of(&self, body: &str) -> i64 {
        let started = Instant::now();
        loop {
            let found: Option<i64> = rusqlite::OptionalExtension::optional(
                self.cli.sqlite().query_row(
                    "SELECT message_id FROM messages WHERE text=?1",
                    [body],
                    |row| row.get(0),
                ),
            )
            .unwrap();
            if let Some(id) = found {
                return id;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the send never committed {body:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn pending(&self, body: &str) -> i64 {
        self.cli.seed_message(self.director, self.worker, body)
    }

    fn writes_to(&self, pane: &str) -> Vec<String> {
        self.cli
            .shim_calls()
            .into_iter()
            .filter(|line| line.starts_with(&format!("send-keys -t {pane} ")))
            .collect()
    }

    fn escapes_to(&self, pane: &str) -> usize {
        self.writes_to(pane)
            .iter()
            .filter(|line| line.ends_with(" Escape"))
            .count()
    }

    fn lock_busy_stderr(&self, new: i64, preview: i64) -> String {
        format!(
            "Message {new} was persisted. member {}'s pane input remained busy; preview {preview} was not sent.",
            self.worker
        )
    }
}

fn payload(output: &std::process::Output) -> serde_json::Value {
    assert_eq!(code(output), 0, "{}", stderr(output));
    serde_json::from_str(&stdout(output)).unwrap()
}

fn wait_output(child: std::process::Child) -> std::process::Output {
    child.wait_with_output().unwrap()
}

fn release_after(lock: std::fs::File, delay: Duration) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        drop(lock);
    })
}

fn assert_no_capture(cli: &Cli) {
    assert!(
        cli.shim_calls()
            .iter()
            .all(|line| !line.starts_with("capture-pane")),
        "{:?}",
        cli.shim_calls()
    );
}

#[test]
fn a_delivery_is_escape_then_one_notification_line_then_enter_without_capture() {
    let fixture = Fixture::new();
    let output = fixture.send("assignment");
    let result = payload(&output);
    assert_eq!(result["notification_sent"], true);
    let message = &result["message"];
    let id = message["message_id"].as_i64().unwrap();
    assert!(fixture.stamp(id).is_some());
    assert_eq!(fixture.state(id), "input_required");
    let ts = message["created_at"].as_str().unwrap();
    assert_eq!(
        fixture.writes_to(&fixture.pane),
        [
            format!("send-keys -t {} Escape", fixture.pane),
            format!(
                "send-keys -t {} -l -- [cafleet msg {id} from {} {ts}] assignment {CLAUSES}",
                fixture.pane, fixture.director
            ),
            format!("send-keys -t {} Enter", fixture.pane),
        ]
    );
    assert_no_capture(&fixture.cli);
    assert_eq!(stderr(&output), "");
    let polled = payload(
        &fixture
            .cli
            .run(&["message", "poll", &fixture.worker.to_string(), "--json"]),
    );
    assert_eq!(
        polled[0]["message_id"], id,
        "a delivered message stays in the inbox until ACKed"
    );
}

#[test]
fn one_send_drains_pending_messages_oldest_first_with_one_escape() {
    let fixture = Fixture::new();
    let older = fixture.pending("older");
    let result = payload(&fixture.send("newer"));
    let newer = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(
        result["notification_sent"], true,
        "the new message is drained after the older one"
    );
    let literals = fixture.cli.literal_calls();
    assert_eq!(literals.len(), 2, "{literals:?}");
    for (literal, id) in literals.iter().zip([older, newer]) {
        assert!(
            literal.contains(&format!("[cafleet msg {id} from ")),
            "{literals:?}"
        );
        assert!(fixture.stamp(id).is_some());
    }
    assert_eq!(fixture.escapes_to(&fixture.pane), 1);
    let writes = fixture.writes_to(&fixture.pane);
    assert!(writes[0].ends_with(" Escape"), "{writes:?}");
}

#[test]
fn budget_exhaustion_leaves_later_messages_pending_without_a_diagnostic() {
    let fixture = Fixture::new();
    let older: Vec<i64> = (0..4)
        .map(|index| fixture.pending(&format!("older {index}")))
        .collect();
    let output = fixture.send("newest");
    let result = payload(&output);
    let newest = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["notification_sent"], false);
    assert_eq!(stderr(&output), "");
    let mut ids = older.clone();
    ids.push(newest);
    let stamped: Vec<bool> = ids.iter().map(|id| fixture.stamp(*id).is_some()).collect();
    let delivered = stamped.iter().take_while(|stamped| **stamped).count();
    assert!(delivered >= 1, "{stamped:?}");
    assert!(
        stamped[delivered..].iter().all(|stamped| !stamped),
        "delivery is an oldest-first prefix: {stamped:?}"
    );
    assert!(delivered < ids.len(), "{stamped:?}");
    for id in &ids[delivered..] {
        assert!(!fixture.claimed(*id), "no claim is left behind");
    }
    assert_eq!(fixture.cli.literal_calls().len(), delivered);
}

#[test]
fn a_message_acked_between_claim_and_stamp_counts_as_sent_and_the_next_line_has_no_escape() {
    let mut fixture = Fixture::new();
    fixture.cli.block_tmux("literal");
    let child = fixture.spawn_send("acked while typing");
    fixture.cli.wait_for_tmux_block();
    let first = fixture.id_of("acked while typing");
    let mut conn = cafleet::db::connect(&fixture.cli.db_url()).unwrap();
    cafleet::broker::ack_message(&mut conn, first).unwrap();
    let second = fixture.pending("arrived while typing");
    fixture.cli.release_tmux();
    let output = wait_output(child);
    let result = payload(&output);
    assert_eq!(result["notification_sent"], true);
    assert_eq!(fixture.stamp(first), None);
    assert_eq!(fixture.state(first), "completed");
    assert!(fixture.stamp(second).is_some());
    assert_eq!(fixture.escapes_to(&fixture.pane), 1);
    let literals = fixture.cli.literal_calls();
    assert_eq!(literals.len(), 2, "{literals:?}");
    assert!(literals[1].contains(&format!("[cafleet msg {second} from ")));
    let writes = fixture.writes_to(&fixture.pane);
    let second_literal = writes
        .iter()
        .position(|line| line.contains(&format!("[cafleet msg {second} from ")))
        .unwrap();
    assert!(
        !writes[second_literal - 1].ends_with(" Escape"),
        "{writes:?}"
    );
}

#[test]
fn a_member_deregistered_during_the_lock_wait_ends_without_input() {
    let fixture = Fixture::new();
    let lock = fixture.cli.lock_pane(&fixture.pane);
    let child = fixture.spawn_send("target leaves");
    fixture.wait_for_id_of("target leaves");
    std::thread::sleep(Duration::from_millis(200));
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE members SET status='deregistered',deregistered_at='2026-10-24T00:00:00+00:00' WHERE member_id=?1",
            [fixture.worker],
        )
        .unwrap();
    drop(lock);
    let output = wait_output(child);
    let result = payload(&output);
    assert_eq!(result["notification_sent"], false);
    assert_eq!(stderr(&output), "");
    assert!(fixture.writes_to(&fixture.pane).is_empty());
}

#[test]
fn a_lock_released_within_the_wait_still_delivers() {
    for delay in [Duration::from_millis(400), Duration::from_millis(1250)] {
        let fixture = Fixture::new();
        let lock = fixture.cli.lock_pane(&fixture.pane);
        let started = Instant::now();
        let holder = release_after(lock, delay);
        let output = fixture.send("delivered after the lock frees");
        holder.join().unwrap();
        let result = payload(&output);
        assert_eq!(result["notification_sent"], true, "{delay:?}");
        assert_eq!(stderr(&output), "", "{delay:?}");
        assert!(started.elapsed() >= delay);
        assert_eq!(fixture.cli.literal_calls().len(), 1, "{delay:?}");
    }
}

#[test]
fn a_lock_still_busy_at_the_reserved_deadline_reports_the_lock_busy_diagnostic() {
    let fixture = Fixture::new();
    let _lock = fixture.cli.lock_pane(&fixture.pane);
    let started = Instant::now();
    let output = fixture.send("stays pending");
    let waited = started.elapsed();
    let result = payload(&output);
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["notification_sent"], false);
    assert_eq!(stderr(&output).trim_end(), fixture.lock_busy_stderr(id, id));
    assert!(waited >= Duration::from_millis(1400), "{waited:?}");
    assert!(waited < Duration::from_secs(4), "{waited:?}");
    assert_eq!(fixture.stamp(id), None);
    assert!(!fixture.claimed(id));
    assert!(fixture.writes_to(&fixture.pane).is_empty());
}

#[test]
fn the_lock_busy_diagnostic_names_the_oldest_pending_preview() {
    let fixture = Fixture::new();
    let older = fixture.pending("older");
    let _lock = fixture.cli.lock_pane(&fixture.pane);
    let output = fixture.send("newer");
    let result = payload(&output);
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(
        stderr(&output).trim_end(),
        fixture.lock_busy_stderr(id, older)
    );
}

#[test]
fn a_concurrent_drainer_settling_every_message_during_the_lock_wait_reports_nothing() {
    let fixture = Fixture::new();
    let lock = fixture.cli.lock_pane(&fixture.pane);
    let child = fixture.spawn_send("delivered by another sender");
    let id = fixture.wait_for_id_of("delivered by another sender");
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE messages SET notified_at=created_at WHERE message_id=?1",
            [id],
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    drop(lock);
    let output = wait_output(child);
    let result = payload(&output);
    assert_eq!(result["notification_sent"], false);
    assert_eq!(stderr(&output), "");
    assert!(fixture.writes_to(&fixture.pane).is_empty());
}

#[test]
fn acked_and_already_notified_rows_are_excluded_from_preview_selection() {
    let fixture = Fixture::new();
    let acked = fixture.pending("acked");
    let notified = fixture.pending("already notified");
    let ack = fixture.cli.run(&["message", "ack", &acked.to_string()]);
    assert_eq!(code(&ack), 0, "{}", stderr(&ack));
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE messages SET notified_at=created_at WHERE message_id=?1",
            [notified],
        )
        .unwrap();
    let result = payload(&fixture.send("deliver this"));
    assert_eq!(result["notification_sent"], true);
    let literals = fixture.cli.literal_calls();
    assert_eq!(literals.len(), 1);
    assert!(
        literals[0].ends_with(&format!("] deliver this {CLAUSES}")),
        "{literals:?}"
    );
}

#[test]
fn a_failed_escape_preserves_the_committed_message_and_releases_its_claim() {
    let mut fixture = Fixture::new();
    fixture.cli.fail_subcommand = Some("send-keys".into());
    let output = fixture.send("persist exactly once");
    let result = payload(&output);
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["notification_sent"], false);
    assert_eq!(fixture.stamp(id), None);
    assert!(!fixture.claimed(id));
    assert_eq!(
        fixture
            .cli
            .sqlite()
            .query_row("SELECT count(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let error = stderr(&output);
    assert!(
        error.starts_with(&format!("Message {id} was persisted. ")),
        "{error}"
    );
    assert!(error.contains("forced failure"), "{error}");
    assert_eq!(fixture.cli.literal_calls().len(), 0);
}

#[test]
fn a_notification_stamp_failure_survives_a_new_wait_without_resubmission() {
    let fixture = Fixture::new();
    fixture.cli.sqlite().execute_batch("CREATE TRIGGER reject_notification_stamp BEFORE UPDATE OF notified_at ON messages
        WHEN NEW.notified_at IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected notification stamp failure'); END;").unwrap();
    let output = fixture.send("submitted before stamp failure");
    let result = payload(&output);
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["notification_sent"], false);
    assert_eq!(fixture.stamp(id), None);
    let error = stderr(&output);
    assert!(error.contains(&id.to_string()), "{error}");
    assert!(error.contains("uncertain"), "{error}");
    assert!(
        error.contains("injected notification stamp failure"),
        "{error}"
    );
    assert!(fixture.claimed(id));
    for _ in 0..2 {
        let waiting = fixture.cli.run(&[
            "message",
            "wait",
            &fixture.director.to_string(),
            "--timeout",
            "1",
            "--json",
        ]);
        let result = payload(&waiting);
        let errors = result["notification_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["message_id"], id);
        assert_eq!(errors[0]["delivery"], "uncertain");
    }
    let enters = || {
        fixture
            .cli
            .shim_calls()
            .iter()
            .filter(|line| line.ends_with(" Enter"))
            .count()
    };
    assert_eq!(
        enters(),
        1,
        "fresh waits preserve the durable claim and never inject another preview"
    );
    fixture
        .cli
        .sqlite()
        .execute_batch("DROP TRIGGER reject_notification_stamp")
        .unwrap();
    let later = payload(&fixture.send("later message still gets its own notification"));
    let later_id = later["message"]["message_id"].as_i64().unwrap();
    assert_eq!(later["notification_sent"], true);
    assert!(fixture.stamp(later_id).is_some());
    assert_eq!(fixture.stamp(id), None);
    assert_eq!(enters(), 2);
}

#[test]
fn a_member_to_director_send_is_delivered_into_the_directors_pane() {
    let fixture = Fixture::new();
    let output = fixture.cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &fixture.worker.to_string(),
        "--to-member-id",
        &fixture.director.to_string(),
        "reply for the Director",
        "--json",
    ]);
    let result = payload(&output);
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["notification_sent"], true);
    assert_eq!(stderr(&output), "");
    assert!(fixture.stamp(id).is_some());
    assert_eq!(fixture.state(id), "input_required");
    let ts = result["message"]["created_at"].as_str().unwrap();
    assert_eq!(
        fixture.writes_to("%0"),
        [
            "send-keys -t %0 Escape".to_owned(),
            format!(
                "send-keys -t %0 -l -- [cafleet msg {id} from {} {ts}] reply for the Director {CLAUSES}",
                fixture.worker
            ),
            "send-keys -t %0 Enter".to_owned(),
        ]
    );
    assert!(fixture.writes_to(&fixture.pane).is_empty());
}

#[test]
fn self_sends_produce_no_pane_input() {
    let fixture = Fixture::new();
    for member in [fixture.director, fixture.worker] {
        let output = fixture.cli.run(&[
            "message",
            "send",
            "--from-member-id",
            &member.to_string(),
            "--to-member-id",
            &member.to_string(),
            "note to self",
            "--json",
        ]);
        let result = payload(&output);
        assert_eq!(result["notification_sent"], false);
        assert_eq!(stderr(&output), "");
    }
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn pending_placement_persists_without_capture_or_notification_stamp() {
    let fixture = Fixture::new();
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [fixture.worker],
        )
        .unwrap();
    let result = payload(&fixture.send("wait for placement"));
    assert_eq!(result["notification_sent"], false);
    assert_eq!(
        fixture.stamp(result["message"]["message_id"].as_i64().unwrap()),
        None
    );
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn a_long_body_is_replaced_by_the_show_pointer() {
    let fixture = Fixture::new();
    let body = "x".repeat(4001);
    let result = payload(&fixture.send(&body));
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["message"]["text"], body);
    let literals = fixture.cli.literal_calls();
    assert_eq!(literals.len(), 1);
    assert!(
        literals[0].ends_with(&format!(
            "] Body is 4001 characters; run cafleet message show {id} --json to read it. {CLAUSES}"
        )),
        "{literals:?}"
    );
}

#[test]
fn the_cli_text_limit_does_not_truncate_the_delivered_line() {
    let mut fixture = Fixture::new();
    fixture.cli.set_env("CAFLEET_MAX_TEXT_LEN", "5");
    let body = "a body longer than five characters";
    payload(&fixture.send(body));
    let literals = fixture.cli.literal_calls();
    assert!(
        literals[0].ends_with(&format!("] {body} {CLAUSES}")),
        "{literals:?}"
    );
}

fn terminate(child: &std::process::Child) {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
}

fn wait_within(child: &mut std::process::Child, limit: Duration) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(started.elapsed() < limit, "the process outlived {limit:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn terminated_by_sigterm(status: std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.signal() == Some(nix::sys::signal::Signal::SIGTERM as i32)
}

#[test]
fn a_sigterm_during_the_literal_write_takes_effect_after_the_notification_stamp() {
    let mut fixture = Fixture::new();
    fixture.cli.block_tmux("literal");
    let mut child = fixture.spawn_send("deliver before terminating");
    fixture.cli.wait_for_tmux_block();
    terminate(&child);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the SIGTERM is deferred while pane input is in flight"
    );
    fixture.cli.release_tmux();
    let status = wait_within(&mut child, Duration::from_secs(10));
    assert!(terminated_by_sigterm(status), "{status:?}");
    let id = fixture.id_of("deliver before terminating");
    assert!(
        fixture.stamp(id).is_some(),
        "message {id} is stamped before the exit"
    );
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .any(|line| line == &format!("send-keys -t {} Enter", fixture.pane)),
        "{:?}",
        fixture.cli.shim_calls()
    );
}

#[test]
fn a_sigterm_outside_any_span_terminates_at_once() {
    let fixture = Fixture::new();
    let mut child = fixture.cli.spawn(&[
        "message",
        "wait",
        &fixture.director.to_string(),
        "--timeout",
        "30",
        "--json",
    ]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the wait is still sleeping"
    );
    let signalled = Instant::now();
    terminate(&child);
    let status = wait_within(&mut child, Duration::from_secs(5));
    assert!(terminated_by_sigterm(status), "{status:?}");
    assert!(
        signalled.elapsed() < Duration::from_secs(1),
        "{:?}",
        signalled.elapsed()
    );
}

#[test]
fn a_sigterm_during_member_create_takes_effect_after_the_registry_commit() {
    let mut fixture = Fixture::new();
    fixture.cli.block_tmux("split-window");
    let mut child = fixture.cli.spawn(&[
        "member",
        "create",
        "--fleet-id",
        &fixture.fleet.to_string(),
        "--name",
        "spawned",
        "--description",
        "test member",
        "wait for the Director",
    ]);
    fixture.cli.wait_for_tmux_block();
    terminate(&child);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the SIGTERM is deferred between the pane spawn and the registry commit"
    );
    fixture.cli.release_tmux();
    let status = wait_within(&mut child, Duration::from_secs(10));
    assert!(terminated_by_sigterm(status), "{status:?}");
    let pane: Option<String> = fixture
        .cli
        .sqlite()
        .query_row(
            "SELECT p.mux_pane_id FROM members m JOIN member_placements p ON p.member_id=m.member_id
             WHERE m.fleet_id=?1 AND m.name='spawned' AND m.status='active'",
            [fixture.fleet],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pane.as_deref(), Some("%7"), "the spawned pane is registered");
}

#[test]
fn preview_sanitization_keeps_stored_text_exact_and_native_commands_separate() {
    let fixture = Fixture::new();
    let body = "! printf danger\r\nnext\nline\rlast\tcolumn\u{1b}\u{7f}";
    let result = payload(&fixture.send(body));
    assert_eq!(result["message"]["text"], body);
    let id = result["message"]["message_id"].as_i64().unwrap();
    let stored: String = fixture
        .cli
        .sqlite()
        .query_row(
            "SELECT text FROM messages WHERE message_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, body);
    let literals = fixture.cli.literal_calls();
    let literal = &literals[0];
    assert!(literal.contains(" -l -- [cafleet msg "), "{literal}");
    assert!(
        literal.ends_with(&format!(
            "] ! printf danger⏎next⏎line⏎last␉column\\x1b\\x7f {CLAUSES}"
        )),
        "{literal}"
    );
}
