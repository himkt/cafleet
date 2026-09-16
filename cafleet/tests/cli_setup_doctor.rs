//! Setup installation and doctor environment-report contracts.

mod common;

use cafleet::db::head_version;
use common::{Cli, VERSION, code, stderr, stdout};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn installed_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                assert!(
                    path.is_file(),
                    "expected an installed file: {}",
                    path.display()
                );
                assert!(
                    files
                        .insert(
                            path.strip_prefix(root).unwrap().to_path_buf(),
                            std::fs::read(&path).unwrap()
                        )
                        .is_none()
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn assert_installed_skills(root: &Path) {
    let expected = cafleet::embedded::SKILLS
        .iter()
        .filter(|(path, _)| path.starts_with("cafleet/") || path.starts_with("cafleet-design-doc/"))
        .map(|(path, bytes)| (PathBuf::from(path), bytes.to_vec()))
        .collect::<BTreeMap<_, _>>();
    assert!(
        expected
            .keys()
            .any(|path| path.starts_with("cafleet/reference/runtime"))
    );
    assert_eq!(
        installed_files(root),
        expected,
        "installed package at {}",
        root.display()
    );
}

fn assert_installed_preset(path: &Path, embedded: &str) {
    assert_eq!(
        std::fs::read(path).unwrap(),
        cafleet::embedded::lookup(cafleet::embedded::PRESETS, embedded).unwrap()
    );
}

fn assert_keys(value: &serde_json::Value, expected: &[&str]) {
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        expected
    );
}

fn assert_doctor_shape(payload: &serde_json::Value) {
    assert_keys(
        payload,
        &["multiplexer", "database", "coding_agents", "issues"],
    );
    assert_keys(
        &payload["multiplexer"],
        &[
            "ok",
            "backend",
            "session",
            "window_id",
            "pane_id",
            "presence_var",
            "presence_value",
            "error",
        ],
    );
    assert_keys(
        &payload["database"],
        &["ok", "schema_version", "head_version", "error"],
    );
    assert_keys(
        &payload["coding_agents"],
        &["ok", "cli_version", "agents", "superseded"],
    );
    let agents = payload["coding_agents"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 3);
    for (agent, name) in agents.iter().zip(["claude", "codex", "opencode"]) {
        assert_keys(
            agent,
            &[
                "coding_agent",
                "path",
                "source",
                "recorded_version",
                "installed_at",
                "state",
                "error",
            ],
        );
        assert_eq!(agent["coding_agent"], name);
    }
    for row in payload["coding_agents"]["superseded"].as_array().unwrap() {
        assert_keys(
            row,
            &["coding_agent", "path", "recorded_version", "installed_at"],
        );
    }
}

