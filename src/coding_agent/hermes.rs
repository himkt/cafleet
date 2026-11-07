use serde_json::Value;
use std::path::PathBuf;
use std::time::Instant;

pub(crate) const PLUGINS_KEY: &str = "plugins";
pub(crate) const PLUGINS_DISABLED_KEY: &str = "plugins.disabled";
pub(crate) const PLUGINS_ENABLED_KEY: &str = "plugins.enabled";
pub(crate) const GATEWAY_INJECTION_KEY: &str = "plugins.entries.cafleet.allow_gateway_injection";
pub(crate) const PLUGIN_NAME: &str = "cafleet";

/// Runs `hermes` with `HERMES_HOME=<base>`.
pub(crate) struct HermesCli {
    executable: PathBuf,
    base: PathBuf,
}

/// Each call runs until `deadline`; calls of one concurrent batch pass the same deadline.
pub(crate) trait HermesConfig: Sync {
    /// The trimmed stdout of `hermes config path`.
    fn config_path(&self, deadline: Instant) -> Result<String, String>;
    /// `hermes config get --json KEY`; `None` is the *not set* answer.
    fn get(&self, key: &str, deadline: Instant) -> Result<Option<Value>, String>;
    /// `hermes config set KEY VALUE`.
    fn set(&self, key: &str, value: &str, deadline: Instant) -> Result<(), String>;
}

/// A `hermes` process that ran to completion.
struct HermesOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl HermesOutput {
    /// The stdout of a command that exited 0, or its stderr text as the cause.
    fn succeeded(self) -> Result<String, String> {
        if self.exit_code == 0 {
            return Ok(self.stdout);
        }
        let stderr = self.stderr.trim();
        Err(if stderr.is_empty() {
            format!("exited with code {}", self.exit_code)
        } else {
            stderr.to_owned()
        })
    }
}

impl HermesCli {
    pub(crate) fn new(executable: PathBuf, base: PathBuf) -> Self {
        Self { executable, base }
    }

    fn run(&self, args: &[&str], deadline: Instant) -> Result<HermesOutput, String> {
        let output = crate::coding_agent::run_bounded_output(
            &self.executable,
            args,
            &[("HERMES_HOME", &self.base)],
            deadline,
        )?;
        let exit_code = output
            .status
            .code()
            .ok_or_else(|| format!("hermes {} ended by {}", args.join(" "), output.status))?;
        Ok(HermesOutput {
            exit_code,
            stdout: String::from_utf8(output.stdout)
                .map_err(|_| format!("hermes {} printed non-UTF-8 output", args.join(" ")))?,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

impl HermesConfig for HermesCli {
    fn config_path(&self, deadline: Instant) -> Result<String, String> {
        self.run(&["config", "path"], deadline)?
            .succeeded()
            .map(|stdout| stdout.trim().to_owned())
    }

    fn get(&self, key: &str, deadline: Instant) -> Result<Option<Value>, String> {
        let output = self.run(&["config", "get", "--json", key], deadline)?;
        let not_set = format!("Config key not set: {key}");
        if output.exit_code == 1 && output.stderr.lines().any(|line| line.trim() == not_set) {
            return Ok(None);
        }
        let stdout = output.succeeded()?;
        serde_json::from_str(&stdout)
            .map(Some)
            .map_err(|cause| format!("{key} printed no JSON value: {cause}"))
    }

    fn set(&self, key: &str, value: &str, deadline: Instant) -> Result<(), String> {
        self.run(&["config", "set", key, value], deadline)?
            .succeeded()
            .map(drop)
    }
}

/// The plugin directory under the Hermes home that owns `config_path`.
pub(crate) fn plugin_dir(config_path: &str) -> Result<PathBuf, String> {
    crate::coding_agent::hermes_settlement::home_of_config_path(config_path)
        .map(|home| home.join(crate::assets::HERMES_PLUGIN_DIR))
}

/// The CAFleet-relevant members of the resolved `plugins` subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginState {
    pub(crate) disabled_lists_cafleet: bool,
    pub(crate) enabled: Vec<String>,
    pub(crate) injection_allowed: bool,
}

impl PluginState {
    /// The keys whose values still keep the plugin from loading with gateway injection.
    pub(crate) fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.enabled.iter().any(|name| name == PLUGIN_NAME) {
            missing.push(PLUGINS_ENABLED_KEY);
        }
        if !self.injection_allowed {
            missing.push(GATEWAY_INJECTION_KEY);
        }
        missing
    }
}

pub(crate) fn read_plugin_state(
    hermes: &dyn HermesConfig,
    deadline: Instant,
) -> Result<PluginState, String> {
    plugin_state(hermes.get(PLUGINS_KEY, deadline)?)
}

/// Interprets a `plugins` reading: a missing member is that key's *not set* state, and a
/// *not set* `plugins` key leaves all three unset.
pub(crate) fn plugin_state(plugins: Option<Value>) -> Result<PluginState, String> {
    let plugins = plugins.unwrap_or_else(|| Value::Object(Default::default()));
    if !plugins.is_object() {
        return Err(format!("{PLUGINS_KEY} is not a JSON object"));
    }
    let disabled = plugin_list(PLUGINS_DISABLED_KEY, plugins.get("disabled"))?;
    Ok(PluginState {
        disabled_lists_cafleet: disabled.iter().any(|name| name == PLUGIN_NAME),
        enabled: plugin_list(PLUGINS_ENABLED_KEY, plugins.get("enabled"))?,
        injection_allowed: plugins.pointer("/entries/cafleet/allow_gateway_injection")
            == Some(&Value::Bool(true)),
    })
}

/// A set plugin list, which must be a JSON array of strings.
fn plugin_list(key: &str, value: Option<&Value>) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|cause| format!("{key} is not a JSON array of strings: {cause}")),
    }
}

