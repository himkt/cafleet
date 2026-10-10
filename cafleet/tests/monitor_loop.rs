//! The monitor loop's delivery pass and loop start, driven through the binary
//! against the tmux shim.

mod common;

use std::process::Output;

use cafleet::time::format_utc;
use chrono::{Duration, Utc};
use common::{CAPTURE_FAILS, Cli, LoopChild, REST_CAPTURE, code, stderr, stdout};

const FLEET: i64 = 1;
const DIRECTOR: i64 = 1;
const MONITOR: i64 = 2;
const WORKER: i64 = 3;
const DIRECTOR_PANE: &str = "%0";
const MONITOR_PANE: &str = "%2";
const WORKER_PANE: &str = "%7";

const WORKING_CAPTURE: &str = "✻ Cogitating… (12s · esc to interrupt)\n\n> \n";

/// Fleet 1 with the Director on `%0`, the monitor member on `%2` and one
/// worker on `%7`; the shim lists `%0` and `%7` as live unless overridden.
fn fleet_with_worker(cli: &Cli) {
    assert_eq!(cli.seeded_fleet(), (FLEET, DIRECTOR));
    assert_eq!(cli.seed_member(FLEET, "worker"), WORKER);
}

fn rest(cli: &Cli, pane: &str) {
    cli.set_pane_capture(pane, &REST_CAPTURE.replace("{pane}", pane));
}

fn send(cli: &Cli, from: i64, to: i64, text: &str) -> Output {
    cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &from.to_string(),
        "--to-member-id",
        &to.to_string(),
        text,
        "--json",
    ])
}

fn sent_message(output: &Output) -> (i64, bool) {
    assert_eq!(code(output), 0, "stderr: {}", stderr(output));
    let payload: serde_json::Value = serde_json::from_str(stdout(output).trim()).unwrap();
    (
        payload["message"]["message_id"].as_i64().unwrap(),
        payload["notification_sent"].as_bool().unwrap(),
    )
}

fn preview_keystrokes(cli: &Cli, pane: &str, message_id: i64) -> usize {
    cli.shim_count(&format!(
        "send-keys -t {pane} -l [cafleet msg {message_id} from "
    ))
}

fn notified(cli: &Cli, message_id: i64) -> bool {
    cli.sqlite()
        .query_row(
            "SELECT notified_at IS NOT NULL FROM messages WHERE message_id=?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn live_loops(cli: &Cli) -> i64 {
    cli.sqlite()
        .query_row(
            "SELECT COUNT(*) FROM monitor_runtime WHERE pid IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

/// Terminates a loop that a command under test started detached.
struct DetachedLoop(i64);

impl Drop for DetachedLoop {
    fn drop(&mut self) {
        let pid = nix::unistd::Pid::from_raw(i32::try_from(self.0).expect("PID fits i32"));
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
    }
}

#[test]
fn a_preview_held_on_a_working_pane_fires_on_a_later_tick_once_the_pane_rests() {
    let cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_pane_capture(WORKER_PANE, WORKING_CAPTURE);
    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);

    let (message_id, notification_sent) =
        sent_message(&send(&cli, DIRECTOR, WORKER, "first assignment"));
    assert!(
        !notification_sent,
        "a working pane holds the sender's attempt"
    );
    monitor.wait_ticks(2);
    assert_eq!(preview_keystrokes(&cli, WORKER_PANE, message_id), 0);
    assert!(!notified(&cli, message_id));

    rest(&cli, WORKER_PANE);
    monitor.wait_until("the held preview to be keystroked", || {
        preview_keystrokes(&cli, WORKER_PANE, message_id) == 1 && notified(&cli, message_id)
    });
    monitor.wait_ticks(2);
    assert_eq!(
        preview_keystrokes(&cli, WORKER_PANE, message_id),
        1,
        "a held message is keystroked exactly once"
    );

    let output = monitor.stop();
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(&format!(
            "tick -> preview msg {message_id} member {WORKER}\n"
        )),
        "stdout: {}",
        stdout(&output)
    );
}

#[test]
fn a_tick_with_nothing_owed_captures_no_pane_and_sends_no_keystroke() {
    let cli = Cli::new();
    fleet_with_worker(&cli);
    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);
    monitor.wait_ticks(3);
    let output = monitor.stop();
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let calls = cli.shim_calls();
    assert!(
        calls
            .iter()
            .all(|line| !line.starts_with("capture-pane") && !line.starts_with("send-keys")),
        "{calls:?}"
    );
}

