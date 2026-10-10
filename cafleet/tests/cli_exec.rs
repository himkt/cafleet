//! `cafleet member exec` and `cafleet member exec-run`, driven through the
//! binary against the tmux shim.

mod common;

use std::process::Output;

use cafleet::time::format_utc;
use chrono::{Duration, Utc};
use common::{Cli, LoopChild, code, stderr, stdout, write_file};

const FLEET: i64 = 1;
const DIRECTOR: i64 = 1;
const WORKER: i64 = 3;
const WORKER_PANE: &str = "%7";

const WORKING_CAPTURE: &str = "✻ Cogitating… (12s · esc to interrupt)\n\n> \n";

/// Fleet 1 with the Director on `%0` and one worker on `%7`. Dropping the
/// fixture terminates the loop a `member exec` started detached.
struct ExecFleet {
    cli: Cli,
}

impl ExecFleet {
    fn new() -> Self {
        let mut cli = Cli::new();
        // `exec-run` resolves `sh` through PATH; the shim directory stays first.
        cli.set_env("PATH", &format!("{}:/bin:/usr/bin", cli.shim_dir.display()));
        assert_eq!(cli.seeded_fleet(), (FLEET, DIRECTOR));
        assert_eq!(cli.seed_member(FLEET, "worker"), WORKER);
        ExecFleet { cli }
    }

    fn exec(&self, args: &[&str]) -> Output {
        let worker = WORKER.to_string();
        let mut argv = vec!["member", "exec", worker.as_str()];
        argv.extend_from_slice(args);
        self.cli.run(&argv)
    }

    fn exec_rows(&self) -> i64 {
        self.cli
            .sqlite()
            .query_row("SELECT COUNT(*) FROM member_execs", [], |row| row.get(0))
            .unwrap()
    }

