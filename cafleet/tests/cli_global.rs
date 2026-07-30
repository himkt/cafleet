//! Step 6 CLI contract tests: the global surface — `--version`, the required
//! `--fleet-id` callback, and the stale-assets guard (SPEC §6.3, §7.2, §10).

mod common;

use common::{Cli, VERSION, code, stderr, stdout};

#[test]
fn version_prints_the_pinned_line_and_bypasses_every_guard() {
    let cli = Cli::new();
    let output = cli.run_outside_tmux(&["--version"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), format!("cafleet {VERSION}\n"));
}

#[test]
fn an_unknown_pre_subcommand_option_is_a_parse_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["--json", "fleet", "list"]);
    assert_eq!(code(&output), 2, "only --version lives before the subcommand");
}

#[test]
fn missing_fleet_id_is_the_shared_callback_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "show"]);
    assert_eq!(code(&output), 1, "the callback error is application-class");
    assert!(
        stderr(&output).contains(
            "Error: --fleet-id <int> is required for this subcommand. \
             Create a fleet with 'cafleet fleet create' and pass its id."
        ),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn a_non_integer_fleet_id_is_a_parse_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "show", "--fleet-id", "abc"]);
    assert_eq!(code(&output), 2);
}

#[test]
fn the_guard_blocks_fleet_scoped_groups_when_no_install_is_recorded() {
    let cli = Cli::new();
    cli.migrate();
    for args in [
        ["fleet", "list"].as_slice(),
        ["member", "list", "--fleet-id", "1"].as_slice(),
        ["message", "poll", "--fleet-id", "1", "--member-id", "1"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 1, "guarded: {args:?}");
        assert!(
            stderr(&output)
                .contains("Error: no assets install is recorded; run 'cafleet setup' first"),
            "got: {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_guard_reports_stale_agents_in_ascending_order() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("codex", "0.1.0");
    cli.seed_asset_row("claude", "0.2.0");
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "Error: stale assets detected (claude=0.2.0, codex=0.1.0; CLI {VERSION}); \
             run 'cafleet setup' to reinstall"
        )),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn help_renders_before_the_guard_even_with_no_install() {
    let cli = Cli::new();
    for args in [
        ["fleet", "--help"].as_slice(),
        ["fleet", "create", "--help"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 0, "help always prints: {args:?}");
        assert!(!stdout(&output).is_empty());
    }
}

#[test]
fn unscoped_commands_reject_fleet_id() {
    let cli = Cli::new();
    cli.ready();
    for args in [
        ["setup", "--fleet-id", "1"].as_slice(),
        ["fleet", "create", "--fleet-id", "1", "--name", "x", "--coding-agent", "claude"]
            .as_slice(),
        ["fleet", "list", "--fleet-id", "1"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 2, "--fleet-id is unknown here: {args:?}");
    }
}
