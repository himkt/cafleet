//! Step 6 CLI contract tests: `setup` (refinery db half + offline embedded
//! assets half) and `doctor` (SPEC §6.3, §8).

mod common;

use common::{Cli, VERSION, code, stderr, stdout};

#[test]
fn plain_setup_installs_and_records_all_three_agents_on_a_fresh_database() {
    let cli = Cli::new();
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("applied migrations to head (7)."),
        "fresh DB reports the created-and-migrated line, got: {out}"
    );
    for (agent, skills_dir) in [
        ("claude", ".claude/skills"),
        ("codex", ".codex/skills"),
        ("opencode", ".config/opencode/skills"),
    ] {
        assert!(
            cli.home
                .path()
                .join(skills_dir)
                .join("cafleet/SKILL.md")
                .is_file(),
            "{agent} skills installed under {skills_dir}: {out}"
        );
        assert!(
            out.contains(&format!("{agent}: installed cafleet")),
            "per-target skills echo for {agent}, got: {out}"
        );
    }
    let claude_at = out.find("claude: installed cafleet").unwrap();
    let codex_at = out.find("codex: installed cafleet").unwrap();
    let opencode_at = out.find("opencode: installed cafleet").unwrap();
    assert!(
        claude_at < codex_at && codex_at < opencode_at,
        "fixed install order claude, codex, opencode: {out}"
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

    let again = cli.run(&["setup"]);
    assert_eq!(code(&again), 0);
    assert!(
        stdout(&again).contains("Already at head (7); nothing to do."),
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
fn plain_setup_installs_at_the_resolved_path_despite_rows_recorded_elsewhere() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row_at("codex", "/codex-old", "0.1.0");
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains(&format!(
            "codex: installed cafleet, cafleet-design-doc, cafleet-research (v{VERSION})"
        )),
        "the records-elsewhere agent installs at the resolved path anyway: {out}"
    );
    assert!(
        cli.home.path().join(".codex/skills/cafleet").is_dir(),
        "codex assets land at the resolved path"
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
                "/codex-old".to_string(),
                "0.1.0".to_string()
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
        ],
        "a fresh row lands at the resolved path; the superseded row is untouched"
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
        combined.contains(
            "assets half failed: CODEX_HOME must be an absolute path (got 'relative/path')"
        ),
        "got: {combined}"
    );
    assert!(
        combined.contains("applied migrations to head (7)."),
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
fn plain_setup_fails_the_assets_half_on_an_invalid_config_path_variable() {
    let mut cli = Cli::new();
    cli.set_env("CODEX_HOME", "relative/path");
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 1);
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        combined.contains(
            "assets half failed: CODEX_HOME must be an absolute path (got 'relative/path')"
        ),
        "plain setup resolves all three identity paths: {combined}"
    );
    assert!(
        combined.contains("applied migrations to head (7)."),
        "the db half is unaffected: {combined}"
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
fn setup_accepts_space_delimited_coding_agent_values() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "--coding-agent", "claude", "codex"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(cli.home.path().join(".claude/skills/cafleet").is_dir());
    assert!(cli.home.path().join(".codex/skills/cafleet").is_dir());
    assert!(
        !cli.home.path().join(".config/opencode/skills").exists(),
        "only the named agents install"
    );
    let agents: Vec<String> = cli
        .asset_rows()
        .into_iter()
        .map(|(agent, _, _)| agent)
        .collect();
    assert_eq!(agents, vec!["claude".to_string(), "codex".to_string()]);
}

