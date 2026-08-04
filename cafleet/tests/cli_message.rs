//! CLI contract tests: the `message` group and the shared positional-`TEXT` /
//! `--file` body reader (SPEC §6.3 *message group*, *text-body input*).

mod common;

use common::{Cli, code, stderr, stdout, write_file};

fn fleet_with_member(cli: &Cli) -> (i64, i64, i64) {
    let (fleet_id, director_id) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    (fleet_id, director_id, member_id)
}

#[test]
fn send_prints_the_header_and_the_compact_echo() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "hello worker",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.starts_with("Message sent.\n"), "got: {out}");
    assert!(
        out.contains(&format!("| from:{director_id} |")),
        "got: {out}"
    );
    assert!(out.contains("hello worker"), "got: {out}");
}

#[test]
fn send_truncates_the_echo_but_never_the_persisted_text() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let long_text = "a".repeat(250);
    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        &long_text,
    ]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(
        out.contains(&format!("{}…", "a".repeat(200))),
        "the echo truncates at max_text_len (200), got: {out}"
    );
    assert!(
        !out.contains(&long_text),
        "the echo never carries the full body"
    );

    let stored: String = cli
        .sqlite()
        .query_row(
            "SELECT text FROM messages WHERE type='unicast'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, long_text, "persistence is never truncated");
}

#[test]
fn poll_and_ack_walk_the_delivery_lifecycle_with_subject_ids_only() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "task one",
    ]);

    let output = cli.run(&["message", "poll", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("task one"),
        "got: {}",
        stdout(&output)
    );

    let message_id: i64 = cli
        .sqlite()
        .query_row(
            "SELECT message_id FROM messages WHERE type='unicast'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let output = cli.run(&["message", "ack", &message_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).starts_with("Message acknowledged.\n"),
        "got: {}",
        stdout(&output)
    );

    let output = cli.run(&["message", "poll", &member_id.to_string()]);
    assert!(
        stdout(&output).contains("No messages found."),
        "got: {}",
        stdout(&output)
    );
}

