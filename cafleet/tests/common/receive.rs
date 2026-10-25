#![allow(dead_code)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Output};
use std::time::{Duration, Instant};

use crate::common::{Cli, code, stderr, stdout, write_executable};

const SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
stage=other
pane=
case "$1" in
    capture-pane)
        stage=capture
        shift
        while [ "$#" -gt 0 ]; do
            if [ "$1" = "-t" ]; then shift; pane="$1"; break; fi
            shift
        done
        ;;
    send-keys)
        shift
        while [ "$#" -gt 0 ]; do
            if [ "$1" = "-t" ]; then shift; pane="$1"; shift; break; fi
            shift
        done
        case "$1" in
            -l) stage=literal ;;
            Enter) stage=enter ;;
            Escape) stage=escape ;;
            *) stage=key ;;
        esac
        ;;
    display-message) printf 'main|@1|%%0\n'; exit 0 ;;
esac
if [ "$stage" = capture ]; then
    printf 'unexpected capture-pane\n' >&2
    exit 1
fi
printf '%s\t%s\t%s\t\n' "$PPID" "$stage" "$pane" >> "$CAFLEET_TEST_RECEIVE_EVENTS"
control="$CAFLEET_TEST_RECEIVE_CONTROL"
if [ -f "$control/gate-$stage-$pane" ]; then
    loops=0
    while [ ! -e "$control/release-$stage-$pane" ] && [ "$loops" -lt 600 ]; do
        /bin/sleep 0.01
        loops=$((loops + 1))
    done
fi
if [ -f "$control/delay-$stage-$pane" ]; then
    IFS= read -r delay < "$control/delay-$stage-$pane"
    /bin/sleep "$delay"
fi
if [ -f "$control/fail-$stage-$pane" ]; then
    IFS= read -r cause < "$control/fail-$stage-$pane"
    printf '%s\tfailed\t%s\t%s\n' "$PPID" "$pane" "$cause" >> "$CAFLEET_TEST_RECEIVE_EVENTS"
    printf '%s\n' "$cause" >&2
    exit 1
fi
printf '%s\t%s_done\t%s\t\n' "$PPID" "$stage" "$pane" >> "$CAFLEET_TEST_RECEIVE_EVENTS"
"#;

#[derive(Clone)]
pub struct Worker {
    pub id: i64,
    pub pane: String,
}

