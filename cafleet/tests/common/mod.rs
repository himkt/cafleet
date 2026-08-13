//! Shared fixture for the CLI integration tests (Step 6, SPEC §6.3/§10):
//! drives the compiled `cafleet` binary against a temp `CAFLEET_DATABASE_URL`,
//! a temp `HOME`, and a fake `tmux` shim on `PATH` that records every argv
//! line to a log (SPEC §9 *CLI conformance*).
//!
//! The shim answers `display-message` with `main|@1|%0`, `split-window` with
//! `%7`, `capture-pane` with a canned two-line buffer, and `list-panes` with
//! `%0`/`%7`; setting `fail_subcommand` makes exactly that tmux subcommand
//! exit non-zero (for rollback-path tests).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const TMUX_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
if [ -n "$CAFLEET_TEST_TMUX_FAIL" ] && [ "$1" = "$CAFLEET_TEST_TMUX_FAIL" ]; then
    echo "forced failure" >&2
    exit 1
fi
case "$1" in
    display-message) printf 'main|@1|%%0\n' ;;
    split-window) printf '%%7\n' ;;
    capture-pane) printf 'line1\nline2\n' ;;
    list-panes) printf '%%0\n%%7\n' ;;
esac
exit 0
"#;

pub struct Cli {
    pub home: TempDir,
    pub shim_dir: PathBuf,
    pub shim_log: PathBuf,
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
        Cli {
            home,
            shim_dir,
            shim_log,
            fail_subcommand: None,
            extra_env: Vec::new(),
        }
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
            .env("CAFLEET_TEST_TMUX_LOG", &self.shim_log);
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

    /// Migrate to head via plain `cafleet setup`, then strip the recorded
    /// installs and installed asset dirs so every fixture starts from a
    /// records-free database regardless of setup's install-all-agents
    /// assets half.
    pub fn migrate(&self) {
        let output = self.run(&["setup"]);
        assert!(
            output.status.success(),
            "plain setup must succeed: {}",
            text(&output.stderr)
        );
        self.sqlite()
            .execute("DELETE FROM asset_installs", [])
            .unwrap();
        for dir in [".claude", ".codex", ".config/opencode", ".opencode"] {
            let path = self.home.path().join(dir);
            if path.exists() {
                std::fs::remove_dir_all(&path).unwrap();
            }
        }
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

    /// Migrate + record a current-version install so the stale-assets guard
    /// passes.
    pub fn ready(&self) {
        self.migrate();
        self.seed_asset_row("claude", VERSION);
    }

    /// `ready()` + a bare fleet (id 1, director member 1) via `fleet create` —
    /// no monitor member yet, for the monitor-first / one-per-fleet guard
    /// tests.
    pub fn with_bare_fleet(&self) -> (i64, i64) {
        self.ready();
        let output = self.run(&[
            "fleet",
            "create",
            "--name",
            "testfleet",
            "--coding-agent",
            "claude",
        ]);
        assert!(
            output.status.success(),
            "fleet create must succeed: {}",
            text(&output.stderr)
        );
        (1, 1)
    }

    /// `with_bare_fleet()` + the fleet's monitor member (id 2) via
    /// `member create --role monitor`, satisfying the monitor-first guard.
    pub fn with_fleet(&self) -> (i64, i64) {
        let ids = self.with_bare_fleet();
        self.create_monitor(ids.0);
        ids
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

    /// Spawn a long-running command (e.g. `monitor start`) inside the fake
    /// tmux context, with stdout/stderr piped for later collection.
    pub fn spawn(&self, args: &[&str]) -> std::process::Child {
        self.command(args, true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    pub fn shim_calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.shim_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
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