#[test]
fn the_delivery_pass_runs_with_the_wake_interval_zero() {
    let cli = Cli::new();
    fleet_with_worker(&cli);
    let message_id = cli.seed_message(DIRECTOR, WORKER, "queued before the loop");
    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);

    monitor.wait_until("the queued preview to be keystroked", || {
        preview_keystrokes(&cli, WORKER_PANE, message_id) == 1 && notified(&cli, message_id)
    });
    let output = monitor.stop();
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let calls = cli.shim_calls();
    assert!(
        calls.iter().all(|line| !line.contains("[cafleet] tick:")),
        "interval 0 sends no wake: {calls:?}"
    );
}

#[test]
fn a_missing_director_pane_stops_the_loop() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_env("CAFLEET_TEST_TMUX_PANES", WORKER_PANE);

    let mut monitor = LoopChild::spawn(&cli, &["monitor", "1", "--tick", "1"]);
    let output = monitor.wait_for_exit("the stop for a missing Director pane");
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains(&format!(
            "director pane {DIRECTOR_PANE} is gone; stopping\n"
        )),
        "stdout: {}",
        stdout(&output)
    );
    assert_eq!(cli.monitor_pid(FLEET), None);
}

#[test]
fn a_per_pane_capture_error_does_not_abort_the_tick() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    let second_worker = cli.seed_member(FLEET, "helper");
    let second_pane = format!("%{}", second_worker + 4);
    cli.set_env(
        "CAFLEET_TEST_TMUX_PANES",
        &format!("{DIRECTOR_PANE} {WORKER_PANE} {second_pane}"),
    );
    cli.set_pane_capture(WORKER_PANE, CAPTURE_FAILS);
    let failing = cli.seed_message(DIRECTOR, WORKER, "behind the failing capture");
    let healthy = cli.seed_message(DIRECTOR, second_worker, "behind the healthy capture");

    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);
    monitor.wait_until("the healthy pane's preview", || {
        preview_keystrokes(&cli, &second_pane, healthy) == 1
    });
    monitor.wait_ticks(2);
    assert_eq!(preview_keystrokes(&cli, WORKER_PANE, failing), 0);
    assert!(!notified(&cli, failing));

    rest(&cli, WORKER_PANE);
    monitor.wait_until("the recovered pane's preview", || {
        preview_keystrokes(&cli, WORKER_PANE, failing) == 1
    });
    let output = monitor.stop();
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
}

#[test]
fn three_consecutive_pane_listing_failures_stop_the_loop() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "SFFF");

    let mut monitor = LoopChild::spawn(&cli, &["monitor", "1", "--tick", "1", "--interval", "0"]);
    let output = monitor.wait_for_exit("the exit after three failing ticks");
    assert_eq!(code(&output), 1, "stdout: {}", stdout(&output));
    assert!(stdout(&output).contains("monitor loop started (fleet 1, tick 1s, pid "));
    assert_eq!(
        cli.shim_count("list-panes"),
        4,
        "the startup probe, then three failing ticks"
    );
    assert_eq!(
        cli.monitor_pid(FLEET),
        None,
        "the exit clears the runtime row"
    );
}

#[test]
fn one_successful_pane_listing_resets_the_failure_count() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "SFFSFFF");

    let mut monitor = LoopChild::spawn(&cli, &["monitor", "1", "--tick", "1", "--interval", "0"]);
    let output = monitor.wait_for_exit("the exit after the second failing run");
    assert_eq!(code(&output), 1, "stdout: {}", stdout(&output));
    assert_eq!(
        cli.shim_count("list-panes"),
        7,
        "two failures, a success, then three failures"
    );
    assert_eq!(cli.monitor_pid(FLEET), None);
}

#[test]
fn a_loop_whose_startup_probe_fails_claims_nothing() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "F");

    let mut monitor = LoopChild::spawn(&cli, &["monitor", "1", "--tick", "1"]);
    let output = monitor.wait_for_exit("the failed startup probe");
    assert_eq!(code(&output), 1, "stdout: {}", stdout(&output));
    assert!(!stdout(&output).contains("monitor loop started"));
    assert_eq!(cli.shim_count("list-panes"), 1);
    assert_eq!(live_loops(&cli), 0);
}

