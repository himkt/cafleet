use crate::coding_agent::CodingAgent;
use crate::coding_agent::hermes::HermesConfig;
use crate::config::EnvLookup;
use crate::config::opencode_skills_base;
use crate::error::CafleetError;
use rusqlite::Connection;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub(crate) struct AgentPaths {
    pub(crate) identity: String,
    base: PathBuf,
    skills_dir: PathBuf,
    presets: Vec<(String, PathBuf)>,
    plugins_dir: Option<PathBuf>,
}

/// How setup reaches Hermes for the plugin configuration.
pub(crate) enum HermesSetup<'a> {
    Available(&'a dyn HermesConfig),
    /// `--coding-agent hermes` was given and no `hermes` is on `PATH`.
    MissingRequired,
    /// Plain setup found no `hermes` on `PATH`.
    Absent,
}

/// Each Hermes setup phase (the read batch, every write, the re-read) gets its own budget.
const HERMES_SETUP_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) const OPENCODE_EXTENSION_PREFIX: &str = "opencode/";
pub(crate) const OPENCODE_EXECUTABLE_RECORD: &str = "cafleet.executable";
pub(crate) const HERMES_EXTENSION_PREFIX: &str = "hermes/cafleet/";
pub(crate) const HERMES_PLUGIN_DIR: &str = "plugins/cafleet";
pub(crate) const HERMES_EXECUTABLE_RECORD: &str = "executable";

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
    let plugins_dir = (agent == CodingAgent::Opencode).then(|| opencode_plugins_dir(&base));
    Ok(AgentPaths {
        identity: base.display().to_string(),
        base,
        skills_dir,
        presets,
        plugins_dir,
    })
}

pub(crate) fn opencode_plugins_dir(preset_base: &Path) -> PathBuf {
    preset_base.join("plugins")
}

impl AgentPaths {
    pub(crate) fn base(&self) -> &Path {
        &self.base
    }
}

/// Every embedded extension file under `prefix`, with its path relative to the extension root.
pub(crate) fn embedded_extension(prefix: &str) -> Vec<(&'static str, &'static [u8])> {
    let files = EXTENSIONS
        .iter()
        .filter_map(|(path, bytes)| path.strip_prefix(prefix).map(|rest| (rest, *bytes)))
        .collect::<Vec<_>>();
    assert!(!files.is_empty(), "the {prefix} extension is embedded");
    files
}

pub(crate) fn install_agent(
    conn: &mut Connection,
    agent: CodingAgent,
    paths: &AgentPaths,
    version: &str,
    hermes: HermesSetup<'_>,
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
    let extension_line = |target: &Path| {
        format!(
            "{}: installed extension (v{version}) -> {}",
            agent.name(),
            target.display()
        )
    };
    if let Some(plugins) = &paths.plugins_dir {
        for target in install_opencode_extension(plugins)? {
            report(extension_line(&target))?;
        }
    }
    if agent == CodingAgent::Hermes {
        match hermes {
            HermesSetup::Available(hermes) => {
                let line = install_hermes_plugin(hermes, paths, &mut |plugin_dir| {
                    report(extension_line(plugin_dir))
                })?;
                report(line)?;
            }
            HermesSetup::MissingRequired => {
                return Err(extension_failure(
                    &paths.base,
                    "binary hermes not found on PATH",
                ));
            }
            HermesSetup::Absent => report("hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes".into())?,
        }
    }
    crate::broker::record_asset_install(conn, agent.name(), &paths.identity, version)?;
    Ok(())
}

fn extension_failure(target: &Path, cause: impl std::fmt::Display) -> CafleetError {
    CafleetError::App(format!(
        "failed to install extension into {}: {cause}",
        target.display()
    ))
}

