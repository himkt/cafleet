//! CLI contract tests: the `member` group (create sequencing + rollback,
//! delete, show/list, prompt, ping, capture) and the flattened `monitor`
//! command (SPEC §6.3 *member group*, *monitor*).

mod common;

use common::{Cli, code, stderr, stdout, write_file};

#[test]
fn member_create_spawns_patches_the_pane_and_substitutes_identity() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
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
        .find(|line| line.starts_with("split-window"))
        .expect("split-window was invoked");
    assert!(
        split_line.contains("-e CAFLEET_DATABASE_URL=sqlite:///"),
        "only CAFLEET_DATABASE_URL is forwarded, got: {split_line}"
    );
    assert!(
        split_line.contains(
            "claude --permission-mode dontAsk --name worker FLEET 1 ME 2 DIRECTOR 1 AGENT claude"
        ),
        "the rendered prompt carries literal identity, got: {split_line}"
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
    let (fleet_id, _) = cli.with_fleet();
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
        .find(|line| line.starts_with("split-window"))
        .expect("split-window was invoked");
    assert!(
        split_line.contains("FLEET 1 AGENT claude"),
        "the file body is rendered with identity, got: {split_line}"
    );
}

#[test]
fn member_create_requires_exactly_one_body_source() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let base = [
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "d",
    ];

    let output = cli.run(&base);
    assert_eq!(
        code(&output),
        2,
        "neither PROMPT nor --file is clap's native group error"
    );

    let mut both = base.to_vec();
    both.extend(["prompt", "--file", "f.md"]);
    let output = cli.run(&both);
    assert_eq!(code(&output), 2, "PROMPT and --file conflict at parse time");
}

#[test]
fn member_create_unknown_placeholder_exits_2_and_leaves_no_orphan() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "d",
        "hello {foo}",
    ]);
    assert_eq!(
        code(&output),
        2,
        "the original usage error is re-raised unwrapped"
    );
    assert!(
        stderr(&output).contains(
            "Unknown placeholder 'foo' in custom prompt. Supported placeholders: \
             {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. \
             Double literal braces ({{, }}) to keep them as text."
        ),
        "got: {}",
        stderr(&output)
    );

    let members: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM members WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        members, 1,
        "the substitution failure deregisters the member"
    );
}

#[test]
fn member_create_split_failure_rolls_back_the_registration() {
    let mut cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
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
    assert!(err.contains("Rolled back registration of 2."), "got: {err}");

    let members: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM members WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(members, 1, "no orphan row survives the ladder");
}

