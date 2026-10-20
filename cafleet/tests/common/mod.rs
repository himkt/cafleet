#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use cafleet::broker::{self, NewPlacement};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The capture every pane shows unless [`Cli::set_pane_capture`] overrides it:
/// a claude composer at rest, so a delivery fires immediately.
pub const REST_CAPTURE: &str = "pane:{pane}\nline1\nline2\n> at rest";

/// A capture file holding this line makes `capture-pane` fail for that pane.
pub const CAPTURE_FAILS: &str = "CAPTURE-FAILS";

// `CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT` scripts `list-panes` per call: the Nth
// call takes the Nth letter (`F` fails, anything else succeeds) and calls past
// the end repeat the last letter. The shim has no PATH beyond itself, so the
// call count is read back from its own log with shell builtins only.
const TMUX_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
if [ -n "$CAFLEET_TEST_TMUX_FAIL" ] && [ "$1" = "$CAFLEET_TEST_TMUX_FAIL" ]; then
    echo "forced failure" >&2
    exit 1
fi
case "$1" in
    display-message) printf 'main|@1|%%0\n' ;;
    split-window) printf '%s\n' "${CAFLEET_TEST_NEXT_PANE:-%7}" ;;
    capture-pane)
        while [ "$#" -gt 0 ]; do
            if [ "$1" = "-t" ]; then
                shift
                if [ -e "$CAFLEET_TEST_TMUX_CAPTURE_DIR/$1" ]; then
                    while IFS= read -r line || [ -n "$line" ]; do
                        if [ "$line" = "CAPTURE-FAILS" ]; then
                            echo "forced failure" >&2
                            exit 1
                        fi
                        printf '%s\n' "$line"
                    done < "$CAFLEET_TEST_TMUX_CAPTURE_DIR/$1"
                    exit 0
                fi
                printf 'pane:%s\nline1\nline2\n> at rest\n' "$1"
                exit 0
            fi
            shift
        done
        exit 2
        ;;
    list-panes)
        if [ -n "$CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT" ]; then
            script="$CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT"
            calls=0
            while IFS= read -r line; do
                case "$line" in list-panes*) calls=$((calls + 1)) ;; esac
            done < "$CAFLEET_TEST_TMUX_LOG"
            while [ "$calls" -gt 1 ] && [ "${#script}" -gt 1 ]; do
                script="${script#?}"
                calls=$((calls - 1))
            done
            case "$script" in
                F*)
                    echo "forced failure" >&2
                    exit 1
                    ;;
            esac
        fi
        for pane in ${CAFLEET_TEST_TMUX_PANES:-%0 %7}; do
            printf '%s\n' "$pane"
        done
        ;;
esac
exit 0
"#;

pub struct Cli {
    pub home: TempDir,
    pub shim_dir: PathBuf,
    pub shim_log: PathBuf,
    pub capture_dir: PathBuf,
    pub fail_subcommand: Option<String>,
    pub extra_env: Vec<(String, String)>,
}

impl Cli {
    pub fn new() -> Self {
        let home = TempDir::new().unwrap();
        let shim_dir = home.path().join("shim-bin");
        std::fs::create_dir_all(&shim_dir).unwrap();
        write_executable(&shim_dir.join("tmux"), TMUX_SHIM);
        // member create's spawn preconditions PATH-check the backend binary
        // (SPEC §6.3 step 4) — a no-op claude satisfies the default backend.
        write_executable(&shim_dir.join("claude"), "#!/bin/sh\nexit 0\n");
        let shim_log = home.path().join("tmux-shim.log");
        let capture_dir = home.path().join("pane-captures");
        std::fs::create_dir_all(&capture_dir).unwrap();
        Cli {
            home,
            shim_dir,
            shim_log,
            capture_dir,
            fail_subcommand: None,
            extra_env: Vec::new(),
        }
    }

    /// Make `capture-pane` print `content` for `pane` from now on; a content
    /// of [`CAPTURE_FAILS`] makes the capture fail instead.
    pub fn set_pane_capture(&self, pane: &str, content: &str) {
        std::fs::write(self.capture_dir.join(pane), content).unwrap();
    }

