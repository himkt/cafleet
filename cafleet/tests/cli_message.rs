//! CLI contract tests: the `message` group and the shared positional-`TEXT` /
//! `--file` body reader (SPEC §6.3 *message group*, *text-body input*).

mod common;

use common::{Cli, code, stderr, stdout, write_file};

fn fleet_with_member(cli: &Cli) -> (i64, i64, i64) {
    let (fleet_id, director_id) = cli.seeded_fleet();
    let member_id = cli.seed_member(fleet_id, "worker");
    (fleet_id, director_id, member_id)
}

/// The pinned partial-failure stderr line: persisted id, verbatim raw cause,
/// the no-resend instruction, and both recovery paths.
fn partial_error(message_id: i64, recipient_id: i64, raw: &str) -> String {
    format!(
        "Error: Message {message_id} was persisted, but pane notification failed: {raw}. \
         Do not resend this message. Recover the recipient pane, then run \
         'cafleet member ping {recipient_id}' or have the recipient run \
         'cafleet message poll {recipient_id}'."
    )
}

/// The single persisted unicast row: `(message_id, status_state, text)`.
fn only_unicast_row(cli: &Cli) -> (i64, String, String) {
    let count: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE type='unicast'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "exactly one persisted row — never a duplicate");
    cli.sqlite()
        .query_row(
            "SELECT message_id, status_state, text FROM messages WHERE type='unicast'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn send_truncates_the_echo_but_never_the_persisted_text() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let long_text = "界".repeat(250);
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
    assert!(out.starts_with("Message sent.\n"), "{out}");
    assert!(out.contains(&format!("| from:{director_id} |")), "{out}");
    assert!(
        out.contains(&format!("{}…", "界".repeat(200))),
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
fn send_partial_failure_reports_the_persisted_id_and_recovery() {
    let mut cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    cli.fail_subcommand = Some("send-keys".to_string());
    let mut expected_rows = Vec::new();
    for (body, json) in [("payload one", false), ("payload two", true)] {
        let from = director_id.to_string();
        let to = member_id.to_string();
        let mut args = vec![
            "message",
            "send",
            "--from-member-id",
            &from,
            "--to-member-id",
            &to,
            body,
        ];
        if json {
            args.push("--json");
        }
        let calls_before = cli.shim_calls().len();
        let output = cli.run(&args);
        assert_eq!(code(&output), 1);
        assert_eq!(stdout(&output), "");
        let conn = cli.sqlite();
        let mut statement = conn.prepare("SELECT message_id, status_state, text FROM messages WHERE type='unicast' ORDER BY message_id").unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), expected_rows.len() + 1);
        let new_row = rows.last().unwrap();
        assert_eq!(new_row.1, "input_required");
        assert_eq!(new_row.2, body);
        if let Some((previous_id, _, _)) = expected_rows.last() {
            assert!(new_row.0 > *previous_id);
        }
        expected_rows.push(new_row.clone());
        assert_eq!(rows, expected_rows);
        let raw = "tmux command failed: tmux send-keys -t %7 Escape\nstderr: forced failure";
        assert!(
            stderr(&output).contains(&partial_error(new_row.0, member_id, raw)),
            "{}",
            stderr(&output)
        );
        let calls = cli.shim_calls();
        assert_eq!(&calls[calls_before..], &["send-keys -t %7 Escape"]);
    }
}

#[test]
fn send_with_no_resolvable_multiplexer_persists_then_fails() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let calls_before = cli.shim_calls().len();

    let output = cli.run_outside_tmux(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "resolver gap",
    ]);
    assert_eq!(
        code(&output),
        1,
        "an attempted notification must fail loudly"
    );
    assert_eq!(stdout(&output), "");

    let (message_id, status, text) = only_unicast_row(&cli);
    assert_eq!(
        status, "input_required",
        "resolution failure cannot preempt the insert"
    );
    assert_eq!(text, "resolver gap");

    let raw = "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; \
               run cafleet inside a tmux or herdr session, or set CAFLEET_MULTIPLEXER";
    assert!(
        stderr(&output).contains(&partial_error(message_id, member_id, raw)),
        "got: {}",
        stderr(&output)
    );
    assert_eq!(
        cli.shim_calls().len(),
        calls_before,
        "no backend invocation exists to attempt"
    );
}

#[test]
fn send_with_an_ambiguous_multiplexer_persists_then_fails() {
    let mut cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    cli.set_env("HERDR_ENV", "1");

    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "ambiguous env",
    ]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");

    let (message_id, status, _) = only_unicast_row(&cli);
    assert_eq!(status, "input_required");
    let raw = "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
               set CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate";
    assert!(
        stderr(&output).contains(&partial_error(message_id, member_id, raw)),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn self_send_and_no_pane_skips_stay_successful_without_a_multiplexer() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);

    let output = cli.run_outside_tmux(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &director_id.to_string(),
        "note to self",
        "--json",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["notification_sent"], false,
        "self-send remains an intentional skip"
    );

    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [member_id],
        )
        .unwrap();
    let output = cli.run_outside_tmux(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "queued for pickup",
        "--json",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["notification_sent"], false,
        "a paneless recipient remains an intentional skip"
    );
}

