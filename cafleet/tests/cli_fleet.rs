//! CLI contract tests: the `fleet` group (SPEC §6.3 *fleet group*) —
//! the atomic fleet + Director + monitor bootstrap, positional `FLEET_ID`
//! subjects on `show` / `delete`, the shared `--json`.

mod common;

use common::{Cli, code, stderr, stdout, write_file};

fn table_count(cli: &Cli, table: &str) -> i64 {
    cli.sqlite()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn assert_no_rows_persisted(cli: &Cli) {
    for table in ["fleets", "members", "member_placements"] {
        assert_eq!(
            table_count(cli, table),
            0,
            "{table} must hold zero rows after the rollback"
        );
    }
}

#[test]
fn fleet_create_outside_any_multiplexer_is_the_hardcoded_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run_outside_tmux(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &cli.monitor_prompt_path(),
    ]);
    assert_eq!(code(&output), 1, "no DB writes, exit 1");
    assert!(
        stderr(&output)
            .contains("Error: cafleet fleet create must be run inside a tmux or herdr session"),
        "got: {}",
        stderr(&output)
    );
    assert_no_rows_persisted(&cli);
}

#[test]
fn fleet_create_reports_the_compact_line_with_director_and_monitor() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &cli.monitor_prompt_path(),
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "1 director=1 monitor=2\n");
}

#[test]
fn fleet_create_json_is_the_only_detailed_form() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "beta",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &cli.monitor_prompt_path(),
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["fleet_id"], 1);
    assert_eq!(payload["name"], "beta");
    assert_eq!(payload["director"]["member_id"], 1);
    assert_eq!(payload["director"]["placement"]["mux_pane_id"], "%0");
    assert_eq!(payload["monitor"]["member_id"], 2);
    assert_eq!(payload["monitor"]["name"], "monitor");
    assert_eq!(
        payload["monitor"]["description"],
        "Monitor member for this fleet"
    );
    assert_eq!(
        payload["monitor"]["placement"]["mux_pane_id"], "%7",
        "the monitor placement carries the pane id split-window returned"
    );
    assert_eq!(payload["monitor"]["placement"]["coding_agent"], "claude");
}

#[test]
fn fleet_create_spawns_the_monitor_pane_with_identity_and_model() {
    let cli = Cli::new();
    cli.ready();
    let prompt_file = write_file(
        &cli.home.path().join("monitor-identity.md"),
        b"FLEET {fleet_id} ME {member_id} DIRECTOR {director_member_id} AGENT {coding_agent}",
    );
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &prompt_file,
        "--monitor-model",
        "haiku",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let split_line = cli
        .shim_calls()
        .into_iter()
        .rfind(|line| line.starts_with("split-window"))
        .expect("split-window was invoked for the monitor pane");
    assert!(
        split_line.contains("-e CAFLEET_DATABASE_URL=sqlite:///"),
        "only CAFLEET_DATABASE_URL is forwarded, got: {split_line}"
    );
    assert!(
        split_line.contains("claude --permission-mode dontAsk --name monitor"),
        "the monitor spawns under its hardcoded display name, got: {split_line}"
    );
    assert!(
        split_line.contains("--model haiku"),
        "--monitor-model reaches the backend argv, got: {split_line}"
    );
    assert!(
        split_line.contains("FLEET 1 ME 2 DIRECTOR 1 AGENT claude"),
        "the rendered prompt carries the monitor's own literal identity, got: {split_line}"
    );
}

#[test]
fn fleet_create_reads_the_monitor_prompt_from_stdin() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run_with_stdin(
        &[
            "fleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "claude",
            "--monitor-file",
            "-",
        ],
        "follow your monitor role protocol",
    );
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "1 director=1 monitor=2\n");
}

#[test]
fn fleet_create_missing_required_options_are_parse_errors() {
    let cli = Cli::new();
    cli.ready();
    assert_eq!(
        code(&cli.run(&[
            "fleet",
            "create",
            "--coding-agent",
            "claude",
            "--monitor-file",
            "prompt.md",
        ])),
        2,
        "--name is required"
    );
    assert_eq!(
        code(&cli.run(&[
            "fleet",
            "create",
            "--name",
            "x",
            "--monitor-file",
            "prompt.md"
        ])),
        2,
        "--coding-agent is required"
    );
    assert_eq!(
        code(&cli.run(&[
            "fleet",
            "create",
            "--name",
            "x",
            "--coding-agent",
            "python",
            "--monitor-file",
            "prompt.md",
        ])),
        2,
        "--coding-agent is a choice"
    );

    let output = cli.run(&["fleet", "create", "--name", "x", "--coding-agent", "claude"]);
    assert_eq!(code(&output), 2, "--monitor-file is required");
    assert!(
        stderr(&output).contains("--monitor-file"),
        "clap names the missing flag, got: {}",
        stderr(&output)
    );
}

