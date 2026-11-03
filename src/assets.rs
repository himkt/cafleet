use crate::coding_agent::CodingAgent;
use crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH;
use crate::config::EnvLookup;
use crate::config::opencode_skills_base;
use crate::error::CafleetError;
use rusqlite::Connection;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub(crate) struct AgentPaths {
    pub(crate) identity: String,
    skills_dir: PathBuf,
    presets: Vec<(String, PathBuf)>,
}

/// Runs `hermes` subcommands for setup with `HERMES_HOME` set to the resolved base.
pub(crate) trait HermesConfigRunner {
    fn binary_available(&self) -> bool;
    /// `args` follow the `hermes` binary; the result is its stdout.
    fn run(&mut self, args: &[String]) -> Result<String, CafleetError>;
}

pub(crate) fn agent_paths(
    env: EnvLookup<'_>,
    home: &Path,
    agent: CodingAgent,
) -> Result<AgentPaths, CafleetError> {
    let base = agent.config_dir(env, home)?;
    let skills_dir = match agent {
        CodingAgent::Opencode => opencode_skills_base(home).join("skills"),
        _ => base.join("skills"),
    };
    let presets = match agent {
        CodingAgent::Claude => vec![],
        CodingAgent::Codex => vec![(
            "codex/cafleet.rules".into(),
            base.join("rules/cafleet.rules"),
        )],
        CodingAgent::Opencode => agent
            .policy_choices(None)
            .into_iter()
            .map(|mode| {
                let target = agent
                    .required_preset(&base, &mode)
                    .expect("OpenCode requires a preset for every launch policy");
                (format!("opencode/{mode}.md"), target)
            })
            .collect(),
        CodingAgent::Hermes => vec![],
    };
    Ok(AgentPaths {
        identity: base.display().to_string(),
        skills_dir,
        presets,
    })
}

pub(crate) fn install_agent(
    conn: &mut Connection,
    agent: CodingAgent,
    paths: &AgentPaths,
    version: &str,
    hermes: Option<&mut (dyn HermesConfigRunner + '_)>,
    out: &mut dyn Write,
) -> Result<(), CafleetError> {
    let mut report = |line: String| {
        writeln!(out, "{line}")
            .map_err(|error| CafleetError::App(format!("failed to write setup output: {error}")))
    };
    let fail = |error: std::io::Error| {
        CafleetError::App(format!(
            "failed to install skills into {}: {error}",
            paths.skills_dir.display()
        ))
    };
    let roots = SKILLS
        .iter()
        .filter_map(|(path, _)| {
            path.strip_suffix("/SKILL.md")
                .filter(|root| !root.contains('/'))
        })
        .collect::<Vec<_>>();
    assert!(
        !roots.is_empty(),
        "embedded skills have top-level manifests"
    );
    for skill in &roots {
        let prefix = format!("{skill}/");
        remove_existing_target(&paths.skills_dir.join(skill)).map_err(fail)?;
        for (path, bytes) in SKILLS.iter().filter(|(path, _)| path.starts_with(&prefix)) {
            let target = paths.skills_dir.join(path);
            std::fs::create_dir_all(target.parent().expect("skill file has a parent"))
                .map_err(fail)?;
            std::fs::write(target, bytes).map_err(fail)?;
        }
    }
    report(format!(
        "{}: installed {} (v{version}) -> {}",
        agent.name(),
        roots.join(", "),
        paths.skills_dir.display()
    ))?;
    for (source, target) in &paths.presets {
        let fail = |error: std::io::Error| {
            CafleetError::App(format!(
                "failed to install preset into {}: {error}",
                target.display()
            ))
        };
        let bytes = lookup(PRESETS, source).expect("embedded preset exists");
        std::fs::create_dir_all(target.parent().expect("preset has a parent")).map_err(fail)?;
        remove_existing_target(target).map_err(fail)?;
        std::fs::write(target, bytes).map_err(fail)?;
        report(format!(
            "{}: installed preset (v{version}) -> {}",
            agent.name(),
            target.display()
        ))?;
    }
    if agent == CodingAgent::Hermes {
        match hermes {
            Some(runner) => {
                let config = runner
                    .run(&hermes_args(&["config", "path"]))
                    .map(|stdout| stdout.trim().to_owned())
                    .map_err(|cause| passthrough_failure(&paths.identity, &cause.message()))?;
                report(merge_client_env_passthrough(runner, &config)?)?;
            }
            None => report("hermes: skipped terminal.client_env_passthrough names (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes".into())?,
        }
    }
    crate::broker::record_asset_install(conn, agent.name(), &paths.identity, version)?;
    Ok(())
}

const PASSTHROUGH_KEY: &str = "terminal.client_env_passthrough";

/// Appends each missing CAFleet member name after the existing entries and
/// returns the setup line describing the result.
fn merge_client_env_passthrough(
    runner: &mut dyn HermesConfigRunner,
    config: &str,
) -> Result<String, CafleetError> {
    let fail = |cause: String| passthrough_failure(config, &cause);
    let existing = read_passthrough(runner).map_err(fail)?;
    let missing = missing_member_names(&existing);
    if missing.is_empty() {
        return Ok(format!(
            "hermes: {PASSTHROUGH_KEY} already holds CAFleet's member names -> {config}"
        ));
    }
    let mut merged = existing;
    merged.extend(missing.iter().map(|name| (*name).to_owned()));
    let value = serde_json::to_string(&merged).expect("a list of strings serializes as JSON");
    runner
        .run(&hermes_args(&["config", "set", PASSTHROUGH_KEY, &value]))
        .map_err(|cause| fail(cause.message().into_owned()))?;
    let absent = missing_member_names(&read_passthrough(runner).map_err(fail)?);
    if !absent.is_empty() {
        return Err(fail(format!(
            "{PASSTHROUGH_KEY} still lacks {} after the write",
            absent.join(", ")
        )));
    }
    Ok(format!(
        "hermes: added {PASSTHROUGH_KEY} names -> {config}: {}",
        missing.join(", ")
    ))
}

fn read_passthrough(runner: &mut dyn HermesConfigRunner) -> Result<Vec<String>, String> {
    let stdout = runner
        .run(&hermes_args(&["config", "get", "--json", PASSTHROUGH_KEY]))
        .map_err(|cause| cause.message().into_owned())?;
    serde_json::from_str::<Vec<String>>(&stdout)
        .map_err(|cause| format!("{PASSTHROUGH_KEY} is not a JSON array of strings: {cause}"))
}

fn missing_member_names(present: &[String]) -> Vec<&'static str> {
    HERMES_CLIENT_ENV_PASSTHROUGH
        .iter()
        .copied()
        .filter(|name| !present.iter().any(|value| value == name))
        .collect()
}

