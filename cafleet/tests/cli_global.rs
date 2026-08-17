//! CLI contract tests: the global surface — `--version`, positional subject
//! ids, the removed flag set, the schema-version guard, and the stale-assets
//! guard (SPEC §6.3, §7.2, §10).

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

const NO_INSTALL_ERROR: &str =
    "Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install";

const OUTDATED_ERROR: &str =
    "Error: database schema is outdated (schema 5, head 7); run 'cafleet setup'";

const NO_DATABASE_ERROR: &str = "Error: no cafleet database; run 'cafleet setup'";

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
            stderr(&output).contains(NO_INSTALL_ERROR),
            "got: {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_schema_guard_reports_a_missing_or_empty_database() {
    let cli = Cli::new();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(NO_DATABASE_ERROR),
        "a missing database file classifies as no-database: {}",
        stderr(&output)
    );

    assert!(
        cli.db_path().is_file(),
        "Connection::open created the empty file on the first run"
    );
    let again = cli.run(&["fleet", "list"]);
    assert_eq!(code(&again), 1);
    assert!(
        stderr(&again).contains(NO_DATABASE_ERROR),
        "an empty database file classifies the same way: {}",
        stderr(&again)
    );
}

#[test]
fn the_schema_guard_reports_an_outdated_database() {
    let cli = Cli::new();
    cli.seed_pre_v6_database();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(OUTDATED_ERROR),
        "a behind-head ledger yields the outdated error before any other guard: {}",
        stderr(&output)
    );
}

#[test]
fn the_schema_guard_reports_a_foreign_database() {
    let cli = Cli::new();
    cli.sqlite()
        .execute_batch("CREATE TABLE junk (x INTEGER);")
        .unwrap();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output)
            .contains("Error: database has tables but no schema history — not a cafleet database?"),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn the_schema_guard_reports_a_newer_database() {
    let cli = Cli::new();
    cli.migrate();
    cli.sqlite()
        .execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
             VALUES (8, 'future', '2026-01-01T00:00:00Z', '0')",
            [],
        )
        .unwrap();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(
            "Error: database schema 8 is newer than this cafleet (head 7); upgrade cafleet"
        ),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn an_at_head_database_passes_the_schema_guard() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(
        code(&output),
        0,
        "at head the command proceeds: {}",
        stderr(&output)
    );
}

#[test]
fn the_schema_guard_blocks_every_guarded_group() {
    let cli = Cli::new();
    cli.seed_pre_v6_database();
    for args in [
        ["fleet", "list"].as_slice(),
        ["member", "list", "1"].as_slice(),
        ["message", "poll", "1"].as_slice(),
        ["monitor", "1"].as_slice(),
        ["monitor", "scan", "1"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 1, "guarded: {args:?}");
        assert!(
            stderr(&output).contains(OUTDATED_ERROR),
            "{args:?} → {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_schema_guard_blocks_server_startup() {
    use std::io::Read;
    use std::time::{Duration, Instant};

    let cli = Cli::new();
    cli.seed_pre_v6_database();
    let mut child = cli.spawn(&["server", "--port", "0"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("server kept running against a behind-head database");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = child.try_wait().unwrap().unwrap();
    assert_eq!(status.code(), Some(1));
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(err.contains(OUTDATED_ERROR), "got: {err}");
}

#[test]
fn a_fleet_scoped_command_against_a_pre_v6_database_names_setup_not_sqlite() {
    let cli = Cli::new();
    cli.seed_pre_v6_database();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    let err = stderr(&output);
    assert!(
        !err.contains("no such column: path"),
        "the raw SQLite error never surfaces: {err}"
    );
    assert!(err.contains("run 'cafleet setup'"), "got: {err}");
}

#[test]
fn the_guard_ignores_superseded_rows_at_other_paths() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("claude", VERSION);
    cli.seed_asset_row_at("claude", "/elsewhere/.claude", "0.1.0");
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(
        code(&output),
        0,
        "a stale row at a superseded path never blocks: {}",
        stderr(&output)
    );
}

#[test]
fn the_guard_skips_agents_with_no_row_at_their_resolved_path() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("claude", VERSION);
    cli.seed_asset_row_at("codex", "/codex-old", "0.1.0");
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(
        code(&output),
        0,
        "codex has no row at its resolved path, so it is unchecked: {}",
        stderr(&output)
    );
}

#[test]
fn a_config_location_variable_rekeys_the_guard_to_the_resolved_path() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("custom-claude");
    cli.set_env("CLAUDE_CONFIG_DIR", custom.to_str().unwrap());
    cli.migrate();
    cli.seed_asset_row("claude", VERSION);
    cli.seed_asset_row_at("claude", custom.to_str().unwrap(), "0.1.0");
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1, "the row at $CLAUDE_CONFIG_DIR is current");
    assert!(
        stderr(&output).contains(&format!(
            "Error: stale assets detected (claude=0.1.0; CLI {VERSION}); \
             run 'cafleet setup' to reinstall"
        )),
        "the default-path row is superseded, the env-path row is checked: {}",
        stderr(&output)
    );
}

#[test]
fn a_config_location_variable_supersedes_the_default_path_row() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("custom-claude");
    cli.set_env("CLAUDE_CONFIG_DIR", custom.to_str().unwrap());
    cli.migrate();
    cli.seed_asset_row("claude", VERSION);
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(NO_INSTALL_ERROR),
        "the default-path row no longer counts once the variable relocates the path: {}",
        stderr(&output)
    );
}

#[test]
fn an_invalid_config_location_variable_fails_the_guard_with_the_pinned_error() {
    let mut cli = Cli::new();
    cli.migrate();
    cli.set_env("CLAUDE_CONFIG_DIR", "not/absolute");
    cli.seed_asset_row("codex", VERSION);
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output)
            .contains("Error: CLAUDE_CONFIG_DIR must be an absolute path (got 'not/absolute')"),
        "got: {}",
        stderr(&output)
    );
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
