mod common;

use common::{
    CAPTURE_FAILS, Cli, REST_CAPTURE, code, stderr, stdout, write_executable, write_file,
};
use sha2::{Digest, Sha256};

fn database_records(cli: &Cli) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let conn = cli.sqlite();
    [
        "fleets",
        "members",
        "member_placements",
        "messages",
        "sqlite_sequence",
    ]
    .map(|table| {
        let mut statement = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
    .into()
}

fn assert_no_pane_effects(calls: &[String]) {
    assert!(
        calls
            .iter()
            .all(|line| !line.starts_with("split-window") && !line.starts_with("kill-pane")),
        "{calls:?}"
    );
}

#[test]
fn member_ping_excludes_the_director_before_capture_or_input() {
    let cli = Cli::new();
    let (_, director) = cli.seeded_fleet();
    let output = cli.run(&["member", "ping", &director.to_string(), "--json"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("cannot ping the Director's own pane"));
    assert!(cli.shim_calls().is_empty());
}

fn assert_keys(value: &serde_json::Value, expected: &[&str]) {
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        expected
    );
}

fn assert_capture_content(entry: &serde_json::Value, pane: &str) {
    let content = REST_CAPTURE.replace("{pane}", pane);
    assert_eq!(entry["content"], content);
    assert!(entry["captured_at"].is_string());
    let digest = Sha256::digest(content.as_bytes());
    let expected: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(entry["content_sha256"], expected);
}

#[test]
fn step8_capture_mux_guard_precedes_unknown_member_lookup() {
    let mut cli = Cli::new();
    cli.seeded_fleet();
    cli.set_env("CAFLEET_MULTIPLEXER", "invalid-step8-backend");
    let output = cli.run(&["member", "capture", "999"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("invalid-step8-backend"));
    assert!(!stderr(&output).contains("member 999 not found"));
}

#[test]
fn fleet_capture_live_fleet_guard_precedes_invalid_mux_resolution() {
    let mut cli = Cli::new();
    let (fleet, _) = cli.seeded_fleet();
    let deleted = cli.run(&["fleet", "delete", &fleet.to_string()]);
    assert_eq!(code(&deleted), 0, "{}", stderr(&deleted));
    assert!(
        cli.sqlite()
            .query_row(
                "SELECT deleted_at FROM fleets WHERE fleet_id=?1",
                [fleet],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap()
            .is_some()
    );
    cli.set_env("CAFLEET_MULTIPLEXER", "invalid-step8-backend");
    for id in [999, fleet] {
        let calls_before = cli.shim_calls().len();
        let output = cli.run(&["fleet", "capture", &id.to_string()]);
        assert_eq!(code(&output), 1);
        assert_eq!(
            stderr(&output).trim(),
            format!("Error: fleet {id} not found")
        );
        let calls = cli.shim_calls();
        assert!(calls[calls_before..].is_empty(), "{calls:?}");
    }
}

#[test]
fn member_create_spawns_patches_the_pane_and_substitutes_identity() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_cli_fleet();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "does work",
        "FLEET {fleet_id} ME {member_id} DIRECTOR {director_member_id} AGENT {coding_agent}",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "2 worker backend=claude pane=%7\n");

    let split_line = cli
        .shim_calls()
        .into_iter()
        .rfind(|line| line.starts_with("split-window"))
        .expect("split-window was invoked");
    assert!(
        split_line.contains("-e CAFLEET_DATABASE_URL=sqlite:///"),
        "only CAFLEET_DATABASE_URL is forwarded, got: {split_line}"
    );
    assert!(
        split_line.contains("FLEET 1 ME 2 DIRECTOR 1 AGENT claude"),
        "the rendered prompt carries literal identity, got: {split_line}"
    );
    assert!(
        split_line.contains("--permission-mode dontAsk"),
        "{split_line}"
    );
    assert!(
        split_line.contains("--allowedTools Bash(cafleet message *)"),
        "{split_line}"
    );

    let pane: Option<String> = cli
        .sqlite()
        .query_row(
            "SELECT mux_pane_id FROM member_placements WHERE member_id=2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pane.as_deref(),
        Some("%7"),
        "the pane id is patched post-split"
    );
}

#[test]
fn member_create_accepts_the_prompt_via_file() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_cli_fleet();
    let prompt_file = write_file(
        &cli.home.path().join("prompt.md"),
        b"FLEET {fleet_id} AGENT {coding_agent}",
    );
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "does work",
        "--file",
        &prompt_file,
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let split_line = cli
        .shim_calls()
        .into_iter()
        .rfind(|line| line.starts_with("split-window"))
        .expect("split-window was invoked");
    assert!(
        split_line.contains("FLEET 1 AGENT claude"),
        "the file body is rendered with identity, got: {split_line}"
    );
}