#[test]
fn a_restart_without_flags_keeps_the_stored_tick_and_wake_interval() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    {
        let mut first = LoopChild::start(&cli, FLEET, &["--tick", "2", "--interval", "120"]);
        assert_eq!(code(&first.stop()), 0);
    }
    assert_eq!(cli.monitor_pid(FLEET), None);

    cli.set_env("CAFLEET_MONITOR_WAKE_INTERVAL", "45");
    let mut second = LoopChild::start(&cli, FLEET, &[]);
    let stored: (i64, i64) = cli
        .sqlite()
        .query_row(
            "SELECT tick_seconds, wake_interval_seconds FROM monitor_runtime WHERE fleet_id=?1",
            [FLEET],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (2, 120), "the stored row outranks the environment");
    let pid = second.pid();
    let output = second.stop();
    assert!(
        stdout(&output).contains(&format!(
            "monitor loop started (fleet 1, tick 2s, pid {pid})"
        )),
        "stdout: {}",
        stdout(&output)
    );
}

#[test]
fn the_ready_watchdog_reports_a_silent_member_once() {
    let cli = Cli::new();
    fleet_with_worker(&cli);
    let spoken = cli.seed_member(FLEET, "spoken");
    cli.seed_message(spoken, DIRECTOR, "ready");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET created_at=?1 WHERE member_id IN (?2, ?3)",
            rusqlite::params![
                format_utc(Utc::now() - Duration::seconds(200)),
                WORKER,
                spoken
            ],
        )
        .unwrap();
    let notices = || -> Vec<(i64, i64, Option<i64>, String)> {
        let conn = cli.sqlite();
        let mut statement = conn
            .prepare(
                "SELECT owner_member_id, from_member_id, to_member_id, text FROM messages \
                 WHERE text LIKE '[cafleet] member %' ORDER BY message_id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);
    monitor.wait_until("the silent-member notice", || !notices().is_empty());
    monitor.wait_ticks(3);
    let output = monitor.stop();
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let notices = notices();
    assert_eq!(notices.len(), 1, "one notice, for the silent member only");
    let (owner, from, to, text) = &notices[0];
    assert_eq!((*owner, *from, *to), (DIRECTOR, DIRECTOR, Some(DIRECTOR)));
    assert!(
        text.starts_with(&format!(
            "[cafleet] member {WORKER} (worker) has sent no message "
        )) && text.ends_with(&format!(
            " s after spawn. Its broker commands may be denied: inspect it with \
             cafleet member capture {WORKER} and run cafleet doctor."
        )),
        "{text}"
    );
    let stamped: bool = cli
        .sqlite()
        .query_row(
            "SELECT silence_notice_at IS NOT NULL FROM member_placements WHERE member_id=?1",
            [WORKER],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stamped);
}

#[test]
fn lost_execs_close_with_a_notice() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    let helper = cli.seed_member(FLEET, "helper");
    let runner = cli.seed_member(FLEET, "runner");
    cli.set_env(
        "CAFLEET_TEST_TMUX_PANES",
        &format!(
            "{DIRECTOR_PANE} {WORKER_PANE} %{} %{}",
            helper + 4,
            runner + 4
        ),
    );
    let mut ended = std::process::Command::new("true").spawn().unwrap();
    ended.wait().unwrap();
    let ended_pid = i64::from(ended.id());
    let own_pid = i64::from(std::process::id());

    let ago = |seconds: i64| format_utc(Utc::now() - Duration::seconds(seconds));
    let insert = |member_id: i64, dispatched_at: String, started: Option<(String, i64)>| {
        let (started_at, pid) = started.unzip();
        cli.sqlite()
            .execute(
                "INSERT INTO member_execs \
                 (member_id, command, created_at, dispatched_at, started_at, pid) \
                 VALUES (?1, 'mise //cafleet:test', ?2, ?2, ?3, ?4)",
                rusqlite::params![member_id, dispatched_at, started_at, pid],
            )
            .unwrap();
    };
    insert(WORKER, ago(31), None);
    insert(helper, ago(100), Some((ago(99), ended_pid)));
    insert(runner, ago(100), Some((ago(99), own_pid)));

    let notices = || -> Vec<(i64, i64, Option<i64>, String)> {
        let conn = cli.sqlite();
        let mut statement = conn
            .prepare(
                "SELECT owner_member_id, from_member_id, to_member_id, text FROM messages \
                 WHERE text LIKE '[cafleet] exec %' ORDER BY text",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);
    monitor.wait_until("both lost-exec notices", || notices().len() == 2);
    monitor.wait_ticks(3);
    let output = monitor.stop();
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));

    let to_director = (DIRECTOR, DIRECTOR, Some(DIRECTOR));
    assert_eq!(
        notices()
            .into_iter()
            .map(|(owner, from, to, text)| ((owner, from, to), text))
            .collect::<Vec<_>>(),
        [
            (
                to_director,
                format!(
                    "[cafleet] exec 1 on member {WORKER} (worker) did not start within 30 s of \
                     dispatch. Inspect the pane with cafleet member capture {WORKER} and run it \
                     again if still needed."
                )
            ),
            (
                to_director,
                format!(
                    "[cafleet] exec 2 on member {helper} (helper) ended without reporting an \
                     exit status. Inspect the pane with cafleet member capture {helper}."
                )
            ),
        ],
        "each lost exec is reported once; the exec with a live pid is not"
    );

    let conn = cli.sqlite();
    let mut statement = conn
        .prepare(
            "SELECT finished_at IS NOT NULL AND finished_at = resumed_at, exit_code \
             FROM member_execs ORDER BY exec_id",
        )
        .unwrap();
    let closed: Vec<(bool, Option<i64>)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(closed, [(true, None), (true, None), (false, None)]);
}