/// Writes each OpenCode plugin file, then the executable record, and returns the plugin files.
fn install_opencode_extension(plugins: &Path) -> Result<Vec<PathBuf>, CafleetError> {
    let mut installed = Vec::new();
    for (relative, bytes) in embedded_extension(OPENCODE_EXTENSION_PREFIX) {
        let target = plugins.join(relative);
        let fail = |error: std::io::Error| extension_failure(&target, error);
        std::fs::create_dir_all(target.parent().expect("plugin file has a parent"))
            .map_err(fail)?;
        remove_existing_target(&target).map_err(fail)?;
        std::fs::write(&target, bytes).map_err(fail)?;
        installed.push(target);
    }
    write_executable_record(&plugins.join(OPENCODE_EXECUTABLE_RECORD))?;
    Ok(installed)
}

/// Replaces the Hermes plugin directory with exactly the embedded files and the executable record.
fn install_hermes_extension(plugin_dir: &Path) -> Result<(), CafleetError> {
    let fail = |error: std::io::Error| extension_failure(plugin_dir, error);
    remove_existing_target(plugin_dir).map_err(fail)?;
    std::fs::create_dir_all(plugin_dir).map_err(fail)?;
    for (relative, bytes) in embedded_extension(HERMES_EXTENSION_PREFIX) {
        let target = plugin_dir.join(relative);
        std::fs::create_dir_all(target.parent().expect("plugin file has a parent"))
            .map_err(fail)?;
        std::fs::write(&target, bytes).map_err(fail)?;
    }
    write_executable_record(&plugin_dir.join(HERMES_EXECUTABLE_RECORD))
}

fn write_executable_record(record: &Path) -> Result<(), CafleetError> {
    let running = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            extension_failure(
                record,
                format!("cannot resolve the running cafleet: {error}"),
            )
        })?;
    let fail = |error: std::io::Error| extension_failure(record, error);
    remove_existing_target(record).map_err(fail)?;
    std::fs::write(record, format!("{}\n", running.display())).map_err(fail)
}

/// Why an installed executable record is unusable, or `None` when it names an executable file.
pub(crate) fn executable_record_problem(record: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let text = match std::fs::read_to_string(record) {
        Ok(text) => text,
        Err(error) => return Some(error.to_string()),
    };
    let target = text.trim();
    if target.is_empty() {
        return Some("the record is empty".into());
    }
    match std::fs::metadata(target) {
        Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => None,
        Ok(_) => Some(format!("{target} is not an executable file")),
        Err(error) => Some(format!("{target}: {error}")),
    }
}

/// Why an installed extension file is unusable, or `None` when it equals the embedded bytes.
pub(crate) fn extension_file_problem(target: &Path, embedded: &[u8]) -> Option<String> {
    match std::fs::read(target) {
        Ok(bytes) if bytes == embedded => None,
        Ok(_) => Some("differs from the embedded copy".into()),
        Err(error) => Some(error.to_string()),
    }
}