    fn stored_command(&self, exec_id: i64) -> String {
        self.cli
            .sqlite()
            .query_row(
                "SELECT command FROM member_execs WHERE exec_id=?1",
                [exec_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn dispatched(&self, exec_id: i64) -> bool {
        self.cli
            .sqlite()
            .query_row(
                "SELECT dispatched_at IS NOT NULL FROM member_execs WHERE exec_id=?1",
                [exec_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn dispatch_keystrokes(&self, exec_id: i64) -> usize {
        self.cli
            .shim_calls()
            .iter()
            .filter(|line| {
                **line
                    == format!("send-keys -t {WORKER_PANE} -l ! cafleet member exec-run {exec_id}")
            })
            .count()
    }

    /// Insert a dispatched exec for the worker, as `member exec` leaves it once
    /// its keystroke landed.
    fn seed_dispatched_exec(&self, command: &str) -> i64 {
        let conn = self.cli.sqlite();
        let now = format_utc(Utc::now());
        conn.execute(
            "INSERT INTO member_execs (member_id, command, created_at, dispatched_at) \
             VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![WORKER, command, now],
        )
        .unwrap();
        conn.last_insert_rowid()
    }
}

impl Drop for ExecFleet {
    fn drop(&mut self) {
        if let Some(pid) = self.cli.monitor_pid(FLEET) {
            let pid = nix::unistd::Pid::from_raw(i32::try_from(pid).expect("PID fits i32"));
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        }
    }
}

#[test]
fn member_exec_takes_exactly_one_non_empty_command_source() {
    let fleet = ExecFleet::new();
    let script = write_file(&fleet.cli.home.path().join("script.sh"), b"echo from file");
    let empty = write_file(&fleet.cli.home.path().join("empty.sh"), b"");

    let no_source: &[&str] = &[];
    let both_sources: &[&str] = &["echo inline", "--file", script.as_str()];
    for args in [no_source, both_sources] {
        let output = fleet.exec(args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
    }
    let empty_inline: &[&str] = &[""];
    let empty_file: &[&str] = &["--file", empty.as_str()];
    for args in [empty_inline, empty_file] {
        let output = fleet.exec(args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
        assert!(
            stderr(&output).contains("command may not be empty."),
            "{args:?}: {}",
            stderr(&output)
        );
    }

    assert_eq!(fleet.exec_rows(), 0);
    assert!(
        fleet
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.starts_with("send-keys")),
        "{:?}",
        fleet.cli.shim_calls()
    );
}

#[test]
fn member_exec_stores_a_multi_line_body_verbatim_from_a_file_or_stdin() {
    let fleet = ExecFleet::new();
    let body = "cd \"$HOME\"\nprintf '%s\\n' \"it's a | b\" && exit 3";
    let script = write_file(&fleet.cli.home.path().join("script.sh"), body.as_bytes());

    let output = fleet.exec(&["--file", &script]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(fleet.stored_command(1), body);
    assert_eq!(
        fleet.dispatch_keystrokes(1),
        1,
        "the keystroke carries the exec id, never the command: {:?}",
        fleet.cli.shim_calls()
    );

    let worker = WORKER.to_string();
    let output = fleet
        .cli
        .run_with_stdin(&["member", "exec", &worker, "--file", "-"], body);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(fleet.stored_command(2), body);
}

#[test]
fn member_exec_rejects_the_director_and_a_member_without_a_pane() {
    let fleet = ExecFleet::new();

    let output = fleet
        .cli
        .run(&["member", "exec", &DIRECTOR.to_string(), "echo hi"]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("Error: cannot exec in the Director's own pane"),
        "{}",
        stderr(&output)
    );

    fleet
        .cli
        .sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [WORKER],
        )
        .unwrap();
    let output = fleet.exec(&["echo hi"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "Error: member {WORKER} has no pane yet (pending placement)"
        )),
        "{}",
        stderr(&output)
    );

    assert_eq!(fleet.exec_rows(), 0);
}

#[test]
fn member_exec_dispatches_into_a_pane_at_rest() {
    let fleet = ExecFleet::new();

    let output = fleet.exec(&["mise //cafleet:test"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("Queued exec 1 for member worker ({WORKER_PANE}): dispatched.\n")
    );
    assert!(fleet.dispatched(1));
    assert_eq!(fleet.dispatch_keystrokes(1), 1);
}

#[test]
fn member_exec_holds_the_dispatch_while_the_pane_is_not_at_rest() {
    let fleet = ExecFleet::new();
    fleet.cli.set_pane_capture(WORKER_PANE, WORKING_CAPTURE);

    let output = fleet.exec(&["mise //cafleet:test"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("Queued exec 1 for member worker ({WORKER_PANE}): held.\n")
    );
    assert!(!fleet.dispatched(1));
    assert_eq!(fleet.dispatch_keystrokes(1), 0);

    let output = fleet.exec(&["mise //cafleet:lint", "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "{{\"exec_id\":2,\"member_id\":{WORKER},\"pane_id\":\"{WORKER_PANE}\",\
             \"dispatched\":false}}\n"
        )
    );
}

#[test]
fn exec_run_runs_the_command_once_and_records_its_exit_code() {
    let fleet = ExecFleet::new();
    let ran = fleet.cli.home.path().join("ran.log");
    let exec_id = fleet.seed_dispatched_exec(&format!(
        "printf 'ran\\n' >> '{}'; printf 'command output\\n'; exit 7",
        ran.display()
    ));
    let exec_arg = exec_id.to_string();

    let output = fleet.cli.run(&["member", "exec-run", &exec_arg]);
    assert_eq!(code(&output), 7, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("command output\n[cafleet] exec {exec_id} exited 7\n")
    );
    let (started, has_pid, finished, exit_code): (bool, bool, bool, Option<i64>) = fleet
        .cli
        .sqlite()
        .query_row(
            "SELECT started_at IS NOT NULL, pid IS NOT NULL, finished_at IS NOT NULL, exit_code \
             FROM member_execs WHERE exec_id=?1",
            [exec_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (started, has_pid, finished, exit_code),
        (true, true, true, Some(7))
    );

    let again = fleet.cli.run(&["member", "exec-run", &exec_arg]);
    assert_eq!(code(&again), 1);
    assert!(
        stderr(&again).contains(&format!("Error: exec {exec_id} is not runnable")),
        "{}",
        stderr(&again)
    );
    assert_eq!(std::fs::read_to_string(&ran).unwrap(), "ran\n");
}

/// Close exec 1 as the pane and the loop would, `ran_seconds` after its start.
fn close_exec(fleet: &ExecFleet, ran_seconds: i64, exit_code: Option<i64>) {
    let finished_at = Utc::now();
    let started_at = format_utc(finished_at - Duration::seconds(ran_seconds));
    let finished_at = format_utc(finished_at);
    fleet
        .cli
        .sqlite()
        .execute(
            "UPDATE member_execs SET created_at=?1, dispatched_at=?1, started_at=?1, pid=?2, \
             finished_at=?3, exit_code=?4, resumed_at=?3 WHERE exec_id=1",
            rusqlite::params![
                started_at,
                i64::from(std::process::id()),
                finished_at,
                exit_code
            ],
        )
        .unwrap();
}

fn exec_and_wait<'a>(fleet: &'a ExecFleet) -> LoopChild<'a> {
    let mut waiting = LoopChild::spawn(
        &fleet.cli,
        &["member", "exec", "3", "mise //cafleet:test", "--wait"],
    );
    waiting.wait_until("the exec to be dispatched", || {
        fleet.exec_rows() == 1 && fleet.dispatched(1)
    });
    waiting
}

#[test]
fn member_exec_wait_prints_the_exit_status_once_the_exec_closes() {
    let fleet = ExecFleet::new();
    let mut waiting = exec_and_wait(&fleet);

    close_exec(&fleet, 4, Some(3));
    let output = waiting.wait_for_exit("the wait to observe the closed exec");
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Queued exec 1 for member worker ({WORKER_PANE}): dispatched.\n\
             exec 1 exited 3 after 4 s.\n"
        )
    );
}

#[test]
fn member_exec_wait_exits_one_with_the_lost_exec_text() {
    let fleet = ExecFleet::new();
    let mut waiting = exec_and_wait(&fleet);

    close_exec(&fleet, 4, None);
    let output = waiting.wait_for_exit("the wait to observe the lost exec");
    assert_eq!(code(&output), 1, "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Queued exec 1 for member worker ({WORKER_PANE}): dispatched.\n\
             [cafleet] exec 1 on member {WORKER} (worker) ended without reporting an exit \
             status. Inspect the pane with cafleet member capture {WORKER}.\n"
        )
    );
}