#[test]
fn the_wake_respects_a_held_pane_claim_and_stays_due() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_env(
        "CAFLEET_TEST_TMUX_PANES",
        &format!("{DIRECTOR_PANE} {MONITOR_PANE} {WORKER_PANE}"),
    );
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET keystroke_at=?1 WHERE member_id=?2",
            rusqlite::params![format_utc(Utc::now() + Duration::hours(1)), MONITOR],
        )
        .unwrap();
    let wakes = || cli.shim_count(&format!("send-keys -t {MONITOR_PANE} -l [cafleet] tick:"));

    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "1"]);
    monitor.wait_ticks(4);
    assert_eq!(wakes(), 0, "another process holds the monitor pane");
    let last_wake_at: Option<String> = cli
        .sqlite()
        .query_row(
            "SELECT last_wake_at FROM monitor_runtime WHERE fleet_id=?1",
            [FLEET],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(last_wake_at, None, "a refused claim records no wake");

    cli.sqlite()
        .execute(
            "UPDATE member_placements SET keystroke_at=NULL WHERE member_id=?1",
            [MONITOR],
        )
        .unwrap();
    monitor.wait_until("the wake that stayed due", || wakes() >= 1);
    assert_eq!(code(&monitor.stop()), 0);
}

#[test]
fn a_send_that_leaves_work_owed_starts_the_loop() {
    let cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_pane_capture(WORKER_PANE, WORKING_CAPTURE);

    let (message_id, notification_sent) = sent_message(&send(&cli, DIRECTOR, WORKER, "held task"));
    assert!(!notification_sent);
    let pid = cli
        .monitor_pid(FLEET)
        .expect("the send returns only after the loop is live");
    let _started = DetachedLoop(pid);
    let log = std::fs::read_to_string(cli.monitor_log_path(FLEET)).unwrap();
    assert!(
        log.contains(&format!(
            "monitor loop started (fleet 1, tick 5s, pid {pid})"
        )),
        "{log}"
    );
    assert!(
        !notified(&cli, message_id),
        "the preview stays owed to the loop"
    );
}

#[test]
fn a_command_that_leaves_work_owed_leaves_a_live_loop_alone() {
    let cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_pane_capture(WORKER_PANE, WORKING_CAPTURE);
    let mut monitor = LoopChild::start(&cli, FLEET, &["--tick", "1", "--interval", "0"]);
    let pid = monitor.pid();

    let (_, notification_sent) = sent_message(&send(&cli, DIRECTOR, WORKER, "held task"));
    assert!(!notification_sent);
    assert_eq!(cli.monitor_pid(FLEET), Some(pid));
    assert!(
        !cli.monitor_log_path(FLEET).exists(),
        "no second loop was spawned"
    );
    assert_eq!(code(&monitor.stop()), 0);
}