#[test]
fn space_delimited_and_repeated_flag_forms_are_equivalent() {
    let selections = |cli: &Cli| -> Vec<(String, String)> {
        cli.asset_rows()
            .into_iter()
            .map(|(agent, _, version)| (agent, version))
            .collect()
    };

    let space = Cli::new();
    let output = space.run(&["setup", "--coding-agent", "claude", "codex"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let repeated = Cli::new();
    let output = repeated.run(&[
        "setup",
        "--coding-agent",
        "claude",
        "--coding-agent",
        "codex",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    assert_eq!(selections(&space), selections(&repeated));
}

#[test]
fn setup_accepts_the_mixed_flag_form() {
    let cli = Cli::new();
    let output = cli.run(&[
        "setup",
        "--coding-agent",
        "claude",
        "codex",
        "--coding-agent",
        "opencode",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let agents: Vec<String> = cli
        .asset_rows()
        .into_iter()
        .map(|(agent, _, _)| agent)
        .collect();
    assert_eq!(
        agents,
        vec![
            "claude".to_string(),
            "codex".to_string(),
            "opencode".to_string()
        ]
    );
}

#[test]
fn setup_rejects_an_unknown_value_in_a_space_delimited_list() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "--coding-agent", "claude", "python"]);
    assert_eq!(code(&output), 2);
    let err = stderr(&output);
    assert!(
        err.contains("invalid value 'python'"),
        "clap's native invalid-value error: {err}"
    );
}

#[test]
fn bare_setup_word_is_rejected_with_the_unexpected_argument_error() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "claude"]);
    assert_eq!(code(&output), 2);
    let err = stderr(&output);
    assert!(
        err.contains("unexpected argument 'claude'"),
        "an agent name without the flag is not a selection: {err}"
    );
}

#[test]
fn a_word_following_the_flag_is_rejected_with_the_invalid_value_error() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "--coding-agent", "claude", "extra"]);
    assert_eq!(code(&output), 2);
    let err = stderr(&output);
    assert!(
        err.contains("invalid value 'extra'"),
        "the word is consumed as another flag value: {err}"
    );
}

#[test]
fn setup_help_documents_the_multi_value_flag() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "--help"]);
    assert_eq!(code(&output), 0);
    let help: String = stdout(&output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        help.contains(
            "Install the named agent's assets (space-delimited, repeatable; default: all agents)"
        ),
        "got: {help}"
    );
}

fn seed_all_current(cli: &Cli) {
    for agent in ["claude", "codex", "opencode"] {
        cli.seed_asset_row(agent, VERSION);
    }
}

#[test]
fn doctor_reports_a_healthy_environment_with_no_issues() {
    let cli = Cli::new();
    cli.migrate();
    seed_all_current(&cli);
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.starts_with(&format!("cafleet {VERSION}\n")),
        "the version line leads the report: {out}"
    );
    assert!(out.contains("✓ multiplexer"), "got: {out}");
    for detail in [
        "tmux",
        "main",
        "@1",
        "%0",
        "TMUX=/tmp/tmux-1000/default,123,0",
    ] {
        assert!(out.contains(detail), "multiplexer detail {detail}: {out}");
    }
    assert!(out.contains("✓ database"), "got: {out}");
    assert!(out.contains("schema 7 (head)"), "got: {out}");
    assert!(out.contains("✓ coding agents"), "got: {out}");
    assert_eq!(
        out.matches(&format!("✓ {VERSION}")).count(),
        3,
        "three ok setup cells: {out}"
    );
    assert!(out.contains("no issues found"), "got: {out}");
}

#[test]
fn doctor_renders_every_section_on_a_multiplexer_failure() {
    let cli = Cli::new();
    cli.migrate();
    seed_all_current(&cli);
    let output = cli.run_outside_tmux(&["doctor"]);
    assert_eq!(code(&output), 1, "a rendered issue exits non-zero");
    let out = stdout(&output);
    assert!(out.contains("✗ multiplexer"), "got: {out}");
    assert!(
        out.contains("✓ database"),
        "no early abort — the database section renders: {out}"
    );
    assert!(
        out.contains("✓ coding agents"),
        "no early abort — the coding-agents table renders: {out}"
    );
    assert!(out.contains("1 issue found"), "singular footer: {out}");
}

#[test]
fn doctor_reports_a_missing_database() {
    let cli = Cli::new();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    assert!(out.contains("✗ database"), "got: {out}");
    assert!(
        out.contains("no database — run: cafleet setup"),
        "got: {out}"
    );
    assert_eq!(
        out.matches("– cafleet setup --coding-agent").count(),
        3,
        "every agent renders the not-installed state: {out}"
    );
    assert!(
        out.contains("1 issue found"),
        "the – state never counts: {out}"
    );
}