#[test]
fn member_create_split_failure_rolls_back_the_registration() {
    let mut cli = Cli::new();
    let (fleet_id, _) = cli.with_cli_fleet();
    cli.fail_subcommand = Some("split-window".to_string());
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "d",
        "prompt",
    ]);
    assert_eq!(code(&output), 1);
    let err = stderr(&output);
    assert!(err.contains("tmux split-window failed:"), "got: {err}");
    assert!(err.contains("forced failure"), "got: {err}");
    assert!(
        err.contains("unknown") && err.contains("unconfirmed"),
        "got: {err}"
    );
    assert!(!err.contains("Rolled back registration"), "got: {err}");

    let members: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM members WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(members, 1, "only the Director remains active");
    let placement_count: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM member_placements WHERE member_id=2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(placement_count, 0);
    assert!(
        !cli.shim_calls()
            .iter()
            .any(|line| line.starts_with("send-keys") || line.starts_with("kill-pane")),
        "a failed split with no confirmed id must not guess a pane: {:?}",
        cli.shim_calls()
    );
}

#[test]
fn member_create_validates_model_and_effort_before_any_side_effect() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_cli_fleet();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "d",
        "--coding-agent",
        "claude",
        "--effort",
        "turbo",
        "prompt",
    ]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains(
            "--effort for the claude backend must be one of low, medium, high, \
             xhigh, max (got 'turbo')."
        ),
        "got: {}",
        stderr(&output)
    );
    let members: i64 = cli
        .sqlite()
        .query_row("SELECT COUNT(*) FROM members", [], |row| row.get(0))
        .unwrap();
    assert_eq!(members, 1, "validation precedes registration");
}

#[test]
fn member_create_requires_the_backend_binary_on_path() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_cli_fleet();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "d",
        "--coding-agent",
        "codex",
        "prompt",
    ]);
    assert_eq!(
        code(&output),
        1,
        "the precondition fires before registration"
    );
    assert!(
        stderr(&output).contains("binary codex not found on PATH"),
        "got: {}",
        stderr(&output)
    );
    let members: i64 = cli
        .sqlite()
        .query_row("SELECT COUNT(*) FROM members", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        members, 1,
        "no registration side effect before the precondition"
    );
}

