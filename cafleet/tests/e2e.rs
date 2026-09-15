//! End-to-end binary tests (design § Success Criteria): atomic fleet create
//! (fleet + Director + monitor member in one invocation) → member create
//! (tmux shim) → message send/poll/ack → one monitor tick waking the monitor
//! member's pane, all against a fresh temp DB.

mod common;

use std::io;
use std::process::{Child, Output};
use std::time::{Duration, Instant};

use common::{Cli, code, stderr, stdout, text};

const DEADLINE: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

struct MonitorChild<'a> {
    child: Option<Child>,
    cli: &'a Cli,
    name: &'static str,
}

impl<'a> MonitorChild<'a> {
    fn spawn(cli: &'a Cli, name: &'static str, args: &[&str]) -> Self {
        Self {
            child: Some(cli.spawn(args)),
            cli,
            name,
        }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("uncollected child").id()
    }

    fn reap(&mut self) -> io::Result<Output> {
        let child = self.child.as_mut().expect("collect each child once");
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        self.child.take().unwrap().wait_with_output()
    }

    fn fail(&mut self, reason: &str) -> ! {
        let output = self.reap().expect("terminate and reap failed monitor");
        panic!(
            "{}: {reason}; status={}; stdout={}; stderr={}; shim={:?}",
            self.name,
            output.status,
            stdout(&output),
            stderr(&output),
            self.cli.shim_calls()
        );
    }

    fn wait_until(&mut self, description: &str, mut observed: impl FnMut() -> bool) {
        let deadline = Instant::now() + DEADLINE;
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
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn stop(&mut self) -> Output {
        let pid = nix::unistd::Pid::from_raw(i32::try_from(self.id()).expect("child PID fits i32"));
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM)
            .expect("request graceful monitor shutdown");
        self.wait_for_exit("graceful monitor shutdown")
    }

    fn wait_for_exit(&mut self, description: &str) -> Output {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.reap().expect("collect completed monitor");
            }
            if Instant::now() >= deadline {
                self.fail(&format!("deadline waiting for {description}"));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for MonitorChild<'_> {
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        match self.reap() {
            Ok(output) => eprintln!(
                "{} reaped after assertion failure: status={}; stdout={}; stderr={}; shim={:?}",
                self.name,
                output.status,
                stdout(&output),
                stderr(&output),
                self.cli.shim_calls()
            ),
            Err(error) if std::thread::panicking() => {
                eprintln!("{} cleanup failed: {error}", self.name)
            }
            Err(error) => panic!("{} cleanup failed: {error}", self.name),
        }
    }
}

fn install_distinct_pane_shim(cli: &Cli) {
    std::fs::write(
        cli.shim_dir.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
case "$1" in
    display-message) printf 'main|@1|%%0\n' ;;
    split-window) printf '%s\n' "${CAFLEET_TEST_NEXT_PANE:?required next pane}" ;;
    list-panes) printf '%%0\n%%7\n%%8\n%%9\n' ;;
esac
"#,
    )
    .unwrap();
}

#[test]
fn end_to_end_lifecycle_with_one_monitor_tick() {
    let mut cli = Cli::new();
    install_distinct_pane_shim(&cli);
    cli.set_env("CAFLEET_TEST_NEXT_PANE", "%7");
    cli.install();

    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "e2e",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &cli.monitor_prompt_path(),
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "1 director=1 monitor=2\n");

    let monitor_id = Cli::BOOTSTRAP_MONITOR_ID;
    cli.set_env("CAFLEET_TEST_NEXT_PANE", "%8");
    let worker_id = cli.create_member(1, "worker");
    cli.set_env("CAFLEET_TEST_NEXT_PANE", "%9");
    let helper_id = cli.create_member(1, "helper");
    let conn = cli.sqlite();
    for (member, pane) in [
        (1, "%0"),
        (monitor_id, "%7"),
        (worker_id, "%8"),
        (helper_id, "%9"),
    ] {
        assert_eq!(
            conn.query_row(
                "SELECT mux_pane_id FROM member_placements WHERE member_id=?1",
                [member],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            pane
        );
    }

    let output = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        "1",
        "--to-member-id",
        &worker_id.to_string(),
        "e2e task",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let message_id: i64 = cli
        .sqlite()
        .query_row(
            "SELECT message_id FROM messages WHERE type='unicast'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let output = cli.run(&["message", "poll", &worker_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("e2e task"),
        "got: {}",
        stdout(&output)
    );

    let output = cli.run(&["message", "ack", &message_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(stdout(&output).starts_with("Message acknowledged.\n"));

    let output = cli.run(&["message", "poll", &worker_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("No messages found."));

    let payload = format!(
        "[cafleet] tick: fleet 1 — health-check your 2 members: \
         {worker_id} (worker; coding_agent=claude; unacked=0), \
         {helper_id} (helper; coding_agent=claude; unacked=0). \
         Director: 1 (Director; coding_agent=claude; unacked=0). \
         Follow your monitor role protocol. \
         Resume your work if something was still running."
    );

    let mut child = MonitorChild::spawn(
        &cli,
        "primary monitor",
        &["monitor", "1", "--tick", "1", "--interval", "1"],
    );
    let pid = i64::from(child.id());
    child.wait_until("owned live runtime slot", || conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM monitor_runtime WHERE fleet_id=1 AND pid=?1 AND last_tick_at IS NOT NULL)",
        [pid], |row| row.get::<_, bool>(0),
    ).unwrap());

    let mut second = MonitorChild::spawn(&cli, "second monitor", &["monitor", "1", "--tick", "1"]);
    let output = second.wait_for_exit("second monitor refusal");
    assert_eq!(
        code(&output),
        1,
        "the atomic claim refuses a second loop; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("Error: monitor already running for fleet 1"),
        "got: {}",
        stderr(&output)
    );

    let wake_keystroke = format!("send-keys -t %7 -l {payload}");
    child.wait_until("committed monitor-directed wake", || {
        let committed = conn
            .query_row(
                "SELECT last_wake_at FROM monitor_runtime WHERE fleet_id=1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
            .is_some_and(|timestamp| !timestamp.is_empty());
        let calls = cli.shim_calls();
        committed && calls.iter().any(|line| line == &wake_keystroke)
    });
    let loop_output = child.stop();
    let loop_stdout = text(&loop_output.stdout);
    let calls = cli.shim_calls();
    assert_eq!(
        code(&loop_output),
        0,
        "stdout: {loop_stdout}; stderr: {}; shim: {calls:?}",
        stderr(&loop_output)
    );
    assert!(
        loop_stdout.contains(&format!(
            "monitor loop started (fleet 1, tick 1s, pid {pid})"
        )),
        "stdout: {loop_stdout}; stderr: {}; shim: {calls:?}",
        stderr(&loop_output)
    );
    assert!(
        loop_stdout.contains(&format!("tick -> wake monitor {monitor_id} (2 members)")),
        "stdout: {loop_stdout}; stderr: {}; shim: {calls:?}",
        stderr(&loop_output)
    );
    assert!(
        calls.iter().any(|line| line == &wake_keystroke),
        "monitor-directed wake: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .filter(|line| line.contains("[cafleet] tick:"))
            .all(|line| line == &wake_keystroke),
        "every wake targets the monitor, including no Director-directed wake: {calls:?}"
    );
    let last_wake: String = conn
        .query_row(
            "SELECT last_wake_at FROM monitor_runtime WHERE fleet_id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    chrono::DateTime::parse_from_rfc3339(&last_wake).expect("successful wake timestamp");
}