#[test]
fn doctor_reports_a_behind_head_schema() {
    let cli = Cli::new();
    cli.migrate();
    cli.sqlite()
        .execute("DELETE FROM refinery_schema_history WHERE version = 7", [])
        .unwrap();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    assert!(
        out.contains("schema 6, head is 7 — run: cafleet setup"),
        "got: {out}"
    );
    assert!(
        out.contains("coding agents"),
        "the coding-agents section still renders: {out}"
    );
}

#[test]
fn doctor_completes_the_report_against_a_pre_v6_database() {
    let cli = Cli::new();
    cli.seed_pre_v6_database();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1, "the database issue exits 1");
    let out = stdout(&output);
    assert!(out.contains("✓ multiplexer"), "got: {out}");
    assert!(out.contains("✗ database"), "got: {out}");
    assert!(
        out.contains("schema 5, head is 7 — run: cafleet setup"),
        "got: {out}"
    );
    assert_eq!(
        out.matches("– cafleet setup --coding-agent").count(),
        3,
        "every resolvable agent renders the not-installed state: {out}"
    );
    assert!(
        out.contains("1 issue found"),
        "only the database issue counts: {out}"
    );
    let combined = format!("{out}{}", stderr(&output));
    assert!(
        !combined.contains("no such column"),
        "no raw SQLite error aborts the report: {combined}"
    );
}

#[test]
fn doctor_json_against_a_pre_v6_database_keeps_the_shape() {
    let cli = Cli::new();
    cli.seed_pre_v6_database();
    let output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&output), 1);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();

    assert_eq!(payload["database"]["ok"], false);
    assert_eq!(payload["database"]["schema_version"], 5);
    assert_eq!(payload["database"]["head_version"], 7);

    let agents = payload["coding_agents"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 3);
    for agent in agents {
        assert_eq!(agent["state"], "not_installed");
        assert_eq!(agent["recorded_version"], serde_json::Value::Null);
        assert_eq!(agent["installed_at"], serde_json::Value::Null);
    }
    assert_eq!(
        payload["coding_agents"]["superseded"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "no recorded rows are read on a behind-head schema"
    );
    assert_eq!(payload["issues"], 1, "only the database issue counts");
}

#[test]
fn doctor_renders_not_installed_when_the_table_is_dropped_at_head() {
    let cli = Cli::new();
    cli.migrate();
    cli.sqlite()
        .execute_batch("DROP TABLE asset_installs;")
        .unwrap();
    let output = cli.run(&["doctor"]);
    assert_eq!(
        code(&output),
        0,
        "the missing table carries no issue: {}",
        stdout(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("✓ database"), "got: {out}");
    assert_eq!(
        out.matches("– cafleet setup --coding-agent").count(),
        3,
        "every agent renders not-installed with no recorded data: {out}"
    );
    assert!(out.contains("no issues found"), "got: {out}");
}

#[test]
fn doctor_reports_a_newer_schema_than_the_cli() {
    let cli = Cli::new();
    cli.migrate();
    cli.sqlite()
        .execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
             VALUES (99, 'future', '2026-01-01T00:00:00', 'x')",
            [],
        )
        .unwrap();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    assert!(
        stdout(&output).contains("schema 99 is newer than this CLI (head 7) — upgrade cafleet"),
        "got: {}",
        stdout(&output)
    );
}

#[test]
fn doctor_reports_an_unversioned_database() {
    let cli = Cli::new();
    let conn = rusqlite::Connection::open(cli.db_path()).unwrap();
    conn.execute_batch("CREATE TABLE junk (x INTEGER);")
        .unwrap();
    drop(conn);
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    assert!(
        out.contains("database has tables but no schema history — not a cafleet database?"),
        "got: {out}"
    );
    assert_eq!(
        out.matches("– cafleet setup --coding-agent").count(),
        3,
        "a missing asset_installs table renders every agent as –: {out}"
    );
}

#[test]
fn doctor_reports_a_database_connection_failure() {
    let mut cli = Cli::new();
    let dir = cli.home.path().join("dbdir");
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite:///{}", dir.display());
    cli.set_env("CAFLEET_DATABASE_URL", &url);
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    assert!(out.contains("✗ database"), "got: {out}");
    assert!(
        out.contains("coding agents"),
        "the report continues past a connection failure: {out}"
    );
}

