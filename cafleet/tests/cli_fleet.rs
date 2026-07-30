//! Step 6 CLI contract tests: the `fleet` group (SPEC §6.3 *fleet group*).

mod common;

use common::{Cli, code, stderr, stdout};

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
    ]);
    assert_eq!(code(&output), 1, "no DB writes, exit 1");
    assert!(
        stderr(&output)
            .contains("Error: cafleet fleet create must be run inside a tmux or herdr session"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn fleet_create_reports_the_compact_line_and_backfills_the_director() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "1 director=1\n");
}

#[test]
fn fleet_create_full_and_json_forms() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
        "--full",
    ]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(
        out.starts_with("1\n1\nname:             alpha\ncreated_at:       "),
        "got: {out}"
    );
    assert!(out.contains("director_name:    Director"), "got: {out}");
    assert!(out.contains("pane:             main:@1:%0"), "got: {out}");

    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "beta",
        "--coding-agent",
        "claude",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["fleet_id"], 1);
    assert_eq!(payload["name"], "beta");
    assert_eq!(payload["director"]["member_id"], 1);
    assert_eq!(payload["director"]["placement"]["mux_pane_id"], "%0");
}

#[test]
fn fleet_create_missing_required_options_are_parse_errors() {
    let cli = Cli::new();
    cli.ready();
    assert_eq!(
        code(&cli.run(&["fleet", "create", "--coding-agent", "claude"])),
        2,
        "--name is required"
    );
    assert_eq!(
        code(&cli.run(&["fleet", "create", "--name", "x"])),
        2,
        "--coding-agent is required"
    );
    assert_eq!(
        code(&cli.run(&["fleet", "create", "--name", "x", "--coding-agent", "python"])),
        2,
        "--coding-agent is a choice"
    );
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
    ]);
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(out.contains("FLEET_ID"), "the header row, got: {out}");
    assert!(out.contains("alpha"), "got: {out}");

    let json_output = cli.run(&["fleet", "list", "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&json_output).trim()).unwrap();
    assert_eq!(payload[0]["fleet_id"], 1);
    assert_eq!(payload[0]["member_count"], 1);
}

#[test]
fn fleet_show_renders_the_row_and_hides_nothing_soft_deleted() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let output = cli.run(&["fleet", "show", "--fleet-id", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(out.contains("testfleet"), "got: {out}");

    cli.run(&["fleet", "delete", "--fleet-id", &fleet_id.to_string()]);
    let output = cli.run(&["fleet", "show", "--fleet-id", &fleet_id.to_string()]);
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
    let output = cli.run(&["fleet", "show", "--fleet-id", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: fleet '999' not found."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn fleet_delete_reports_the_count_and_is_idempotent() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();
    let output = cli.run(&["fleet", "delete", "--fleet-id", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("Deleted fleet 1. Deregistered 1 members."),
        "got: {}",
        stdout(&output)
    );

    let again = cli.run(&["fleet", "delete", "--fleet-id", &fleet_id.to_string()]);
    assert_eq!(code(&again), 0);
    assert!(
        stdout(&again).contains("Deleted fleet 1. Deregistered 0 members."),
        "got: {}",
        stdout(&again)
    );
}