#[test]
fn member_create_unknown_fleet_is_a_usage_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        "999",
        "--name",
        "worker",
        "--description",
        "d",
        "prompt",
    ]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("Fleet '999' not found."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn member_creation_uses_its_assignment_without_a_registry_role() {
    let cli = Cli::new();
    let (fleet, _) = cli.with_cli_fleet();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet.to_string(),
        "--name",
        "reviewer",
        "--description",
        "Review the assigned change",
        "--json",
        "read the supplied assignment",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let member: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let id = member["member_id"].as_i64().expect("new member id");
    let record: (String, String) = cli
        .sqlite()
        .query_row(
            "SELECT name, description FROM members WHERE member_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        record,
        ("reviewer".into(), "Review the assigned change".into())
    );
    assert_eq!(cli.shim_count("split-window"), 1);
}

#[test]
fn member_creation_inherits_the_director_backend_and_applies_explicit_overrides() {
    for (override_backend, model, expected) in [
        (None, None, "codex"),
        (None, Some("claude-opus-5-5"), "codex"),
        (Some("claude"), Some("claude-opus-5-5"), "claude"),
        (
            Some("opencode"),
            Some("opencode/glm-5.3-flash-free"),
            "opencode",
        ),
    ] {
        let cli = Cli::new();
        cli.install();
        for binary in ["codex", "opencode"] {
            write_executable(&cli.shim_dir.join(binary), "#!/bin/sh\nexit 0\n");
        }
        let fleet = cli.run(&[
            "fleet",
            "create",
            "--name",
            "backend",
            "--coding-agent",
            "codex",
        ]);
        assert_eq!(code(&fleet), 0, "{}", stderr(&fleet));
        let mut args = vec![
            "member",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "worker",
            "--description",
            "test",
        ];
        if let Some(backend) = override_backend {
            args.extend(["--coding-agent", backend]);
        }
        if let Some(model) = model {
            args.extend(["--model", model]);
        }
        args.push("AGENT {coding_agent} MEMBER {member_id}");
        let output = cli.run(&args);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert_eq!(
            stdout(&output),
            format!("2 worker backend={expected} pane=%7\n")
        );
        let split = cli
            .shim_calls()
            .into_iter()
            .find(|line| line.starts_with("split-window"))
            .unwrap();
        assert!(
            split.contains(&format!("AGENT {expected} MEMBER 2")),
            "{split}"
        );
        if let Some(model) = model {
            assert!(split.contains(&format!("--model {model}")), "{split}");
        }
    }
}

#[test]
fn member_creation_rejects_unsupported_effort_before_registration() {
    let cli = Cli::new();
    cli.with_cli_fleet();
    write_executable(&cli.shim_dir.join("opencode"), "#!/bin/sh\nexit 0\n");
    let records = database_records(&cli);
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        "1",
        "--name",
        "worker",
        "--description",
        "test",
        "--coding-agent",
        "opencode",
        "--effort",
        "low",
        "prompt",
    ]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("opencode does not support reasoning effort."),
        "{}",
        stderr(&output)
    );
    assert_eq!(database_records(&cli), records);
    assert_no_pane_effects(&cli.shim_calls());
}

#[test]
fn member_prompt_source_validation_preserves_registration_and_panes() {
    let cli = Cli::new();
    cli.with_cli_fleet();
    let missing = cli
        .home
        .path()
        .join("missing.md")
        .to_str()
        .unwrap()
        .to_owned();
    let empty = write_file(&cli.home.path().join("empty.md"), b" \n");
    let invalid = write_file(&cli.home.path().join("invalid.md"), &[0xff]);
    let valid = write_file(&cli.home.path().join("valid.md"), b"assignment");
    let records = database_records(&cli);
    for (source, expected) in [
        (vec!["--file", missing.as_str()], 1),
        (vec!["--file", empty.as_str()], 1),
        (vec!["--file", invalid.as_str()], 1),
        (vec!["--file", valid.as_str(), "also inline"], 2),
        (vec![], 2),
    ] {
        let mut args = vec![
            "member",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "worker",
            "--description",
            "test",
        ];
        args.extend(source);
        let output = cli.run(&args);
        assert_eq!(code(&output), expected, "{args:?}: {}", stderr(&output));
        assert_eq!(database_records(&cli), records);
        assert_no_pane_effects(&cli.shim_calls());
    }
}

#[test]
fn member_creation_reads_stdin_and_preserves_literal_braces() {
    let cli = Cli::new();
    cli.with_cli_fleet();
    let output = cli.run_with_stdin(
        &[
            "member",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "worker",
            "--description",
            "test",
            "--file",
            "-",
        ],
        "MEMBER {member_id} OBJECT {{value}} AGENT {coding_agent}",
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.contains("MEMBER 2 OBJECT {value} AGENT claude"))
    );
}

