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
    assert_eq!(stdout(&output), "3 worker backend=claude pane=%7\n");

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
        split_line.contains(
            "claude --permission-mode dontAsk --name worker FLEET 1 ME 3 DIRECTOR 1 AGENT claude"
        ),
        "the rendered prompt carries literal identity, got: {split_line}"
    );

    let pane: Option<String> = cli
        .sqlite()
        .query_row(
            "SELECT mux_pane_id FROM member_placements WHERE member_id=3",
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
        .rfind(|line| line.starts_with("split-window"))
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
        members, 2,
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
    assert!(err.contains("Rolled back registration of 3."), "got: {err}");

    let members: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM members WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(members, 2, "no orphan row survives the ladder");
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
    assert_eq!(members, 2, "validation precedes registration");
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
        members, 2,
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
fn member_create_role_monitor_registers_the_monitor_kind() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_bare_fleet();
    let monitor_id = cli.create_monitor(fleet_id);

    let output = cli.run(&["member", "show", &monitor_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["kind"], "monitor");
}

#[test]
fn member_create_without_a_monitor_hits_the_monitor_first_guard() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_bare_fleet();
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
    assert_ne!(
        code(&output),
        0,
        "the monitor-first guard rejects the spawn"
    );
    assert!(
        stderr(&output).contains(&format!(
            "fleet {fleet_id} has no active monitor member; spawn one with --role monitor first"
        )),
        "got: {}",
        stderr(&output)
    );
    let members: i64 = cli
        .sqlite()
        .query_row("SELECT COUNT(*) FROM members", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        members, 2,
        "Director + the deleted bootstrap monitor; the guard fires before any registration"
    );
    let splits = cli
        .shim_calls()
        .iter()
        .filter(|line| line.starts_with("split-window"))
        .count();
    assert_eq!(
        splits, 1,
        "only the bootstrap's monitor spawn split a pane; the guard fires before any pane effect"
    );
}

#[test]
fn member_create_role_monitor_twice_hits_the_one_per_fleet_guard() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_bare_fleet();
    let monitor_id = cli.create_monitor(fleet_id);

    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--role",
        "monitor",
        "--name",
        "monitor2",
        "--description",
        "d",
        "prompt",
    ]);
    assert_ne!(
        code(&output),
        0,
        "the one-per-fleet guard rejects the spawn"
    );
    assert!(
        stderr(&output).contains(&format!(
            "fleet {fleet_id} already has an active monitor member (member {monitor_id})"
        )),
        "got: {}",
        stderr(&output)
    );

    let deleted = cli.run(&["member", "delete", &monitor_id.to_string()]);
    assert_eq!(code(&deleted), 0, "stderr: {}", stderr(&deleted));
    cli.create_monitor(fleet_id);
}

#[test]
fn member_create_rejects_any_role_value_but_monitor() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_bare_fleet();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet_id.to_string(),
        "--name",
        "worker",
        "--description",
        "d",
        "--role",
        "builder",
        "prompt",
    ]);
    assert_eq!(code(&output), 2, "clap rejects the value at parse time");
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
    assert!(out.starts_with("3 members:\n"), "got: {out}");
    assert!(
        out.contains("  member_id  name           kind      backend   pane_id  idle"),
        "got: {out}"
    );
    assert!(out.contains("director"), "got: {out}");
    assert!(out.contains("monitor"), "got: {out}");
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

#[test]
fn monitor_loop_form_still_parses_the_positional_and_flags() {
    let cli = Cli::new();
    cli.ready();

    let output = cli.run(&["monitor", "999", "--tick", "1", "--interval", "5"]);
    assert_eq!(
        code(&output),
        1,
        "the loop form parses and reaches the live-fleet guard"
    );
    assert!(
        stderr(&output).contains("Error: fleet 999 not found"),
        "got: {}",
        stderr(&output)
    );

    let bare = cli.run(&["monitor"]);
    assert_eq!(code(&bare), 2, "a fleet id or a subcommand is required");

    let leaked = cli.run(&["monitor", "1", "--lines", "20"]);
    assert_eq!(
        code(&leaked),
        2,
        "scan flags do not parse on the loop form: {}",
        stderr(&leaked)
    );

    let no_fleet = cli.run(&["monitor", "scan"]);
    assert_eq!(
        code(&no_fleet),
        2,
        "the scan form carries its own FLEET_ID positional: {}",
        stderr(&no_fleet)
    );
}

