//! Step 6 CLI contract tests: the `message` group and the shared
//! `--text` / `--text-file` body reader (SPEC §6.3 *message group*,
//! *text-body input*).

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
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "send",
        "--fleet-id",
        &fleet_id.to_string(),
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "--text",
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
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    let long_text = "a".repeat(250);
    let output = cli.run(&[
        "message",
        "send",
        "--fleet-id",
        &fleet_id.to_string(),
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "--text",
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
fn quiet_send_and_ack_print_the_bare_message_id() {
    let cli = Cli::new();
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "send",
        "--fleet-id",
        &fleet_id.to_string(),
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "--text",
        "hi",
        "--quiet",
    ]);
    assert_eq!(code(&output), 0);
    let message_id: i64 = stdout(&output).trim().parse().expect("a bare message id");

    let output = cli.run(&[
        "message",
        "ack",
        "--fleet-id",
        &fleet_id.to_string(),
        "--member-id",
        &member_id.to_string(),
        "--message-id",
        &message_id.to_string(),
        "--quiet",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output).trim(), message_id.to_string());
}

#[test]
fn poll_and_ack_walk_the_delivery_lifecycle() {
    let cli = Cli::new();
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    cli.run(&[
        "message",
        "send",
        "--fleet-id",
        &fleet_id.to_string(),
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "--text",
        "task one",
    ]);

    let output = cli.run(&[
        "message",
        "poll",
        "--fleet-id",
        &fleet_id.to_string(),
        "--member-id",
        &member_id.to_string(),
    ]);
    assert_eq!(code(&output), 0);
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
    let output = cli.run(&[
        "message",
        "ack",
        "--fleet-id",
        &fleet_id.to_string(),
        "--member-id",
        &member_id.to_string(),
        "--message-id",
        &message_id.to_string(),
    ]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).starts_with("Message acknowledged.\n"),
        "got: {}",
        stdout(&output)
    );

    let output = cli.run(&[
        "message",
        "poll",
        "--fleet-id",
        &fleet_id.to_string(),
        "--member-id",
        &member_id.to_string(),
    ]);
    assert!(
        stdout(&output).contains("No messages found."),
        "got: {}",
        stdout(&output)
    );
}

#[test]
fn the_fleet_gate_runs_before_the_handler_body() {
    let cli = Cli::new();
    let (fleet_id, _, _) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "poll",
        "--fleet-id",
        &fleet_id.to_string(),
        "--member-id",
        "999",
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("Error: member 999 is not in fleet {fleet_id}.")),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn text_body_usage_errors_exit_2() {
    let cli = Cli::new();
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    let base = [
        "message",
        "send",
        "--fleet-id",
        "1",
        "--from-member-id",
        "1",
        "--to-member-id",
        "2",
    ];
    let _ = (fleet_id, director_id, member_id);

    let output = cli.run(&base);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("Provide exactly one of --text or --text-file."),
        "got: {}",
        stderr(&output)
    );

    let mut both = base.to_vec();
    both.extend(["--text", "x", "--text-file", "f.txt"]);
    let output = cli.run(&both);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("--text and --text-file are mutually exclusive."),
        "got: {}",
        stderr(&output)
    );

    let mut empty = base.to_vec();
    empty.extend(["--text", "   "]);
    let output = cli.run(&empty);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("text may not be empty."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn text_file_surfaces_are_application_errors() {
    let cli = Cli::new();
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    let base = |file: &str| {
        vec![
            "message".to_string(),
            "send".to_string(),
            "--fleet-id".to_string(),
            fleet_id.to_string(),
            "--from-member-id".to_string(),
            director_id.to_string(),
            "--to-member-id".to_string(),
            member_id.to_string(),
            "--text-file".to_string(),
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
            "--text-file {}: file does not exist or is not a regular file.",
            missing.display()
        )),
        "got: {}",
        stderr(&output)
    );

    let empty = write_file(&cli.home.path().join("empty.txt"), b" \n ");
    let output = run(&empty);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("--text-file {empty}: file is empty.")),
        "got: {}",
        stderr(&output)
    );

    let binary = write_file(&cli.home.path().join("bin.dat"), &[0xff, 0xfe, 0x00]);
    let output = run(&binary);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("--text-file {binary}: file is not valid UTF-8.")),
        "got: {}",
        stderr(&output)
    );

    let args = base("-");
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = cli.run_with_stdin(&refs, "   ");
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("--text-file -: stdin is empty."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn broadcast_prints_the_recipients_and_delivered_counts() {
    let cli = Cli::new();
    let (fleet_id, director_id, _member_id) = fleet_with_member(&cli);
    let output = cli.run(&[
        "message",
        "broadcast",
        "--fleet-id",
        &fleet_id.to_string(),
        "--from-member-id",
        &director_id.to_string(),
        "--text",
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
fn show_full_json_pins_the_typed_column_envelope() {
    let cli = Cli::new();
    let (fleet_id, director_id, member_id) = fleet_with_member(&cli);
    cli.run(&[
        "message",
        "send",
        "--fleet-id",
        &fleet_id.to_string(),
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "--text",
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

    let output = cli.run(&[
        "message",
        "show",
        "--fleet-id",
        &fleet_id.to_string(),
        "--member-id",
        &member_id.to_string(),
        "--message-id",
        &message_id.to_string(),
        "--full",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
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