#[test]
fn doctor_treats_not_installed_as_informational() {
    let cli = Cli::new();
    cli.migrate();
    let output = cli.run(&["doctor"]);
    assert_eq!(
        code(&output),
        0,
        "the – state never fails: {}",
        stdout(&output)
    );
    let out = stdout(&output);
    assert_eq!(out.matches("– cafleet setup --coding-agent").count(), 3);
    assert!(out.contains("no issues found"), "got: {out}");
}

#[test]
fn doctor_setup_cells_cover_ok_stale_and_not_installed() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("claude", VERSION);
    cli.seed_asset_row("codex", "0.1.0");
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    assert!(out.contains(&format!("✓ {VERSION}")), "got: {out}");
    assert!(
        out.contains("✗ 0.1.0 → cafleet setup --coding-agent codex"),
        "got: {out}"
    );
    assert!(
        out.contains("– cafleet setup --coding-agent opencode"),
        "the EN DASH cell carries the remedy: {out}"
    );
    assert!(
        out.contains("1 issue found"),
        "only the stale cell counts: {out}"
    );
}

#[test]
fn doctor_reports_the_env_source_and_a_resolution_error() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("cfg-claude");
    cli.migrate();
    cli.set_env("CLAUDE_CONFIG_DIR", custom.to_str().unwrap());
    cli.set_env("CODEX_HOME", "rel");
    cli.seed_asset_row_at("claude", custom.to_str().unwrap(), VERSION);
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    assert!(
        out.contains("~/cfg-claude"),
        "the env path ~-abbreviates: {out}"
    );
    assert!(out.contains("$CLAUDE_CONFIG_DIR"), "got: {out}");
    assert!(out.contains("default"), "got: {out}");
    assert!(
        out.contains("✗ CODEX_HOME is not an absolute path"),
        "got: {out}"
    );
    assert!(
        out.contains("rel"),
        "the raw invalid value shows in the path column: {out}"
    );
    assert!(out.contains("1 issue found"), "got: {out}");
}

#[test]
fn doctor_lists_superseded_rows_as_footnotes() {
    let cli = Cli::new();
    cli.migrate();
    cli.seed_asset_row("codex", VERSION);
    cli.seed_asset_row_at("codex", "/codex-old", "0.1.0");
    cli.seed_asset_row_at("claude", "/b-old", "0.1.0");
    cli.seed_asset_row_at("claude", "/a-old", "0.1.0");
    let output = cli.run(&["doctor"]);
    assert_eq!(
        code(&output),
        0,
        "footnotes never count as issues: {}",
        stdout(&output)
    );
    let out = stdout(&output);
    let notes: Vec<&str> = out
        .lines()
        .filter(|line| line.trim_start().starts_with("note: "))
        .map(str::trim_start)
        .collect();
    assert_eq!(
        notes,
        vec![
            "note: claude was previously set up at /a-old",
            "note: claude was previously set up at /b-old",
            "note: codex was previously set up at /codex-old",
        ],
        "one line per superseded row, ascending (coding_agent, path): {out}"
    );
    assert!(out.contains("no issues found"), "got: {out}");
}

#[test]
fn doctor_frames_the_table_by_display_width() {
    fn display_width(line: &str) -> usize {
        line.chars()
            .map(|c| {
                if ('\u{4E00}'..='\u{9FFF}').contains(&c) {
                    2
                } else {
                    1
                }
            })
            .sum()
    }

    let mut cli = Cli::new();
    let custom = cli.home.path().join("設定");
    cli.set_env("CLAUDE_CONFIG_DIR", custom.to_str().unwrap());
    cli.migrate();
    let output = cli.run(&["doctor"]);
    let out = stdout(&output);
    let frame: Vec<&str> = out
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            ['┌', '├', '└', '│'].iter().any(|c| trimmed.starts_with(*c))
        })
        .collect();
    assert_eq!(
        frame.len(),
        7,
        "top, header, separator, three rows, bottom: {out}"
    );
    let widths: Vec<usize> = frame.iter().map(|line| display_width(line)).collect();
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "every frame line has the same display width {widths:?}: {out}"
    );
}

