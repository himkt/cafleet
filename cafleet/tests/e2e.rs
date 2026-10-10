//! End-to-end binary tests (design § Success Criteria): atomic fleet create
//! (fleet + Director + monitor member + the detached monitor loop in one
//! invocation) → member create (tmux shim) → message send/poll/ack → one loop
//! tick waking the monitor member's pane, all against a fresh temp DB.

mod common;

use std::time::{Duration, Instant};

use common::{Cli, LoopChild, code, stderr, stdout};

const DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The loop `fleet create` started detached; stopped when the test ends.
struct DetachedLoop(i64);

impl DetachedLoop {
    fn terminate(&self) {
        let pid = nix::unistd::Pid::from_raw(i32::try_from(self.0).expect("PID fits i32"));
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
    }
}

impl Drop for DetachedLoop {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn wait_for(cli: &Cli, description: &str, mut observed: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !observed() {
        assert!(
            Instant::now() < deadline,
            "deadline waiting for {description}; log={:?}; shim={:?}",
            std::fs::read_to_string(cli.monitor_log_path(1)),
            cli.shim_calls()
        );
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn end_to_end_lifecycle_with_one_monitor_tick() {
    let mut cli = Cli::new();
    cli.set_env("CAFLEET_TEST_TMUX_PANES", "%0 %7 %8 %9");
    cli.set_env("CAFLEET_MONITOR_WAKE_INTERVAL", "1");
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
    assert_eq!(stdout(&output), cli.fleet_create_text(1, 1, 2));
    let pid = cli.monitor_pid(1).expect("fleet create started the loop");
    let monitor_loop = DetachedLoop(pid);

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

    let mut second = LoopChild::spawn(&cli, &["monitor", "1", "--tick", "1"]);
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

    let payload = format!(
        "[cafleet] tick: fleet 1 — health-check your 2 members: \
         {worker_id} (worker; coding_agent=claude; unacked=0), \
         {helper_id} (helper; coding_agent=claude; unacked=0). \
         Director: 1 (Director; coding_agent=claude; unacked=0). \
         Follow your monitor role protocol. \
         Resume your work if something was still running."
    );
    let wake_keystroke = format!("send-keys -t %7 -l {payload}");
    wait_for(&cli, "the committed monitor-directed wake", || {
        let committed = conn
            .query_row(
                "SELECT last_wake_at FROM monitor_runtime WHERE fleet_id=1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
            .is_some_and(|timestamp| !timestamp.is_empty());
        committed && cli.shim_calls().iter().any(|line| line == &wake_keystroke)
    });

    monitor_loop.terminate();
    wait_for(&cli, "the loop to clear its runtime row", || {
        cli.monitor_pid(1).is_none()
    });
    let log = std::fs::read_to_string(cli.monitor_log_path(1)).unwrap();
    let calls = cli.shim_calls();
    assert!(
        log.contains(&format!(
            "monitor loop started (fleet 1, tick 5s, pid {pid})"
        )),
        "log: {log}; shim: {calls:?}"
    );
    assert!(
        log.contains(&format!("tick -> wake monitor {monitor_id} (2 members)")),
        "log: {log}; shim: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .filter(|line| line.contains("[cafleet] tick:"))
            .all(|line| line.starts_with("send-keys -t %7 -l [cafleet] tick: fleet 1 ")),
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