#[test]
fn fleet_create_monitor_file_errors_name_the_flag() {
    let cli = Cli::new();
    cli.ready();

    let missing = cli.home.path().join("no-such-prompt.md");
    let missing_path = missing.to_str().unwrap();
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        missing_path,
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "--monitor-file {missing_path}: file does not exist or is not a regular file."
        )),
        "got: {}",
        stderr(&output)
    );

    let empty_path = write_file(&cli.home.path().join("empty-prompt.md"), b"   \n");
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &empty_path,
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("--monitor-file {empty_path}: file is empty.")),
        "got: {}",
        stderr(&output)
    );

    let output = cli.run_with_stdin(
        &[
            "fleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "claude",
            "--monitor-file",
            "-",
        ],
        "",
    );
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("--monitor-file -: stdin is empty."),
        "got: {}",
        stderr(&output)
    );
    assert_no_rows_persisted(&cli);
}

#[test]
fn fleet_create_substitution_failure_rolls_back_everything() {
    let cli = Cli::new();
    cli.ready();
    let prompt_file = write_file(&cli.home.path().join("bad-prompt.md"), b"hello {typo}");
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &prompt_file,
    ]);
    assert_eq!(code(&output), 2, "the substitution usage error is exit 2");
    assert!(
        stderr(&output).contains(
            "Unknown placeholder 'typo' in custom prompt. Supported placeholders: \
             {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. \
             Double literal braces ({{, }}) to keep them as text."
        ),
        "got: {}",
        stderr(&output)
    );
    assert_no_rows_persisted(&cli);
}

#[test]
fn fleet_create_split_failure_rolls_back_everything_and_is_retryable() {
    let mut cli = Cli::new();
    cli.ready();
    cli.fail_subcommand = Some("split-window".to_string());
    let prompt_path = cli.monitor_prompt_path();
    let args: [&str; 8] = [
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &prompt_path,
    ];
    let output = cli.run(&args);
    assert_eq!(code(&output), 1);
    let err = stderr(&output);
    assert!(
        err.contains("Error: tmux split-window failed:"),
        "got: {err}"
    );
    assert!(err.contains("Rolled back fleet creation."), "got: {err}");
    assert_no_rows_persisted(&cli);
    assert!(
        !cli.shim_calls()
            .iter()
            .any(|line| line.starts_with("send-keys") || line.starts_with("kill-pane")),
        "a pre-spawn failure has no pane to kill, got: {:?}",
        cli.shim_calls()
    );

    cli.fail_subcommand = None;
    let output = cli.run(&args);
    assert_eq!(
        code(&output),
        0,
        "the command retries as-is: {}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "1 director=1 monitor=2\n");
}

#[test]
fn fleet_list_reports_empty_then_the_created_fleet() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("No fleets found."),
        "got: {}",
        stdout(&output)
    );

    cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &cli.monitor_prompt_path(),
    ]);
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(out.contains("FLEET_ID"), "the header row, got: {out}");
    assert!(out.contains("alpha"), "got: {out}");

    let json_output = cli.run(&["fleet", "list", "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&json_output).trim()).unwrap();
    assert_eq!(payload[0]["fleet_id"], 1);
    assert_eq!(
        payload[0]["member_count"], 2,
        "the bootstrap registers the Director and the monitor"
    );
}

#[test]
fn fleet_show_takes_the_positional_subject_and_returns_soft_deleted_rows() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let output = cli.run(&["fleet", "show", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("testfleet"), "got: {out}");

    cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    let output = cli.run(&["fleet", "show", &fleet_id.to_string()]);
    assert_eq!(
        code(&output),
        0,
        "soft-deleted rows are returned intentionally"
    );
    assert!(
        stdout(&output).contains("deleted_at:"),
        "got: {}",
        stdout(&output)
    );
}

#[test]
fn fleet_show_missing_is_the_pinned_application_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "show", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: fleet '999' not found."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn fleet_show_subject_parse_errors_exit_2() {
    let cli = Cli::new();
    cli.ready();
    assert_eq!(
        code(&cli.run(&["fleet", "show"])),
        2,
        "the positional FLEET_ID is required"
    );
    assert_eq!(
        code(&cli.run(&["fleet", "show", "abc"])),
        2,
        "a non-integer subject is clap's invalid-value error"
    );
}

#[test]
fn fleet_delete_reports_the_count_and_is_idempotent() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let output = cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("Deleted fleet 1. Deregistered 2 members."),
        "got: {}",
        stdout(&output)
    );

    let again = cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    assert_eq!(code(&again), 0);
    assert!(
        stdout(&again).contains("Deleted fleet 1. Deregistered 0 members."),
        "got: {}",
        stdout(&again)
    );
}

#[test]
fn fleet_delete_json_reports_the_deregistered_count() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let output = cli.run(&["fleet", "delete", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["deregistered_count"], 2);
}