#[test]
fn member_create_validates_model_and_effort_before_any_side_effect() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
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
    let (fleet_id, _) = cli.with_fleet();
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
fn member_create_no_longer_parses_a_role_option() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        "1",
        "--name",
        "watch",
        "--description",
        "d",
        "--role",
        "monitor",
        "prompt",
    ]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("unexpected argument '--role'"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn member_show_takes_the_positional_subject() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");

    let output = cli.run(&["member", "show", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), format!("{member_id} worker active\n"));

    let output = cli.run(&["member", "show", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["kind"], "member");
    assert_eq!(
        payload["placement"]["mux_pane_id"], "%7",
        "the detailed view is JSON-only"
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
    let (fleet_id, _) = cli.with_fleet();
    cli.create_member(fleet_id, "worker");
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
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    let output = cli.run(&["member", "delete", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.starts_with("Member deleted.\n"), "got: {out}");
    assert!(out.contains("%7 (killed)"), "got: {out}");
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.starts_with("kill-pane -t %7")),
        "the pane is killed immediately"
    );
}

#[test]
fn member_delete_of_the_root_director_is_rejected() {
    let cli = Cli::new();
    let (_, director_id) = cli.with_fleet();
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
fn member_prompt_dispatches_and_validates_the_text() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");

    let output = cli.run(&["member", "prompt", &member_id.to_string(), "hello worker"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("Sent prompt"), "got: {out}");
    assert!(out.contains("worker (%7)."), "got: {out}");
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.contains("-l hello worker")),
        "the plain form types the text literally"
    );

    let output = cli.run(&["member", "prompt", &member_id.to_string(), "a\nb"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("text may not contain newlines."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn member_prompt_shell_form_types_the_bang_line_without_the_esc_safeguard() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    let output = cli.run(&[
        "member",
        "prompt",
        &member_id.to_string(),
        "--shell",
        "ls -la",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("Sent shell prompt"), "got: {out}");
    assert!(out.contains("worker (%7)."), "got: {out}");
    let calls = cli.shim_calls();
    assert!(
        calls.iter().any(|line| line.contains("-l ! ls -la")),
        "the shell form types the bang line literally: {calls:?}"
    );
    assert!(
        !calls.iter().any(|line| line.contains("Escape")),
        "no Esc before the shell form — it honors the ! shortcut: {calls:?}"
    );
}

#[test]
fn member_capture_of_a_pending_placement_is_a_hard_error() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [member_id],
        )
        .unwrap();

    let output = cli.run(&["member", "capture", &member_id.to_string()]);
    assert_eq!(code(&output), 1, "capture rejects a pending placement");
    assert!(
        stderr(&output).contains(&format!(
            "member {member_id} has no pane yet (pending placement) — nothing to capture."
        )),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn member_ping_skips_a_pending_placement_and_exits_zero() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [member_id],
        )
        .unwrap();

    let output = cli.run(&["member", "ping", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "the skip path is a success");
    assert!(
        stdout(&output).contains(
            "Member worker has no pane yet (pending placement) — ping skipped; \
             it will poll its inbox on spawn."
        ),
        "got: {}",
        stdout(&output)
    );

    let output = cli.run(&["member", "ping", &member_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["pane_id"], serde_json::Value::Null);
    assert_eq!(payload["skipped"], true);
    assert!(
        !cli.shim_calls()
            .iter()
            .any(|line| line.contains("message poll")),
        "the skip sends no keystroke"
    );

    // The skip is a no-op, not a terminal state: once the placement binds a
    // pane, the same member is pinged normally.
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id='%9' WHERE member_id=?1",
            [member_id],
        )
        .unwrap();
    let output = cli.run(&["member", "ping", &member_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["pane_id"], "%9");
    assert_eq!(payload["skipped"], false);
    assert!(
        cli.shim_calls().iter().any(|line| line.contains(&format!(
            "send-keys -t %9 -l cafleet message poll {member_id} — then resume your work"
        ))),
        "the late-bound pane is pinged normally"
    );
}

#[test]
fn member_ping_dispatches_the_subject_only_poll_keystroke() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    let output = cli.run(&["member", "ping", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(
        payload["skipped"], false,
        "the skipped key is present on both paths"
    );
    assert!(
        cli.shim_calls().iter().any(|line| line.contains(&format!(
            "-l cafleet message poll {member_id} — then resume your work"
        ))),
        "the poll keystroke carries the subject-only form: {:?}",
        cli.shim_calls()
    );
}

#[test]
fn member_capture_text_emits_the_content_only() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    let output = cli.run(&["member", "capture", &member_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "line1\nline2",
        "the raw capture content, no envelope, no trailing newline"
    );

    let with_ansi = cli.run(&["member", "capture", &member_id.to_string(), "--ansi"]);
    assert_eq!(code(&with_ansi), 0, "--ansi is accepted");
    assert!(
        stdout(&with_ansi).contains("line1"),
        "got: {}",
        stdout(&with_ansi)
    );
}

#[test]
fn member_capture_json_carries_the_content_hash() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    let output = cli.run(&["member", "capture", &member_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();

    let keys: Vec<&str> = payload
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "member_id",
            "pane_id",
            "lines",
            "content",
            "captured_at",
            "content_sha256",
        ],
        "the capture key order is pinned"
    );
    assert_eq!(payload["member_id"], member_id);
    assert_eq!(payload["pane_id"], "%7");
    assert_eq!(payload["lines"], 20);
    let content = payload["content"].as_str().unwrap();
    assert!(
        content.contains("line1"),
        "the shim's canned buffer, got: {content}"
    );

    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(content.as_bytes());
    let expected: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        payload["content_sha256"], expected,
        "mode-exact hash of the emitted content"
    );
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.contains("capture-pane -p -t %7 -S -1020")),
        "the default --lines is 20, over-fetched by the A8 margin"
    );
}

#[test]
fn monitor_no_longer_parses_a_capture_subcommand() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    let output = cli.run(&[
        "monitor",
        "capture",
        &fleet_id.to_string(),
        &member_id.to_string(),
    ]);
    assert_eq!(
        code(&output),
        2,
        "monitor takes only the positional FLEET_ID: {}",
        stderr(&output)
    );
}