    /// The fleet's `monitor_runtime` owner pid; `None` when no loop holds it.
    pub fn monitor_pid(&self, fleet_id: i64) -> Option<i64> {
        self.sqlite()
            .query_row(
                "SELECT pid FROM monitor_runtime WHERE fleet_id=?1",
                [fleet_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
    }

    /// The text `fleet create` prints once the fleet's loop is live.
    pub fn fleet_create_text(&self, fleet_id: i64, director_id: i64, monitor_id: i64) -> String {
        let pid = self
            .monitor_pid(fleet_id)
            .expect("fleet create returns only after the loop is live");
        format!("{fleet_id} director={director_id} monitor={monitor_id}\nmonitor loop: pid {pid}\n")
    }

    pub fn monitor_log_path(&self, fleet_id: i64) -> PathBuf {
        self.home.path().join(format!("monitor-{fleet_id}.log"))
    }

    /// Set an extra environment variable (e.g. a backend config-location
    /// variable) for every subsequent run.
    pub fn set_env(&mut self, key: &str, value: &str) {
        self.extra_env.push((key.to_string(), value.to_string()));
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.path().join("cafleet.db")
    }

    pub fn db_url(&self) -> String {
        format!("sqlite:///{}", self.db_path().display())
    }

    fn command(&self, args: &[&str], inside_tmux: bool) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cafleet"));
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home.path())
            .env("PATH", &self.shim_dir)
            .env("CAFLEET_DATABASE_URL", self.db_url())
            .env("CAFLEET_TEST_TMUX_LOG", &self.shim_log)
            .env("CAFLEET_TEST_TMUX_CAPTURE_DIR", &self.capture_dir);
        if let Some(fail) = &self.fail_subcommand {
            cmd.env("CAFLEET_TEST_TMUX_FAIL", fail);
        }
        for (key, value) in &self.extra_env {
            cmd.env(key, value);
        }
        if inside_tmux {
            cmd.env("TMUX", "/tmp/tmux-1000/default,123,0")
                .env("TMUX_PANE", "%0");
        }
        cmd
    }

    /// Run inside the fake tmux context (shim on PATH, TMUX/TMUX_PANE set).
    pub fn run(&self, args: &[&str]) -> Output {
        self.command(args, true).output().unwrap()
    }

    /// Run inside the fake tmux context with `dir` as the working directory.
    pub fn run_in(&self, dir: &Path, args: &[&str]) -> Output {
        self.command(args, true).current_dir(dir).output().unwrap()
    }

    /// Run with no multiplexer presence variables set.
    pub fn run_outside_tmux(&self, args: &[&str]) -> Output {
        self.command(args, false).output().unwrap()
    }

    pub fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        use std::io::Write;
        let mut child = self
            .command(args, true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    pub fn sqlite(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db_path()).unwrap()
    }

    pub fn migrate(&self) {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        assert_eq!(
            cafleet::db::migrate_to_head(&mut conn).unwrap(),
            cafleet::db::head_version()
        );
    }

    pub fn install(&self) {
        let output = self.run(&["setup"]);
        assert!(output.status.success(), "setup: {}", text(&output.stderr));
    }

    /// The agent's recorded-path identity at its default (no env override)
    /// resolution under the test HOME: claude → `~/.claude`, codex →
    /// `~/.codex`, opencode → `~/.opencode` (the preset base).
    pub fn identity_path(&self, coding_agent: &str) -> String {
        let dir = match coding_agent {
            "claude" => ".claude",
            "codex" => ".codex",
            "opencode" => ".opencode",
            other => panic!("unknown coding agent '{other}'"),
        };
        self.home.path().join(dir).to_str().unwrap().to_string()
    }

    /// Seed a row at the agent's default identity path.
    pub fn seed_asset_row(&self, coding_agent: &str, version: &str) {
        self.seed_asset_row_at(coding_agent, &self.identity_path(coding_agent), version);
    }