#[test]
fn poll_of_an_unknown_member_is_the_existence_error() {
    let cli = Cli::new();
    let _ = fleet_with_member(&cli);
    let output = cli.run(&["message", "poll", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: Member 999 not found"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn text_body_usage_errors_exit_2() {
    let cli = Cli::new();
    let _ = fleet_with_member(&cli);
    let base = [
        "message",
        "send",
        "--from-member-id",
        "1",
        "--to-member-id",
        "2",
    ];

    let output = cli.run(&base);
    assert_eq!(
        code(&output),
        2,
        "neither TEXT nor --file is clap's native group error"
    );

    let mut both = base.to_vec();
    both.extend(["x", "--file", "f.txt"]);
    let output = cli.run(&both);
    assert_eq!(code(&output), 2, "TEXT and --file conflict at parse time");

    let mut empty = base.to_vec();
    empty.push("   ");
    let output = cli.run(&empty);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("text may not be empty."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn file_surfaces_are_application_errors() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let base = |file: &str| {
        vec![
            "message".to_string(),
            "send".to_string(),
            "--from-member-id".to_string(),
            director_id.to_string(),
            "--to-member-id".to_string(),
            member_id.to_string(),
            "--file".to_string(),
            file.to_string(),
        ]
    };
    let run = |file: &str| {
        let args = base(file);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli.run(&refs)
    };

    let missing = cli.home.path().join("nope.txt");
    let output = run(missing.to_str().unwrap());
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "--file {}: file does not exist or is not a regular file.",
            missing.display()
        )),
        "got: {}",
        stderr(&output)
    );

    let empty = write_file(&cli.home.path().join("empty.txt"), b" \n ");
    let output = run(&empty);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("--file {empty}: file is empty.")),
        "got: {}",
        stderr(&output)
    );

    let binary = write_file(&cli.home.path().join("bin.dat"), &[0xff, 0xfe, 0x00]);
    let output = run(&binary);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("--file {binary}: file is not valid UTF-8.")),
        "got: {}",
        stderr(&output)
    );

    let args = base("-");
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = cli.run_with_stdin(&refs, "   ");
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("--file -: stdin is empty."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn send_rejects_a_cross_fleet_pair() {
    let cli = Cli::new();
    let (_, director_id, _member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "beta",
        "--coding-agent",
        "claude",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let stranger_id: i64 = stdout(&output)
        .trim()
        .split("director=")
        .nth(1)
        .expect("the compact fleet-create line names the director")
        .parse()
        .expect("the director id is an integer");

    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &stranger_id.to_string(),
        "hi",
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "Error: members {director_id} and {stranger_id} are not in the same fleet."
        )),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn send_and_broadcast_reject_an_unknown_sender() {
    let cli = Cli::new();
    let (_, _, member_id) = fleet_with_member(&cli);

    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        "999",
        "--to-member-id",
        &member_id.to_string(),
        "hi",
    ]);
    assert_eq!(code(&output), 1, "the sender check precedes the recipient");
    assert!(
        stderr(&output).contains("Error: Sender member not found or not active: 999"),
        "got: {}",
        stderr(&output)
    );

    let output = cli.run(&["message", "broadcast", "--from-member-id", "999", "hi"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: Sender member not found or not active: 999"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn send_rejects_a_missing_destination() {
    let cli = Cli::new();
    let (_, director_id, _) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        "999",
        "hi",
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: Destination member not found: 999"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn ack_guards_are_existence_and_state_only() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);

    let output = cli.run(&["message", "ack", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: Message 999 not found"),
        "got: {}",
        stderr(&output)
    );

    cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "task",
    ]);
    let message_id: i64 = cli
        .sqlite()
        .query_row(
            "SELECT message_id FROM messages WHERE type='unicast'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let output = cli.run(&["message", "ack", &message_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let output = cli.run(&["message", "ack", &message_id.to_string()]);
    assert_eq!(code(&output), 1, "input_required is the only ackable state");
    assert!(
        stderr(&output).contains("Error: Cannot ACK message in state completed"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn show_of_an_unknown_message_is_the_existence_error() {
    let cli = Cli::new();
    let _ = fleet_with_member(&cli);
    let output = cli.run(&["message", "show", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: Message 999 not found"),
        "got: {}",
        stderr(&output)
    );
}

/// Success criterion: `--json` output carries complete, untruncated message
/// bodies on every message subcommand; text output stays truncated to
/// `CAFLEET_MAX_TEXT_LEN`.
#[test]
fn json_is_untruncated_on_every_message_subcommand() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let long_text = "a".repeat(250);

    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        &long_text,
        "--json",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["message"]["text"], long_text,
        "send --json is complete"
    );
    let message_id = payload["message"]["message_id"]
        .as_i64()
        .expect("the send envelope names the message id");

    let output = cli.run(&["message", "poll", &member_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload[0]["text"], long_text, "poll --json is complete");

    let output = cli.run(&["message", "poll", &member_id.to_string()]);
    let out = stdout(&output);
    assert!(
        out.contains(&format!("{}…", "a".repeat(200))),
        "poll text truncates at max_text_len (200), got: {out}"
    );
    assert!(
        !out.contains(&long_text),
        "poll text never carries the full body"
    );

    let output = cli.run(&["message", "show", &message_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["message"]["text"], long_text,
        "show --json is complete"
    );

    let output = cli.run(&["message", "ack", &message_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["message"]["text"], long_text,
        "ack --json is complete"
    );
}

#[test]
fn broadcast_prints_the_recipients_and_delivered_counts() {
    let cli = Cli::new();
    let (_, director_id, _member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "broadcast",
        "--from-member-id",
        &director_id.to_string(),
        "all hands",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("recipients=1 delivered=1"),
        "one member peer, preview landed via the shim, got: {out}"
    );
    assert!(out.contains("broadcast id="), "got: {out}");
}

#[test]
fn broadcast_json_summary_carries_the_null_recipient() {
    let cli = Cli::new();
    let (_, director_id, _member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "broadcast",
        "--from-member-id",
        &director_id.to_string(),
        "all hands",
        "--json",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let envelope = &payload[0];
    assert_eq!(envelope["recipients"], 1);
    assert_eq!(envelope["delivered"], 1);
    let summary = &envelope["message"];
    assert_eq!(summary["type"], "broadcast_summary");
    assert_eq!(
        summary["to_member_id"],
        serde_json::Value::Null,
        "the summary row has no single recipient — never a 0 sentinel"
    );
    assert_eq!(
        summary["origin_message_id"], summary["message_id"],
        "self-referential origin"
    );
    assert_eq!(summary["text"], "Broadcast sent to 1 recipients");
}

#[test]
fn show_json_pins_the_typed_column_envelope() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "hi",
    ]);
    let message_id: i64 = cli
        .sqlite()
        .query_row(
            "SELECT message_id FROM messages WHERE type='unicast'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let output = cli.run(&["message", "show", &message_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let raw = stdout(&output);
    let payload: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
    let ts = payload["message"]["created_at"].as_str().unwrap();
    let expected = format!(
        r#"{{"message":{{"message_id":{message_id},"owner_member_id":{member_id},"from_member_id":{director_id},"to_member_id":{member_id},"type":"unicast","created_at":"{ts}","status_state":"input_required","status_timestamp":"{ts}","origin_message_id":null,"text":"hi"}}}}"#
    );
    assert_eq!(
        raw.trim(),
        expected,
        "compact JSON with the pinned key order"
    );
}