#[test]
fn plain_setup_installs_and_records_all_three_agents_on_a_fresh_database() {
    let cli = Cli::new();
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains(&format!("applied migrations to head ({}).", head_version())),
        "{out}"
    );
    let targets = [
        ("claude", ".claude/skills"),
        ("codex", ".codex/skills"),
        ("opencode", ".config/opencode/skills"),
    ];
    let expected_rows = targets
        .iter()
        .map(|(agent, _)| {
            (
                agent.to_string(),
                cli.identity_path(agent),
                VERSION.to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(cli.asset_rows(), expected_rows);
    assert_eq!(
        cli.sqlite()
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        head_version()
    );
    let mut offsets = Vec::new();
    for (agent, relative) in targets {
        let skills = cli.home.path().join(relative);
        assert_installed_skills(&skills);
        offsets.push(
            out.find(&format!(
                "{agent}: installed cafleet, cafleet-design-doc (v{VERSION}) -> {}",
                skills.display()
            ))
            .unwrap(),
        );
        std::fs::write(skills.join("cafleet/SKILL.md"), "corrupt").unwrap();
        std::fs::write(skills.join("cafleet/extra.txt"), "stale").unwrap();
        let stale = skills.join("cafleet-research");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("SKILL.md"), "stale").unwrap();
    }
    assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
    let presets = [
        ("codex", ".codex/rules/cafleet.rules", "codex/cafleet.rules"),
        (
            "opencode",
            ".opencode/agents/cafleet.md",
            "opencode/cafleet.md",
        ),
    ];
    for (agent, relative, embedded) in presets {
        let preset = cli.home.path().join(relative);
        assert_installed_preset(&preset, embedded);
        assert!(
            out.contains(&format!(
                "{agent}: installed preset (v{VERSION}) -> {}",
                preset.display()
            )),
            "{out}"
        );
        std::fs::write(preset, "corrupt").unwrap();
    }
    cli.seed_asset_row_at("codex", "/codex-old", "0.1.0");
    cli.seed_asset_row_at("opencode", "/opencode-old", "0.2.0");
    let recorded = cli.asset_rows();
    let again = cli.run(&["setup"]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert!(stdout(&again).contains(&format!(
        "Already at head ({}); nothing to do.",
        head_version()
    )));
    assert!(!stdout(&again).contains("cafleet-research"));
    for (_, relative) in targets {
        assert_installed_skills(&cli.home.path().join(relative));
    }
    for (_, relative, embedded) in presets {
        assert_installed_preset(&cli.home.path().join(relative), embedded);
    }
    assert_eq!(cli.asset_rows(), recorded);
}

#[test]
fn setup_refuses_an_unversioned_database_with_existing_tables() {
    let cli = Cli::new();
    let conn = rusqlite::Connection::open(cli.db_path()).unwrap();
    conn.execute_batch("CREATE TABLE junk (x INTEGER); INSERT INTO junk VALUES (42);")
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
    let conn = cli.sqlite();
    assert_eq!(
        conn.query_row("SELECT x FROM junk", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        42
    );
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('refinery_schema_history', 'asset_installs')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn selector_setup_deduplicates_and_installs_in_the_fixed_order() {
    let cli = Cli::new();
    let output = cli.run(&[
        "setup",
        "--coding-agent",
        "opencode",
        "claude",
        "--coding-agent",
        "opencode",
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
    assert_eq!(out.matches("opencode: installed cafleet").count(), 1);
    assert_eq!(
        cli.asset_rows(),
        vec![
            ("claude".into(), cli.identity_path("claude"), VERSION.into()),
            (
                "opencode".into(),
                cli.identity_path("opencode"),
                VERSION.into()
            )
        ]
    );
}

#[test]
fn selector_setup_installs_at_the_env_resolved_paths() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("codex-custom");
    cli.set_env("CODEX_HOME", custom.to_str().unwrap());
    cli.set_env("CLAUDE_CONFIG_DIR", "relative/path");
    let output = cli.run(&["setup", "--coding-agent", "codex"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_installed_skills(&custom.join("skills"));
    assert_installed_preset(&custom.join("rules/cafleet.rules"), "codex/cafleet.rules");
    for tree in [".codex", ".claude", ".config/opencode", ".opencode"] {
        assert!(!cli.home.path().join(tree).exists(), "{tree}");
    }
    let out = stdout(&output);
    assert!(!out.contains("claude: installed"), "{out}");
    assert!(!out.contains("opencode: installed"), "{out}");
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
        combined.contains(&format!("applied migrations to head ({}).", head_version())),
        "the db half is unaffected: {combined}"
    );
    assert_eq!(
        cli.sqlite()
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        head_version()
    );
    assert_installed_skills(&cli.home.path().join(".claude/skills"));
    assert_eq!(
        cli.asset_rows(),
        vec![("claude".into(), cli.identity_path("claude"), VERSION.into())]
    );
    for tree in [".codex", ".config/opencode", ".opencode"] {
        assert!(!cli.home.path().join(tree).exists(), "{tree}");
    }
}

#[test]
fn setup_help_documents_the_multi_value_flag() {
    let cli = Cli::new();
    let output = cli.run(&["setup", "--help"]);
    assert_eq!(code(&output), 0);
    let help = stdout(&output);
    assert!(help.contains("--coding-agent <AGENT>..."), "{help}");
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
    assert!(
        out.contains(&format!("schema {} (head)", head_version())),
        "got: {out}"
    );
    assert!(out.contains("✓ coding agents"), "got: {out}");
    assert_eq!(
        out.matches(&format!("✓ {VERSION}")).count(),
        3,
        "three ok setup cells: {out}"
    );
    assert!(out.contains("no issues found"), "got: {out}");
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
fn doctor_completes_the_report_against_a_pre_v6_database() {
    let cli = Cli::new();
    cli.seed_pre_v6_database();
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1, "the database issue exits 1");
    let out = stdout(&output);
    assert!(out.contains("✓ multiplexer"), "got: {out}");
    assert!(out.contains("✗ database"), "got: {out}");
    assert!(
        out.contains(&format!(
            "schema 5, head is {} — run: cafleet setup",
            head_version()
        )),
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
    let output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&output), 1);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_doctor_shape(&payload);

    assert_eq!(payload["database"]["ok"], false);
    assert_eq!(payload["database"]["schema_version"], 5);
    assert_eq!(payload["database"]["head_version"], head_version());

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
fn doctor_json_null_contracts_on_failures() {
    let mut cli = Cli::new();
    cli.set_env("CODEX_HOME", "rel");
    let output = cli.run_outside_tmux(&["doctor"]);
    assert_eq!(code(&output), 1);
    let out = stdout(&output);
    for detail in [
        "✗ multiplexer",
        "✗ database",
        "✗ coding agents",
        "no database — run: cafleet setup",
        "✗ CODEX_HOME is not an absolute path",
        "rel",
        "claude",
        "codex",
        "opencode",
        "3 issues found",
    ] {
        assert!(out.contains(detail), "{detail}: {out}");
    }
    assert_eq!(out.matches("– cafleet setup --coding-agent").count(), 2);
    let output = cli.run_outside_tmux(&["doctor", "--json"]);
    assert_eq!(code(&output), 1);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_doctor_shape(&payload);

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

    for key in ["window_id", "presence_var", "presence_value"] {
        assert_eq!(payload["multiplexer"][key], serde_json::Value::Null);
    }
    assert_eq!(payload["database"]["head_version"], head_version());
    assert_eq!(
        payload["database"]["error"],
        "no database — run: cafleet setup"
    );
    assert_eq!(payload["coding_agents"]["ok"], false);
    assert!(
        payload["coding_agents"]["superseded"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for (index, agent) in agents.iter().enumerate() {
        assert_eq!(agent["recorded_version"], serde_json::Value::Null);
        assert_eq!(agent["installed_at"], serde_json::Value::Null);
        if index != 1 {
            assert_eq!(agent["state"], "not_installed");
            assert_eq!(agent["error"], serde_json::Value::Null);
        }
    }
    assert_eq!(payload["issues"], 3, "multiplexer + database + codex");
}

#[test]
fn step6_doctor_open_and_path_failures_still_render_all_sections_in_text_and_json() {
    let mut cli = Cli::new();
    let directory = cli.home.path().join("not-a-database");
    std::fs::create_dir(&directory).unwrap();
    cli.set_env(
        "CAFLEET_DATABASE_URL",
        &format!("sqlite:///{}", directory.display()),
    );
    cli.set_env("CLAUDE_CONFIG_DIR", "relative");
    let output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&output), 1);
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_doctor_shape(&payload);
    assert_eq!(
        payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["multiplexer", "database", "coding_agents", "issues"]
    );
    assert_eq!(payload["multiplexer"]["ok"], true);
    assert_eq!(payload["database"]["ok"], false);
    assert_eq!(
        payload["database"]["schema_version"],
        serde_json::Value::Null
    );
    assert_eq!(
        payload["database"]["head_version"],
        cafleet::db::head_version()
    );
    assert!(
        payload["database"]["error"]
            .as_str()
            .unwrap()
            .contains("failed to open database")
    );
    let agents = payload["coding_agents"]["agents"].as_array().unwrap();
    assert_eq!(
        agents
            .iter()
            .map(|a| a["coding_agent"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claude", "codex", "opencode"]
    );
    assert_eq!(agents[0]["path"], serde_json::Value::Null);
    assert_eq!(agents[0]["source"], "CLAUDE_CONFIG_DIR");
    assert_eq!(
        agents[0]["error"],
        "CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')"
    );
    for agent in agents {
        assert_eq!(agent["recorded_version"], serde_json::Value::Null);
        assert_eq!(agent["installed_at"], serde_json::Value::Null);
    }
    assert_eq!(agents[0]["state"], "error");
    assert_eq!(agents[1]["state"], "not_installed");
    assert_eq!(agents[2]["state"], "not_installed");
    assert_eq!(payload["issues"], 2);
    let output = cli.run(&["doctor"]);
    assert_eq!(code(&output), 1);
    let text = stdout(&output);
    assert!(text.find("multiplexer").unwrap() < text.find("database").unwrap());
    assert!(text.find("database").unwrap() < text.find("coding agents").unwrap());
    assert!(text.contains("✗ database"));
    assert!(text.contains("failed to open database"));
    assert!(text.contains("CLAUDE_CONFIG_DIR is not an absolute path"));
    assert!(text.contains("2 issues found"));
}

#[test]
fn step6_doctor_non_head_states_do_not_query_malformed_asset_records() {
    for ahead in [false, true] {
        let cli = Cli::new();
        cli.migrate();
        let conn = cli.sqlite();
        if ahead {
            conn.execute(
                "UPDATE refinery_schema_history SET version=?1 WHERE version=?2",
                rusqlite::params![cafleet::db::head_version() + 1, cafleet::db::head_version()],
            )
            .unwrap();
        } else {
            conn.execute(
                "DELETE FROM refinery_schema_history WHERE version=?1",
                [cafleet::db::head_version()],
            )
            .unwrap();
        }
        conn.execute_batch(
            "DROP TABLE asset_installs; CREATE TABLE asset_installs(wrong_column TEXT)",
        )
        .unwrap();
        drop(conn);
        let head = head_version();
        let recorded = if ahead { head + 1 } else { head - 1 };
        let remedy = if ahead {
            format!("schema {recorded} is newer than this CLI (head {head}) — upgrade cafleet")
        } else {
            format!("schema {recorded}, head is {head} — run: cafleet setup")
        };
        let output = cli.run(&["doctor"]);
        assert_eq!(code(&output), 1);
        let out = stdout(&output);
        assert!(out.contains(&remedy), "{out}");
        assert!(out.contains("coding agents"), "{out}");
        assert_eq!(out.matches("– cafleet setup --coding-agent").count(), 3);
        assert!(out.contains("1 issue found"), "{out}");
        let output = cli.run(&["doctor", "--json"]);
        assert_eq!(code(&output), 1);
        let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
        assert_doctor_shape(&payload);
        assert_eq!(
            payload["database"],
            serde_json::json!({"ok": false,
            "schema_version": recorded, "head_version": head, "error": remedy})
        );
        assert!(
            payload["coding_agents"]["superseded"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for agent in payload["coding_agents"]["agents"].as_array().unwrap() {
            assert_eq!(agent["recorded_version"], serde_json::Value::Null);
            assert_eq!(agent["installed_at"], serde_json::Value::Null);
            assert_eq!(agent["error"], serde_json::Value::Null);
        }
        assert_eq!(payload["database"]["ok"], false);
        assert_eq!(payload["coding_agents"]["ok"], true);
        assert_eq!(
            payload["coding_agents"]["agents"].as_array().unwrap().len(),
            3
        );
        assert!(
            payload["coding_agents"]["agents"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["state"] == "not_installed")
        );
        assert_eq!(payload["issues"], 1);
    }
}

#[test]
fn doctor_reports_mixed_assets_in_text_and_json() {
    let mut cli = Cli::new();
    let custom = cli.home.path().join("設定");
    cli.set_env("CLAUDE_CONFIG_DIR", custom.to_str().unwrap());
    cli.migrate();
    cli.seed_asset_row_at("claude", custom.to_str().unwrap(), VERSION);
    cli.seed_asset_row("codex", "0.1.0");
    cli.seed_asset_row_at("codex", "/codex-old", "0.0.9");
    cli.seed_asset_row_at("claude", "/b-old", "0.1.0");
    cli.seed_asset_row_at("claude", "/a-old", "0.1.0");
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

    assert!(out.contains("~/設定"), "{out}");
    assert!(out.contains("$CLAUDE_CONFIG_DIR"), "{out}");
    assert!(out.contains("default"), "{out}");
    let notes = out
        .lines()
        .filter(|line| line.trim_start().starts_with("note: "))
        .map(str::trim_start)
        .collect::<Vec<_>>();
    assert_eq!(
        notes,
        vec![
            "note: claude was previously set up at /a-old",
            "note: claude was previously set up at /b-old",
            "note: codex was previously set up at /codex-old"
        ]
    );
    let frame = out
        .lines()
        .filter(|line| {
            ['┌', '├', '└', '│']
                .iter()
                .any(|c| line.trim_start().starts_with(*c))
        })
        .collect::<Vec<_>>();
    assert_eq!(frame.len(), 7, "{out}");
    let widths = frame
        .iter()
        .map(|line| unicode_width::UnicodeWidthStr::width(*line))
        .collect::<Vec<_>>();
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "{widths:?}: {out}"
    );
    let output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&output), 1, "exit parity with text mode");
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_doctor_shape(&payload);

    assert_eq!(payload["multiplexer"]["ok"], true);
    assert_eq!(payload["multiplexer"]["backend"], "tmux");
    assert_eq!(payload["multiplexer"]["session"], "main");
    assert_eq!(payload["multiplexer"]["window_id"], "@1");
    assert_eq!(payload["multiplexer"]["pane_id"], "%0");
    assert_eq!(payload["multiplexer"]["presence_var"], "TMUX");
    assert_eq!(
        payload["multiplexer"]["presence_value"],
        "/tmp/tmux-1000/default,123,0"
    );
    assert_eq!(payload["multiplexer"]["error"], serde_json::Value::Null);

    assert_eq!(payload["database"]["ok"], true);
    assert_eq!(payload["database"]["schema_version"], head_version());
    assert_eq!(payload["database"]["head_version"], head_version());
    assert_eq!(payload["database"]["error"], serde_json::Value::Null);

    let agents = payload["coding_agents"]["agents"].as_array().unwrap();
    assert_eq!(payload["coding_agents"]["ok"], false);
    assert_eq!(payload["coding_agents"]["cli_version"], VERSION);
    assert_eq!(agents[0]["coding_agent"], "claude");
    assert_eq!(agents[0]["path"], custom.to_str().unwrap());
    assert_eq!(agents[0]["source"], "CLAUDE_CONFIG_DIR");
    assert_eq!(agents[0]["state"], "ok");
    assert_eq!(agents[0]["recorded_version"], VERSION);
    assert!(agents[0]["installed_at"].is_string());
    assert!(agents[1]["installed_at"].is_string());
    for agent in agents {
        assert_eq!(agent["error"], serde_json::Value::Null);
    }
    assert_eq!(agents[1]["path"], cli.identity_path("codex"));
    assert_eq!(agents[2]["path"], cli.identity_path("opencode"));
    assert_eq!(agents[2]["source"], "default");
    assert_eq!(agents[1]["coding_agent"], "codex");
    assert_eq!(agents[1]["source"], "default");
    assert_eq!(agents[1]["state"], "stale");
    assert_eq!(agents[1]["recorded_version"], "0.1.0");
    assert_eq!(agents[2]["coding_agent"], "opencode");
    assert_eq!(agents[2]["state"], "not_installed");
    assert_eq!(agents[2]["recorded_version"], serde_json::Value::Null);
    assert_eq!(agents[2]["installed_at"], serde_json::Value::Null);

    let superseded = payload["coding_agents"]["superseded"].as_array().unwrap();
    assert_eq!(superseded.len(), 3);
    for (row, agent, path, version) in [
        (&superseded[0], "claude", "/a-old", "0.1.0"),
        (&superseded[1], "claude", "/b-old", "0.1.0"),
        (&superseded[2], "codex", "/codex-old", "0.0.9"),
    ] {
        assert_eq!(row["coding_agent"], agent);
        assert_eq!(row["path"], path);
        assert_eq!(row["recorded_version"], version);
        assert!(row["installed_at"].is_string());
    }

    assert_eq!(payload["issues"], 1);
}

#[test]
fn doctor_treats_empty_and_absent_asset_tables_as_informational() {
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