fn passthrough_failure(config: &str, cause: &str) -> CafleetError {
    CafleetError::App(format!(
        "failed to install hermes {PASSTHROUGH_KEY} names into {config}: {cause}"
    ))
}

fn hermes_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn remove_existing_target(target: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

include!(concat!(env!("OUT_DIR"), "/embedded_data.rs"));

pub(crate) fn lookup(
    table: &'static [(&'static str, &'static [u8])],
    path: &str,
) -> Option<&'static [u8]> {
    table
        .iter()
        .find(|(entry, _)| *entry == path)
        .map(|(_, bytes)| *bytes)
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::assets::HermesConfigRunner;
    use crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH;
    use crate::error::CafleetError;
    use serde_json::Value;

    pub(crate) const FAKE_CONFIG_PATH: &str = "/srv/hermes/profiles/work/config.yaml";

    const PASSTHROUGH_KEY: &str = "terminal.client_env_passthrough";

    /// Answers only `config path` and the passthrough get/set; any other command panics.
    pub(crate) struct FakeHermes {
        pub(crate) available: bool,
        pub(crate) passthrough: Value,
        pub(crate) path_failure: Option<String>,
        pub(crate) get_failure: Option<String>,
        pub(crate) set_failure: Option<String>,
        pub(crate) ignore_set: bool,
        pub(crate) calls: Vec<Vec<String>>,
    }

    impl FakeHermes {
        pub(crate) fn complete() -> Self {
            Self::holding(&HERMES_CLIENT_ENV_PASSTHROUGH)
        }
        pub(crate) fn holding(names: &[&str]) -> Self {
            Self::with_passthrough(serde_json::json!(names))
        }
        pub(crate) fn with_passthrough(passthrough: Value) -> Self {
            Self {
                available: true,
                passthrough,
                path_failure: None,
                get_failure: None,
                set_failure: None,
                ignore_set: false,
                calls: Vec::new(),
            }
        }
        pub(crate) fn path_call() -> Vec<String> {
            vec!["config".into(), "path".into()]
        }
        pub(crate) fn passthrough_get_call() -> Vec<String> {
            ["config", "get", "--json", PASSTHROUGH_KEY]
                .map(str::to_owned)
                .to_vec()
        }
        pub(crate) fn passthrough_set_call(names: &[&str]) -> Vec<String> {
            vec![
                "config".into(),
                "set".into(),
                PASSTHROUGH_KEY.into(),
                serde_json::to_string(names).unwrap(),
            ]
        }
    }

    fn compact_list(value: &str) -> Value {
        assert!(
            !value.contains('\n') && !value.contains(", "),
            "compact JSON: {value}"
        );
        serde_json::from_str(value).expect("setup writes the merged list as JSON")
    }

    impl HermesConfigRunner for FakeHermes {
        fn binary_available(&self) -> bool {
            self.available
        }
        fn run(&mut self, args: &[String]) -> Result<String, CafleetError> {
            self.calls.push(args.to_vec());
            if !self.available {
                return Err(CafleetError::App("binary hermes not found on PATH".into()));
            }
            match args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["config", "path"] => match &self.path_failure {
                    Some(cause) => Err(CafleetError::App(cause.clone())),
                    None => Ok(format!("{FAKE_CONFIG_PATH}\n")),
                },
                ["config", "get", "--json", PASSTHROUGH_KEY] => match &self.get_failure {
                    Some(cause) => Err(CafleetError::App(cause.clone())),
                    None => Ok(format!("{}\n", self.passthrough)),
                },
                ["config", "set", PASSTHROUGH_KEY, value] => match &self.set_failure {
                    Some(cause) => Err(CafleetError::App(cause.clone())),
                    None => {
                        let parsed = compact_list(value);
                        if !self.ignore_set {
                            self.passthrough = parsed;
                        }
                        Ok(String::new())
                    }
                },
                other => panic!("unexpected hermes command {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::test_support::FAKE_CONFIG_PATH;
    use crate::assets::test_support::FakeHermes;
    use crate::assets::*;
    use crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH;
    use std::os::unix::fs::symlink;

    fn expected_presets(agent: CodingAgent, base: &Path) -> Vec<(&'static str, PathBuf)> {
        match agent {
            CodingAgent::Claude | CodingAgent::Hermes => vec![],
            CodingAgent::Codex => vec![("codex/cafleet.rules", base.join("rules/cafleet.rules"))],
            CodingAgent::Opencode => vec![
                ("opencode/cafleet.md", base.join("agents/cafleet.md")),
                (
                    "opencode/cafleet-ask.md",
                    base.join("agents/cafleet-ask.md"),
                ),
            ],
        }
    }

    fn runner_for(
        agent: CodingAgent,
        fake: &mut FakeHermes,
    ) -> Option<&mut dyn HermesConfigRunner> {
        (agent == CodingAgent::Hermes).then_some(fake as &mut dyn HermesConfigRunner)
    }

    fn expected_skills_dir(agent: CodingAgent, base: &Path, home: &Path) -> PathBuf {
        match agent {
            CodingAgent::Opencode => home.join(".config/opencode/skills"),
            _ => base.join("skills"),
        }
    }

    fn output_lines(output: Vec<u8>) -> Vec<String> {
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn asset_install_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM asset_installs", [], |row| row.get(0))
            .unwrap()
    }

    fn frontmatter_and_body(preset: &str) -> (serde_json::Value, String) {
        let text = std::str::from_utf8(lookup(PRESETS, preset).expect("embedded preset"))
            .unwrap()
            .to_owned();
        let rest = text
            .strip_prefix("---\n")
            .expect("frontmatter opens the preset");
        let (frontmatter, body) = rest
            .split_once("\n---\n")
            .expect("frontmatter closes before the body");
        (serde_json::from_str(frontmatter).unwrap(), body.to_owned())
    }

    fn paragraphs(body: &str) -> Vec<String> {
        body.split("\n\n")
            .map(str::trim)
            .filter(|paragraph| !paragraph.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let external = home.path().join("operator");
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("keep"), "operator bytes").unwrap();
        for agent in CodingAgent::all() {
            let custom = home.path().join(format!("custom-{}", agent.name()));
            let paths = agent_paths(
                &|name| (name == agent.config_variable()).then(|| custom.display().to_string()),
                home.path(),
                *agent,
            )
            .unwrap();
            let skills_dir = expected_skills_dir(*agent, &custom, home.path());
            let presets = expected_presets(*agent, &custom);
            std::fs::create_dir_all(&skills_dir).unwrap();
            symlink(&external, skills_dir.join("cafleet")).unwrap();
            for (_, target) in &presets {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                symlink(external.join("keep"), target).unwrap();
            }
            let mut fake = FakeHermes::complete();
            for _ in 0..2 {
                install_agent(
                    &mut conn,
                    *agent,
                    &paths,
                    "test-version",
                    runner_for(*agent, &mut fake),
                    &mut Vec::<u8>::new(),
                )
                .unwrap();
                for (relative, bytes) in SKILLS {
                    assert_eq!(std::fs::read(skills_dir.join(relative)).unwrap(), *bytes);
                }
                for (source, target) in &presets {
                    assert_eq!(
                        std::fs::read(target).unwrap(),
                        lookup(PRESETS, source).unwrap(),
                        "{}",
                        target.display()
                    );
                }
                assert_eq!(
                    std::fs::read(external.join("keep")).unwrap(),
                    b"operator bytes"
                );
            }
            if *agent == CodingAgent::Opencode {
                assert!(!custom.join("skills").exists());
            }
            let record: String = conn
                .query_row(
                    "SELECT cafleet_version FROM asset_installs WHERE coding_agent=?1 AND path=?2",
                    rusqlite::params![agent.name(), paths.identity],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(record, "test-version");
        }
    }

    #[test]
    fn installation_failure_records_only_completed_installations() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let paths = agent_paths(&|_| None, home.path(), CodingAgent::Codex).unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/rules"), "blocked parent").unwrap();
        assert!(
            install_agent(
                &mut conn,
                CodingAgent::Codex,
                &paths,
                "version",
                None,
                &mut Vec::<u8>::new()
            )
            .is_err()
        );
        assert_eq!(asset_install_count(&conn), 0);
    }

    #[test]
    fn installation_prints_the_skills_line_then_one_preset_line_per_target_in_install_order() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        for agent in CodingAgent::all() {
            let custom = home.path().join(format!("custom-{}", agent.name()));
            let paths = agent_paths(
                &|name| (name == agent.config_variable()).then(|| custom.display().to_string()),
                home.path(),
                *agent,
            )
            .unwrap();
            let mut output = Vec::new();
            let mut fake = FakeHermes::complete();
            install_agent(
                &mut conn,
                *agent,
                &paths,
                "9.8.7",
                runner_for(*agent, &mut fake),
                &mut output,
            )
            .unwrap();
            let mut lines = output_lines(output);
            let presets = expected_presets(*agent, &custom);
            if *agent == CodingAgent::Hermes {
                assert_eq!(
                    lines.split_off(1),
                    [format!(
                        "hermes: terminal.client_env_passthrough already holds CAFleet's member names -> {FAKE_CONFIG_PATH}"
                    )],
                    "the passthrough line alone follows the skills line"
                );
            }
            assert_eq!(lines.len(), 1 + presets.len(), "{lines:?}");
            let skills_suffix = format!(
                " (v9.8.7) -> {}",
                expected_skills_dir(*agent, &custom, home.path()).display()
            );
            assert!(
                lines[0].starts_with(&format!("{}: installed ", agent.name()))
                    && !lines[0].contains("installed preset")
                    && lines[0].ends_with(&skills_suffix),
                "skills line first: {lines:?}"
            );
            for ((_, target), line) in presets.iter().zip(&lines[1..]) {
                assert_eq!(
                    *line,
                    format!(
                        "{}: installed preset (v9.8.7) -> {}",
                        agent.name(),
                        target.display()
                    )
                );
            }
        }
    }

    #[test]
    fn opencode_preset_failure_fails_installation_and_records_nothing() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let base = home.path().join("preset-base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("agents"), "blocked parent").unwrap();
        let paths = agent_paths(
            &|name| (name == "OPENCODE_CONFIG_DIR").then(|| base.display().to_string()),
            home.path(),
            CodingAgent::Opencode,
        )
        .unwrap();
        let mut output = Vec::new();
        let failure = install_agent(
            &mut conn,
            CodingAgent::Opencode,
            &paths,
            "1.0.0",
            None,
            &mut output,
        )
        .expect_err("a blocked preset target fails installation");
        assert!(
            failure.message().starts_with(&format!(
                "failed to install preset into {}: ",
                base.join("agents/cafleet.md").display()
            )),
            "{}",
            failure.message()
        );
        assert_eq!(asset_install_count(&conn), 0);
        let lines = output_lines(output);
        assert_eq!(lines.len(), 1, "only the completed skills line: {lines:?}");
        assert!(
            lines[0].starts_with("opencode: installed ")
                && lines[0].ends_with(&format!(
                    "-> {}",
                    home.path().join(".config/opencode/skills").display()
                )),
            "{lines:?}"
        );
        assert!(!base.join("agents/cafleet-ask.md").exists());
    }

    #[test]
    fn opencode_default_installation_uses_the_native_config_directory_for_skills_and_presets() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let paths = agent_paths(&|_| None, home.path(), CodingAgent::Opencode).unwrap();
        let base = home.path().join(".config/opencode");
        assert_eq!(paths.identity, base.display().to_string());
        let mut output = Vec::new();
        install_agent(
            &mut conn,
            CodingAgent::Opencode,
            &paths,
            "1.0.0",
            None,
            &mut output,
        )
        .unwrap();
        for (source, target) in expected_presets(CodingAgent::Opencode, &base) {
            assert_eq!(
                std::fs::read(&target).unwrap(),
                lookup(PRESETS, source).unwrap(),
                "{}",
                target.display()
            );
        }
        for (relative, bytes) in SKILLS {
            assert_eq!(
                std::fs::read(base.join("skills").join(relative)).unwrap(),
                *bytes
            );
        }
        let lines = output_lines(output);
        assert_eq!(
            lines[1..],
            [
                format!(
                    "opencode: installed preset (v1.0.0) -> {}",
                    base.join("agents/cafleet.md").display()
                ),
                format!(
                    "opencode: installed preset (v1.0.0) -> {}",
                    base.join("agents/cafleet-ask.md").display()
                ),
            ]
        );
        let recorded: Vec<String> = conn
            .prepare("SELECT path FROM asset_installs WHERE coding_agent='opencode'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(recorded, [base.display().to_string()]);
    }

    const OPENCODE_PRESETS: [&str; 2] = ["opencode/cafleet.md", "opencode/cafleet-ask.md"];

    fn key_order(value: &serde_json::Value) -> Vec<String> {
        value.as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn codex_rules_preset_is_the_single_broad_cafleet_allow() {
        assert_eq!(
            std::str::from_utf8(lookup(PRESETS, "codex/cafleet.rules").unwrap()).unwrap(),
            "prefix_rule(pattern = [\"cafleet\"], decision = \"allow\")\n"
        );
    }

    #[test]
    fn opencode_shell_maps_end_with_the_broad_cafleet_allow_and_hold_no_director_only_entry() {
        for preset in OPENCODE_PRESETS {
            let (frontmatter, _) = frontmatter_and_body(preset);
            let shell = &frontmatter["permission"]["shell"];
            let keys = key_order(shell);
            assert_eq!(keys.first().map(String::as_str), Some("*"), "{preset}");
            assert_eq!(
                keys.last().map(String::as_str),
                Some("cafleet *"),
                "{preset}"
            );
            assert_eq!(shell["cafleet *"], "allow", "{preset}");
            for key in &keys {
                assert!(
                    !["cafleet member", "cafleet permission", "cafleet input"]
                        .iter()
                        .any(|prefix| key.starts_with(prefix)),
                    "{preset}: {key}"
                );
                if key != "*" {
                    assert_eq!(shell[key], "allow", "{preset}: {key}");
                }
            }
        }
    }

    #[test]
    fn cafleet_ask_preset_frontmatter_differs_only_in_description_and_shell_base() {
        let (default, _) = frontmatter_and_body("opencode/cafleet.md");
        let (ask, _) = frontmatter_and_body("opencode/cafleet-ask.md");
        assert_eq!(
            ask["description"],
            "CAFleet-spawned member whose unmatched shell commands open a native permission form for the Director to answer."
        );
        assert_eq!(ask["permission"]["shell"]["*"], "ask");
        assert_eq!(default["permission"]["shell"]["*"], "deny");
        assert_eq!(
            key_order(&ask["permission"]["shell"]),
            key_order(&default["permission"]["shell"])
        );
        assert_eq!(key_order(&ask), key_order(&default));
        assert_eq!(
            key_order(&ask["permission"]),
            key_order(&default["permission"])
        );
        let mut normalized = ask.clone();
        normalized["description"] = default["description"].clone();
        normalized["permission"]["shell"]["*"] = "deny".into();
        assert_eq!(normalized, default);
    }

    #[test]
    fn opencode_preset_bodies_hold_three_unwrapped_paragraphs_and_one_trailing_newline() {
        for preset in OPENCODE_PRESETS {
            let (_, body) = frontmatter_and_body(preset);
            assert!(
                body.ends_with('\n') && !body.ends_with("\n\n"),
                "{preset}: {body:?}"
            );
            let blocks = paragraphs(&body);
            assert_eq!(blocks[0], "# CAFleet member agent", "{preset}");
            assert_eq!(blocks.len(), 4, "{preset}: {blocks:?}");
            for paragraph in &blocks[1..] {
                assert!(!paragraph.contains('\n'), "{preset}: {paragraph:?}");
            }
        }
    }

    #[test]
    fn opencode_preset_authority_paragraph_routes_director_only_needs_to_the_director() {
        let sentence = "Your role and assignment, not the broad `cafleet *` pattern, decide which CAFleet commands you run; Director-only commands refuse inside a member and you send those needs to the Director.";
        for preset in OPENCODE_PRESETS {
            let (_, body) = frontmatter_and_body(preset);
            let authority = &paragraphs(&body)[2];
            assert!(authority.contains(sentence), "{preset}: {authority}");
            assert!(
                authority.contains("Git write authority"),
                "{preset} keeps the Git-write authority clause: {authority}"
            );
            assert!(
                !body.contains("Broad tool patterns do not grant Director-only actions"),
                "{preset}: {body}"
            );
        }
    }

    #[test]
    fn cafleet_ask_preset_body_replaces_only_the_permission_paragraph() {
        let (_, default) = frontmatter_and_body("opencode/cafleet.md");
        let (_, ask) = frontmatter_and_body("opencode/cafleet-ask.md");
        let default = paragraphs(&default);
        let ask = paragraphs(&ask);
        assert_eq!(ask.len(), default.len());
        let permission = default
            .iter()
            .position(|paragraph| paragraph.starts_with("You are a CAFleet member spawned"))
            .expect("the cafleet preset has a permission paragraph");
        assert_eq!(
            ask[permission],
            "You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell allows its listed patterns, including `cafleet *`, the listed Git commands, inspection utilities and CAFleet mise tasks, and asks for every other command. An asked command opens a native permission form; CAFleet notifies the Director, who answers it, and you continue from the result OpenCode returns. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials."
        );
        for (index, (actual, expected)) in ask.iter().zip(&default).enumerate() {
            if index != permission {
                assert_eq!(actual, expected, "paragraph {index}");
            }
        }
    }

    mod hermes {
        use crate::assets::tests::*;
        use serde_json::json;

        struct Installed {
            result: Result<(), CafleetError>,
            lines: Vec<String>,
            rows: i64,
            base: PathBuf,
            config: PathBuf,
            skills: PathBuf,
        }

        fn after_path(calls: &[Vec<String>]) -> Vec<Vec<String>> {
            assert_eq!(
                calls.first(),
                Some(&FakeHermes::path_call()),
                "setup resolves the config path first"
            );
            calls[1..].to_vec()
        }

        fn install(fake: Option<&mut FakeHermes>) -> Installed {
            let home = tempfile::tempdir().unwrap();
            let mut conn = crate::db::test_support::migrated_conn(&home);
            let paths = agent_paths(&|_| None, home.path(), CodingAgent::Hermes).unwrap();
            let mut output = Vec::new();
            let result = install_agent(
                &mut conn,
                CodingAgent::Hermes,
                &paths,
                "1.2.3",
                fake.map(|fake| fake as &mut dyn HermesConfigRunner),
                &mut output,
            );
            let base = home.path().join(".hermes");
            let skills = base.join("skills");
            for (relative, bytes) in SKILLS {
                assert_eq!(std::fs::read(skills.join(relative)).unwrap(), *bytes);
            }
            Installed {
                result,
                lines: output_lines(output),
                rows: asset_install_count(&conn),
                base,
                config: PathBuf::from(FAKE_CONFIG_PATH),
                skills,
            }
        }

        fn skills_line(installed: &Installed) -> String {
            let roots = SKILLS
                .iter()
                .filter_map(|(path, _)| {
                    path.strip_suffix("/SKILL.md")
                        .filter(|root| !root.contains('/'))
                })
                .collect::<Vec<_>>();
            assert!(roots.contains(&"cafleet") && roots.contains(&"cafleet-design-doc"));
            format!(
                "hermes: installed {} (v1.2.3) -> {}",
                roots.join(", "),
                installed.skills.display()
            )
        }

        fn passthrough_held_line(installed: &Installed) -> String {
            format!(
                "hermes: terminal.client_env_passthrough already holds CAFleet's member names -> {}",
                installed.config.display()
            )
        }

        fn passthrough_added_line(installed: &Installed, added: &[&str]) -> String {
            format!(
                "hermes: added terminal.client_env_passthrough names -> {}: {}",
                installed.config.display(),
                added.join(", ")
            )
        }

        fn assert_failed(installed: &Installed, cause: Option<&str>) {
            assert_failed_into(installed, &installed.config, cause);
        }

        fn assert_failed_into(installed: &Installed, target: &Path, cause: Option<&str>) {
            let failure = installed
                .result
                .as_ref()
                .expect_err("the passthrough merge fails setup for hermes");
            assert!(matches!(failure, CafleetError::App(_)));
            assert_eq!(failure.exit_code(), 1);
            let prefix = format!(
                "failed to install hermes terminal.client_env_passthrough names into {}: ",
                target.display()
            );
            let actual = failure
                .message()
                .strip_prefix(&prefix)
                .unwrap_or_else(|| panic!("{}", failure.message()))
                .to_owned();
            match cause {
                Some(cause) => assert_eq!(actual, cause),
                None => assert!(!actual.is_empty()),
            }
            assert_eq!(
                installed.rows, 0,
                "no asset_installs row for a failed merge"
            );
            assert_eq!(installed.lines, [skills_line(installed)]);
        }

        #[test]
        fn hermes_paths_use_the_home_skills_dir_without_presets() {
            let home = Path::new("/home/tester");
            let default = agent_paths(&|_| None, home, CodingAgent::Hermes).unwrap();
            assert_eq!(default.identity, "/home/tester/.hermes");
            assert_eq!(
                default.skills_dir,
                PathBuf::from("/home/tester/.hermes/skills")
            );
            assert!(default.presets.is_empty());
            let custom = agent_paths(
                &|name| (name == "HERMES_HOME").then(|| "/srv/hermes".into()),
                home,
                CodingAgent::Hermes,
            )
            .unwrap();
            assert_eq!(custom.identity, "/srv/hermes");
            assert_eq!(custom.skills_dir, PathBuf::from("/srv/hermes/skills"));
            assert!(custom.presets.is_empty());
            assert_eq!(
                agent_paths(
                    &|name| (name == "HERMES_HOME").then(|| "relative".into()),
                    home,
                    CodingAgent::Hermes
                )
                .err()
                .expect("a relative HERMES_HOME fails")
                .message(),
                "HERMES_HOME must be an absolute path (got 'relative')"
            );
        }

        #[test]
        fn hermes_setup_reads_only_the_config_path_and_the_passthrough() {
            let mut fake = FakeHermes::complete();
            let installed = install(Some(&mut fake));
            installed.result.as_ref().unwrap();
            assert_eq!(
                fake.calls,
                [FakeHermes::path_call(), FakeHermes::passthrough_get_call()],
                "setup issues no approvals.deny command"
            );
            assert_eq!(
                installed.lines,
                [skills_line(&installed), passthrough_held_line(&installed)]
            );
            assert_ne!(installed.config, installed.base.join("config.yaml"));
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_appends_only_the_missing_passthrough_names_after_existing_entries() {
            let mut fake = FakeHermes::holding(&["PATH", "TMUX", "CAFLEET_DATABASE_URL"]);
            let installed = install(Some(&mut fake));
            installed.result.as_ref().unwrap();
            let added = HERMES_CLIENT_ENV_PASSTHROUGH
                .into_iter()
                .filter(|name| !["TMUX", "CAFLEET_DATABASE_URL"].contains(name))
                .collect::<Vec<_>>();
            assert_eq!(added[0], "CAFLEET_MEMBER_ID");
            let mut merged = vec!["PATH", "TMUX", "CAFLEET_DATABASE_URL"];
            merged.extend(&added);
            assert_eq!(
                after_path(&fake.calls),
                [
                    FakeHermes::passthrough_get_call(),
                    FakeHermes::passthrough_set_call(&merged),
                    FakeHermes::passthrough_get_call(),
                ]
            );
            assert_eq!(fake.passthrough, json!(merged));
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    passthrough_added_line(&installed, &added),
                ]
            );
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_appends_the_member_id_to_an_existing_installation_list() {
            let previous = HERMES_CLIENT_ENV_PASSTHROUGH
                .into_iter()
                .filter(|name| *name != "CAFLEET_MEMBER_ID")
                .collect::<Vec<_>>();
            let mut fake = FakeHermes::holding(&previous);
            let installed = install(Some(&mut fake));
            installed.result.as_ref().unwrap();
            let mut merged = previous.clone();
            merged.push("CAFLEET_MEMBER_ID");
            assert_eq!(fake.passthrough, json!(merged));
            assert_eq!(
                installed.lines[1],
                passthrough_added_line(&installed, &["CAFLEET_MEMBER_ID"])
            );
        }

        #[test]
        fn hermes_setup_writes_no_passthrough_when_every_member_name_is_already_present() {
            let mut existing = vec!["PATH"];
            existing.extend(HERMES_CLIENT_ENV_PASSTHROUGH.iter().rev());
            let mut fake = FakeHermes::holding(&existing);
            let installed = install(Some(&mut fake));
            installed.result.as_ref().unwrap();
            assert_eq!(
                after_path(&fake.calls),
                [FakeHermes::passthrough_get_call()]
            );
            assert_eq!(fake.passthrough, json!(existing));
            assert_eq!(
                installed.lines,
                [skills_line(&installed), passthrough_held_line(&installed)]
            );
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_is_idempotent_from_an_empty_passthrough_list() {
            let mut fake = FakeHermes::holding(&[]);
            let first = install(Some(&mut fake));
            first.result.as_ref().unwrap();
            assert_eq!(fake.passthrough, json!(HERMES_CLIENT_ENV_PASSTHROUGH));
            assert_eq!(
                first.lines[1],
                passthrough_added_line(&first, &HERMES_CLIENT_ENV_PASSTHROUGH)
            );
            fake.calls.clear();
            let second = install(Some(&mut fake));
            second.result.as_ref().unwrap();
            assert_eq!(
                after_path(&fake.calls),
                [FakeHermes::passthrough_get_call()]
            );
            assert_eq!(fake.passthrough, json!(HERMES_CLIENT_ENV_PASSTHROUGH));
            assert_eq!(second.lines[1], passthrough_held_line(&second));
        }

        #[test]
        fn hermes_setup_compares_passthrough_names_by_exact_string() {
            let near = ["cafleet_database_url", " TMUX", "TMUX_PANE ", "Herdr_Env"];
            let mut fake = FakeHermes::holding(&near);
            let installed = install(Some(&mut fake));
            installed.result.as_ref().unwrap();
            let mut merged = near.to_vec();
            merged.extend(HERMES_CLIENT_ENV_PASSTHROUGH);
            assert_eq!(
                after_path(&fake.calls)[1],
                FakeHermes::passthrough_set_call(&merged)
            );
        }

        #[test]
        fn hermes_setup_fails_for_a_passthrough_value_that_is_not_an_array_of_strings() {
            for passthrough in [
                json!(null),
                json!("CAFLEET_DATABASE_URL"),
                json!({"0": "CAFLEET_DATABASE_URL"}),
                json!([1, 2]),
                json!(["CAFLEET_DATABASE_URL", null]),
            ] {
                let mut fake = FakeHermes::with_passthrough(passthrough.clone());
                let installed = install(Some(&mut fake));
                assert_failed(&installed, None);
                assert_eq!(
                    after_path(&fake.calls),
                    [FakeHermes::passthrough_get_call()],
                    "{passthrough}"
                );
            }
        }

        #[test]
        fn hermes_setup_fails_when_the_passthrough_cannot_be_read() {
            let mut fake = FakeHermes::holding(&[]);
            fake.get_failure = Some("exited with exit status: 2".into());
            let installed = install(Some(&mut fake));
            assert_failed(&installed, Some("exited with exit status: 2"));
            assert_eq!(
                after_path(&fake.calls),
                [FakeHermes::passthrough_get_call()]
            );
        }

        #[test]
        fn hermes_setup_fails_when_the_passthrough_write_fails() {
            let mut fake = FakeHermes::holding(&["PATH"]);
            fake.set_failure =
                Some("managed key terminal.client_env_passthrough cannot be set".into());
            let installed = install(Some(&mut fake));
            assert_failed(
                &installed,
                Some("managed key terminal.client_env_passthrough cannot be set"),
            );
            assert_eq!(after_path(&fake.calls).len(), 2);
            assert_eq!(fake.passthrough, json!(["PATH"]));
        }

        #[test]
        fn hermes_setup_fails_when_the_reread_passthrough_lacks_a_name() {
            let mut fake = FakeHermes::holding(&["PATH"]);
            fake.ignore_set = true;
            let installed = install(Some(&mut fake));
            assert_failed(&installed, None);
            let mut merged = vec!["PATH"];
            merged.extend(HERMES_CLIENT_ENV_PASSTHROUGH);
            assert_eq!(
                after_path(&fake.calls),
                [
                    FakeHermes::passthrough_get_call(),
                    FakeHermes::passthrough_set_call(&merged),
                    FakeHermes::passthrough_get_call()
                ]
            );
        }

        #[test]
        fn explicit_hermes_setup_without_the_binary_fails_with_the_path_cause() {
            let mut fake = FakeHermes::holding(&[]);
            fake.available = false;
            let installed = install(Some(&mut fake));
            assert_failed_into(
                &installed,
                &installed.base,
                Some("binary hermes not found on PATH"),
            );
            assert_eq!(fake.calls, [FakeHermes::path_call()]);
        }

        #[test]
        fn hermes_setup_fails_into_the_resolved_base_when_the_config_path_is_unavailable() {
            let mut fake = FakeHermes::holding(&[]);
            fake.path_failure = Some("exited with exit status: 1".into());
            let installed = install(Some(&mut fake));
            assert_failed_into(
                &installed,
                &installed.base,
                Some("exited with exit status: 1"),
            );
            assert_eq!(fake.calls, [FakeHermes::path_call()]);
        }

        #[test]
        fn hermes_setup_without_a_runner_installs_skills_records_the_row_and_reports_the_skip() {
            let installed = install(None);
            installed.result.as_ref().unwrap();
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    "hermes: skipped terminal.client_env_passthrough names (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes".to_owned(),
                ]
            );
            assert_eq!(installed.rows, 1);
        }
    }
}
