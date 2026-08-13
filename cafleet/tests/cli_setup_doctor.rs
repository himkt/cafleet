//! Step 6 CLI contract tests: `setup` (refinery db half + offline embedded
//! assets half) and `doctor` (SPEC §6.3, §8).

mod common;

use common::{Cli, VERSION, code, stderr, stdout};

const GUIDANCE_LINE: &str = "no assets install recorded; \
     run 'cafleet setup --coding-agent <agent>' to install (agents: claude, codex, opencode)";

#[test]
fn plain_setup_migrates_to_head_and_prints_the_guidance_line_on_an_empty_table() {
    let cli = Cli::new();
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("applied migrations to head (6)."),
        "fresh DB reports the created-and-migrated line, got: {out}"
    );
    assert!(out.contains(GUIDANCE_LINE), "got: {out}");
    assert!(
        !cli.home.path().join(".claude/skills").exists(),
        "the empty-table no-flag form installs nothing"
    );
    assert!(cli.asset_rows().is_empty(), "no rows are recorded");

    let again = cli.run(&["setup"]);
    assert_eq!(code(&again), 0);
    assert!(
        stdout(&again).contains("Already at head (6); nothing to do."),
        "got: {}",
        stdout(&again)
    );
    assert!(stdout(&again).contains(GUIDANCE_LINE));
}

#[test]
fn setup_refuses_an_unversioned_database_with_existing_tables() {
    let cli = Cli::new();
    let conn = rusqlite::Connection::open(cli.db_path()).unwrap();
    conn.execute_batch("CREATE TABLE junk (x INTEGER);")
        .unwrap();
    drop(conn);

    let output = cli.run(&["setup"]);
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
fn selector_setup_installs_the_named_agents_at_resolved_paths_and_records_rows() {
    let cli = Cli::new();
    let output = cli.run(&[
        "setup",
        "--coding-agent",
        "claude",
        "--coding-agent",
        "codex",
        "--coding-agent",
        "opencode",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);

    for (agent, skills_dir) in [
        ("claude", ".claude/skills"),
        ("codex", ".codex/skills"),
        ("opencode", ".config/opencode/skills"),
    ] {
        let dir = cli.home.path().join(skills_dir);
        assert!(
            out.contains(&format!(
                "{agent}: installed cafleet, cafleet-design-doc, cafleet-research \
                 (v{VERSION}) -> {}",
                dir.display()
            )),
            "per-target skills echo for {agent}, got: {out}"
        );
        for skill in ["cafleet", "cafleet-design-doc", "cafleet-research"] {
            let installed = dir.join(skill).join("SKILL.md");
            assert!(installed.is_file(), "missing {}", installed.display());
        }
    }
    let codex_preset = cli.home.path().join(".codex/rules/cafleet.rules");
    let opencode_preset = cli.home.path().join(".opencode/agents/cafleet.md");
    assert!(codex_preset.is_file());
    assert!(opencode_preset.is_file());
    assert!(
        out.contains(&format!(
            "codex: installed preset (v{VERSION}) -> {}",
            codex_preset.display()
        )),
        "got: {out}"
    );
    assert!(
        out.contains(&format!(
            "opencode: installed preset (v{VERSION}) -> {}",
            opencode_preset.display()
        )),
        "got: {out}"
    );

    assert_eq!(
        cli.asset_rows(),
        vec![
            (
                "claude".to_string(),
                cli.identity_path("claude"),
                VERSION.to_string()
            ),
            (
                "codex".to_string(),
                cli.identity_path("codex"),
                VERSION.to_string()
            ),
            (
                "opencode".to_string(),
                cli.identity_path("opencode"),
                VERSION.to_string()
            ),
        ]
    );
}

#[test]
fn selector_setup_installs_only_the_named_agents() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "--coding-agent", "codex"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(!out.contains("claude: installed"), "got: {out}");
    assert!(!out.contains("opencode: installed"), "got: {out}");
    assert!(cli.home.path().join(".codex/skills/cafleet").is_dir());
    assert!(!cli.home.path().join(".claude/skills").exists());
    assert!(!cli.home.path().join(".config/opencode/skills").exists());
    assert!(!cli.home.path().join(".opencode").exists());
    assert_eq!(
        cli.asset_rows(),
        vec![(
            "codex".to_string(),
            cli.identity_path("codex"),
            VERSION.to_string()
        )]
    );
}

#[test]
fn selector_setup_deduplicates_and_installs_in_the_fixed_order() {
    let cli = Cli::new();
    let output = cli.run(&[
        "setup",
        "--coding-agent",
        "opencode",
        "--coding-agent",
        "claude",
        "--coding-agent",
        "claude",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert_eq!(
        out.matches("claude: installed cafleet").count(),
        1,
        "duplicates are deduplicated: {out}"
    );
    let claude_at = out.find("claude: installed cafleet").unwrap();
    let opencode_at = out.find("opencode: installed cafleet").unwrap();
    assert!(
        claude_at < opencode_at,
        "fixed order claude, codex, opencode regardless of flag order: {out}"
    );
    let agents: Vec<String> = cli
        .asset_rows()
        .into_iter()
        .map(|(agent, _, _)| agent)
        .collect();
    assert_eq!(agents, vec!["claude".to_string(), "opencode".to_string()]);
}

#[test]
fn selector_setup_installs_at_the_env_resolved_paths() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("codex-custom");
    cli.set_env("CODEX_HOME", custom.to_str().unwrap());
    let output = cli.run(&["setup", "--coding-agent", "codex"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        custom.join("skills/cafleet/SKILL.md").is_file(),
        "skills land under $CODEX_HOME"
    );
    assert!(
        custom.join("rules/cafleet.rules").is_file(),
        "the preset lands under $CODEX_HOME"
    );
    assert!(
        !cli.home.path().join(".codex").exists(),
        "nothing lands at the default path"
    );
    assert_eq!(
        cli.asset_rows(),
        vec![(
            "codex".to_string(),
            custom.to_str().unwrap().to_string(),
            VERSION.to_string()
        )]
    );
}

#[test]
fn opencode_skills_stay_at_the_fixed_discovery_path_when_the_variable_is_set() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("oc-custom");
    cli.set_env("OPENCODE_CONFIG_DIR", custom.to_str().unwrap());
    let output = cli.run(&["setup", "--coding-agent", "opencode"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        cli.home
            .path()
            .join(".config/opencode/skills/cafleet/SKILL.md")
            .is_file(),
        "skills stay at the fixed discovery path"
    );
    assert!(
        !custom.join("skills").exists(),
        "no skills land under $OPENCODE_CONFIG_DIR"
    );
    assert!(
        custom.join("agents/cafleet.md").is_file(),
        "the preset relocates to $OPENCODE_CONFIG_DIR"
    );
    assert!(
        !cli.home.path().join(".opencode").exists(),
        "nothing lands at the default preset base"
    );
    assert_eq!(
        cli.asset_rows(),
        vec![(
            "opencode".to_string(),
            custom.to_str().unwrap().to_string(),
            VERSION.to_string()
        )]
    );
}

#[test]
fn no_flag_setup_refreshes_only_agents_recorded_at_their_resolved_paths() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("claude", "0.1.0");
    cli.seed_asset_row_at("codex", "/codex-old", "0.1.0");
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains(&format!(
            "claude: installed cafleet, cafleet-design-doc, cafleet-research (v{VERSION})"
        )),
        "the recorded-at-resolved-path agent is refreshed: {out}"
    );
    assert!(
        out.contains(
            "codex: no install at ~/.codex (previously set up at /codex-old); \
             run 'cafleet setup --coding-agent codex'"
        ),
        "the records-elsewhere agent gets the hint line: {out}"
    );
    assert!(!out.contains("opencode:"), "no line for a no-rows agent: {out}");
    assert!(!cli.home.path().join(".codex").exists(), "codex installs nothing");
    assert_eq!(
        cli.asset_rows(),
        vec![
            (
                "claude".to_string(),
                cli.identity_path("claude"),
                VERSION.to_string()
            ),
            (
                "codex".to_string(),
                "/codex-old".to_string(),
                "0.1.0".to_string()
            ),
        ],
        "claude upserted to the CLI version; the superseded codex row untouched"
    );
}

#[test]
fn the_hint_names_the_most_recent_superseded_row() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row_dated("codex", "/a-old", "0.1.0", "2026-07-01T00:00:00.000000+00:00");
    cli.seed_asset_row_dated("codex", "/b-new", "0.1.0", "2026-08-01T00:00:00.000000+00:00");
    cli.seed_asset_row_dated("claude", "/t2", "0.1.0", "2026-07-01T00:00:00.000000+00:00");
    cli.seed_asset_row_dated("claude", "/t1", "0.1.0", "2026-07-01T00:00:00.000000+00:00");
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("codex: no install at ~/.codex (previously set up at /b-new)"),
        "the greatest installed_at wins: {out}"
    );
    assert!(
        out.contains("claude: no install at ~/.claude (previously set up at /t1)"),
        "an installed_at tie breaks by ascending path: {out}"
    );
}