/// Installs the Hermes extension and enables its plugin: one concurrent `config path` and
/// `plugins` read, the extension (reported through `installed` before the plugin is evaluated),
/// then the plugin writes. Returns the plugin setup line.
fn install_hermes_plugin(
    hermes: &dyn HermesConfig,
    paths: &AgentPaths,
    installed: &mut dyn FnMut(&Path) -> Result<(), CafleetError>,
) -> Result<String, CafleetError> {
    use crate::coding_agent::hermes::PLUGIN_NAME;
    let (config, plugins) = crate::coding_agent::hermes::read_batch(
        hermes,
        std::time::Instant::now() + HERMES_SETUP_BUDGET,
    );
    let (config, plugin_dir) = config
        .and_then(|config| {
            crate::coding_agent::hermes::plugin_dir(&config).map(|dir| (config, dir))
        })
        .map_err(|cause| extension_failure(&paths.base, cause))?;
    install_hermes_extension(&plugin_dir)?;
    installed(&plugin_dir)?;
    let fail = |cause: String| {
        CafleetError::App(format!(
            "failed to enable hermes plugin {PLUGIN_NAME} in {config}: {cause}"
        ))
    };
    let state = plugins
        .and_then(crate::coding_agent::hermes::plugin_state)
        .map_err(fail)?;
    let wrote = crate::coding_agent::hermes::enable_plugin(hermes, &state, HERMES_SETUP_BUDGET)
        .map_err(fail)?;
    Ok(if wrote {
        format!("hermes: enabled plugin {PLUGIN_NAME} -> {config}")
    } else {
        format!("hermes: plugin {PLUGIN_NAME} already enabled -> {config}")
    })
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
    use crate::coding_agent::hermes::HermesConfig;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::Condvar;
    use std::sync::Mutex;
    use std::time::Duration;
    use std::time::Instant;

    pub(crate) use crate::coding_agent::hermes::GATEWAY_INJECTION_KEY as INJECTION_KEY;
    pub(crate) use crate::coding_agent::hermes::PLUGINS_DISABLED_KEY as DISABLED_KEY;
    pub(crate) use crate::coding_agent::hermes::PLUGINS_ENABLED_KEY as ENABLED_KEY;
    pub(crate) use crate::coding_agent::hermes::PLUGINS_KEY;

    /// A scripted answer to one exact call.
    pub(crate) enum Reply {
        /// `config path` prints this path.
        Path(String),
        /// `config get` reads this value; `None` is *not set*.
        Value(Option<Value>),
        /// The call fails with this cause.
        Fail(String),
    }

    struct State {
        config: Value,
        scripted: Vec<(Vec<String>, Reply)>,
        ignored_sets: Vec<&'static str>,
        calls: Vec<(Vec<String>, Instant)>,
        arrived: usize,
    }

    /// Serves `config path`, `config get --json KEY` and `config set KEY VALUE` from an
    /// in-memory configuration tree. A scripted reply answers the next call with its exact
    /// argv once.
    pub(crate) struct FakeHermes {
        state: Mutex<State>,
        batch_arrivals: Condvar,
        rendezvous: bool,
        profile: tempfile::TempDir,
    }

    /// How long each half of a rendezvous batch waits for the other half.
    const RENDEZVOUS_WAIT: Duration = Duration::from_secs(2);

    impl FakeHermes {
        pub(crate) fn new() -> Self {
            Self {
                state: Mutex::new(State {
                    config: Value::Object(Default::default()),
                    scripted: Vec::new(),
                    ignored_sets: Vec::new(),
                    calls: Vec::new(),
                    arrived: 0,
                }),
                batch_arrivals: Condvar::new(),
                rendezvous: false,
                profile: tempfile::tempdir().unwrap(),
            }
        }

        pub(crate) fn enabled() -> Self {
            let mut fake = Self::new();
            fake.put(ENABLED_KEY, serde_json::json!(["cafleet"]));
            fake.put(INJECTION_KEY, Value::Bool(true));
            fake
        }

        /// Makes `config path` and the `plugins` read each wait until the other has started,
        /// so a serial batch is slower than [`RENDEZVOUS_WAIT`].
        pub(crate) fn rendezvous(mut self) -> Self {
            self.rendezvous = true;
            self
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().expect("no fake hermes call panicked")
        }

        /// Stores `value` at the dotted `key`, creating the parent objects.
        pub(crate) fn put(&mut self, key: &str, value: Value) {
            store(&mut self.state().config, key, value);
        }

        pub(crate) fn value(&self, key: &str) -> Option<Value> {
            read(&self.state().config, key)
        }

        pub(crate) fn reply(&mut self, call: Vec<String>, reply: Reply) {
            self.state().scripted.push((call, reply));
        }

        /// Accepts every set of `key` without storing it, like a managed scope that wins.
        pub(crate) fn ignore_sets(&mut self, key: &'static str) {
            self.state().ignored_sets.push(key);
        }

        pub(crate) fn calls(&self) -> Vec<Vec<String>> {
            self.state()
                .calls
                .iter()
                .map(|(call, _)| call.clone())
                .collect()
        }

        pub(crate) fn deadlines(&self) -> Vec<Instant> {
            self.state()
                .calls
                .iter()
                .map(|(_, deadline)| *deadline)
                .collect()
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

        /// Logs `call` and returns its scripted reply, if any.
        fn record(&self, call: Vec<String>, deadline: Instant) -> Option<Reply> {
            let batch = call == Self::path_call() || call == Self::get_call(PLUGINS_KEY);
            let mut state = self.state();
            state.calls.push((call.clone(), deadline));
            if self.rendezvous && batch {
                state.arrived += 1;
                self.batch_arrivals.notify_all();
                let (waited, _) = self
                    .batch_arrivals
                    .wait_timeout_while(state, RENDEZVOUS_WAIT, |state| state.arrived < 2)
                    .expect("no fake hermes call panicked");
                state = waited;
            }
            let index = state
                .scripted
                .iter()
                .position(|(scripted, _)| *scripted == call)?;
            Some(state.scripted.remove(index).1)
        }
    }

    fn store(config: &mut Value, key: &str, value: Value) {
        let mut node = config;
        let mut parts = key.split('.').peekable();
        while let Some(part) = parts.next() {
            let object = node
                .as_object_mut()
                .unwrap_or_else(|| panic!("{key} descends through an object"));
            if parts.peek().is_none() {
                object.insert(part.to_owned(), value);
                return;
            }
            node = object
                .entry(part)
                .or_insert_with(|| Value::Object(Default::default()));
        }
    }

    fn read(config: &Value, key: &str) -> Option<Value> {
        key.split('.')
            .try_fold(config, |node, part| node.get(part))
            .cloned()
    }

    impl HermesConfig for FakeHermes {
        fn config_path(&self, deadline: Instant) -> Result<String, String> {
            match self.record(Self::path_call(), deadline) {
                None => Ok(self.config_path().display().to_string()),
                Some(Reply::Path(path)) => Ok(path),
                Some(Reply::Fail(cause)) => Err(cause),
                Some(Reply::Value(_)) => panic!("config path answers a path"),
            }
        }

        fn get(&self, key: &str, deadline: Instant) -> Result<Option<Value>, String> {
            match self.record(Self::get_call(key), deadline) {
                None => Ok(read(&self.state().config, key)),
                Some(Reply::Value(value)) => Ok(value),
                Some(Reply::Fail(cause)) => Err(cause),
                Some(Reply::Path(_)) => panic!("config get answers a value"),
            }
        }

        fn set(&self, key: &str, value: &str, deadline: Instant) -> Result<(), String> {
            if let Some(reply) = self.record(Self::set_call(key, value), deadline) {
                return match reply {
                    Reply::Fail(cause) => Err(cause),
                    Reply::Path(_) | Reply::Value(_) => {
                        panic!("config set answers success or a cause")
                    }
                };
            }
            assert!(
                !value.contains('\n') && !value.contains(", "),
                "compact JSON: {value}"
            );
            let stored = serde_json::from_str(value)
                .unwrap_or_else(|cause| panic!("setup writes JSON values: {value}: {cause}"));
            let mut state = self.state();
            if !state.ignored_sets.contains(&key) {
                store(&mut state.config, key, stored);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::test_support::DISABLED_KEY;
    use crate::assets::test_support::ENABLED_KEY;
    use crate::assets::test_support::FakeHermes;
    use crate::assets::test_support::INJECTION_KEY;
    use crate::assets::test_support::PLUGINS_KEY;
    use crate::assets::test_support::Reply;
    use crate::assets::*;
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

    fn hermes_for(agent: CodingAgent, fake: &FakeHermes) -> HermesSetup<'_> {
        match agent {
            CodingAgent::Hermes => HermesSetup::Available(fake),
            _ => HermesSetup::Absent,
        }
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
            let fake = FakeHermes::new();
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
                    hermes_for(*agent, &fake),
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
                HermesSetup::Absent,
                &mut Vec::<u8>::new()
            )
            .is_err()
        );
        assert_eq!(asset_install_count(&conn), 0);
    }

    #[test]
    fn installation_prints_skills_then_presets_then_extensions_then_hermes_enablement() {
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
            let fake = FakeHermes::new();
            let extensions = extension_targets(*agent, &custom, &fake);
            install_agent(
                &mut conn,
                *agent,
                &paths,
                "9.8.7",
                hermes_for(*agent, &fake),
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
            HermesSetup::Absent,
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
            HermesSetup::Absent,
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
            HermesSetup::Absent,
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
        let fake = FakeHermes::new();
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
            HermesSetup::Absent,
            &mut Vec::<u8>::new(),
        )
        .unwrap();
        let fake = FakeHermes::new();
        assert_embedded_extensions(&extension_targets(CodingAgent::Opencode, &base, &fake));
        assert_executable_record(&plugins.join("cafleet.executable"));
    }

    mod hermes {
        use crate::assets::tests::*;
        use serde_json::json;
        use std::time::Duration;
        use std::time::Instant;

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
        }

        fn install_with(hermes: HermesSetup<'_>) -> Installed {
            let home = tempfile::tempdir().unwrap();
            let mut conn = crate::db::test_support::migrated_conn(&home);
            let paths = agent_paths(&|_| None, home.path(), CodingAgent::Hermes).unwrap();
            let mut output = Vec::new();
            let result = install_agent(
                &mut conn,
                CodingAgent::Hermes,
                &paths,
                "1.2.3",
                hermes,
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

        fn install(fake: &FakeHermes) -> Installed {
            install_with(HermesSetup::Available(fake))
        }

        /// The calls after the opening batch, which holds `config path` and the `plugins` read
        /// in either order.
        fn after_batch(calls: &[Vec<String>]) -> Vec<Vec<String>> {
            assert!(
                calls.len() >= 2,
                "setup opens with the read batch: {calls:?}"
            );
            let mut batch = calls[..2].to_vec();
            batch.sort();
            let mut expected = vec![FakeHermes::path_call(), get(PLUGINS_KEY)];
            expected.sort();
            assert_eq!(batch, expected, "setup opens with the read batch");
            calls[2..].to_vec()
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
            format!(
                "hermes: installed {} (v1.2.3) -> {}",
                roots.join(", "),
                installed.skills.display()
            )
        }

        fn plugin_line(fake: &FakeHermes) -> String {
            extension_line(CodingAgent::Hermes, "1.2.3", &fake.plugin_dir())
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
            }
            assert_eq!(
                installed.rows, 0,
                "no asset_installs row for a failed setup"
            );
        }

        fn extension_prefix(target: &Path) -> String {
            format!("failed to install extension into {}: ", target.display())
        }

        fn plugin_prefix(fake: &FakeHermes) -> String {
            format!(
                "failed to enable hermes plugin cafleet in {}: ",
                fake.config_path().display()
            )
        }

        fn assert_plugin_failed(installed: &Installed, fake: &FakeHermes, cause: Cause<'_>) {
            assert_failure(installed, &plugin_prefix(fake), cause);
            assert_eq!(installed.lines, [skills_line(installed), plugin_line(fake)]);
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
            let fake = FakeHermes::new();
            let installed = install(&fake);
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
            let fake = FakeHermes::new();
            let plugin_dir = fake.plugin_dir();
            let targets = extension_targets(CodingAgent::Hermes, Path::new("/unused"), &fake);
            std::fs::create_dir_all(&plugin_dir).unwrap();
            std::fs::write(plugin_dir.join("stale.py"), "stale").unwrap();
            std::fs::write(plugin_dir.join("executable"), "/nowhere/cafleet\n").unwrap();
            for (_, target) in &targets {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, "tampered").unwrap();
            }
            let installed = install(&fake);
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
            let fake = FakeHermes::new();
            let plugins = fake.plugin_dir().parent().unwrap().to_owned();
            std::fs::create_dir_all(plugins.parent().unwrap()).unwrap();
            std::fs::write(&plugins, "blocked parent").unwrap();
            let installed = install(&fake);
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
            assert!(
                after_batch(&fake.calls()).is_empty(),
                "the plugin writes follow the extension"
            );
        }

        #[test]
        fn hermes_setup_runs_its_two_reads_concurrently_under_one_shared_deadline() {
            let fake = FakeHermes::enabled().rendezvous();
            let started = Instant::now();
            let installed = install(&fake);
            let finished = Instant::now();
            installed.result.as_ref().unwrap();
            assert!(
                finished - started < Duration::from_secs(2),
                "a serial batch waits out the rendezvous: {:?}",
                finished - started
            );
            assert!(after_batch(&fake.calls()).is_empty());
            let deadlines = fake.deadlines();
            assert_eq!(deadlines[0], deadlines[1], "one deadline covers the batch");
            assert!(
                deadlines[0] >= started + Duration::from_secs(10)
                    && deadlines[0] <= finished + Duration::from_secs(10),
                "the batch gets ten seconds"
            );
        }

        #[test]
        fn hermes_setup_writes_serially_then_rereads_plugins_once_each_with_its_own_budget() {
            let fake = FakeHermes::new();
            let installed = install(&fake);
            installed.result.as_ref().unwrap();
            let calls = fake.calls();
            assert_eq!(
                after_batch(&calls),
                [
                    set(ENABLED_KEY, r#"["cafleet"]"#),
                    set(INJECTION_KEY, "true"),
                    get(PLUGINS_KEY),
                ],
                "setup runs no 'hermes plugins enable' and no approvals.deny command"
            );
            let deadlines = fake.deadlines();
            assert!(
                deadlines.windows(2).all(|pair| pair[0] <= pair[1]),
                "each later phase starts its own ten seconds: {deadlines:?}"
            );
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    plugin_line(&fake),
                    enabled_line(&fake),
                ]
            );
            assert_eq!(fake.value(ENABLED_KEY), Some(json!(["cafleet"])));
            assert_eq!(fake.value(INJECTION_KEY), Some(json!(true)));
            assert_eq!(
                fake.value(DISABLED_KEY),
                None,
                "plugins.disabled is read only"
            );
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_appends_cafleet_after_the_existing_enabled_plugins() {
            let mut fake = FakeHermes::new();
            fake.put(ENABLED_KEY, json!(["alpha", "beta"]));
            fake.put(DISABLED_KEY, json!(["gamma"]));
            let installed = install(&fake);
            installed.result.as_ref().unwrap();
            assert_eq!(
                after_batch(&fake.calls())[0],
                set(ENABLED_KEY, r#"["alpha","beta","cafleet"]"#)
            );
            assert_eq!(
                fake.value(ENABLED_KEY),
                Some(json!(["alpha", "beta", "cafleet"]))
            );
            assert_eq!(fake.value(DISABLED_KEY), Some(json!(["gamma"])));
            assert_eq!(installed.lines[2], enabled_line(&fake));
        }

        #[test]
        fn hermes_setup_sets_gateway_injection_unless_it_is_exactly_true() {
            for value in [json!(false), json!("true")] {
                let mut fake = FakeHermes::new();
                fake.put(ENABLED_KEY, json!(["cafleet"]));
                fake.put(INJECTION_KEY, value.clone());
                let installed = install(&fake);
                installed.result.as_ref().unwrap();
                assert_eq!(
                    after_batch(&fake.calls()),
                    [set(INJECTION_KEY, "true"), get(PLUGINS_KEY)],
                    "{value}"
                );
                assert_eq!(fake.value(INJECTION_KEY), Some(json!(true)));
                assert_eq!(fake.value(ENABLED_KEY), Some(json!(["cafleet"])));
                assert_eq!(installed.lines[2], enabled_line(&fake), "{value}");
            }
        }

        #[test]
        fn hermes_setup_with_the_plugin_already_enabled_only_reads() {
            let mut fake = FakeHermes::enabled();
            fake.put(ENABLED_KEY, json!(["alpha", "cafleet"]));
            let installed = install(&fake);
            installed.result.as_ref().unwrap();
            assert!(
                after_batch(&fake.calls()).is_empty(),
                "steady state performs only the two reads: {:?}",
                fake.calls()
            );
            assert_eq!(fake.value(ENABLED_KEY), Some(json!(["alpha", "cafleet"])));
            assert_eq!(installed.lines[2], already_enabled_line(&fake));
            assert_eq!(installed.rows, 1);
        }

        #[test]
        fn hermes_setup_treats_a_not_set_plugins_key_or_a_missing_member_as_unset() {
            for plugins in [
                None,
                Some(json!({})),
                Some(json!({"entries": {"cafleet": {}}})),
            ] {
                let mut fake = FakeHermes::new();
                fake.reply(get(PLUGINS_KEY), Reply::Value(plugins.clone()));
                let installed = install(&fake);
                installed.result.as_ref().unwrap();
                assert_eq!(
                    writes(&fake.calls()),
                    [
                        set(ENABLED_KEY, r#"["cafleet"]"#),
                        set(INJECTION_KEY, "true")
                    ],
                    "{plugins:?}"
                );
                assert_eq!(installed.lines[2], enabled_line(&fake), "{plugins:?}");
            }
        }

        #[test]
        fn hermes_setup_refuses_a_plugins_disabled_entry_for_cafleet() {
            let mut fake = FakeHermes::new();
            fake.put(DISABLED_KEY, json!(["other", "cafleet"]));
            let installed = install(&fake);
            assert_plugin_failed(
                &installed,
                &fake,
                Cause::Exact(
                    "plugins.disabled lists cafleet; remove it from plugins.disabled, then rerun 'cafleet setup --coding-agent hermes'",
                ),
            );
            assert!(after_batch(&fake.calls()).is_empty());
            assert_eq!(fake.value(DISABLED_KEY), Some(json!(["other", "cafleet"])));
            assert_eq!(fake.value(ENABLED_KEY), None);
        }

        #[test]
        fn hermes_setup_fails_for_a_plugins_value_outside_its_reviewed_shape() {
            for (key, value, cause) in [
                (DISABLED_KEY, json!("cafleet"), None),
                (ENABLED_KEY, json!(["cafleet", null]), None),
                (
                    PLUGINS_KEY,
                    json!("cafleet"),
                    Some("plugins is not a JSON object"),
                ),
            ] {
                let mut fake = FakeHermes::new();
                fake.put(key, value.clone());
                let installed = install(&fake);
                let cause = match cause {
                    Some(cause) => Cause::Exact(cause),
                    None => Cause::Contains("is not a JSON array of strings"),
                };
                assert_plugin_failed(&installed, &fake, cause);
                assert!(writes(&fake.calls()).is_empty(), "{key} {value}");
                assert_eq!(fake.value(key), Some(value));
            }
        }

        #[test]
        fn hermes_setup_fails_with_the_cause_of_a_failed_plugin_command() {
            let mut fake = FakeHermes::new();
            fake.reply(
                set(ENABLED_KEY, r#"["cafleet"]"#),
                Reply::Fail("config.yaml is locked by another process".into()),
            );
            let installed = install(&fake);
            assert_plugin_failed(
                &installed,
                &fake,
                Cause::Exact("config.yaml is locked by another process"),
            );
            assert_eq!(fake.value(ENABLED_KEY), None);
            assert_eq!(
                after_batch(&fake.calls()),
                [set(ENABLED_KEY, r#"["cafleet"]"#)],
                "a failed write stops before the next write and the re-read"
            );

            let mut fake = FakeHermes::new();
            fake.reply(
                get(PLUGINS_KEY),
                Reply::Fail("did not finish within its deadline".into()),
            );
            let installed = install(&fake);
            assert_plugin_failed(
                &installed,
                &fake,
                Cause::Exact("did not finish within its deadline"),
            );
            assert!(after_batch(&fake.calls()).is_empty());
        }

        #[test]
        fn hermes_setup_fails_when_the_reread_lacks_the_enablement() {
            for (key, cause) in [
                (
                    ENABLED_KEY,
                    "plugins.enabled still lacks cafleet after the write",
                ),
                (
                    INJECTION_KEY,
                    "plugins.entries.cafleet.allow_gateway_injection is still not true after the write",
                ),
            ] {
                let mut fake = FakeHermes::new();
                fake.ignore_sets(key);
                let installed = install(&fake);
                assert_plugin_failed(&installed, &fake, Cause::Exact(cause));
                assert_eq!(
                    after_batch(&fake.calls()),
                    [
                        set(ENABLED_KEY, r#"["cafleet"]"#),
                        set(INJECTION_KEY, "true"),
                        get(PLUGINS_KEY),
                    ],
                    "{key}: one re-read follows the writes"
                );
            }
        }

        #[test]
        fn explicit_hermes_setup_without_the_binary_fails_with_the_path_cause() {
            let installed = install_with(HermesSetup::MissingRequired);
            assert_failure(
                &installed,
                &extension_prefix(&installed.base),
                Cause::Exact("binary hermes not found on PATH"),
            );
            assert_eq!(installed.lines, [skills_line(&installed)]);
            assert!(!installed.base.join("plugins").exists());
        }

        #[test]
        fn hermes_setup_fails_into_the_resolved_base_when_the_config_path_is_unavailable() {
            for plugins_failure in [false, true] {
                let mut fake = FakeHermes::new();
                fake.reply(
                    FakeHermes::path_call(),
                    Reply::Fail("did not finish within its deadline".into()),
                );
                if plugins_failure {
                    fake.reply(get(PLUGINS_KEY), Reply::Fail("permission denied".into()));
                }
                let installed = install(&fake);
                assert_failure(
                    &installed,
                    &extension_prefix(&installed.base),
                    Cause::Exact("did not finish within its deadline"),
                );
                assert_eq!(installed.lines, [skills_line(&installed)]);
                assert!(after_batch(&fake.calls()).is_empty());
                assert!(!fake.plugin_dir().exists());
                assert!(!installed.base.join("plugins").exists());
            }
        }

        #[test]
        fn hermes_setup_fails_into_the_resolved_base_when_the_config_path_is_not_absolute() {
            let mut fake = FakeHermes::new();
            fake.reply(FakeHermes::path_call(), Reply::Path("config.yaml".into()));
            let installed = install(&fake);
            assert_failure(
                &installed,
                &extension_prefix(&installed.base),
                Cause::Exact("printed no absolute file path: config.yaml"),
            );
            assert_eq!(installed.lines, [skills_line(&installed)]);
            assert!(after_batch(&fake.calls()).is_empty());
            assert!(!installed.base.join("plugins").exists());
        }

        #[test]
        fn plain_setup_without_hermes_installs_skills_records_the_row_and_reports_the_skip() {
            let installed = install_with(HermesSetup::Absent);
            installed.result.as_ref().unwrap();
            assert_eq!(
                installed.lines,
                [
                    skills_line(&installed),
                    "hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes".to_owned(),
                ]
            );
            assert_eq!(installed.rows, 1);
            assert!(!installed.base.join("plugins").exists());
        }
    }
}