/// Runs `config path` and the `plugins` read as one concurrent batch under `deadline`.
pub(crate) fn read_batch(
    hermes: &dyn HermesConfig,
    deadline: Instant,
) -> (Result<String, String>, Result<Option<Value>, String>) {
    std::thread::scope(|scope| {
        let path = scope.spawn(|| hermes.config_path(deadline));
        let plugins = scope.spawn(|| hermes.get(PLUGINS_KEY, deadline));
        (joined(path), joined(plugins))
    })
}

pub(crate) fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Writes the plugin keys `state` lacks, serially because each set rewrites `config.yaml`, then
/// re-reads `plugins` once when something was written; returns whether it wrote.
pub(crate) fn enable_plugin(
    hermes: &dyn HermesConfig,
    state: &PluginState,
    budget: std::time::Duration,
) -> Result<bool, String> {
    if state.disabled_lists_cafleet {
        return Err(format!(
            "{PLUGINS_DISABLED_KEY} lists {PLUGIN_NAME}; remove it from {PLUGINS_DISABLED_KEY}, then rerun 'cafleet setup --coding-agent hermes'"
        ));
    }
    let missing = state.missing();
    if missing.contains(&PLUGINS_ENABLED_KEY) {
        let mut enabled = state.enabled.clone();
        enabled.push(PLUGIN_NAME.to_owned());
        let value = serde_json::to_string(&enabled).expect("a list of strings serializes as JSON");
        hermes.set(PLUGINS_ENABLED_KEY, &value, Instant::now() + budget)?;
    }
    if missing.contains(&GATEWAY_INJECTION_KEY) {
        hermes.set(GATEWAY_INJECTION_KEY, "true", Instant::now() + budget)?;
    }
    if missing.is_empty() {
        return Ok(false);
    }
    let written = read_plugin_state(hermes, Instant::now() + budget)?;
    if written.missing().contains(&PLUGINS_ENABLED_KEY) {
        return Err(format!(
            "{PLUGINS_ENABLED_KEY} still lacks {PLUGIN_NAME} after the write"
        ));
    }
    if written.missing().contains(&GATEWAY_INJECTION_KEY) {
        return Err(format!(
            "{GATEWAY_INJECTION_KEY} is still not true after the write"
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::hermes::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Serves `config path`, `get` and `set` from an in-memory map. A set listed in `ignored`
    /// exits 0 without the value sticking, as under a managed scope.
    struct Fake {
        config_path: String,
        config: Mutex<BTreeMap<String, serde_json::Value>>,
        ignored: Vec<&'static str>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(plugins: Option<serde_json::Value>) -> Self {
            Self {
                config_path: "/home/tester/.hermes/config.yaml".into(),
                config: Mutex::new(
                    plugins
                        .map(|plugins| BTreeMap::from([(PLUGINS_KEY.to_owned(), plugins)]))
                        .unwrap_or_default(),
                ),
                ignored: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn plugins(&self) -> serde_json::Value {
            self.config.lock().unwrap()[PLUGINS_KEY].clone()
        }
    }

    impl HermesConfig for Fake {
        fn config_path(&self, _: Instant) -> Result<String, String> {
            self.calls.lock().unwrap().push("config path".into());
            Ok(self.config_path.clone())
        }

        fn get(&self, key: &str, _: Instant) -> Result<Option<serde_json::Value>, String> {
            self.calls.lock().unwrap().push(format!("get {key}"));
            assert_eq!(
                key, PLUGINS_KEY,
                "setup and doctor read only the plugins subtree"
            );
            Ok(self.config.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str, _: Instant) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("set {key} {value}"));
            if self.ignored.contains(&key) {
                return Ok(());
            }
            let value: serde_json::Value = serde_json::from_str(value).unwrap();
            let mut config = self.config.lock().unwrap();
            let plugins = config
                .entry(PLUGINS_KEY.to_owned())
                .or_insert_with(|| json!({}));
            let pointer = format!(
                "/{}",
                key.strip_prefix("plugins.").unwrap().replace('.', "/")
            );
            let mut target = plugins;
            for part in pointer.trim_start_matches('/').split('/') {
                target = target
                    .as_object_mut()
                    .unwrap()
                    .entry(part)
                    .or_insert_with(|| json!({}));
            }
            *target = value;
            Ok(())
        }
    }

    fn state(
        disabled_lists_cafleet: bool,
        enabled: &[&str],
        injection_allowed: bool,
    ) -> PluginState {
        PluginState {
            disabled_lists_cafleet,
            enabled: enabled.iter().map(|name| (*name).to_owned()).collect(),
            injection_allowed,
        }
    }

    fn budget() -> Duration {
        Duration::from_secs(10)
    }

    #[test]
    fn a_not_set_plugins_key_and_each_absent_member_are_not_set() {
        assert_eq!(plugin_state(None).unwrap(), state(false, &[], false));
        assert_eq!(
            plugin_state(Some(json!({}))).unwrap(),
            state(false, &[], false)
        );
        assert_eq!(
            plugin_state(Some(json!({"enabled": ["alpha"]}))).unwrap(),
            state(false, &["alpha"], false)
        );
    }

    #[test]
    fn the_plugins_subtree_yields_disabled_enabled_and_injection_states() {
        assert_eq!(
            plugin_state(Some(json!({
                "disabled": ["other", "cafleet"],
                "enabled": ["alpha", "cafleet"],
                "entries": {"cafleet": {"allow_gateway_injection": true}},
                "unrelated": 1
            })))
            .unwrap(),
            state(true, &["alpha", "cafleet"], true)
        );
        for injection in [json!(false), json!("true"), json!(1), json!(null)] {
            assert!(
                !plugin_state(Some(
                    json!({"entries": {"cafleet": {"allow_gateway_injection": injection}}})
                ))
                .unwrap()
                .injection_allowed,
                "{injection}"
            );
        }
    }

    #[test]
    fn plugin_lists_must_be_arrays_of_strings_and_plugins_an_object() {
        for (plugins, prefix) in [
            (
                json!({"disabled": "cafleet"}),
                "plugins.disabled is not a JSON array of strings: ",
            ),
            (
                json!({"enabled": ["cafleet", null]}),
                "plugins.enabled is not a JSON array of strings: ",
            ),
        ] {
            let failure = plugin_state(Some(plugins.clone())).unwrap_err();
            assert!(failure.starts_with(prefix), "{plugins}: {failure}");
        }
        assert_eq!(
            plugin_state(Some(json!(["cafleet"]))).unwrap_err(),
            "plugins is not a JSON object"
        );
    }

    #[test]
    fn missing_names_the_enablement_keys_in_order() {
        assert_eq!(
            state(false, &["alpha"], false).missing(),
            [PLUGINS_ENABLED_KEY, GATEWAY_INJECTION_KEY]
        );
        assert_eq!(
            state(false, &["cafleet"], false).missing(),
            [GATEWAY_INJECTION_KEY]
        );
        assert_eq!(state(false, &[], true).missing(), [PLUGINS_ENABLED_KEY]);
        assert!(state(true, &["cafleet"], true).missing().is_empty());
    }

    #[test]
    fn read_batch_runs_config_path_and_the_plugins_read_concurrently_under_one_deadline() {
        let (sender, receiver) = mpsc::channel();
        let (started, first) = mpsc::channel::<()>();
        /// The first read waits for the second to start, so a serial batch fails.
        struct Batch {
            first_started: Mutex<Option<mpsc::Sender<()>>>,
            first: Mutex<mpsc::Receiver<()>>,
            second_started: Mutex<mpsc::Sender<()>>,
            second: Mutex<mpsc::Receiver<()>>,
            deadlines: Mutex<Vec<Instant>>,
        }
        impl Batch {
            fn meet(&self, deadline: Instant) {
                self.deadlines.lock().unwrap().push(deadline);
                let first = self.first_started.lock().unwrap().take();
                match first {
                    Some(signal) => {
                        signal.send(()).unwrap();
                        self.second
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .expect("the other read starts while this one runs");
                    }
                    None => {
                        self.first
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .expect("the first read started");
                        self.second_started.lock().unwrap().send(()).unwrap();
                    }
                }
            }
        }
        impl HermesConfig for Batch {
            fn config_path(&self, deadline: Instant) -> Result<String, String> {
                self.meet(deadline);
                Ok("/home/tester/.hermes/config.yaml".into())
            }
            fn get(
                &self,
                key: &str,
                deadline: Instant,
            ) -> Result<Option<serde_json::Value>, String> {
                assert_eq!(key, PLUGINS_KEY);
                self.meet(deadline);
                Ok(Some(json!({"enabled": ["cafleet"]})))
            }
            fn set(&self, _: &str, _: &str, _: Instant) -> Result<(), String> {
                panic!("the read batch writes nothing")
            }
        }
        let batch = Batch {
            first_started: Mutex::new(Some(started)),
            first: Mutex::new(first),
            second_started: Mutex::new(sender),
            second: Mutex::new(receiver),
            deadlines: Mutex::new(Vec::new()),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let (path, plugins) = read_batch(&batch, deadline);
        assert_eq!(path.unwrap(), "/home/tester/.hermes/config.yaml");
        assert_eq!(plugins.unwrap(), Some(json!({"enabled": ["cafleet"]})));
        assert_eq!(*batch.deadlines.lock().unwrap(), [deadline, deadline]);
    }

    #[test]
    fn an_enabled_plugin_is_left_unwritten_and_unread() {
        let fake = Fake::new(None);
        assert!(
            !enable_plugin(&fake, &state(false, &["alpha", "cafleet"], true), budget()).unwrap()
        );
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    }

    #[test]
    fn a_first_install_appends_cafleet_sets_injection_serially_then_re_reads_plugins_once() {
        let fake = Fake::new(Some(json!({"enabled": ["alpha", "beta"]})));
        let before = plugin_state(Some(fake.plugins())).unwrap();
        assert!(enable_plugin(&fake, &before, budget()).unwrap());
        assert_eq!(
            fake.calls(),
            [
                format!("set {PLUGINS_ENABLED_KEY} [\"alpha\",\"beta\",\"cafleet\"]"),
                format!("set {GATEWAY_INJECTION_KEY} true"),
                format!("get {PLUGINS_KEY}"),
            ]
        );
        assert_eq!(
            plugin_state(Some(fake.plugins())).unwrap(),
            state(false, &["alpha", "beta", "cafleet"], true)
        );
    }

    #[test]
    fn only_the_missing_key_is_written() {
        let fake = Fake::new(Some(json!({"enabled": ["cafleet"]})));
        assert!(enable_plugin(&fake, &state(false, &["cafleet"], false), budget()).unwrap());
        assert_eq!(
            fake.calls(),
            [
                format!("set {GATEWAY_INJECTION_KEY} true"),
                format!("get {PLUGINS_KEY}")
            ]
        );
    }

    #[test]
    fn a_disabled_entry_for_cafleet_is_refused_before_any_call() {
        let fake = Fake::new(None);
        assert_eq!(
            enable_plugin(&fake, &state(true, &[], false), budget()).unwrap_err(),
            "plugins.disabled lists cafleet; remove it from plugins.disabled, then rerun 'cafleet setup --coding-agent hermes'"
        );
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn a_write_that_does_not_stick_fails_the_re_read() {
        for (ignored, expected) in [
            (
                PLUGINS_ENABLED_KEY,
                "plugins.enabled still lacks cafleet after the write",
            ),
            (
                GATEWAY_INJECTION_KEY,
                "plugins.entries.cafleet.allow_gateway_injection is still not true after the write",
            ),
        ] {
            let mut fake = Fake::new(None);
            fake.ignored.push(ignored);
            assert_eq!(
                enable_plugin(&fake, &state(false, &[], false), budget()).unwrap_err(),
                expected
            );
            assert_eq!(fake.calls().last().unwrap(), &format!("get {PLUGINS_KEY}"));
        }
    }

    #[test]
    fn plugin_dir_names_the_cafleet_plugin_under_the_home_owning_the_config_path() {
        assert_eq!(
            plugin_dir("/srv/hermes/profiles/work/config.yaml").unwrap(),
            std::path::PathBuf::from("/srv/hermes/profiles/work/plugins/cafleet")
        );
        assert!(plugin_dir("config.yaml").is_err());
    }

    mod cli {
        use crate::coding_agent::hermes::*;
        use crate::coding_agent::test_support::executable_stub;
        use std::time::Duration;

        fn with_busy_retry<T>(mut run: impl FnMut() -> Result<T, String>) -> Result<T, String> {
            for _ in 0..50 {
                match run() {
                    Err(cause) if cause.contains("Text file busy") => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    result => return result,
                }
            }
            panic!("an executable stub stayed busy")
        }

        fn cli(body: &str) -> (tempfile::TempDir, HermesCli) {
            let dir = tempfile::tempdir().unwrap();
            let executable = executable_stub(dir.path(), "hermes", body);
            let base = dir.path().join("hermes home");
            (dir, HermesCli::new(executable, base))
        }

        fn deadline() -> Instant {
            Instant::now() + Duration::from_secs(5)
        }

        #[test]
        fn config_path_runs_under_the_resolved_base_and_trims_its_stdout() {
            let (dir, hermes) = cli(
                "[ \"$*\" = 'config path' ] || exit 9\nprintf '  %s/config.yaml  \\n' \"$HERMES_HOME\"",
            );
            assert_eq!(
                with_busy_retry(|| hermes.config_path(deadline())).unwrap(),
                format!("{}/config.yaml", dir.path().join("hermes home").display())
            );
        }

        #[test]
        fn get_reads_json_and_treats_only_the_exact_not_set_line_under_exit_one_as_absent() {
            let (_dir, hermes) = cli(
                "case \"$*\" in\n  'config get --json plugins') printf '%s\\n' '{\"enabled\":[\"cafleet\"]}' ;;\n  'config get --json absent') printf 'noise\\n  Config key not set: absent  \\n' >&2; exit 1 ;;\n  'config get --json other') printf 'Config key not set: absent\\n' >&2; exit 1 ;;\n  'config get --json exit2') printf 'Config key not set: exit2\\n' >&2; exit 2 ;;\n  'config get --json near') printf 'Config key not set: near.extra\\n' >&2; exit 1 ;;\n  'config get --json prefixed') printf 'Error: Config key not set: prefixed\\n' >&2; exit 1 ;;\n  'config get --json bare') printf 'noise\\n' >&2; exit 1 ;;\n  'config get --json text') printf 'not json\\n' ;;\n  *) exit 9 ;;\nesac",
            );
            for (key, cause) in [
                ("near", "Config key not set: near.extra"),
                ("prefixed", "Error: Config key not set: prefixed"),
                ("bare", "noise"),
            ] {
                assert_eq!(
                    with_busy_retry(|| hermes.get(key, deadline())).unwrap_err(),
                    cause,
                    "{key}"
                );
            }
            assert_eq!(
                with_busy_retry(|| hermes.get("plugins", deadline())).unwrap(),
                Some(serde_json::json!({"enabled": ["cafleet"]}))
            );
            assert_eq!(
                with_busy_retry(|| hermes.get("absent", deadline())).unwrap(),
                None
            );
            assert_eq!(
                with_busy_retry(|| hermes.get("other", deadline())).unwrap_err(),
                "Config key not set: absent"
            );
            assert_eq!(
                with_busy_retry(|| hermes.get("exit2", deadline())).unwrap_err(),
                "Config key not set: exit2"
            );
            assert!(
                with_busy_retry(|| hermes.get("text", deadline()))
                    .unwrap_err()
                    .starts_with("text printed no JSON value: ")
            );
        }

        #[test]
        fn set_passes_the_key_and_value_and_reports_a_failure_with_its_stderr() {
            let (dir, hermes) = cli(
                "[ \"$1 $2\" = 'config set' ] || exit 9\ncase \"$3\" in\n  locked) printf 'config.yaml is locked\\n' >&2; exit 2 ;;\n  silent) exit 4 ;;\nesac\nprintf '%s=%s' \"$3\" \"$4\" > \"$(dirname \"$0\")/set.log\"",
            );
            with_busy_retry(|| hermes.set("plugins.enabled", "[\"cafleet\"]", deadline())).unwrap();
            assert_eq!(
                std::fs::read_to_string(dir.path().join("set.log")).unwrap(),
                "plugins.enabled=[\"cafleet\"]"
            );
            assert_eq!(
                with_busy_retry(|| hermes.set("locked", "true", deadline())).unwrap_err(),
                "config.yaml is locked"
            );
            assert_eq!(
                with_busy_retry(|| hermes.set("silent", "true", deadline())).unwrap_err(),
                "exited with code 4"
            );
        }
    }
}