pub struct Fixture {
    pub cli: Cli,
    pub fleet: i64,
    pub director: i64,
    pub control: PathBuf,
    pub event_path: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let mut cli = Cli::new();
        let (fleet, director) = cli.seeded_fleet();
        let control = cli.home.path().join("receive-control");
        std::fs::create_dir(&control).unwrap();
        let event_path = cli.home.path().join("receive-events");
        cli.set_env("CAFLEET_TEST_RECEIVE_CONTROL", control.to_str().unwrap());
        cli.set_env("CAFLEET_TEST_RECEIVE_EVENTS", event_path.to_str().unwrap());
        write_executable(&cli.shim_dir.join("tmux"), SHIM);
        Self {
            cli,
            fleet,
            director,
            control,
            event_path,
        }
    }

    pub fn worker(&self, name: &str) -> Worker {
        let id = self.cli.seed_member(self.fleet, name);
        let pane = format!("%{}", id + 4);
        Worker { id, pane }
    }

    /// The Director's seeded pane, addressable like a worker's.
    pub fn director_pane(&self) -> Worker {
        Worker {
            id: self.director,
            pane: "%0".into(),
        }
    }

    pub fn message(&self, sender: i64, recipient: i64, body: &str) -> i64 {
        self.cli.seed_message(sender, recipient, body)
    }

    pub fn assignment(&self, worker: &Worker, body: &str) -> i64 {
        self.message(self.director, worker.id, body)
    }

    pub fn reply(&self, worker: &Worker, body: &str) -> i64 {
        self.message(worker.id, self.director, body)
    }

    pub fn spawn(&self, subject: i64, timeout: u64) -> Run {
        self.spawn_args(
            &[
                "message",
                "wait",
                &subject.to_string(),
                "--timeout",
                &timeout.to_string(),
                "--json",
            ],
            timeout + 7,
        )
    }

    pub fn spawn_args(&self, args: &[&str], cleanup_seconds: u64) -> Run {
        Run {
            child: Some(self.cli.spawn(args)),
            started: Instant::now(),
            bound: Duration::from_secs(cleanup_seconds),
            control: self.control.clone(),
        }
    }

    pub fn events(&self) -> Vec<Event> {
        let content = match std::fs::read_to_string(&self.event_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => panic!("cannot read receive events: {error}"),
        };
        content
            .lines()
            .map(|line| {
                let fields: Vec<_> = line.splitn(4, '\t').collect();
                assert_eq!(fields.len(), 4, "{line:?}");
                Event {
                    pid: fields[0].parse().unwrap(),
                    stage: fields[1].into(),
                    pane: fields[2].into(),
                    detail: fields[3].into(),
                }
            })
            .collect()
    }

    pub fn count(&self, stage: &str, worker: &Worker) -> usize {
        self.events()
            .iter()
            .filter(|event| event.stage == stage && event.pane == worker.pane)
            .count()
    }

    pub fn refuse_escape(&self, worker: &Worker) {
        self.fail(worker, "escape", "escape refused by test");
    }

    pub fn fail(&self, worker: &Worker, stage: &str, cause: &str) {
        assert!(!cause.is_empty());
        std::fs::write(
            self.control.join(format!("fail-{stage}-{}", worker.pane)),
            format!("{cause}\n"),
        )
        .unwrap();
    }

    pub fn delay(&self, worker: &Worker, stage: &str, seconds: &str) {
        std::fs::write(
            self.control.join(format!("delay-{stage}-{}", worker.pane)),
            format!("{seconds}\n"),
        )
        .unwrap();
    }

    pub fn gate(&self, worker: &Worker, stage: &str) -> Gate {
        let key = format!("{stage}-{}", worker.pane);
        std::fs::write(self.control.join(format!("gate-{key}")), b"gate").unwrap();
        Gate {
            release: self.control.join(format!("release-{key}")),
        }
    }

    pub fn lock(&self, worker: &Worker) -> File {
        self.cli.lock_pane(&worker.pane)
    }

    pub fn stamp(&self, id: i64) -> Option<String> {
        self.cli
            .sqlite()
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn ack(&self, id: i64) {
        let output = self.cli.run(&["message", "ack", &id.to_string(), "--json"]);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
    }

    pub fn records(&self) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        let conn = self.cli.sqlite();
        [
            "fleets",
            "members",
            "member_placements",
            "messages",
            "sqlite_sequence",
        ]
        .iter()
        .map(|table| {
            let mut statement = conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let width = statement.column_count();
            statement
                .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
        .collect()
    }
}

pub struct Event {
    pub pid: u32,
    pub stage: String,
    pub pane: String,
    pub detail: String,
}

pub struct Gate {
    release: PathBuf,
}

impl Gate {
    pub fn release(&self) {
        std::fs::write(&self.release, b"release").unwrap();
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

pub struct Run {
    child: Option<Child>,
    pub started: Instant,
    bound: Duration,
    control: PathBuf,
}

impl Run {
    pub fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    pub fn assert_running(&mut self) {
        if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = self.child.take().unwrap().wait_with_output().unwrap();
            panic!(
                "wait exited early: status={}, stdout={}, stderr={}",
                output.status,
                stdout(&output),
                stderr(&output)
            );
        }
    }

    pub fn until(&mut self, description: &str, mut observed: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if observed() {
                return;
            }
            self.assert_running();
            assert!(
                Instant::now() < deadline,
                "deadline observing {description}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn signal(&self, signal: nix::sys::signal::Signal) {
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(self.pid() as i32), signal).unwrap();
    }

    pub fn output(mut self) -> Output {
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.child.take().unwrap().wait_with_output().unwrap();
            }
            if self.started.elapsed() >= self.bound {
                release_all(&self.control);
                let mut child = self.child.take().unwrap();
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "finite receive exceeded cleanup bound: stdout={}, stderr={}",
                    stdout(&output),
                    stderr(&output)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn release_all(control: &Path) {
    for entry in std::fs::read_dir(control).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if let Some(key) = name.strip_prefix("gate-") {
            std::fs::write(control.join(format!("release-{key}")), b"release").unwrap();
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        release_all(&self.control);
        if let Some(mut child) = self.child.take() {
            if child.try_wait().unwrap().is_none() {
                child.kill().unwrap();
            }
            child.wait().unwrap();
        }
    }
}

pub fn json(output: &Output) -> serde_json::Value {
    assert_eq!(code(output), 0, "{}", stderr(output));
    serde_json::from_str(&stdout(output)).unwrap()
}

pub fn assert_result(value: &serde_json::Value, subject: i64, outcome: &str) {
    assert_keys(
        value,
        &["member_id", "outcome", "messages", "notification_errors"],
    );
    assert_eq!(value["member_id"], subject);
    assert_eq!(value["outcome"], outcome);
    assert!(value["messages"].is_array());
    assert!(value["notification_errors"].is_array());
    if outcome == "messages" {
        assert!(!value["messages"].as_array().unwrap().is_empty());
    } else {
        assert!(value["messages"].as_array().unwrap().is_empty());
    }
}

pub fn assert_keys(value: &serde_json::Value, expected: &[&str]) {
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

/// No text reached any pane: no literal write and no Enter (an Escape attempt
/// is allowed).
pub fn assert_no_text_input(fixture: &Fixture) {
    assert!(
        fixture.cli.shim_calls().iter().all(|line| {
            !(line.starts_with("send-keys") && (line.contains(" -l ") || line.ends_with(" Enter")))
        }),
        "{:?}",
        fixture.cli.shim_calls()
    );
}

pub fn assert_no_capture(fixture: &Fixture) {
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.starts_with("capture-pane")),
        "{:?}",
        fixture.cli.shim_calls()
    );
}

pub fn assert_no_input(fixture: &Fixture) {
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.starts_with("send-keys")),
        "{:?}",
        fixture.cli.shim_calls()
    );
}