#[test]
fn an_invalid_variable_fails_the_assets_half_with_the_pinned_error() {
    let mut cli = Cli::new();
    cli.set_env("CODEX_HOME", "relative/path");
    let output = cli.run(&["setup", "--coding-agent", "codex"]);
    assert_eq!(code(&output), 1);
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        combined.contains("assets half failed: CODEX_HOME must be an absolute path (got 'relative/path')"),
        "got: {combined}"
    );
    assert!(
        combined.contains("applied migrations to head (6)."),
        "the db half is unaffected: {combined}"
    );
}

#[test]
fn an_invalid_variable_of_an_untargeted_agent_does_not_fail_setup() {
    let mut cli = Cli::new();
    cli.set_env("CODEX_HOME", "relative/path");
    let output = cli.run(&["setup", "--coding-agent", "claude"]);
    assert_eq!(
        code(&output),
        0,
        "claude's install never reads CODEX_HOME: {}",
        stderr(&output)
    );
    assert!(cli.home.path().join(".claude/skills/cafleet").is_dir());
}

#[test]
fn no_flag_setup_fails_on_an_invalid_variable_of_a_recorded_agent() {
    let mut cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("codex", VERSION);
    cli.set_env("CODEX_HOME", "relative/path");
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 1);
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        combined.contains("assets half failed: CODEX_HOME must be an absolute path (got 'relative/path')"),
        "classifying a recorded agent resolves its path: {combined}"
    );
}

#[test]
fn setup_rejects_positionals_unknown_agents_and_the_removed_skip_flag() {
    let cli = Cli::new();
    assert_eq!(code(&cli.run(&["setup", "extra"])), 2);
    assert_eq!(code(&cli.run(&["setup", "--coding-agent", "python"])), 2);
    assert_eq!(code(&cli.run(&["setup", "--skip", "claude"])), 2);
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