#[test]
fn broadcast_keeps_exit_zero_and_counts_when_previews_fail() {
    let mut cli = Cli::new();
    let (_, director_id, _member_id) = fleet_with_member(&cli);
    cli.fail_subcommand = Some("send-keys".to_string());

    let output = cli.run(&[
        "message",
        "broadcast",
        "--from-member-id",
        &director_id.to_string(),
        "failing fanout",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stderr(&output), "", "no per-recipient error list appears");
    let out = stdout(&output);
    assert!(
        out.contains("recipients=2 delivered=0"),
        "failed previews only lower the delivered count, got: {out}"
    );
}

#[test]
fn poll_and_ack_walk_the_delivery_lifecycle_with_subject_ids_only() {
    let cli = Cli::new();
    let (_, director_id, member_id) = fleet_with_member(&cli);
    let missing = cli.run(&["message", "ack", "999"]);
    assert_eq!(code(&missing), 1);
    assert!(
        stderr(&missing).contains("Error: Message 999 not found"),
        "{}",
        stderr(&missing)
    );
    let sent = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director_id.to_string(),
        "--to-member-id",
        &member_id.to_string(),
        "task one",
    ]);

    assert_eq!(code(&sent), 0, "{}", stderr(&sent));
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

    let repeated = cli.run(&["message", "ack", &message_id.to_string()]);
    assert_eq!(code(&repeated), 1);
    assert!(
        stderr(&repeated).contains("Error: Cannot ACK message in state completed"),
        "{}",
        stderr(&repeated)
    );

    let output = cli.run(&["message", "poll", &member_id.to_string()]);
    assert_eq!(code(&output), 0);
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
fn blank_message_body_is_a_usage_error_after_guards() {
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
        "--monitor-file",
        &cli.monitor_prompt_path(),
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let stranger_id: i64 = stdout(&output)
        .split_whitespace()
        .find_map(|token| token.strip_prefix("director="))
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
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["message"]["text"], long_text,
        "send --json is complete"
    );
    assert_eq!(
        payload["notification_sent"], true,
        "the delivered-preview success envelope is unchanged"
    );
    let message_id = payload["message"]["message_id"]
        .as_i64()
        .expect("the send envelope names the message id");

    let output = cli.run(&["message", "poll", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload[0]["text"], long_text, "poll --json is complete");

    let output = cli.run(&["message", "poll", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
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
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let raw = stdout(&output);
    let shown: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
    let ts = shown["message"]["created_at"].as_str().unwrap();
    let body = serde_json::to_string(&long_text).unwrap();
    let expected = format!(
        r#"{{"message":{{"message_id":{message_id},"owner_member_id":{member_id},"from_member_id":{director_id},"to_member_id":{member_id},"type":"unicast","created_at":"{ts}","status_state":"input_required","status_timestamp":"{ts}","origin_message_id":null,"text":{body}}}}}"#
    );
    assert_eq!(
        raw.trim(),
        expected,
        "compact JSON with the pinned key order"
    );
    assert_eq!(
        shown["message"]["text"], long_text,
        "show --json is complete"
    );

    let output = cli.run(&["message", "ack", &message_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
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
        out.contains("recipients=2 delivered=2"),
        "the monitor and worker peers, previews landed via the shim, got: {out}"
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
    assert_eq!(envelope["recipients"], 2);
    assert_eq!(envelope["delivered"], 2);
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
    assert_eq!(summary["text"], "Broadcast sent to 2 recipients");
    let summary_id = summary["message_id"].as_i64().unwrap();
    let shown = cli.run(&["message", "show", &summary_id.to_string(), "--json"]);
    assert_eq!(code(&shown), 0, "{}", stderr(&shown));
    let fetched: serde_json::Value = serde_json::from_str(stdout(&shown).trim()).unwrap();
    assert_eq!(
        fetched["message"], *summary,
        "show retains the complete summary, including null recipient"
    );
}

#[test]
fn integrity_invalid_stored_message_enum_exits_one_without_success_output_or_panic() {
    for field in ["type", "status_state"] {
        let cli = Cli::new();
        let (_, director, worker) = fleet_with_member(&cli);
        let id = cli.seed_message(director, worker, "integrity");
        let conn = cli.sqlite();
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute_batch(&format!("UPDATE messages SET {field}='corrupt-enum'"))
            .unwrap();
        for json in [false, true] {
            let id_arg = id.to_string();
            let mut args = vec!["message", "show", &id_arg];
            if json {
                args.push("--json");
            }
            let shown = cli.run(&args);
            assert_eq!(code(&shown), 1, "invalid {field}: {}", stdout(&shown));
            assert!(stdout(&shown).is_empty());
            let detail = stderr(&shown);
            assert!(
                detail.starts_with("Error: ") && !detail.contains("panicked"),
                "{detail}"
            );
        }
    }
}