#[test]
fn monitor_scan_prints_director_first_then_members_ascending() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let alpha_id = cli.create_member(fleet_id, "alpha");
    let beta_id = cli.create_member(fleet_id, "beta");

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.starts_with(
            "=== 1 (Director; kind=director; coding_agent=claude; pane=%0; captured_at="
        ),
        "the Director's section leads, got: {out}"
    );

    assert!(
        out.contains("=== 2 (monitor; kind=monitor; coding_agent=claude; pane=%7; captured_at="),
        "the monitor member's section rides the scan, got: {out}"
    );
    let alpha_header =
        format!("=== {alpha_id} (alpha; kind=member; coding_agent=claude; pane=%7; captured_at=");
    let beta_header =
        format!("=== {beta_id} (beta; kind=member; coding_agent=claude; pane=%7; captured_at=");
    assert!(out.contains(&alpha_header), "got: {out}");
    assert!(out.contains(&beta_header), "got: {out}");
    assert!(
        out.find(&alpha_header).unwrap() < out.find(&beta_header).unwrap(),
        "members ascend by member_id: {out}"
    );
    assert!(
        out.contains(&format!("line1\nline2\n\n{alpha_header}")),
        "one blank line separates sections, got: {out}"
    );

    let with_ansi = cli.run(&["monitor", "scan", &fleet_id.to_string(), "--ansi"]);
    assert_eq!(code(&with_ansi), 0, "--ansi is accepted");
}

#[test]
fn monitor_scan_json_pins_the_key_order() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let entries = payload.as_array().expect("a top-level array");
    assert_eq!(entries.len(), 3, "the Director, the monitor, one member");

    for entry in entries {
        let keys: Vec<&str> = entry
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
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
            "the scan key order is pinned"
        );
    }

    let director = &entries[0];
    assert_eq!(director["member_id"], 1);
    assert_eq!(director["name"], "Director");
    assert_eq!(director["kind"], "director");
    assert_eq!(director["coding_agent"], "claude");
    assert_eq!(director["pane_id"], "%0");
    assert_eq!(director["lines"], 20);
    assert_eq!(director["error"], serde_json::Value::Null);
    assert!(
        director["captured_at"].as_str().is_some(),
        "captured_at is stamped on a successful capture"
    );

    let content = director["content"].as_str().unwrap();
    assert!(
        content.contains("line1"),
        "the shim's canned buffer, got: {content}"
    );
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(content.as_bytes());
    let expected: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        director["content_sha256"], expected,
        "mode-exact hash of the emitted content"
    );

    let monitor = &entries[1];
    assert_eq!(monitor["member_id"], 2);
    assert_eq!(monitor["kind"], "monitor");

    let member = &entries[2];
    assert_eq!(member["member_id"], member_id);
    assert_eq!(member["kind"], "member");
    assert_eq!(member["pane_id"], "%7");
}

#[test]
fn monitor_scan_annotates_a_pending_placement_and_exits_zero() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [member_id],
        )
        .unwrap();

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string()]);
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

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string(), "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let entry = payload
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["member_id"] == member_id)
        .expect("the pending-placement member stays in the roster");
    assert_eq!(entry["pane_id"], serde_json::Value::Null);
    assert_eq!(entry["lines"], 20, "lines echoes the requested depth");
    assert_eq!(entry["content"], serde_json::Value::Null);
    assert_eq!(entry["captured_at"], serde_json::Value::Null);
    assert_eq!(entry["content_sha256"], serde_json::Value::Null);
    assert_eq!(entry["error"], "pane not available (pending placement)");
}

#[test]
fn monitor_scan_annotates_failed_captures_and_exits_zero() {
    let mut cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    cli.create_member(fleet_id, "worker");
    cli.fail_subcommand = Some("capture-pane".to_string());

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string()]);
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

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string(), "--json"]);
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
fn monitor_scan_excludes_a_member_without_a_placement_row() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let member_id = cli.create_member(fleet_id, "worker");
    cli.sqlite()
        .execute(
            "DELETE FROM member_placements WHERE member_id=?1",
            [member_id],
        )
        .unwrap();

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains(&format!("=== {member_id} ")),
        "a placementless member is excluded, got: {}",
        stdout(&output)
    );
}

#[test]
fn monitor_scan_of_a_memberless_fleet_captures_the_director_only() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_bare_fleet();

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string()]);
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

#[test]
fn monitor_scan_rejects_an_unknown_or_deleted_fleet() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();

    let output = cli.run(&["monitor", "scan", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: fleet 999 not found"),
        "got: {}",
        stderr(&output)
    );

    let deleted = cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    assert_eq!(code(&deleted), 0, "stderr: {}", stderr(&deleted));
    let output = cli.run(&["monitor", "scan", &fleet_id.to_string()]);
    assert_eq!(code(&output), 1, "a soft-deleted fleet is not scannable");
    assert!(
        stderr(&output).contains(&format!("Error: fleet {fleet_id} not found")),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn monitor_scan_honors_the_lines_flag() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();

    let output = cli.run(&["monitor", "scan", &fleet_id.to_string(), "--lines", "5"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.contains("capture-pane -p -t %0 -S -1005")),
        "the requested depth rides the A8 over-fetch: {:?}",
        cli.shim_calls()
    );

    let zero = cli.run(&["monitor", "scan", &fleet_id.to_string(), "--lines", "0"]);
    assert_eq!(code(&zero), 2, "lines must be >= 1: {}", stderr(&zero));
}
