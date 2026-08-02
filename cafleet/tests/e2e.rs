//! Step 9 end-to-end binary tests (design § Success Criteria): fleet create →
//! member create (tmux shim) → message send/poll/ack → one monitor tick, all
//! against a fresh temp DB.

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
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "1 director=1\n");

    let worker_id = cli.create_member(1, "worker");
    let helper_id = cli.create_member(1, "helper");
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        "1",
        "--name",
        "watch",
        "--description",
        "the watcher",
        "--role",
        "monitor",
        "--text",
        "follow your monitor role protocol",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let output = cli.run(&[
        "message",
        "send",
        "--fleet-id",
        "1",
        "--from-member-id",
        "1",
        "--to-member-id",
        &worker_id.to_string(),
        "--text",
        "e2e task",
        "--quiet",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let message_id: i64 = stdout(&output).trim().parse().unwrap();

    let output = cli.run(&[
        "message",
        "poll",
        "--fleet-id",
        "1",
        "--member-id",
        &worker_id.to_string(),
    ]);
    assert!(
        stdout(&output).contains("e2e task"),
        "got: {}",
        stdout(&output)
    );

    let output = cli.run(&[
        "message",
        "ack",
        "--fleet-id",
        "1",
        "--member-id",
        &worker_id.to_string(),
        "--message-id",
        &message_id.to_string(),
    ]);
    assert!(stdout(&output).starts_with("Message acknowledged.\n"));

    let output = cli.run(&[
        "message",
        "poll",
        "--fleet-id",
        "1",
        "--member-id",
        &worker_id.to_string(),
    ]);
    assert!(stdout(&output).contains("No messages found."));

    let mut child = cli.spawn(&["monitor", "start", "--fleet-id", "1", "--tick", "1"]);
    std::thread::sleep(Duration::from_secs(1));

    let second = cli.run(&["monitor", "start", "--fleet-id", "1", "--tick", "1"]);
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
        "the ready-handshake startup line, got: {loop_stdout}"
    );
    assert!(
        loop_stdout.contains(&format!("due member {helper_id} (helper) [")),
        "got: {loop_stdout}"
    );
    assert!(
        loop_stdout.contains(&format!("due member {worker_id} (worker) [")),
        "got: {loop_stdout}"
    );
    assert!(
        loop_stdout.contains("-> wake monitor"),
        "got: {loop_stdout}"
    );

    assert!(
        cli.shim_calls()
            .iter()
            .any(|line| line.contains("[monitor] wake: 2 members due")),
        "the wake keystroke reached the watcher's pane"
    );

    let conn = cli.sqlite();
    for member_id in [worker_id, helper_id] {
        let last_ping: Option<String> = conn
            .query_row(
                "SELECT last_ping_at FROM monitor_config WHERE member_id=?1",
                [member_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            last_ping.is_some(),
            "the successful wake advanced member {member_id}'s last_ping_at"
        );
    }
}

#[test]
fn monitor_start_without_a_watcher_warns_and_still_runs() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.with_fleet();

    let mut child = cli.spawn(&[
        "monitor",
        "start",
        "--fleet-id",
        &fleet_id.to_string(),
        "--tick",
        "1",
    ]);
    std::thread::sleep(Duration::from_millis(1500));
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(
        text(&output.stderr).contains(&format!(
            "Warning: fleet {fleet_id} has no monitoring member; the monitor heartbeat \
             will wake no member. Spawn one first with 'cafleet member create --role monitor'."
        )),
        "the warn-but-run line goes to stderr, got: {}",
        text(&output.stderr)
    );
    assert!(
        text(&output.stdout).contains(&format!(
            "monitor loop started (fleet {fleet_id}, tick 1s, pid "
        )),
        "got: {}",
        text(&output.stdout)
    );
}