#[test]
fn doctor_json_mirrors_the_report() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("cfg-claude");
    cli.set_env("CLAUDE_CONFIG_DIR", custom.to_str().unwrap());
    cli.migrate();
    cli.seed_asset_row_at("claude", custom.to_str().unwrap(), VERSION);
    cli.seed_asset_row("codex", "0.1.0");
    cli.seed_asset_row_at("codex", "/codex-old", "0.0.9");
    let output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&output), 1, "exit parity with text mode");
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();

    assert_eq!(payload["multiplexer"]["ok"], true);
    assert_eq!(payload["multiplexer"]["backend"], "tmux");
    assert_eq!(payload["multiplexer"]["presence_var"], "TMUX");
    assert_eq!(
        payload["multiplexer"]["presence_value"],
        "/tmp/tmux-1000/default,123,0"
    );
    assert_eq!(payload["multiplexer"]["error"], serde_json::Value::Null);

    assert_eq!(payload["database"]["ok"], true);
    assert_eq!(payload["database"]["schema_version"], 7);
    assert_eq!(payload["database"]["head_version"], 7);
    assert_eq!(payload["database"]["error"], serde_json::Value::Null);

    let agents = payload["coding_agents"]["agents"].as_array().unwrap();
    assert_eq!(payload["coding_agents"]["ok"], false);
    assert_eq!(payload["coding_agents"]["cli_version"], VERSION);
    assert_eq!(agents[0]["coding_agent"], "claude");
    assert_eq!(agents[0]["path"], custom.to_str().unwrap());
    assert_eq!(agents[0]["source"], "CLAUDE_CONFIG_DIR");
    assert_eq!(agents[0]["state"], "ok");
    assert_eq!(agents[0]["recorded_version"], VERSION);
    assert_eq!(agents[1]["coding_agent"], "codex");
    assert_eq!(agents[1]["source"], "default");
    assert_eq!(agents[1]["state"], "stale");
    assert_eq!(agents[1]["recorded_version"], "0.1.0");
    assert_eq!(agents[2]["coding_agent"], "opencode");
    assert_eq!(agents[2]["state"], "not_installed");
    assert_eq!(agents[2]["recorded_version"], serde_json::Value::Null);
    assert_eq!(agents[2]["installed_at"], serde_json::Value::Null);

    let superseded = payload["coding_agents"]["superseded"].as_array().unwrap();
    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0]["coding_agent"], "codex");
    assert_eq!(superseded[0]["path"], "/codex-old");
    assert_eq!(superseded[0]["recorded_version"], "0.0.9");

    assert_eq!(payload["issues"], 1);
}

#[test]
fn doctor_json_null_contracts_on_failures() {
    let mut cli = Cli::new();
    cli.set_env("CODEX_HOME", "rel");
    let output = cli.run_outside_tmux(&["doctor", "--json"]);
    assert_eq!(code(&output), 1);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();

    assert_eq!(payload["multiplexer"]["ok"], false);
    assert_eq!(payload["multiplexer"]["backend"], serde_json::Value::Null);
    assert_eq!(payload["multiplexer"]["session"], serde_json::Value::Null);
    assert_eq!(payload["multiplexer"]["pane_id"], serde_json::Value::Null);
    assert!(
        payload["multiplexer"]["error"].is_string(),
        "the resolver error lands in the error field"
    );

    assert_eq!(payload["database"]["ok"], false);
    assert_eq!(
        payload["database"]["schema_version"],
        serde_json::Value::Null,
        "no ledger means a null schema_version"
    );

    let agents = payload["coding_agents"]["agents"].as_array().unwrap();
    assert_eq!(agents[1]["coding_agent"], "codex");
    assert_eq!(agents[1]["state"], "error");
    assert_eq!(agents[1]["path"], serde_json::Value::Null);
    assert_eq!(agents[1]["source"], "CODEX_HOME");
    assert_eq!(
        agents[1]["error"],
        "CODEX_HOME must be an absolute path (got 'rel')"
    );

    assert_eq!(payload["issues"], 3, "multiplexer + database + codex");
}