#[test]
fn commands_that_leave_nothing_owed_never_start_a_loop() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    let paneless = cli.seed_member(FLEET, "paneless");
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [paneless],
        )
        .unwrap();
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "F");

    for recipient in [DIRECTOR, paneless] {
        let output = cli.run_outside_tmux(&[
            "message",
            "send",
            "--from-member-id",
            &DIRECTOR.to_string(),
            "--to-member-id",
            &recipient.to_string(),
            "no keystroke is owed",
            "--json",
        ]);
        let (_, notification_sent) = sent_message(&output);
        assert!(!notification_sent);
    }

    let (_, notification_sent) = sent_message(&send(&cli, DIRECTOR, WORKER, "delivered"));
    assert!(notification_sent);

    assert_eq!(live_loops(&cli), 0);
    assert!(!cli.monitor_log_path(FLEET).exists());
    assert_eq!(cli.shim_count("list-panes"), 0);
}

#[test]
fn a_fully_delivered_broadcast_never_starts_a_loop() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "F");

    let output = cli.run(&[
        "message",
        "broadcast",
        "--from-member-id",
        &DIRECTOR.to_string(),
        "delivered to every pane",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("recipients=2 delivered=2"),
        "{}",
        stdout(&output)
    );
    assert_eq!(live_loops(&cli), 0);
    assert!(!cli.monitor_log_path(FLEET).exists());
    assert_eq!(cli.shim_count("list-panes"), 0);
}

#[test]
fn a_send_that_leaves_work_owed_where_no_loop_can_start_exits_one() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_pane_capture(WORKER_PANE, WORKING_CAPTURE);
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "F");

    let output = send(&cli, DIRECTOR, WORKER, "held task");
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    let (message_id, status): (i64, String) = cli
        .sqlite()
        .query_row(
            "SELECT message_id, status_state FROM messages WHERE type='unicast'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "input_required");
    assert!(
        stderr(&output).contains(&format!(
            "Error: Message {message_id} was persisted, but monitor loop for fleet 1 did not \
             start; see {}. Do not resend this message; run 'cafleet doctor'.",
            cli.monitor_log_path(FLEET).display()
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(live_loops(&cli), 0);
}

#[test]
fn a_broadcast_that_leaves_work_owed_where_no_loop_can_start_exits_one() {
    let mut cli = Cli::new();
    fleet_with_worker(&cli);
    cli.set_pane_capture(WORKER_PANE, WORKING_CAPTURE);
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "F");

    let output = cli.run(&[
        "message",
        "broadcast",
        "--from-member-id",
        &DIRECTOR.to_string(),
        "held fanout",
    ]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    let summary_id: i64 = cli
        .sqlite()
        .query_row(
            "SELECT message_id FROM messages WHERE type='broadcast_summary'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        stderr(&output).contains(&format!(
            "Error: Broadcast {summary_id} was persisted, but monitor loop for fleet 1 did not \
             start; see {}. Do not resend it; run 'cafleet doctor'.",
            cli.monitor_log_path(FLEET).display()
        )),
        "{}",
        stderr(&output)
    );
    let pending: i64 = cli
        .sqlite()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE type='unicast' AND status_state='input_required'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 2, "every delivery row stays persisted");
}

#[test]
fn fleet_create_compensates_when_the_loop_does_not_start() {
    let mut cli = Cli::new();
    cli.ready();
    cli.set_env("CAFLEET_TEST_TMUX_LIST_PANES_SCRIPT", "F");

    let output = cli.run(&[
        "fleet",
        "create",
        "--name",
        "testfleet",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &cli.monitor_prompt_path(),
    ]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "", "no ids are printed");
    assert!(
        stderr(&output).contains(&format!(
            "Error: monitor loop for fleet 1 did not start; see {}",
            cli.monitor_log_path(FLEET).display()
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        cli.shim_count("kill-pane -t %7"),
        1,
        "the spawned monitor pane is killed: {:?}",
        cli.shim_calls()
    );
    let conn = cli.sqlite();
    let deleted: bool = conn
        .query_row(
            "SELECT deleted_at IS NOT NULL FROM fleets WHERE fleet_id=?1",
            [FLEET],
            |row| row.get(0),
        )
        .unwrap();
    assert!(deleted, "the committed fleet is soft-deleted");
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM members WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);
}
