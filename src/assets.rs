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
    use crate::assets::HermesOutput;
    use crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH;
    use crate::error::CafleetError;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    pub(crate) const PASSTHROUGH_KEY: &str = "terminal.client_env_passthrough";
    pub(crate) const DISABLED_KEY: &str = "plugins.disabled";
    pub(crate) const ENABLED_KEY: &str = "plugins.enabled";
    pub(crate) const INJECTION_KEY: &str = "plugins.entries.cafleet.allow_gateway_injection";
    /// Hermes prints this unrelated line to stderr on every command.
    pub(crate) const OUT_OF_SYNC: &str =
        "⚠ install out of sync with its lockfile; run 'hermes update'";

    pub(crate) enum Reply {
        Exit {
            code: i32,
            stdout: String,
            stderr: String,
        },
        /// A spawn failure or an expired deadline.
        Fail(String),
    }

    impl Reply {
        pub(crate) fn exit(code: i32, stderr: &str) -> Self {
            Self::Exit {
                code,
                stdout: String::new(),
                stderr: stderr.to_owned(),
            }
        }
    }

    /// Serves `config path`, `config get --json KEY` and `config set KEY VALUE`
    /// from an in-memory configuration. A scripted reply answers the next call
    /// with its exact argv once; any other command panics.
    pub(crate) struct FakeHermes {
        pub(crate) available: bool,
        pub(crate) config: BTreeMap<String, Value>,
        pub(crate) scripted: Vec<(Vec<String>, Reply)>,
        pub(crate) ignored_sets: Vec<&'static str>,
        pub(crate) calls: Vec<Vec<String>>,
        profile: tempfile::TempDir,
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
                config: BTreeMap::from([(PASSTHROUGH_KEY.to_owned(), passthrough)]),
                scripted: Vec::new(),
                ignored_sets: Vec::new(),
                calls: Vec::new(),
                profile: tempfile::tempdir().unwrap(),
            }
        }
        pub(crate) fn enabled() -> Self {
            let mut fake = Self::complete();
            fake.set(ENABLED_KEY, serde_json::json!(["cafleet"]));
            fake.set(INJECTION_KEY, Value::Bool(true));
            fake
        }
        pub(crate) fn set(&mut self, key: &str, value: Value) {
            self.config.insert(key.to_owned(), value);
        }
        pub(crate) fn value(&self, key: &str) -> Option<&Value> {
            self.config.get(key)
        }
        pub(crate) fn reply(&mut self, call: Vec<String>, reply: Reply) {
            self.scripted.push((call, reply));
        }
        /// An active profile re-homes Hermes, so the printed path lies outside `HERMES_HOME`.
        pub(crate) fn config_path(&self) -> PathBuf {
            self.profile.path().join("profiles/work/config.yaml")
        }
        pub(crate) fn plugin_dir(&self) -> PathBuf {
            self.profile.path().join("profiles/work/plugins/cafleet")
        }
        pub(crate) fn path_call() -> Vec<String> {
            vec!["config".into(), "path".into()]
        }
        pub(crate) fn get_call(key: &str) -> Vec<String> {
            ["config", "get", "--json", key].map(str::to_owned).to_vec()
        }
        pub(crate) fn set_call(key: &str, value: &str) -> Vec<String> {
            ["config", "set", key, value].map(str::to_owned).to_vec()
        }
        pub(crate) fn passthrough_get_call() -> Vec<String> {
            Self::get_call(PASSTHROUGH_KEY)
        }
        pub(crate) fn passthrough_set_call(names: &[&str]) -> Vec<String> {
            Self::set_call(PASSTHROUGH_KEY, &serde_json::to_string(names).unwrap())
        }
    }

    fn output(exit_code: i32, stdout: String, diagnostic: &str) -> HermesOutput {
        HermesOutput {
            exit_code,
            stdout,
            stderr: format!("{OUT_OF_SYNC}\n{diagnostic}"),
        }
    }

    fn stored_value(value: &str) -> Value {
        assert!(
            !value.contains('\n') && !value.contains(", "),
            "compact JSON: {value}"
        );
        serde_json::from_str(value)
            .unwrap_or_else(|cause| panic!("setup writes JSON values: {value}: {cause}"))
    }

    impl HermesConfigRunner for FakeHermes {
        fn binary_available(&self) -> bool {
            self.available
        }
        fn run(&mut self, args: &[String]) -> Result<HermesOutput, CafleetError> {
            self.calls.push(args.to_vec());
            if !self.available {
                return Err(CafleetError::App("binary hermes not found on PATH".into()));
            }
            if let Some(index) = self.scripted.iter().position(|(call, _)| call == args) {
                return match self.scripted.remove(index).1 {
                    Reply::Exit {
                        code,
                        stdout,
                        stderr,
                    } => Ok(HermesOutput {
                        exit_code: code,
                        stdout,
                        stderr,
                    }),
                    Reply::Fail(cause) => Err(CafleetError::App(cause)),
                };
            }
            match args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["config", "path"] => {
                    Ok(output(0, format!("{}\n", self.config_path().display()), ""))
                }
                ["config", "get", "--json", key] => Ok(match self.config.get(*key) {
                    Some(value) => output(0, format!("{value}\n"), ""),
                    None => output(1, String::new(), &format!("Config key not set: {key}\n")),
                }),
                ["config", "set", key, value] => {
                    let stored = stored_value(value);
                    if !self.ignored_sets.contains(key) {
                        self.config.insert((*key).to_owned(), stored);
                    }
                    Ok(output(0, String::new(), ""))
                }
                other => panic!("unexpected hermes command {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::test_support::DISABLED_KEY;
    use crate::assets::test_support::ENABLED_KEY;
    use crate::assets::test_support::FakeHermes;
    use crate::assets::test_support::INJECTION_KEY;
    use crate::assets::test_support::OUT_OF_SYNC;
    use crate::assets::test_support::PASSTHROUGH_KEY;
    use crate::assets::test_support::Reply;
    use crate::assets::*;
    use crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH;
    use std::os::unix::fs::PermissionsExt;
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

    fn extension_targets(
        agent: CodingAgent,
        base: &Path,
        fake: &FakeHermes,
    ) -> Vec<(&'static str, PathBuf)> {
        let (prefix, root) = match agent {
            CodingAgent::Claude | CodingAgent::Codex => return vec![],
            CodingAgent::Opencode => ("opencode/", base.join("plugins")),
            CodingAgent::Hermes => ("hermes/cafleet/", fake.plugin_dir()),
        };
        let targets = EXTENSIONS
            .iter()
            .filter_map(|(path, _)| {
                path.strip_prefix(prefix)
                    .map(|rest| (*path, root.join(rest)))
            })
            .collect::<Vec<_>>();
        assert!(!targets.is_empty(), "{} bundles an extension", agent.name());
        targets
    }

    fn extension_record(agent: CodingAgent, base: &Path, fake: &FakeHermes) -> Option<PathBuf> {
        match agent {
            CodingAgent::Claude | CodingAgent::Codex => None,
            CodingAgent::Opencode => Some(base.join("plugins/cafleet.executable")),
            CodingAgent::Hermes => Some(fake.plugin_dir().join("executable")),
        }
    }

    fn extension_line(agent: CodingAgent, version: &str, target: &Path) -> String {
        format!(
            "{}: installed extension (v{version}) -> {}",
            agent.name(),
            target.display()
        )
    }

    fn assert_executable_record(record: &Path) {
        let running = std::env::current_exe().unwrap().canonicalize().unwrap();
        assert!(running.is_absolute());
        assert_ne!(
            std::fs::metadata(&running).unwrap().permissions().mode() & 0o111,
            0
        );
        assert_eq!(
            std::fs::read_to_string(record).unwrap(),
            format!("{}\n", running.display()),
            "{}",
            record.display()
        );
    }

    fn assert_embedded_extensions(targets: &[(&'static str, PathBuf)]) {
        for (source, target) in targets {
            assert_eq!(
                std::fs::read(target).unwrap(),
                lookup(EXTENSIONS, source).unwrap(),
                "{}",
                target.display()
            );
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
            let mut fake = FakeHermes::complete();
            let extensions = extension_targets(*agent, &custom, &fake);
            std::fs::create_dir_all(&skills_dir).unwrap();
            symlink(&external, skills_dir.join("cafleet")).unwrap();
            for (_, target) in &presets {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                symlink(external.join("keep"), target).unwrap();
            }
            match agent {
                CodingAgent::Opencode => {
                    for (_, target) in &extensions {
                        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                        symlink(external.join("keep"), target).unwrap();
                    }
                }
                CodingAgent::Hermes => {
                    let plugin_dir = fake.plugin_dir();
                    std::fs::create_dir_all(plugin_dir.parent().unwrap()).unwrap();
                    symlink(&external, &plugin_dir).unwrap();
                }
                CodingAgent::Claude | CodingAgent::Codex => {}
            }
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
                assert_embedded_extensions(&extensions);
                if let Some(record) = extension_record(*agent, &custom, &fake) {
                    assert_executable_record(&record);
                }
                assert_eq!(
                    std::fs::read(external.join("keep")).unwrap(),
                    b"operator bytes"
                );
                assert_eq!(
                    std::fs::read_dir(&external).unwrap().count(),
                    1,
                    "a replaced symlink's referent stays intact"
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
    fn installation_prints_skills_then_presets_then_extensions_then_hermes_merges() {
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
            let extensions = extension_targets(*agent, &custom, &fake);
            install_agent(
                &mut conn,
                *agent,
                &paths,
                "9.8.7",
                runner_for(*agent, &mut fake),
                &mut output,
            )
            .unwrap();
            let lines = output_lines(output);
            let mut expected = expected_presets(*agent, &custom)
                .iter()
                .map(|(_, target)| {
                    format!(
                        "{}: installed preset (v9.8.7) -> {}",
                        agent.name(),
                        target.display()
                    )
                })
                .collect::<Vec<_>>();
            match agent {
                CodingAgent::Opencode => expected.extend(
                    extensions
                        .iter()
                        .map(|(_, target)| extension_line(*agent, "9.8.7", target)),
                ),
                CodingAgent::Hermes => expected.extend([
                    extension_line(*agent, "9.8.7", &fake.plugin_dir()),
                    format!(
                        "hermes: terminal.client_env_passthrough already holds CAFleet's member names -> {}",
                        fake.config_path().display()
                    ),
                    format!(
                        "hermes: enabled plugin cafleet -> {}",
                        fake.config_path().display()
                    ),
                ]),
                CodingAgent::Claude | CodingAgent::Codex => {}
            }
            let skills_suffix = format!(
                " (v9.8.7) -> {}",
                expected_skills_dir(*agent, &custom, home.path()).display()
            );
            assert!(
                lines[0].starts_with(&format!("{}: installed ", agent.name()))
                    && !lines[0].contains("installed preset")
                    && !lines[0].contains("installed extension")
                    && lines[0].ends_with(&skills_suffix),
                "skills line first: {lines:?}"
            );
            assert_eq!(lines[1..], expected, "{}", agent.name());
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
        assert!(
            !base.join("plugins").exists(),
            "extensions install after presets"
        );
    }

    #[test]
    fn opencode_extension_failure_fails_installation_and_records_nothing() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let base = home.path().join("preset-base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("plugins"), "blocked parent").unwrap();
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
        .expect_err("a blocked extension target fails installation");
        assert_eq!(failure.exit_code(), 1);
        assert!(
            failure.message().starts_with(&format!(
                "failed to install extension into {}: ",
                base.join("plugins/cafleet.ts").display()
            )),
            "{}",
            failure.message()
        );
        assert_eq!(asset_install_count(&conn), 0);
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
            ],
            "no extension line for a failed extension"
        );
        assert_eq!(
            std::fs::read(base.join("plugins")).unwrap(),
            b"blocked parent"
        );
    }

    #[test]
    fn opencode_default_installation_uses_the_native_config_directory_for_every_asset() {
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
        let fake = FakeHermes::complete();
        let extensions = extension_targets(CodingAgent::Opencode, &base, &fake);
        assert_eq!(
            extensions
                .iter()
                .map(|(_, target)| target.clone())
                .collect::<Vec<_>>(),
            [base.join("plugins/cafleet.ts")]
        );
        assert_embedded_extensions(&extensions);
        assert_executable_record(&base.join("plugins/cafleet.executable"));
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
                extension_line(
                    CodingAgent::Opencode,
                    "1.0.0",
                    &base.join("plugins/cafleet.ts")
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

    #[test]
    fn opencode_rerun_replaces_a_tampered_extension_and_record() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let paths = agent_paths(&|_| None, home.path(), CodingAgent::Opencode).unwrap();
        let base = home.path().join(".config/opencode");
        let plugins = base.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("cafleet.ts"), "tampered").unwrap();
        std::fs::write(plugins.join("cafleet.executable"), "/nowhere/cafleet\n").unwrap();
        install_agent(
            &mut conn,
            CodingAgent::Opencode,
            &paths,
            "1.0.0",
            None,
            &mut Vec::<u8>::new(),
        )
        .unwrap();
        let fake = FakeHermes::complete();
        assert_embedded_extensions(&extension_targets(CodingAgent::Opencode, &base, &fake));
        assert_executable_record(&plugins.join("cafleet.executable"));
    }

    mod hermes {
        use crate::assets::tests::*;
        use serde_json::json;

        struct Installed {
            result: Result<(), CafleetError>,
            lines: Vec<String>,
            rows: i64,
            base: PathBuf,
            skills: PathBuf,
            _home: tempfile::TempDir,
        }

        enum Cause<'a> {
            Exact(&'a str),
            Contains(&'a str),
            Any,
        }

        fn install_with(runner: Option<&mut dyn HermesConfigRunner>) -> Installed {
            let home = tempfile::tempdir().unwrap();
            let mut conn = crate::db::test_support::migrated_conn(&home);
            let paths = agent_paths(&|_| None, home.path(), CodingAgent::Hermes).unwrap();
            let mut output = Vec::new();
            let result = install_agent(
                &mut conn,
                CodingAgent::Hermes,
                &paths,
                "1.2.3",
                runner,
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
                skills,
                _home: home,
            }
        }

        fn install(fake: &mut FakeHermes) -> Installed {
            install_with(Some(fake))
        }

        fn after_path(calls: &[Vec<String>]) -> Vec<Vec<String>> {
            assert_eq!(
                calls.first(),
                Some(&FakeHermes::path_call()),
                "setup resolves the config path first"
            );
            calls[1..].to_vec()
        }

        fn passthrough_calls(calls: &[Vec<String>]) -> Vec<Vec<String>> {
            after_path(calls)
                .into_iter()
                .filter(|call| call.iter().any(|arg| arg == PASSTHROUGH_KEY))
                .collect()
        }

        fn plugin_calls(calls: &[Vec<String>]) -> Vec<Vec<String>> {
            after_path(calls)
                .into_iter()
                .filter(|call| !call.iter().any(|arg| arg == PASSTHROUGH_KEY))
                .collect()
        }

        fn get(key: &str) -> Vec<String> {
            FakeHermes::get_call(key)
        }

        fn set(key: &str, value: &str) -> Vec<String> {
            FakeHermes::set_call(key, value)
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

        fn plugin_line(fake: &FakeHermes) -> String {
            extension_line(CodingAgent::Hermes, "1.2.3", &fake.plugin_dir())
        }

        fn passthrough_held_line(fake: &FakeHermes) -> String {
            format!(
                "hermes: terminal.client_env_passthrough already holds CAFleet's member names -> {}",
                fake.config_path().display()
            )
        }

        fn passthrough_added_line(fake: &FakeHermes, added: &[&str]) -> String {
            format!(
                "hermes: added terminal.client_env_passthrough names -> {}: {}",
                fake.config_path().display(),
                added.join(", ")
            )
        }

        fn enabled_line(fake: &FakeHermes) -> String {
            format!(
                "hermes: enabled plugin cafleet -> {}",
                fake.config_path().display()
            )
        }

        fn already_enabled_line(fake: &FakeHermes) -> String {
            format!(
                "hermes: plugin cafleet already enabled -> {}",
                fake.config_path().display()
            )
        }

        fn assert_failure(installed: &Installed, prefix: &str, cause: Cause<'_>) {
            let failure = installed
                .result
                .as_ref()
                .expect_err("the hermes step fails setup");
            assert!(matches!(failure, CafleetError::App(_)));
            assert_eq!(failure.exit_code(), 1);
            let actual = failure
                .message()
                .strip_prefix(prefix)
                .unwrap_or_else(|| panic!("{prefix:?}: {}", failure.message()))
                .to_owned();
            match cause {
                Cause::Exact(cause) => assert_eq!(actual, cause),
                Cause::Contains(cause) => assert!(actual.contains(cause), "{actual}"),
                Cause::Any => assert!(!actual.is_empty()),
            }
            assert_eq!(
                installed.rows, 0,
                "no asset_installs row for a failed setup"
            );
        }

        fn passthrough_prefix(target: &Path) -> String {
            format!(
                "failed to install hermes terminal.client_env_passthrough names into {}: ",
                target.display()
            )
        }

        fn plugin_prefix(fake: &FakeHermes) -> String {
            format!(
                "failed to enable hermes plugin cafleet in {}: ",
                fake.config_path().display()
            )
        }

        fn assert_passthrough_failed(installed: &Installed, fake: &FakeHermes, cause: Cause<'_>) {
            assert_failure(installed, &passthrough_prefix(&fake.config_path()), cause);
            assert_eq!(installed.lines, [skills_line(installed), plugin_line(fake)]);
            assert!(
                plugin_calls(&fake.calls).is_empty(),
                "the enablement merge follows the passthrough merge"
            );
        }

        fn assert_plugin_failed(installed: &Installed, fake: &FakeHermes, cause: Cause<'_>) {
            assert_failure(installed, &plugin_prefix(fake), cause);
            assert_eq!(
                installed.lines,
                [
                    skills_line(installed),
                    plugin_line(fake),
                    passthrough_held_line(fake)
                ]
            );
        }

        fn writes(calls: &[Vec<String>]) -> Vec<Vec<String>> {
            calls
                .iter()
                .filter(|call| call[1] == "set")
                .cloned()
                .collect()
        }

        fn files_under(root: &Path) -> Vec<PathBuf> {
            let mut files = Vec::new();
            for entry in std::fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    files.extend(files_under(&path));
                } else {
                    files.push(path);
                }
            }
            files.sort();
            files
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
        fn hermes_setup_installs_the_plugin_into_the_home_that_owns_the_config_path() {
            let mut fake = FakeHermes::complete();
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            assert_embedded_extensions(&extension_targets(
                CodingAgent::Hermes,
                &installed.base,
                &fake,
            ));
            assert_executable_record(&fake.plugin_dir().join("executable"));
            assert!(
                !installed.base.join("plugins").exists(),
                "the plugin follows the active profile, not HERMES_HOME"
            );
            assert_eq!(installed.lines[1], plugin_line(&fake));
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_replaces_a_stale_plugin_directory_with_exactly_the_embedded_files() {
            let mut fake = FakeHermes::complete();
            let plugin_dir = fake.plugin_dir();
            let targets = extension_targets(CodingAgent::Hermes, Path::new("/unused"), &fake);
            std::fs::create_dir_all(&plugin_dir).unwrap();
            std::fs::write(plugin_dir.join("stale.py"), "stale").unwrap();
            std::fs::write(plugin_dir.join("executable"), "/nowhere/cafleet\n").unwrap();
            for (_, target) in &targets {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, "tampered").unwrap();
            }
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            assert_embedded_extensions(&targets);
            assert_executable_record(&plugin_dir.join("executable"));
            let mut expected = targets
                .iter()
                .map(|(_, target)| target.clone())
                .collect::<Vec<_>>();
            expected.push(plugin_dir.join("executable"));
            expected.sort();
            assert_eq!(files_under(&plugin_dir), expected);
        }

        #[test]
        fn hermes_extension_failure_fails_setup_before_any_configuration_write() {
            let mut fake = FakeHermes::complete();
            let plugins = fake.plugin_dir().parent().unwrap().to_owned();
            std::fs::create_dir_all(plugins.parent().unwrap()).unwrap();
            std::fs::write(&plugins, "blocked parent").unwrap();
            let installed = install(&mut fake);
            let failure = installed
                .result
                .as_ref()
                .expect_err("a blocked plugin directory fails setup");
            assert_eq!(failure.exit_code(), 1);
            assert!(
                failure.message().starts_with(&format!(
                    "failed to install extension into {}: ",
                    fake.plugin_dir().display()
                )),
                "{}",
                failure.message()
            );
            assert_eq!(installed.rows, 0);
            assert_eq!(installed.lines, [skills_line(&installed)]);
            assert_eq!(
                fake.calls,
                [FakeHermes::path_call()],
                "the configuration merges follow the extension"
            );
        }

        #[test]
        fn hermes_setup_issues_the_complete_config_call_sequence() {
            let mut fake = FakeHermes::complete();
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            assert_eq!(
                fake.calls,
                [
                    FakeHermes::path_call(),
                    FakeHermes::passthrough_get_call(),
                    get(DISABLED_KEY),
                    get(ENABLED_KEY),
                    set(ENABLED_KEY, r#"["cafleet"]"#),
                    get(INJECTION_KEY),
                    set(INJECTION_KEY, "true"),
                    get(ENABLED_KEY),
                    get(INJECTION_KEY),
                ],
                "setup runs no 'hermes plugins enable' and no approvals.deny command"
            );
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    plugin_line(&fake),
                    passthrough_held_line(&fake),
                    enabled_line(&fake),
                ]
            );
            assert_eq!(fake.value(ENABLED_KEY), Some(&json!(["cafleet"])));
            assert_eq!(fake.value(INJECTION_KEY), Some(&json!(true)));
            assert_eq!(
                fake.value(DISABLED_KEY),
                None,
                "plugins.disabled is read only"
            );
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_appends_cafleet_after_the_existing_enabled_plugins() {
            let mut fake = FakeHermes::complete();
            fake.set(ENABLED_KEY, json!(["alpha", "beta"]));
            fake.set(DISABLED_KEY, json!(["gamma"]));
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            assert_eq!(
                plugin_calls(&fake.calls)[..3],
                [
                    get(DISABLED_KEY),
                    get(ENABLED_KEY),
                    set(ENABLED_KEY, r#"["alpha","beta","cafleet"]"#),
                ]
            );
            assert_eq!(
                fake.value(ENABLED_KEY),
                Some(&json!(["alpha", "beta", "cafleet"]))
            );
            assert_eq!(fake.value(DISABLED_KEY), Some(&json!(["gamma"])));
            assert_eq!(installed.lines[3], enabled_line(&fake));
        }

        #[test]
        fn hermes_setup_sets_gateway_injection_unless_it_is_exactly_true() {
            for value in [json!(false), json!("true"), json!(1), json!(null)] {
                let mut fake = FakeHermes::complete();
                fake.set(ENABLED_KEY, json!(["cafleet"]));
                fake.set(INJECTION_KEY, value.clone());
                let installed = install(&mut fake);
                installed.result.as_ref().unwrap();
                assert_eq!(
                    plugin_calls(&fake.calls),
                    [
                        get(DISABLED_KEY),
                        get(ENABLED_KEY),
                        get(INJECTION_KEY),
                        set(INJECTION_KEY, "true"),
                        get(ENABLED_KEY),
                        get(INJECTION_KEY),
                    ],
                    "{value}"
                );
                assert_eq!(fake.value(INJECTION_KEY), Some(&json!(true)));
                assert_eq!(fake.value(ENABLED_KEY), Some(&json!(["cafleet"])));
                assert_eq!(installed.lines[3], enabled_line(&fake), "{value}");
            }
        }

        #[test]
        fn hermes_setup_with_the_plugin_already_enabled_only_reads() {
            let mut fake = FakeHermes::enabled();
            fake.set(ENABLED_KEY, json!(["alpha", "cafleet"]));
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            assert_eq!(
                plugin_calls(&fake.calls),
                [
                    get(DISABLED_KEY),
                    get(ENABLED_KEY),
                    get(INJECTION_KEY),
                    get(ENABLED_KEY),
                    get(INJECTION_KEY),
                ]
            );
            assert_eq!(fake.value(ENABLED_KEY), Some(&json!(["alpha", "cafleet"])));
            assert_eq!(installed.lines[3], already_enabled_line(&fake));
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_enablement_is_idempotent() {
            let mut fake = FakeHermes::complete();
            let first = install(&mut fake);
            first.result.as_ref().unwrap();
            assert_eq!(first.lines[3], enabled_line(&fake));
            fake.calls.clear();
            let second = install(&mut fake);
            second.result.as_ref().unwrap();
            assert!(writes(&fake.calls).is_empty(), "{:?}", fake.calls);
            assert_eq!(second.lines[3], already_enabled_line(&fake));
            assert_eq!(fake.value(ENABLED_KEY), Some(&json!(["cafleet"])));
            assert_eq!(fake.value(INJECTION_KEY), Some(&json!(true)));
        }

        #[test]
        fn hermes_setup_treats_only_an_exact_not_set_line_as_an_absent_key() {
            for stderr in [
                format!("{OUT_OF_SYNC}\nConfig key not set: plugins.enabled\n"),
                format!("  Config key not set: plugins.enabled  \n{OUT_OF_SYNC}\n"),
            ] {
                let mut fake = FakeHermes::complete();
                fake.reply(get(ENABLED_KEY), Reply::exit(1, &stderr));
                let installed = install(&mut fake);
                installed.result.as_ref().unwrap();
                assert!(
                    writes(&fake.calls).contains(&set(ENABLED_KEY, r#"["cafleet"]"#)),
                    "{stderr:?}"
                );
            }
            for (key, code, stderr) in [
                (
                    ENABLED_KEY,
                    1,
                    format!("{OUT_OF_SYNC}\nConfig key not set: plugins.enabled.extra\n"),
                ),
                (
                    ENABLED_KEY,
                    1,
                    format!("{OUT_OF_SYNC}\nError: Config key not set: plugins.enabled\n"),
                ),
                (
                    ENABLED_KEY,
                    1,
                    format!("{OUT_OF_SYNC}\nConfig key not set: plugins.disabled\n"),
                ),
                (ENABLED_KEY, 1, format!("{OUT_OF_SYNC}\n")),
                (
                    ENABLED_KEY,
                    2,
                    "Config key not set: plugins.enabled\n".to_owned(),
                ),
                (
                    DISABLED_KEY,
                    1,
                    format!("{OUT_OF_SYNC}\nConfig key not set: plugins\n"),
                ),
                (
                    INJECTION_KEY,
                    1,
                    format!("{OUT_OF_SYNC}\nConfig key not set: plugins.entries.cafleet\n"),
                ),
            ] {
                let mut fake = FakeHermes::complete();
                fake.reply(get(key), Reply::exit(code, &stderr));
                let installed = install(&mut fake);
                assert_plugin_failed(&installed, &fake, Cause::Any);
                assert_eq!(
                    plugin_calls(&fake.calls).last(),
                    Some(&get(key)),
                    "{key} {code} {stderr:?}"
                );
                assert!(
                    !writes(&fake.calls).iter().any(|call| call[2] == key),
                    "{key} {stderr:?}"
                );
            }
        }

        #[test]
        fn hermes_setup_refuses_a_plugins_disabled_entry_for_cafleet() {
            for enabled in [None, Some(json!(["cafleet"]))] {
                let mut fake = FakeHermes::complete();
                fake.set(DISABLED_KEY, json!(["other", "cafleet"]));
                if let Some(enabled) = &enabled {
                    fake.set(ENABLED_KEY, enabled.clone());
                    fake.set(INJECTION_KEY, json!(true));
                }
                let installed = install(&mut fake);
                assert_plugin_failed(
                    &installed,
                    &fake,
                    Cause::Exact(
                        "plugins.disabled lists cafleet; remove it from plugins.disabled, then rerun 'cafleet setup --coding-agent hermes'",
                    ),
                );
                assert_eq!(plugin_calls(&fake.calls), [get(DISABLED_KEY)]);
                assert_eq!(fake.value(DISABLED_KEY), Some(&json!(["other", "cafleet"])));
                assert_eq!(fake.value(ENABLED_KEY), enabled.as_ref());
            }
        }

        #[test]
        fn hermes_setup_fails_for_plugin_lists_that_are_not_arrays_of_strings() {
            for (key, value) in [
                (DISABLED_KEY, json!("cafleet")),
                (DISABLED_KEY, json!({"cafleet": true})),
                (DISABLED_KEY, json!(["other", 1])),
                (DISABLED_KEY, json!(null)),
                (ENABLED_KEY, json!("cafleet")),
                (ENABLED_KEY, json!(null)),
                (ENABLED_KEY, json!(["cafleet", null])),
                (ENABLED_KEY, json!({})),
            ] {
                let mut fake = FakeHermes::complete();
                fake.set(key, value.clone());
                let installed = install(&mut fake);
                assert_plugin_failed(&installed, &fake, Cause::Any);
                assert!(writes(&fake.calls).is_empty(), "{key} {value}");
                assert_eq!(fake.value(key), Some(&value));
            }
        }

        #[test]
        fn hermes_setup_fails_with_the_cause_of_a_failed_plugin_command() {
            let mut fake = FakeHermes::complete();
            fake.reply(
                set(ENABLED_KEY, r#"["cafleet"]"#),
                Reply::exit(
                    2,
                    &format!("{OUT_OF_SYNC}\nconfig.yaml is locked by another process\n"),
                ),
            );
            let installed = install(&mut fake);
            assert_plugin_failed(
                &installed,
                &fake,
                Cause::Contains("config.yaml is locked by another process"),
            );
            assert_eq!(fake.value(ENABLED_KEY), None);
            assert!(!plugin_calls(&fake.calls).contains(&get(INJECTION_KEY)));

            let mut fake = FakeHermes::complete();
            fake.reply(
                get(DISABLED_KEY),
                Reply::exit(
                    3,
                    &format!("{OUT_OF_SYNC}\npermission denied: config.yaml\n"),
                ),
            );
            let installed = install(&mut fake);
            assert_plugin_failed(
                &installed,
                &fake,
                Cause::Contains("permission denied: config.yaml"),
            );
            assert_eq!(plugin_calls(&fake.calls), [get(DISABLED_KEY)]);

            let mut fake = FakeHermes::complete();
            fake.reply(
                get(INJECTION_KEY),
                Reply::Fail("did not finish within its deadline".into()),
            );
            let installed = install(&mut fake);
            assert_plugin_failed(
                &installed,
                &fake,
                Cause::Contains("did not finish within its deadline"),
            );
            assert!(
                !writes(&fake.calls)
                    .iter()
                    .any(|call| call[2] == INJECTION_KEY)
            );
        }

        #[test]
        fn hermes_setup_fails_when_the_reread_lacks_the_enablement() {
            for key in [ENABLED_KEY, INJECTION_KEY] {
                let mut fake = FakeHermes::complete();
                fake.ignored_sets.push(key);
                let installed = install(&mut fake);
                assert_plugin_failed(&installed, &fake, Cause::Any);
                let calls = plugin_calls(&fake.calls);
                assert!(
                    calls.starts_with(&[
                        get(DISABLED_KEY),
                        get(ENABLED_KEY),
                        set(ENABLED_KEY, r#"["cafleet"]"#),
                        get(INJECTION_KEY),
                        set(INJECTION_KEY, "true"),
                    ]),
                    "{key}: {calls:?}"
                );
                assert!(
                    calls[5..]
                        .iter()
                        .all(|call| [get(ENABLED_KEY), get(INJECTION_KEY)].contains(call)),
                    "{key}: only re-reads follow the writes: {calls:?}"
                );
            }
        }

        #[test]
        fn hermes_setup_appends_only_the_missing_passthrough_names_after_existing_entries() {
            let mut fake = FakeHermes::holding(&["PATH", "TMUX", "CAFLEET_DATABASE_URL"]);
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            let added = HERMES_CLIENT_ENV_PASSTHROUGH
                .into_iter()
                .filter(|name| !["TMUX", "CAFLEET_DATABASE_URL"].contains(name))
                .collect::<Vec<_>>();
            assert_eq!(added[0], "CAFLEET_MEMBER_ID");
            let mut merged = vec!["PATH", "TMUX", "CAFLEET_DATABASE_URL"];
            merged.extend(&added);
            assert_eq!(
                after_path(&fake.calls)[..3],
                [
                    FakeHermes::passthrough_get_call(),
                    FakeHermes::passthrough_set_call(&merged),
                    FakeHermes::passthrough_get_call(),
                ],
                "the passthrough merge precedes the enablement merge"
            );
            assert_eq!(fake.value(PASSTHROUGH_KEY), Some(&json!(merged)));
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    plugin_line(&fake),
                    passthrough_added_line(&fake, &added),
                    enabled_line(&fake),
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
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            let mut merged = previous.clone();
            merged.push("CAFLEET_MEMBER_ID");
            assert_eq!(fake.value(PASSTHROUGH_KEY), Some(&json!(merged)));
            assert_eq!(
                installed.lines[2],
                passthrough_added_line(&fake, &["CAFLEET_MEMBER_ID"])
            );
        }

        #[test]
        fn hermes_setup_writes_no_passthrough_when_every_member_name_is_already_present() {
            let mut existing = vec!["PATH"];
            existing.extend(HERMES_CLIENT_ENV_PASSTHROUGH.iter().rev());
            let mut fake = FakeHermes::holding(&existing);
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            assert_eq!(
                passthrough_calls(&fake.calls),
                [FakeHermes::passthrough_get_call()]
            );
            assert_eq!(fake.value(PASSTHROUGH_KEY), Some(&json!(existing)));
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    plugin_line(&fake),
                    passthrough_held_line(&fake),
                    enabled_line(&fake),
                ]
            );
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_is_idempotent_from_an_empty_passthrough_list() {
            let mut fake = FakeHermes::holding(&[]);
            let first = install(&mut fake);
            first.result.as_ref().unwrap();
            assert_eq!(
                fake.value(PASSTHROUGH_KEY),
                Some(&json!(HERMES_CLIENT_ENV_PASSTHROUGH))
            );
            assert_eq!(
                first.lines[2],
                passthrough_added_line(&fake, &HERMES_CLIENT_ENV_PASSTHROUGH)
            );
            fake.calls.clear();
            let second = install(&mut fake);
            second.result.as_ref().unwrap();
            assert_eq!(
                passthrough_calls(&fake.calls),
                [FakeHermes::passthrough_get_call()]
            );
            assert_eq!(
                fake.value(PASSTHROUGH_KEY),
                Some(&json!(HERMES_CLIENT_ENV_PASSTHROUGH))
            );
            assert_eq!(second.lines[2], passthrough_held_line(&fake));
        }

        #[test]
        fn hermes_setup_compares_passthrough_names_by_exact_string() {
            let near = ["cafleet_database_url", " TMUX", "TMUX_PANE ", "Herdr_Env"];
            let mut fake = FakeHermes::holding(&near);
            let installed = install(&mut fake);
            installed.result.as_ref().unwrap();
            let mut merged = near.to_vec();
            merged.extend(HERMES_CLIENT_ENV_PASSTHROUGH);
            assert_eq!(
                passthrough_calls(&fake.calls)[1],
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
                let installed = install(&mut fake);
                assert_passthrough_failed(&installed, &fake, Cause::Any);
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
            fake.reply(
                FakeHermes::passthrough_get_call(),
                Reply::Fail("did not finish within its deadline".into()),
            );
            let installed = install(&mut fake);
            assert_passthrough_failed(
                &installed,
                &fake,
                Cause::Exact("did not finish within its deadline"),
            );

            let mut fake = FakeHermes::holding(&[]);
            fake.reply(
                FakeHermes::passthrough_get_call(),
                Reply::exit(
                    2,
                    &format!("{OUT_OF_SYNC}\npermission denied reading config.yaml\n"),
                ),
            );
            let installed = install(&mut fake);
            assert_passthrough_failed(
                &installed,
                &fake,
                Cause::Contains("permission denied reading config.yaml"),
            );
            assert_eq!(
                after_path(&fake.calls),
                [FakeHermes::passthrough_get_call()]
            );
        }

        #[test]
        fn hermes_setup_treats_an_unset_passthrough_as_a_failed_read() {
            let mut fake = FakeHermes::holding(&[]);
            fake.config.remove(PASSTHROUGH_KEY);
            let installed = install(&mut fake);
            assert_passthrough_failed(
                &installed,
                &fake,
                Cause::Contains("Config key not set: terminal.client_env_passthrough"),
            );
            assert!(writes(&fake.calls).is_empty());
        }

        #[test]
        fn hermes_setup_fails_when_the_passthrough_write_fails() {
            let mut fake = FakeHermes::holding(&["PATH"]);
            let mut merged = vec!["PATH"];
            merged.extend(HERMES_CLIENT_ENV_PASSTHROUGH);
            fake.reply(
                FakeHermes::passthrough_set_call(&merged),
                Reply::exit(
                    1,
                    &format!(
                        "{OUT_OF_SYNC}\nmanaged key terminal.client_env_passthrough cannot be set\n"
                    ),
                ),
            );
            let installed = install(&mut fake);
            assert_passthrough_failed(
                &installed,
                &fake,
                Cause::Contains("managed key terminal.client_env_passthrough cannot be set"),
            );
            assert_eq!(after_path(&fake.calls).len(), 2);
            assert_eq!(fake.value(PASSTHROUGH_KEY), Some(&json!(["PATH"])));
        }

        #[test]
        fn hermes_setup_fails_when_the_reread_passthrough_lacks_a_name() {
            let mut fake = FakeHermes::holding(&["PATH"]);
            fake.ignored_sets.push(PASSTHROUGH_KEY);
            let installed = install(&mut fake);
            assert_passthrough_failed(&installed, &fake, Cause::Any);
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
            let installed = install(&mut fake);
            assert_failure(
                &installed,
                &passthrough_prefix(&installed.base),
                Cause::Exact("binary hermes not found on PATH"),
            );
            assert_eq!(installed.lines, [skills_line(&installed)]);
            assert_eq!(fake.calls, [FakeHermes::path_call()]);
            assert!(!fake.plugin_dir().exists());
        }

        #[test]
        fn hermes_setup_fails_into_the_resolved_base_when_the_config_path_is_unavailable() {
            for (reply, cause) in [
                (
                    Reply::Fail("did not finish within its deadline".into()),
                    Cause::Exact("did not finish within its deadline"),
                ),
                (
                    Reply::exit(1, &format!("{OUT_OF_SYNC}\nno configuration found\n")),
                    Cause::Contains("no configuration found"),
                ),
            ] {
                let mut fake = FakeHermes::holding(&[]);
                fake.reply(FakeHermes::path_call(), reply);
                let installed = install(&mut fake);
                assert_failure(&installed, &passthrough_prefix(&installed.base), cause);
                assert_eq!(installed.lines, [skills_line(&installed)]);
                assert_eq!(fake.calls, [FakeHermes::path_call()]);
                assert!(!fake.plugin_dir().exists());
                assert!(!installed.base.join("plugins").exists());
            }
        }

        #[test]
        fn hermes_setup_without_a_runner_installs_skills_records_the_row_and_reports_the_skip() {
            let installed = install_with(None);
            installed.result.as_ref().unwrap();
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    "hermes: skipped the plugin and terminal.client_env_passthrough names (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes".to_owned(),
                ]
            );
            assert_eq!(installed.rows, 1);
            assert!(!installed.base.join("plugins").exists());
        }
    }
}