#[test]
fn member_placement_is_pending_until_the_owned_pane_is_patched() {
    let cli = Cli::new();
    cli.with_cli_fleet();
    cli.sqlite().execute_batch("CREATE TRIGGER assert_pending_patch BEFORE UPDATE OF mux_pane_id ON member_placements WHEN OLD.mux_pane_id IS NOT NULL BEGIN SELECT RAISE(ABORT, 'placement must begin pending'); END;").unwrap();
    let member = cli.create_member(1, "worker");
    assert_eq!(member, 2);
    assert_eq!(
        cli.sqlite()
            .query_row(
                "SELECT mux_pane_id FROM member_placements WHERE member_id=?1",
                [member],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "%7"
    );
}

#[test]
fn fleet_capture_follows_the_director_back_reference_before_member_id_order() {
    let cli = Cli::new();
    let (fleet, first) = cli.seeded_fleet();
    let second = cli.seed_member(fleet, "second");
    let director = cli.seed_member(fleet, "new-director");
    cli.sqlite()
        .execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            rusqlite::params![director, fleet],
        )
        .unwrap();
    let before = database_records(&cli);
    let output = cli.run(&["fleet", "capture", &fleet.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let entries = payload.as_array().unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["member_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [director, first, second]
    );
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["director", "member", "member"]
    );
    assert_eq!(database_records(&cli), before);
    assert!(
        cli.shim_calls()
            .iter()
            .all(|line| !line.starts_with("send-keys")
                && !line.starts_with("split-window")
                && !line.starts_with("kill-pane"))
    );
}

#[test]
fn fleet_capture_annotates_a_pending_director_and_keeps_other_panes_available() {
    let cli = Cli::new();
    let (fleet, director) = cli.seeded_fleet();
    let worker = cli.seed_member(fleet, "healthy");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [director],
        )
        .unwrap();
    let before = database_records(&cli);
    let output = cli.run(&["fleet", "capture", &fleet.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let entries = payload.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["member_id"], director);
    assert_eq!(entries[0]["kind"], "director");
    for field in ["pane_id", "content", "captured_at", "content_sha256"] {
        assert_eq!(entries[0][field], serde_json::Value::Null, "{field}");
    }
    assert_eq!(
        entries[0]["error"],
        "pane not available (pending placement)"
    );
    assert_eq!(entries[1]["member_id"], worker);
    assert_capture_content(&entries[1], &format!("%{}", worker + 4));
    let text = cli.run(&["fleet", "capture", &fleet.to_string()]);
    assert_eq!(code(&text), 0, "{}", stderr(&text));
    assert!(stdout(&text).contains(&format!(
        "=== {director} (Director; kind=director; coding_agent=claude; pane=—) ===\npane not available (pending placement)"
    )), "{}", stdout(&text));
    assert!(
        stdout(&text).contains(&format!("=== {worker} (healthy;")),
        "{}",
        stdout(&text)
    );
    assert_eq!(database_records(&cli), before);
    let calls = cli.shim_calls();
    assert!(
        calls.iter().all(|line| line.starts_with("capture-pane")),
        "{calls:?}"
    );
    assert_eq!(cli.shim_count("capture-pane"), 2);
}

#[test]
fn fleet_capture_reports_a_failed_pane_and_continues_with_healthy_members() {
    let cli = Cli::new();
    let (fleet, _) = cli.seeded_fleet();
    let member = cli.seed_member(fleet, "healthy");
    cli.set_pane_capture("%0", CAPTURE_FAILS);
    let output = cli.run(&["fleet", "capture", &fleet.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(
        payload[0]["error"]
            .as_str()
            .unwrap()
            .contains("forced failure")
    );
    assert_eq!(payload[1]["member_id"], member);
    assert_eq!(payload[1]["error"], serde_json::Value::Null);
    assert_capture_content(&payload[1], &format!("%{}", member + 4));
}

#[test]
fn fleet_capture_preserves_requested_ansi_and_hashes_the_presented_content() {
    let cli = Cli::new();
    let (fleet, _) = cli.seeded_fleet();
    let styled = "\u{1b}[90m› suggestion\u{1b}[0m";
    cli.set_pane_capture("%0", styled);
    for (flags, content) in [
        (vec!["--json"], "› suggestion"),
        (vec!["--ansi", "--json"], styled),
    ] {
        let fleet = fleet.to_string();
        let mut args = vec!["fleet", "capture", fleet.as_str()];
        args.extend(flags);
        let output = cli.run(&args);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(payload[0]["content"], content);
        let digest = Sha256::digest(content.as_bytes());
        let expected = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(payload[0]["content_sha256"], expected);
    }
}

#[test]
fn member_show_takes_the_positional_subject() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let member_id = cli.seed_member(fleet_id, "worker");

    let output = cli.run(&["member", "show", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), format!("{member_id} worker active\n"));

    let output = cli.run(&["member", "show", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["kind"], "member");
    assert_eq!(
        payload["placement"]["mux_pane_id"], "%6",
        "the detailed view is JSON-only"
    );

    let registered = payload["registered_at"].as_str().unwrap();
    let placed = payload["placement"]["created_at"].as_str().unwrap();
    assert_eq!(
        stdout(&output),
        format!(
            r#"{{"member_id":{member_id},"name":"worker","description":"test member","status":"active","registered_at":"{registered}","kind":"member","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%6","coding_agent":"claude","created_at":"{placed}"}}}}"#
        ) + "\n"
    );

    let output = cli.run(&["member", "show", "99"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: Member 99 not found"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn member_list_takes_the_positional_fleet_subject() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    cli.seed_member(fleet_id, "worker");
    let output = cli.run(&["member", "list", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.starts_with("2 members:\n"), "got: {out}");
    assert!(
        out.contains("  member_id  name           kind      backend   pane_id  idle"),
        "got: {out}"
    );
    assert!(out.contains("director"), "got: {out}");
    assert!(out.contains("worker"), "got: {out}");

    assert_eq!(
        code(&cli.run(&["member", "list"])),
        2,
        "the positional FLEET_ID is required"
    );
}

#[test]
fn member_delete_kills_the_pane_and_reports_it() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let member_id = cli.seed_member(fleet_id, "worker");
    let output = cli.run(&["member", "delete", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.starts_with("Member deleted.\n"), "got: {out}");
    assert!(out.contains("%6 (killed)"), "got: {out}");
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.starts_with("kill-pane -t %6")),
        "the pane is killed immediately"
    );
}

#[test]
fn member_delete_of_the_root_director_is_rejected() {
    let cli = Cli::new();
    let (_, director_id) = cli.seeded_fleet();
    let output = cli.run(&["member", "delete", &director_id.to_string()]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(
            "Error: cannot deregister the root Director; use 'cafleet fleet delete' instead"
        ),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn member_ping_skips_a_pending_placement_and_exits_zero() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let member_id = cli.seed_member(fleet_id, "worker");
    assert_eq!(
        cli.sqlite()
            .execute(
                "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
                [member_id]
            )
            .unwrap(),
        1
    );
    let calls_before = cli.shim_calls().len();
    let capture = cli.run(&["member", "capture", &member_id.to_string()]);
    assert_eq!(code(&capture), 1);
    assert!(
        stderr(&capture).contains(&format!(
            "member {member_id} has no pane yet (pending placement) — nothing to capture."
        )),
        "{}",
        stderr(&capture)
    );
    let calls = cli.shim_calls();
    assert!(
        calls[calls_before..]
            .iter()
            .all(|line| !line.starts_with("capture-pane")),
        "{calls:?}"
    );

    let calls_before = calls.len();
    let output = cli.run(&["member", "ping", &member_id.to_string()]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).contains(
        "Member worker has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn."
    ), "{}", stdout(&output));
    let calls = cli.shim_calls();
    assert!(
        calls[calls_before..]
            .iter()
            .all(|line| !line.starts_with("send-keys")),
        "{calls:?}"
    );

    let calls_before = calls.len();
    let output = cli.run(&["member", "ping", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_keys(&payload, &["member_id", "pane_id", "skipped", "reason"]);
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["pane_id"], serde_json::Value::Null);
    assert_eq!(payload["skipped"], true);
    assert_eq!(payload["reason"], "pending_placement");
    let calls = cli.shim_calls();
    assert!(
        calls[calls_before..]
            .iter()
            .all(|line| !line.starts_with("send-keys")),
        "{calls:?}"
    );

    let pane = format!("%{}", member_id + 6);
    cafleet::broker::update_placement_pane_id(&mut cli.sqlite(), member_id, &pane)
        .unwrap()
        .unwrap();
    cli.set_pane_capture(
        &pane,
        "Ready.\n────────────\n❯ \x1b[0m\n────────────\n? for shortcuts",
    );
    let calls_before = calls.len();
    let output = cli.run(&["member", "ping", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_keys(&payload, &["member_id", "pane_id", "skipped", "reason"]);
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["pane_id"], pane);
    assert_eq!(payload["skipped"], false);
    assert_eq!(payload["reason"], serde_json::Value::Null);
    let calls = cli.shim_calls();
    assert_eq!(
        calls[calls_before..]
            .iter()
            .filter(|line| line.as_str()
                == format!("send-keys -t {pane} -l cafleet message poll {member_id} --json"))
            .count(),
        1,
        "{calls:?}"
    );
}

#[test]
fn member_capture_text_emits_the_content_only() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let member_id = cli.seed_member(fleet_id, "worker");
    let pane = format!("%{}", member_id + 4);
    let output = cli.run(&["member", "capture", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        REST_CAPTURE.replace("{pane}", &pane),
        "the raw capture content, no envelope, no trailing newline"
    );

    let with_ansi = cli.run(&["member", "capture", &member_id.to_string(), "--ansi"]);
    assert_eq!(code(&with_ansi), 0, "--ansi is accepted");
    assert!(
        stdout(&with_ansi).contains("line1"),
        "got: {}",
        stdout(&with_ansi)
    );

    let calls_before = cli.shim_calls().len();
    let output = cli.run(&["member", "capture", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_keys(
        &payload,
        &[
            "member_id",
            "pane_id",
            "lines",
            "content",
            "captured_at",
            "content_sha256",
        ],
    );
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["pane_id"], pane);
    assert_eq!(payload["lines"], 20);
    assert_capture_content(&payload, &pane);
    let calls = cli.shim_calls();
    assert!(
        calls[calls_before..]
            .iter()
            .any(|line| line == &format!("capture-pane -p -e -t {pane} -S -1020")),
        "{calls:?}"
    );
}

#[test]
fn fleet_capture_prints_director_first_then_members_ascending() {
    let cli = Cli::new();
    let (fleet_id, director_id) = cli.seeded_fleet();
    let alpha_id = cli.seed_member(fleet_id, "alpha");
    let beta_id = cli.seed_member(fleet_id, "beta");
    let identities = [
        (director_id, "Director", "director", "%0".to_string()),
        (alpha_id, "alpha", "member", format!("%{}", alpha_id + 4)),
        (beta_id, "beta", "member", format!("%{}", beta_id + 4)),
    ];
    let calls_before = cli.shim_calls().len();
    let output = cli.run(&["fleet", "capture", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let out = stdout(&output);
    let sections = out
        .strip_suffix('\n')
        .unwrap()
        .split("\n\n")
        .collect::<Vec<_>>();
    assert_eq!(sections.len(), identities.len(), "{out}");
    for (section, (id, name, kind, pane)) in sections.iter().zip(&identities) {
        assert!(
            section.starts_with(&format!(
                "=== {id} ({name}; kind={kind}; coding_agent=claude; pane={pane}; captured_at="
            )),
            "{section}"
        );
        assert!(
            section.ends_with(&REST_CAPTURE.replace("{pane}", pane)),
            "{section}"
        );
    }
    let calls = cli.shim_calls();
    for (_, _, _, pane) in &identities {
        assert!(
            calls[calls_before..]
                .iter()
                .any(|line| line == &format!("capture-pane -p -e -t {pane} -S -1020")),
            "{calls:?}"
        );
    }

    let with_ansi = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--ansi"]);
    assert_eq!(code(&with_ansi), 0, "--ansi is accepted");

    let output = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let entries = payload.as_array().unwrap();
    assert_eq!(entries.len(), identities.len());
    let panes = entries
        .iter()
        .map(|entry| entry["pane_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(panes.len(), identities.len());
    for (entry, (id, name, kind, pane)) in entries.iter().zip(&identities) {
        assert_keys(
            entry,
            &[
                "member_id",
                "name",
                "kind",
                "coding_agent",
                "pane_id",
                "lines",
                "content",
                "captured_at",
                "content_sha256",
                "error",
            ],
        );
        assert_eq!(entry["member_id"], *id);
        assert_eq!(entry["name"], *name);
        assert_eq!(entry["kind"], *kind);
        assert_eq!(entry["coding_agent"], "claude");
        assert_eq!(entry["pane_id"], *pane);
        assert_eq!(entry["lines"], 20);
        assert_eq!(entry["error"], serde_json::Value::Null);
        assert_capture_content(entry, pane);
    }

    let calls_before = cli.shim_calls().len();
    let output = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--lines", "5"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let calls = cli.shim_calls();
    for (_, _, _, pane) in &identities {
        assert!(
            calls[calls_before..]
                .iter()
                .any(|line| line == &format!("capture-pane -p -e -t {pane} -S -1005")),
            "{calls:?}"
        );
    }
    let calls_before = calls.len();
    let zero = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--lines", "0"]);
    assert_eq!(code(&zero), 2, "{}", stderr(&zero));
    let calls = cli.shim_calls();
    assert!(
        calls[calls_before..]
            .iter()
            .all(|line| !line.starts_with("capture-pane")),
        "{calls:?}"
    );
}

#[test]
fn fleet_capture_annotates_a_pending_placement_and_exits_zero() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let member_id = cli.seed_member(fleet_id, "worker");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [member_id],
        )
        .unwrap();

    let rowless = cli.seed_member(fleet_id, "rowless");
    assert_eq!(
        cli.sqlite()
            .execute(
                "DELETE FROM member_placements WHERE member_id=?1",
                [rowless]
            )
            .unwrap(),
        1
    );
    let output = cli.run(&["fleet", "capture", &fleet_id.to_string()]);
    assert_eq!(
        code(&output),
        0,
        "an annotated entry never aborts the scan: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains(&format!(
            "=== {member_id} (worker; kind=member; coding_agent=claude; pane=—) ===\n\
             pane not available (pending placement)"
        )),
        "got: {}",
        stdout(&output)
    );

    assert!(!stdout(&output).contains(&format!("=== {rowless} ")));
    let output = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let entries = payload.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry["member_id"] != rowless));
    let entry = payload
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["member_id"] == member_id)
        .expect("the pending-placement member stays in the roster");
    assert_keys(
        entry,
        &[
            "member_id",
            "name",
            "kind",
            "coding_agent",
            "pane_id",
            "lines",
            "content",
            "captured_at",
            "content_sha256",
            "error",
        ],
    );
    assert_eq!(entry["pane_id"], serde_json::Value::Null);
    assert_eq!(entry["lines"], 20, "lines echoes the requested depth");
    assert_eq!(entry["content"], serde_json::Value::Null);
    assert_eq!(entry["captured_at"], serde_json::Value::Null);
    assert_eq!(entry["content_sha256"], serde_json::Value::Null);
    assert_eq!(entry["error"], "pane not available (pending placement)");
}

#[test]
fn fleet_capture_annotates_failed_captures_and_exits_zero() {
    let mut cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    cli.seed_member(fleet_id, "worker");
    cli.fail_subcommand = Some("capture-pane".to_string());

    let output = cli.run(&["fleet", "capture", &fleet_id.to_string()]);
    assert_eq!(
        code(&output),
        0,
        "a scan whose every entry is annotated still exits 0: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains(
            "=== 1 (Director; kind=director; coding_agent=claude; pane=%0) ===\ncapture failed: "
        ),
        "a failed capture keeps its real pane id, got: {out}"
    );
    assert!(
        out.contains("forced failure"),
        "the backend error rides the annotation, got: {out}"
    );

    let output = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let entries = payload.as_array().unwrap();
    assert_eq!(entries[0]["pane_id"], "%0", "the real pane id is kept");
    for entry in entries {
        assert_eq!(entry["content"], serde_json::Value::Null);
        assert_eq!(entry["captured_at"], serde_json::Value::Null);
        assert_eq!(entry["content_sha256"], serde_json::Value::Null);
        assert!(
            entry["error"]
                .as_str()
                .unwrap()
                .starts_with("capture failed: "),
            "got: {}",
            entry["error"]
        );
    }
}

#[test]
fn fleet_capture_of_a_memberless_fleet_captures_the_director_only() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();

    let output = cli.run(&["fleet", "capture", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.starts_with("=== 1 (Director; kind=director;"),
        "got: {out}"
    );
    assert_eq!(
        out.matches("=== 1 (").count(),
        1,
        "exactly one section, got: {out}"
    );
    assert!(
        !out[3..].contains("\n=== "),
        "no member sections follow, got: {out}"
    );
}
