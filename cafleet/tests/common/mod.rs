#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

use cafleet::broker;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn process_test_resource_guard() -> std::sync::MutexGuard<'static, ()> {
    static RESOURCE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // Each test owns its fixture state, so a prior failure only releases this slot.
    RESOURCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The capture every pane shows unless [`Cli::set_pane_capture`] overrides it:
/// a claude composer at rest, so a delivery fires immediately.
pub const REST_CAPTURE: &str = "pane:{pane}\nline1\nline2\n> at rest";

/// A capture file holding this line makes `capture-pane` fail for that pane.
pub const CAPTURE_FAILS: &str = "CAPTURE-FAILS";

// `CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT` scripts `list-panes` per call: the Nth
// call takes the Nth letter (`F` fails, anything else succeeds) and calls past
// the end repeat the last letter. The shim has no PATH beyond itself, so the
// call count is read back from its own log with shell builtins only.
//
// `CAFLEET_TEST_TMUX_BLOCK` (`literal` or `split-window`) makes the matching
// call create `$CAFLEET_TEST_TMUX_RELEASE.blocked` and spin on builtins until
// `$CAFLEET_TEST_TMUX_RELEASE` exists.
const TMUX_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
if [ -n "$CAFLEET_TEST_TMUX_BLOCK" ]; then
    case "$CAFLEET_TEST_TMUX_BLOCK:$1: $* " in
        literal:send-keys:*" -l "*|split-window:split-window:*)
            : > "$CAFLEET_TEST_TMUX_RELEASE.blocked"
            while [ ! -e "$CAFLEET_TEST_TMUX_RELEASE" ]; do :; done
            ;;
    esac
fi
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

    /// Make every later `kind` call (`literal` for `send-keys -l`, or
    /// `split-window`) block until [`Cli::release_tmux`].
    pub fn block_tmux(&mut self, kind: &str) {
        let release = self.home.path().join("tmux-release");
        self.set_env("CAFLEET_TEST_TMUX_BLOCK", kind);
        self.set_env("CAFLEET_TEST_TMUX_RELEASE", release.to_str().unwrap());
    }

    /// Wait until a blocked fake `tmux` call has started.
    pub fn wait_for_tmux_block(&self) {
        let marker = self.home.path().join("tmux-release.blocked");
        let started = std::time::Instant::now();
        while !marker.exists() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "the fake tmux call never blocked"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn release_tmux(&self) {
        std::fs::write(self.home.path().join("tmux-release"), "").unwrap();
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
        self.run_with_stdin_bytes(args, stdin.as_bytes())
    }

    pub fn run_with_stdin_bytes(&self, args: &[&str], stdin: &[u8]) -> Output {
        use std::io::Write;
        let mut child = self
            .command(args, true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
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

    pub fn seed_incompatible_database(&self) {
        self.sqlite()
            .execute_batch(
                "CREATE TABLE refinery_schema_history (
                     version INT4 PRIMARY KEY,
                     name VARCHAR(255),
                     applied_on VARCHAR(255),
                     checksum VARCHAR(255)
                 );
                 INSERT INTO refinery_schema_history (version, name, applied_on, checksum) VALUES
                     (1, 'baseline', '2026-01-01T00:00:00Z', '0');
                 CREATE TABLE asset_installs (
                     coding_agent TEXT NOT NULL,
                     path TEXT NOT NULL,
                     cafleet_version TEXT NOT NULL,
                     installed_at TEXT NOT NULL,
                     PRIMARY KEY (coding_agent, path)
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

    pub fn with_cli_fleet(&self) -> (i64, i64) {
        self.ready();
        let output = self.run(&[
            "fleet",
            "create",
            "--name",
            "testfleet",
            "--coding-agent",
            "claude",
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

    pub fn seeded_fleet(&self) -> (i64, i64) {
        self.ready();
        self.seed_fleet("testfleet")
    }

    pub fn seed_fleet(&self, name: &str) -> (i64, i64) {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let tx = conn.transaction().unwrap();
        let timestamp = "2026-10-23T00:00:00.000000+00:00";
        tx.execute(
            "INSERT INTO fleets(name, created_at) VALUES(?1, ?2)",
            rusqlite::params![name, timestamp],
        )
        .unwrap();
        let fleet = tx.last_insert_rowid();
        tx.execute("INSERT INTO members(fleet_id, name, description, status, registered_at) VALUES(?1, 'Director', 'Root Director for this fleet', 'active', ?2)", rusqlite::params![fleet, timestamp]).unwrap();
        let director = tx.last_insert_rowid();
        tx.execute("INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) VALUES(?1, 'main', '@1', '%0', 'tmux', 'claude', ?2)", rusqlite::params![director, timestamp]).unwrap();
        tx.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            rusqlite::params![director, fleet],
        )
        .unwrap();
        tx.commit().unwrap();
        (fleet, director)
    }

    pub fn seed_member(&self, fleet_id: i64, name: &str) -> i64 {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let tx = conn.transaction().unwrap();
        let timestamp = "2026-10-23T00:00:00.000000+00:00";
        tx.execute("INSERT INTO members(fleet_id, name, description, status, registered_at) VALUES(?1, ?2, 'test member', 'active', ?3)", rusqlite::params![fleet_id, name, timestamp]).unwrap();
        let member = tx.last_insert_rowid();
        let pane = format!("%{}", member + 4);
        tx.execute("INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) VALUES(?1, 'main', '@1', ?2, 'tmux', 'claude', ?3)", rusqlite::params![member, pane, timestamp]).unwrap();
        tx.commit().unwrap();
        member
    }

    pub fn seed_message(&self, sender: i64, recipient: i64, body: &str) -> i64 {
        let mut conn = cafleet::db::connect(&self.db_url()).unwrap();
        let message =
            broker::send_message(&mut conn, sender, &recipient.to_string(), body).unwrap();
        assert_eq!(message.text, body);
        message.message_id
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

    pub fn spawn_with_stdin(&self, args: &[&str]) -> std::process::Child {
        self.command(args, true)
            .stdin(Stdio::piped())
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

pub fn write_executable(path: &Path, contents: &str) {
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