    /// Seed a row at an explicit path (e.g. a superseded location).
    pub fn seed_asset_row_at(&self, coding_agent: &str, path: &str, version: &str) {
        self.seed_asset_row_dated(
            coding_agent,
            path,
            version,
            "2026-07-30T00:00:00.000000+00:00",
        );
    }

    /// Seed a row with an explicit `installed_at` (for recency tie-breaks).
    pub fn seed_asset_row_dated(
        &self,
        coding_agent: &str,
        path: &str,
        version: &str,
        installed_at: &str,
    ) {
        self.sqlite()
            .execute(
                "INSERT INTO asset_installs (coding_agent, path, cafleet_version, installed_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(coding_agent, path) DO UPDATE SET \
                     cafleet_version=excluded.cafleet_version, \
                     installed_at=excluded.installed_at",
                rusqlite::params![coding_agent, path, version, installed_at],
            )
            .unwrap();
    }

    /// The `(coding_agent, path, cafleet_version)` rows in ascending key
    /// order.
    pub fn asset_rows(&self) -> Vec<(String, String, String)> {
        let conn = self.sqlite();
        let mut stmt = conn
            .prepare(
                "SELECT coding_agent, path, cafleet_version FROM asset_installs \
                 ORDER BY coding_agent, path",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// Hand-write a behind-head database: the refinery ledger at version 5
    /// with the baseline (path-less) `asset_installs` shape.
    pub fn seed_pre_v6_database(&self) {
        self.sqlite()
            .execute_batch(
                "CREATE TABLE refinery_schema_history (
                     version INT4 PRIMARY KEY,
                     name VARCHAR(255),
                     applied_on VARCHAR(255),
                     checksum VARCHAR(255)
                 );
                 INSERT INTO refinery_schema_history (version, name, applied_on, checksum) VALUES
                     (1, 'baseline', '2026-01-01T00:00:00Z', '0'),
                     (2, 'drop_director_monitor_enrollment', '2026-01-01T00:00:00Z', '0'),
                     (3, 'strip_monitoring_member_kind', '2026-01-01T00:00:00Z', '0'),
                     (4, 'fleet_level_wake_schedule', '2026-01-01T00:00:00Z', '0'),
                     (5, 'monitor_wake_interval', '2026-01-01T00:00:00Z', '0');
                 CREATE TABLE asset_installs (
                     coding_agent TEXT NOT NULL PRIMARY KEY,
                     cafleet_version TEXT NOT NULL,
                     installed_at TEXT NOT NULL
                 );",
            )
            .unwrap();
    }

    /// Migrate + record a current-version install so the stale-assets guard
    /// passes.
    pub fn ready(&self) {
        self.migrate();
        self.seed_asset_row("claude", VERSION);
    }

    /// The monitor member id the atomic `fleet create` bootstrap allocates in
    /// a fresh database (fleet 1, Director 1, monitor 2).
    pub const BOOTSTRAP_MONITOR_ID: i64 = 2;

    /// Write (once) and return the monitor spawn-prompt file `fleet create
    /// --monitor-file` consumes.
    pub fn monitor_prompt_path(&self) -> String {
        let path = self.home.path().join("monitor-prompt.md");
        if !path.exists() {
            std::fs::write(&path, "follow your monitor role protocol").unwrap();
        }
        path.to_str().unwrap().to_string()
    }

    /// `ready()` + a fleet via the atomic `fleet create` bootstrap: fleet 1,
    /// Director member 1, and monitor member [`Self::BOOTSTRAP_MONITOR_ID`]
    /// spawned from `--monitor-file`.
    pub fn with_cli_fleet(&self) -> (i64, i64) {
        self.ready();
        let output = self.run(&[
            "fleet",
            "create",
            "--name",
            "testfleet",
            "--coding-agent",
            "claude",
            "--monitor-file",
            &self.monitor_prompt_path(),
            "--json",
        ]);
        assert!(
            output.status.success(),
            "fleet create must succeed: {}",
            text(&output.stderr)
        );
        let fleet: serde_json::Value = serde_json::from_str(text(&output.stdout).trim()).unwrap();
        (
            fleet["fleet_id"].as_i64().expect("created fleet id"),
            fleet["director"]["member_id"]
                .as_i64()
                .expect("created Director id"),
        )
    }

    pub fn with_cli_bare_fleet(&self) -> (i64, i64) {
        let ids = self.with_cli_fleet();
        let monitor = broker::active_monitor_member_id(&self.sqlite(), ids.0)
            .unwrap()
            .expect("bootstrap monitor");
        let output = self.run(&["member", "delete", &monitor.to_string()]);
        assert!(
            output.status.success(),
            "monitor delete: {}",
            text(&output.stderr)
        );
        ids
    }

    pub fn seeded_fleet(&self) -> (i64, i64) {
        self.ready();
        self.seed_fleet("testfleet")
    }

    pub fn seed_fleet(&self, name: &str) -> (i64, i64) {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let fleet = broker::create_fleet(
            &mut conn,
            Some(name),
            "main",
            "@1",
            "%0",
            "claude",
            "tmux",
            "monitor",
            "Monitor member for this fleet",
            |_, _, monitor| Ok(format!("%{monitor}")),
        )
        .unwrap();
        (
            fleet["fleet_id"].as_i64().expect("seeded fleet id"),
            fleet["director"]["member_id"]
                .as_i64()
                .expect("seeded Director id"),
        )
    }

    pub fn seeded_bare_fleet(&self) -> (i64, i64) {
        let ids = self.seeded_fleet();
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let monitor = broker::active_monitor_member_id(&conn, ids.0)
            .unwrap()
            .expect("seeded monitor");
        broker::deregister_member(&mut conn, monitor).unwrap();
        ids
    }

    pub fn seed_member(&self, fleet_id: i64, name: &str) -> i64 {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let member = broker::register_member(
            &mut conn,
            fleet_id,
            name,
            "test member",
            &[],
            Some(&NewPlacement {
                backend: "tmux".into(),
                mux_session: "main".into(),
                mux_window_id: "@1".into(),
                mux_pane_id: None,
                coding_agent: "claude".into(),
            }),
            false,
        )
        .unwrap();
        let pane_id = format!("%{}", member.member_id + 4);
        let placement = broker::update_placement_pane_id(&mut conn, member.member_id, &pane_id)
            .unwrap()
            .expect("seeded member placement");
        assert_eq!(placement.mux_pane_id.as_deref(), Some(pane_id.as_str()));
        member.member_id
    }

    pub fn seed_message(&self, sender: i64, recipient: i64, body: &str) -> i64 {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let message =
            broker::send_message(&mut conn, sender, &recipient.to_string(), body).unwrap();
        assert_eq!(message.owner_member_id, recipient);
        assert_eq!(message.text, body);
        message.message_id
    }

    /// Spawn the fleet's monitor member through `member create --role monitor`.
    pub fn create_monitor(&self, fleet_id: i64) -> i64 {
        let output = self.run(&[
            "member",
            "create",
            "--fleet-id",
            &fleet_id.to_string(),
            "--role",
            "monitor",
            "--name",
            "monitor",
            "--description",
            "fleet monitor member",
            "follow your monitor role protocol",
        ]);
        assert!(
            output.status.success(),
            "monitor member create must succeed: {}",
            text(&output.stderr)
        );
        text(&output.stdout)
            .split_whitespace()
            .next()
            .expect("compact member-create output starts with the id")
            .parse()
            .expect("the first token is the member id")
    }

    /// Spawn a member through `member create`; returns its id parsed from the
    /// compact `<id> <name> backend=... pane=...` line.
    pub fn create_member(&self, fleet_id: i64, name: &str) -> i64 {
        let output = self.run(&[
            "member",
            "create",
            "--fleet-id",
            &fleet_id.to_string(),
            "--name",
            name,
            "--description",
            "test member",
            "wait for the Director",
        ]);
        assert!(
            output.status.success(),
            "member create must succeed: {}",
            text(&output.stderr)
        );
        text(&output.stdout)
            .split_whitespace()
            .next()
            .expect("compact member-create output starts with the id")
            .parse()
            .expect("the first token is the member id")
    }

    pub fn spawn(&self, args: &[&str]) -> std::process::Child {
        self.command(args, true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// How many shim calls so far start with `prefix`.
    pub fn shim_count(&self, prefix: &str) -> usize {
        self.shim_calls()
            .iter()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    pub fn shim_calls(&self) -> Vec<String> {
        match std::fs::read_to_string(&self.shim_log) {
            Ok(log) => log.lines().map(str::to_owned).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("cannot read shim log {}: {error}", self.shim_log.display()),
        }
    }
}

const LOOP_DEADLINE: Duration = Duration::from_secs(20);
const LOOP_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A foreground `cafleet monitor` child that is always reaped, with its
/// output and the shim log attached to every failure.
pub struct LoopChild<'a> {
    child: Option<Child>,
    cli: &'a Cli,
}

impl<'a> LoopChild<'a> {
    pub fn spawn(cli: &'a Cli, args: &[&str]) -> Self {
        Self {
            child: Some(cli.spawn(args)),
            cli,
        }
    }

    /// Spawn the loop for `fleet_id` and wait until it owns the runtime row.
    pub fn start(cli: &'a Cli, fleet_id: i64, flags: &[&str]) -> Self {
        let fleet = fleet_id.to_string();
        let mut args = vec!["monitor", fleet.as_str()];
        args.extend_from_slice(flags);
        let mut child = Self::spawn(cli, &args);
        let pid = child.pid();
        child.wait_until("the loop to own the runtime row", || {
            cli.monitor_pid(fleet_id) == Some(pid)
        });
        child
    }

    pub fn pid(&self) -> i64 {
        i64::from(self.child.as_ref().expect("uncollected child").id())
    }

    fn reap(&mut self) -> io::Result<Output> {
        let child = self.child.as_mut().expect("collect each child once");
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        self.child.take().unwrap().wait_with_output()
    }

    fn fail(&mut self, reason: &str) -> ! {
        let output = self.reap().expect("terminate and reap the failed loop");
        panic!(
            "monitor loop: {reason}; status={}; stdout={}; stderr={}; shim={:?}",
            output.status,
            stdout(&output),
            stderr(&output),
            self.cli.shim_calls()
        );
    }

    /// Wait for `observed` while the loop stays alive.
    pub fn wait_until(&mut self, description: &str, mut observed: impl FnMut() -> bool) {
        let deadline = Instant::now() + LOOP_DEADLINE;
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                self.fail(&format!("exited before {description}"));
            }
            if observed() {
                return;
            }
            if Instant::now() >= deadline {
                self.fail(&format!("deadline waiting for {description}"));
            }
            std::thread::sleep(LOOP_POLL_INTERVAL);
        }
    }

    /// Wait until the loop has listed the panes `ticks` more times.
    pub fn wait_ticks(&mut self, ticks: usize) {
        let cli = self.cli;
        let target = cli.shim_count("list-panes") + ticks;
        self.wait_until("further ticks", || cli.shim_count("list-panes") >= target);
    }

    pub fn stop(&mut self) -> Output {
        let pid = nix::unistd::Pid::from_raw(i32::try_from(self.pid()).expect("PID fits i32"));
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM)
            .expect("request graceful loop shutdown");
        self.wait_for_exit("graceful loop shutdown")
    }

    pub fn wait_for_exit(&mut self, description: &str) -> Output {
        let deadline = Instant::now() + LOOP_DEADLINE;
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.reap().expect("collect the completed loop");
            }
            if Instant::now() >= deadline {
                self.fail(&format!("deadline waiting for {description}"));
            }
            std::thread::sleep(LOOP_POLL_INTERVAL);
        }
    }
}

impl Drop for LoopChild<'_> {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.reap();
        }
    }
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

pub fn stdout(output: &Output) -> String {
    text(&output.stdout)
}

pub fn stderr(output: &Output) -> String {
    text(&output.stderr)
}

pub fn code(output: &Output) -> i32 {
    output.status.code().expect("the CLI exits normally")
}

pub fn write_file(path: &Path, contents: &[u8]) -> String {
    std::fs::write(path, contents).unwrap();
    path.to_str().unwrap().to_string()
}
