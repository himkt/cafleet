mod common;

use common::{Cli, code, stderr, stdout};

const IDLE: &str = "Ready.\n────────────────────────────\n❯ \x1b[0m\n────────────────────────────\n? for shortcuts";

struct Fixture {
    cli: Cli,
    fleet: i64,
    director: i64,
    worker: i64,
    pane: String,
}

impl Fixture {
    fn new() -> Self {
        let cli = Cli::new();
        let (fleet, director) = cli.seeded_fleet();
        let worker = cli.seed_member(fleet, "worker");
        let pane = format!("%{}", worker + 4);
        cli.set_pane_capture(&pane, IDLE);
        Self {
            cli,
            fleet,
            director,
            worker,
            pane,
        }
    }

    fn send(&self, body: &str) -> std::process::Output {
        self.cli.run(&[
            "message",
            "send",
            "--from-member-id",
            &self.director.to_string(),
            "--to-member-id",
            &self.worker.to_string(),
            body,
            "--json",
        ])
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

    fn pending(&self, body: &str) -> i64 {
        self.cli.seed_message(self.director, self.worker, body)
    }
}

fn payload(output: &std::process::Output) -> serde_json::Value {
    assert_eq!(code(output), 0, "{}", stderr(output));
    serde_json::from_str(&stdout(output)).unwrap()
}

#[test]
fn idle_preview_is_one_literal_line_followed_by_enter() {
    let fixture = Fixture::new();
    let output = fixture.send("assignment");
    let result = payload(&output);
    assert_eq!(result["notification_sent"], true);
    let message = &result["message"];
    let id = message["message_id"].as_i64().unwrap();
    assert!(fixture.stamp(id).is_some());
    let ts = message["created_at"].as_str().unwrap();
    let writes: Vec<_> = fixture
        .cli
        .shim_calls()
        .into_iter()
        .filter(|line| line.starts_with("send-keys"))
        .collect();
    assert_eq!(
        writes,
        [
            format!(
                "send-keys -t {} -l -- [cafleet msg {id} from {} {ts}] assignment",
                fixture.pane, fixture.director
            ),
            format!("send-keys -t {} Enter", fixture.pane),
        ]
    );
}

#[test]
fn non_idle_previews_stay_pending_even_when_the_message_is_old() {
    for (state, capture) in [
        ("working", format!("✻ Working (esc to interrupt)\n{IDLE}")),
        ("awaiting_user", format!(" ❯ 1. Yes, proceed\n{IDLE}")),
        (
            "draft",
            IDLE.replace("❯ \x1b[0m", "❯ \x1b[0;37munsent draft\x1b[0m"),
        ),
        ("unknown", "unrecognized screen".into()),
    ] {
        let fixture = Fixture::new();
        fixture.cli.set_pane_capture(&fixture.pane, &capture);
        let old = fixture.pending("old assignment");
        fixture
            .cli
            .sqlite()
            .execute(
                "UPDATE messages SET created_at='1900-01-01T00:00:00+00:00' WHERE message_id=?1",
                [old],
            )
            .unwrap();
        let output = fixture.send("new assignment");
        let result = payload(&output);
        assert_eq!(result["notification_sent"], false);
        assert_eq!(fixture.stamp(old), None);
        assert_eq!(
            fixture.stamp(result["message"]["message_id"].as_i64().unwrap()),
            None
        );
        assert_eq!(fixture.cli.shim_count("send-keys"), 0, "{state}");
        if matches!(state, "draft" | "unknown") {
            let new = result["message"]["message_id"].as_i64().unwrap();
            assert!(stderr(&output).contains(&format!(
                "Message {new} was persisted. member {} is {state}; preview {old} was not sent.", fixture.worker)),
                "{}", stderr(&output));
        }
    }
}

#[test]
fn each_send_attempts_only_the_oldest_pending_preview() {
    let fixture = Fixture::new();
    let first = fixture.pending("first");
    let second = fixture.pending("second");
    fixture
        .cli
        .sqlite()
        .execute_batch("UPDATE messages SET created_at='2026-01-01T00:00:00+00:00';")
        .unwrap();
    let output = fixture.send("third");
    let result = payload(&output);
    let third = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["notification_sent"], false);
    assert!(fixture.stamp(first).is_some());
    assert_eq!(fixture.stamp(second), None);
    assert_eq!(fixture.stamp(third), None);
    assert_eq!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .filter(|line| line.contains(" -l "))
            .count(),
        1
    );
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
    let calls = fixture.cli.shim_calls();
    let literals: Vec<_> = calls.iter().filter(|line| line.contains(" -l ")).collect();
    assert_eq!(literals.len(), 1);
    assert!(literals[0].ends_with("] deliver this"), "{literals:?}");
}

#[test]
fn a_failed_preview_preserves_the_committed_message_and_pending_stamp() {
    for failed in ["capture-pane", "send-keys"] {
        let mut fixture = Fixture::new();
        fixture.cli.fail_subcommand = Some(failed.into());
        let output = fixture.send("persist exactly once");
        let result = payload(&output);
        let id = result["message"]["message_id"].as_i64().unwrap();
        assert_eq!(result["notification_sent"], false);
        assert_eq!(fixture.stamp(id), None);
        assert_eq!(
            fixture
                .cli
                .sqlite()
                .query_row("SELECT count(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            stderr(&output).contains(&id.to_string()),
            "{}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("forced failure"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn a_notification_stamp_failure_keeps_pending_state_and_reports_uncertainty() {
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
    assert_eq!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .filter(|line| line.ends_with(" Enter"))
            .count(),
        1
    );
}

#[test]
fn director_and_self_notifications_require_no_pane_io() {
    let fixture = Fixture::new();
    for (sender, recipient) in [
        (fixture.worker, fixture.director),
        (fixture.worker, fixture.worker),
    ] {
        let output = fixture.cli.run_outside_tmux(&[
            "message",
            "send",
            "--from-member-id",
            &sender.to_string(),
            "--to-member-id",
            &recipient.to_string(),
            "read through broker",
            "--json",
        ]);
        let result = payload(&output);
        assert_eq!(result["notification_sent"], false);
    }
    assert!(fixture.cli.shim_calls().is_empty());
    assert_eq!(
        fixture
            .cli
            .sqlite()
            .query_row(
                "SELECT count(*) FROM members WHERE fleet_id=?1",
                [fixture.fleet],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
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
    let calls = fixture.cli.shim_calls();
    let literal = calls.iter().find(|line| line.contains(" -l ")).unwrap();
    assert!(literal.contains(" -l -- [cafleet msg "), "{literal}");
    assert!(
        literal.ends_with("] ! printf danger⏎next⏎line⏎last␉column\\x1b\\x7f"),
        "{literal}"
    );
}
