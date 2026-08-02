//! Step 6 CLI contract tests: `setup` (refinery db half + offline embedded
//! assets half) and `doctor` (SPEC §6.3, §8).

mod common;

use common::{Cli, VERSION, code, stderr, stdout};

#[test]
fn schema_only_setup_migrates_and_reports_the_head() {
    let cli = Cli::new();
    let output = cli.run(&[
        "setup", "--skip", "claude", "--skip", "codex", "--skip", "opencode",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("applied migrations to head (2)."),
        "fresh DB reports the created-and-migrated line, got: {out}"
    );
    assert!(
        out.contains("assets half skipped (all agents skipped)"),
        "got: {out}"
    );

    let again = cli.run(&[
        "setup", "--skip", "claude", "--skip", "codex", "--skip", "opencode",
    ]);
    assert_eq!(code(&again), 0);
    assert!(
        stdout(&again).contains("Already at head (2); nothing to do."),
        "got: {}",
        stdout(&again)
    );
}

#[test]
fn setup_refuses_an_unversioned_database_with_existing_tables() {
    let cli = Cli::new();
    let conn = rusqlite::Connection::open(cli.db_path()).unwrap();
    conn.execute_batch("CREATE TABLE junk (x INTEGER);")
        .unwrap();
    drop(conn);

    let output = cli.run(&[
        "setup", "--skip", "claude", "--skip", "codex", "--skip", "opencode",
    ]);
    assert_eq!(code(&output), 1);
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        combined.contains(
            "DB has existing tables but no refinery_schema_history. \
             Refusing to migrate an unversioned database."
        ),
        "got: {combined}"
    );
    assert!(combined.contains("db half failed:"), "got: {combined}");
}

#[test]
fn full_setup_installs_skills_and_presets_offline_and_records_rows() {
    let cli = Cli::new();
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);

    for (agent, skills_dir) in [
        ("claude", ".claude/skills"),
        ("codex", ".codex/skills"),
        ("opencode", ".config/opencode/skills"),
    ] {
        assert!(
            out.contains(&format!(
                "{agent}: installed cafleet, cafleet-design-doc, cafleet-research (v{VERSION})"
            )),
            "per-target skills echo for {agent}, got: {out}"
        );
        for skill in ["cafleet", "cafleet-design-doc", "cafleet-research"] {
            let installed = cli
                .home
                .path()
                .join(skills_dir)
                .join(skill)
                .join("SKILL.md");
            assert!(installed.is_file(), "missing {}", installed.display());
        }
    }
    assert!(
        cli.home.path().join(".codex/rules/cafleet.rules").is_file(),
        "the codex preset lands at ~/.codex/rules/cafleet.rules"
    );
    assert!(
        cli.home
            .path()
            .join(".opencode/agents/cafleet.md")
            .is_file(),
        "the opencode preset lands at ~/.opencode/agents/cafleet.md"
    );

    let conn = cli.sqlite();
    let mut stmt = conn
        .prepare("SELECT coding_agent, cafleet_version FROM asset_installs ORDER BY coding_agent")
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            ("claude".to_string(), VERSION.to_string()),
            ("codex".to_string(), VERSION.to_string()),
            ("opencode".to_string(), VERSION.to_string()),
        ]
    );
}

#[test]
fn setup_rejects_positional_arguments_and_unknown_skip_values() {
    let cli = Cli::new();
    assert_eq!(code(&cli.run(&["setup", "extra"])), 2);
    assert_eq!(code(&cli.run(&["setup", "--skip", "python"])), 2);
}

#[test]
fn doctor_reports_the_multiplexer_and_the_assets_install_state() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("assets:"), "got: {out}");
    assert!(
        out.contains(&format!("cli_version: {VERSION}")),
        "got: {out}"
    );
    assert!(
        out.contains("ok"),
        "the current-version row reports ok: {out}"
    );

    let json_output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&json_output), 0);
    let payload: serde_json::Value = serde_json::from_str(stdout(&json_output).trim()).unwrap();
    assert_eq!(payload["multiplexer"]["backend"], "tmux");
    assert_eq!(payload["multiplexer"]["presence_var"], "TMUX");
    assert_eq!(payload["assets"]["cli_version"], VERSION);
    assert_eq!(payload["assets"]["installs"][0]["coding_agent"], "claude");
    assert_eq!(payload["assets"]["installs"][0]["current"], true);
}

#[test]
fn doctor_reports_an_empty_install_state_instead_of_blocking() {
    let cli = Cli::new();
    cli.migrate();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 0, "doctor is exempt from the guard");
    assert!(
        stdout(&output).contains("(no assets install recorded; run 'cafleet setup')"),
        "got: {}",
        stdout(&output)
    );
}
