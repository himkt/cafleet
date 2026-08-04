//! CLI contract tests: the global surface — `--version`, positional subject
//! ids, the removed flag set, and the stale-assets guard (SPEC §6.3, §7.2,
//! §10).

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
    assert_eq!(
        code(&output),
        2,
        "only --version lives before the subcommand"
    );
}

#[test]
fn the_guard_blocks_fleet_scoped_groups_when_no_install_is_recorded() {
    let cli = Cli::new();
    cli.migrate();
    for args in [
        ["fleet", "list"].as_slice(),
        ["member", "list", "1"].as_slice(),
        ["message", "poll", "1"].as_slice(),
        ["monitor", "1"].as_slice(),
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
fn fleet_id_is_a_flag_only_on_member_create() {
    let cli = Cli::new();
    cli.ready();
    for args in [
        ["setup", "--fleet-id", "1"].as_slice(),
        [
            "fleet",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "x",
            "--coding-agent",
            "claude",
        ]
        .as_slice(),
        ["fleet", "list", "--fleet-id", "1"].as_slice(),
        ["fleet", "show", "1", "--fleet-id", "1"].as_slice(),
        ["member", "list", "1", "--fleet-id", "1"].as_slice(),
        ["member", "show", "1", "--fleet-id", "1"].as_slice(),
        ["message", "poll", "1", "--fleet-id", "1"].as_slice(),
        ["message", "ack", "1", "--fleet-id", "1"].as_slice(),
        ["monitor", "1", "--fleet-id", "1"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 2, "--fleet-id is unknown here: {args:?}");
        assert!(
            stderr(&output).contains("--fleet-id"),
            "the offending flag is named: {args:?} → {}",
            stderr(&output)
        );
    }
}

#[test]
fn member_id_and_message_id_flags_are_rejected_everywhere() {
    let cli = Cli::new();
    cli.ready();
    for args in [
        ["member", "show", "--member-id", "1"].as_slice(),
        ["member", "ping", "--member-id", "1"].as_slice(),
        ["message", "poll", "--member-id", "1"].as_slice(),
        ["message", "ack", "--message-id", "1"].as_slice(),
        ["message", "show", "--message-id", "1"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(
            code(&output),
            2,
            "the subject rides as a positional, not a flag: {args:?}"
        );
    }
}

/// Success criterion: `--full`, `--quiet`, `--no-ansi`, `--text`, and
/// `--text-file` are rejected everywhere with clap's standard
/// unknown-argument error.
#[test]
fn removed_output_and_body_flags_are_rejected_everywhere() {
    let cli = Cli::new();
    cli.ready();
    let cases: &[(&[&str], &str)] = &[
        (
            &[
                "fleet",
                "create",
                "--name",
                "x",
                "--coding-agent",
                "claude",
                "--full",
            ],
            "--full",
        ),
        (&["fleet", "show", "1", "--full"], "--full"),
        (&["member", "show", "1", "--full"], "--full"),
        (&["member", "list", "1", "--full"], "--full"),
        (&["message", "show", "1", "--full"], "--full"),
        (&["message", "poll", "1", "--full"], "--full"),
        (
            &[
                "message",
                "broadcast",
                "--from-member-id",
                "1",
                "hi",
                "--full",
            ],
            "--full",
        ),
        (
            &[
                "message",
                "send",
                "--from-member-id",
                "1",
                "--to-member-id",
                "2",
                "hi",
                "--quiet",
            ],
            "--quiet",
        ),
        (&["message", "ack", "1", "--quiet"], "--quiet"),
        (&["member", "ping", "1", "--quiet"], "--quiet"),
        (&["member", "capture", "1", "--no-ansi"], "--no-ansi"),
        (
            &[
                "message",
                "send",
                "--from-member-id",
                "1",
                "--to-member-id",
                "2",
                "--text",
                "hi",
            ],
            "--text",
        ),
        (
            &[
                "message",
                "broadcast",
                "--from-member-id",
                "1",
                "--text-file",
                "f.txt",
            ],
            "--text-file",
        ),
        (
            &[
                "member",
                "create",
                "--fleet-id",
                "1",
                "--name",
                "w",
                "--description",
                "d",
                "--text",
                "prompt",
            ],
            "--text",
        ),
    ];
    for (args, flag) in cases {
        let output = cli.run(args);
        assert_eq!(code(&output), 2, "{flag} is deleted: {args:?}");
        assert!(
            stderr(&output).contains(flag),
            "the offending flag is named: {args:?} → {}",
            stderr(&output)
        );
    }
}
