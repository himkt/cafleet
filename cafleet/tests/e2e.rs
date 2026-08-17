//! End-to-end binary tests (design § Success Criteria): atomic fleet create
//! (fleet + Director + monitor member in one invocation) → member create
//! (tmux shim) → message send/poll/ack → one monitor tick waking the monitor
//! member's pane, all against a fresh temp DB.

mod common;

use std::time::Duration;

use common::{Cli, code, stderr, stdout, text};

#[test]
fn end_to_end_lifecycle_with_one_monitor_tick() {
    let cli = Cli::new();
    cli.ready();

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
    let worker_id = cli.create_member(1, "worker");
    let helper_id = cli.create_member(1, "helper");

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
    assert!(
        stdout(&output).contains("e2e task"),
        "got: {}",
        stdout(&output)
    );

    let output = cli.run(&["message", "ack", &message_id.to_string()]);
    assert!(stdout(&output).starts_with("Message acknowledged.\n"));

    let output = cli.run(&["message", "poll", &worker_id.to_string()]);
    assert!(stdout(&output).contains("No messages found."));

    let mut child = cli.spawn(&["monitor", "1", "--tick", "1", "--interval", "1"]);
    std::thread::sleep(Duration::from_secs(1));

    let second = cli.run(&["monitor", "1", "--tick", "1"]);
    assert_eq!(code(&second), 1, "the atomic claim refuses a second loop");
    assert!(
        stderr(&second).contains("Error: monitor already running for fleet 1"),
        "got: {}",
        stderr(&second)
    );

    std::thread::sleep(Duration::from_secs(2));
    child.kill().unwrap();
    let loop_output = child.wait_with_output().unwrap();
    let loop_stdout = text(&loop_output.stdout);
    assert!(
        loop_stdout.contains("monitor loop started (fleet 1, tick 1s, pid "),
        "the startup confirmation line, got: {loop_stdout}"
    );
    assert!(
        loop_stdout.contains(&format!("tick -> wake monitor {monitor_id} (2 members)")),
        "got: {loop_stdout}"
    );

    let payload = format!(
        "[cafleet] tick: fleet 1 — health-check your 2 members: \
         {worker_id} (worker; coding_agent=claude; unacked=0), \
         {helper_id} (helper; coding_agent=claude; unacked=0). \
         Director: 1 (Director; coding_agent=claude; unacked=0). \
         Follow your monitor role protocol. \
         Resume your work if something was still running."
    );
    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.contains("send-keys -t %7 -l") && line.contains(&payload)),
        "the wake keystroke reached the monitor member's pane, got: {:?}",
        cli.shim_calls()
    );
    assert!(
        !cli.shim_calls()
            .iter()
            .any(|line| line.contains("-t %0") && line.contains("[cafleet] tick:")),
        "no code path keystrokes the Director's pane on a timer, got: {:?}",
        cli.shim_calls()
    );

    let conn = cli.sqlite();
    let last_wake: Option<String> = conn
        .query_row(
            "SELECT last_wake_at FROM monitor_runtime WHERE fleet_id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        last_wake.is_some(),
        "the successful wake stamped the fleet's last_wake_at"
    );
}
