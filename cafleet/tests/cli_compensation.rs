//! Creation compensation contract: isolated CLI, SQLite faults, and recorded pane commands.
mod common;

use common::{Cli, code, stderr, stdout};
use std::os::unix::fs::PermissionsExt;
use std::process::Output;

const TMUX: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
case "$1" in
  display-message) printf 'main|@1|%%0\n' ;;
  split-window) printf '%%9\n' ;;
  kill-pane)
    if [ "$CAFLEET_TEST_CLOSE_FAIL" = 1 ]; then printf 'forced close failure\n' >&2; exit 1; fi ;;
  list-panes) printf '%%0\n%%9\n' ;;
esac
"#;

const HERDR: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
case "$1 $2" in
  'pane current') printf '%s\n' '{"result":{"pane":{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1"}}}' ;;
  'pane list') printf '%s\n' '{"result":{"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1"}]}}' ;;
  'pane split')
    if [ "$CAFLEET_TEST_UNKNOWN_PANE" = 1 ]; then printf '%s\n' '{"result":{"pane":{}}}'; else printf '%s\n' '{"result":{"pane":{"pane_id":"w1:p9"}}}'; fi ;;
  'pane run') printf 'primary run failure\n' >&2; exit 1 ;;
  'pane get') printf 'optional layout read failed\n' >&2; exit 1 ;;
  'pane close')
    if [ "$CAFLEET_TEST_CLOSE_FAIL" = 1 ]; then printf 'forced close failure\n' >&2; exit 1; fi ;;
esac
"#;

fn write_shim(cli: &Cli, binary: &str, body: &str) {
    let path = cli.shim_dir.join(binary);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fixture(with_fleet: bool) -> Cli {
    let cli = Cli::new();
    if with_fleet {
        cli.with_fleet();
    } else {
        cli.ready();
    }
    write_shim(&cli, "tmux", TMUX);
    std::fs::write(&cli.shim_log, "").unwrap();
    cli.sqlite()
        .execute_batch(
            "CREATE TABLE compensation_events(event TEXT);
         CREATE TRIGGER record_deregister AFTER UPDATE OF status ON members
         WHEN OLD.status='active' AND NEW.status='deregistered'
         BEGIN INSERT INTO compensation_events VALUES ('deregister'); END;",
        )
        .unwrap();
    cli
}

fn herdr_fixture(with_fleet: bool) -> Cli {
    let mut cli = fixture(with_fleet);
    write_shim(&cli, "herdr", HERDR);
    cli.set_env("CAFLEET_MULTIPLEXER", "herdr");
    cli.set_env("HERDR_ENV", "1");
    cli
}

fn member_create(cli: &Cli, prompt: &str, json: bool) -> Output {
    let mut args = vec![
        "member",
        "create",
        "--fleet-id",
        "1",
        "--name",
        "worker",
        "--description",
        "fixture",
    ];
    if json {
        args.push("--json");
    }
    args.push(prompt);
    cli.run(&args)
}

fn fleet_create(cli: &Cli, json: bool) -> Output {
    let prompt = cli.monitor_prompt_path();
    let mut args = vec![
        "fleet",
        "create",
        "--name",
        "fixture",
        "--coding-agent",
        "claude",
        "--monitor-file",
        &prompt,
    ];
    if json {
        args.push("--json");
    }
    cli.run(&args)
}

fn count(cli: &Cli, table: &str) -> i64 {
    cli.sqlite()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn calls(cli: &Cli) -> Vec<String> {
    cli.shim_calls()
}

fn assert_no_exit_commands(cli: &Cli) {
    assert!(
        calls(cli)
            .iter()
            .all(|line| !line.contains("send-keys") && !line.contains("/exit")),
        "{:?}",
        calls(cli)
    );
}

fn assert_killed_once(cli: &Cli, pane: &str) {
    let expected = if pane.starts_with('%') {
        format!("kill-pane -t {pane}")
    } else {
        format!("pane close {pane}")
    };
    assert_eq!(
        calls(cli).iter().filter(|line| **line == expected).count(),
        1,
        "{:?}",
        calls(cli)
    );
    assert_no_exit_commands(cli);
}

fn assert_member_compensated(cli: &Cli) {
    let conn = cli.sqlite();
    let status: String = conn
        .query_row("SELECT status FROM members WHERE member_id=3", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "deregistered");
    assert_eq!(count(cli, "members"), 3);
    assert_eq!(count(cli, "member_placements"), 2);
    assert_eq!(count(cli, "compensation_events"), 1);
}

fn assert_fleet_compensated(cli: &Cli) {
    for table in ["fleets", "members", "member_placements"] {
        assert_eq!(count(cli, table), 0, "{table}");
    }
}

fn placement_error(cli: &Cli) {
    cli.sqlite().execute_batch("CREATE TRIGGER fail_patch BEFORE UPDATE OF mux_pane_id ON member_placements BEGIN SELECT RAISE(ABORT, 'primary placement failure'); END;").unwrap();
}

fn fleet_insert_error(cli: &Cli) {
    cli.sqlite().execute_batch("CREATE TRIGGER fail_monitor_placement BEFORE INSERT ON member_placements WHEN NEW.member_id=2 BEGIN SELECT RAISE(ABORT, 'primary placement insert failure'); END;").unwrap();
}

#[test]
fn member_registration_failure_never_creates_a_pane_or_partial_placement() {
    let cli = fixture(true);
    cli.sqlite().execute_batch("CREATE TRIGGER fail_registration BEFORE INSERT ON members BEGIN SELECT RAISE(ABORT, 'primary registration failure'); END;").unwrap();
    let output = member_create(&cli, "prompt", false);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("primary registration failure"));
    assert_eq!(count(&cli, "members"), 2);
    assert_eq!(count(&cli, "member_placements"), 2);
    assert!(
        calls(&cli)
            .iter()
            .all(|line| !line.starts_with("split-window") && !line.starts_with("kill-pane"))
    );
}

#[test]
fn member_placeholder_failure_keeps_usage_exit_and_deregisters_without_a_pane() {
    let cli = fixture(true);
    let output = member_create(&cli, "{unknown}", false);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("Unknown placeholder 'unknown'"));
    assert_member_compensated(&cli);
    assert!(
        calls(&cli)
            .iter()
            .all(|line| !line.starts_with("split-window") && !line.starts_with("kill-pane"))
    );
}

#[test]
fn member_placeholder_cleanup_failure_follows_usage_error_without_success_claim() {
    let cli = fixture(true);
    cli.sqlite().execute_batch("CREATE TRIGGER fail_deregister BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT, 'secondary deregister failure'); END;").unwrap();
    let output = member_create(&cli, "{unknown}", false);
    assert_eq!(code(&output), 2);
    let detail = stderr(&output);
    assert!(detail.contains("cleanup failed for member 3:"), "{detail}");
    assert!(
        detail.find("Unknown placeholder 'unknown'").unwrap()
            < detail.find("secondary deregister failure").unwrap()
    );
    assert!(!detail.contains("Rolled back"));
    assert_eq!(count(&cli, "member_placements"), 3);
}

#[test]
fn member_placement_error_kills_pane_and_compensates_registration() {
    let cli = fixture(true);
    placement_error(&cli);
    let output = member_create(&cli, "prompt", false);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("primary placement failure"));
    assert_killed_once(&cli, "%9");
    assert_member_compensated(&cli);
}

#[test]
fn member_vanished_placement_kills_the_owned_pane_and_deregisters_once() {
    let cli = fixture(true);
    cli.sqlite().execute_batch("CREATE TRIGGER vanish_patch BEFORE UPDATE OF mux_pane_id ON member_placements BEGIN DELETE FROM member_placements WHERE member_id=NEW.member_id; SELECT RAISE(IGNORE); END;").unwrap();
    let output = member_create(&cli, "prompt", false);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("placement row vanished before pane-id patch"));
    assert_killed_once(&cli, "%9");
    assert_member_compensated(&cli);
}

#[test]
fn member_pane_cleanup_failure_still_deregisters_and_retains_primary_error() {
    let mut cli = fixture(true);
    cli.set_env("CAFLEET_TEST_CLOSE_FAIL", "1");
    placement_error(&cli);
    let output = member_create(&cli, "prompt", false);
    assert_eq!(code(&output), 1);
    let detail = stderr(&output);
    assert!(detail.contains("cleanup failed for pane %9:"), "{detail}");
    assert!(
        detail.find("primary placement failure").unwrap()
            < detail.find("forced close failure").unwrap()
    );
    assert!(!detail.contains("Rolled back registration"));
    assert_killed_once(&cli, "%9");
    assert_member_compensated(&cli);
}

#[test]
fn member_deregister_failure_does_not_repeat_pane_cleanup_or_claim_full_rollback() {
    let cli = fixture(true);
    placement_error(&cli);
    cli.sqlite().execute_batch("CREATE TRIGGER fail_deregister BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT, 'secondary deregister failure'); END;").unwrap();
    let output = member_create(&cli, "prompt", false);
    assert_eq!(code(&output), 1);
    let detail = stderr(&output);
    assert!(detail.contains("cleanup failed for member 3:"), "{detail}");
    assert!(
        detail.find("primary placement failure").unwrap()
            < detail.find("secondary deregister failure").unwrap()
    );
    assert!(!detail.contains("Rolled back registration"));
    assert_killed_once(&cli, "%9");
    assert_eq!(count(&cli, "compensation_events"), 0);
}

#[test]
fn fleet_placeholder_failure_rolls_back_without_creating_or_killing_panes() {
    let cli = fixture(false);
    std::fs::write(cli.monitor_prompt_path(), "{unknown}").unwrap();
    let output = fleet_create(&cli, false);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("Unknown placeholder 'unknown'"));
    assert_fleet_compensated(&cli);
    assert!(
        calls(&cli)
            .iter()
            .all(|line| !line.starts_with("split-window") && !line.starts_with("kill-pane"))
    );
}

#[test]
fn fleet_post_callback_insert_failure_cleans_the_pane_and_all_bootstrap_rows() {
    let cli = fixture(false);
    fleet_insert_error(&cli);
    let output = fleet_create(&cli, false);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("primary placement insert failure"));
    assert_fleet_compensated(&cli);
    assert_killed_once(&cli, "%9");
}

#[test]
fn fleet_deferred_commit_failure_cleans_the_pane_and_rolls_back_all_rows() {
    let cli = fixture(false);
    cli.sqlite().execute_batch(
        "CREATE TABLE compensation_parent(id INTEGER PRIMARY KEY);
         CREATE TABLE compensation_child(id INTEGER REFERENCES compensation_parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER defer_commit_failure AFTER INSERT ON member_placements WHEN NEW.member_id=2
         BEGIN INSERT INTO compensation_child VALUES (999); END;"
    ).unwrap();
    let output = fleet_create(&cli, false);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("FOREIGN KEY constraint failed"),
        "{}",
        stderr(&output)
    );
    assert_fleet_compensated(&cli);
    assert_eq!(count(&cli, "compensation_child"), 0);
    assert_killed_once(&cli, "%9");
}

#[test]
fn fleet_pane_cleanup_failure_keeps_database_error_and_completed_database_rollback() {
    let mut cli = fixture(false);
    cli.set_env("CAFLEET_TEST_CLOSE_FAIL", "1");
    fleet_insert_error(&cli);
    let output = fleet_create(&cli, false);
    assert_eq!(code(&output), 1);
    let detail = stderr(&output);
    assert!(detail.contains("cleanup failed for pane %9:"), "{detail}");
    assert!(
        detail.find("primary placement insert failure").unwrap()
            < detail.find("forced close failure").unwrap()
    );
    assert_fleet_compensated(&cli);
    assert_killed_once(&cli, "%9");
}

#[test]
fn herdr_member_run_failure_closes_once_and_deregisters_even_when_close_fails() {
    for fail_close in [false, true] {
        let mut cli = herdr_fixture(true);
        if fail_close {
            cli.set_env("CAFLEET_TEST_CLOSE_FAIL", "1");
        }
        let output = member_create(&cli, "prompt", false);
        assert_eq!(code(&output), 1);
        let detail = stderr(&output);
        assert!(detail.contains("primary run failure"), "{detail}");
        if fail_close {
            assert!(
                detail.contains("cleanup failed for pane w1:p9:"),
                "{detail}"
            );
        }
        assert_killed_once(&cli, "w1:p9");
        assert_member_compensated(&cli);
    }
}

#[test]
fn herdr_fleet_run_failure_closes_once_and_rolls_back_even_when_close_fails() {
    for fail_close in [false, true] {
        let mut cli = herdr_fixture(false);
        if fail_close {
            cli.set_env("CAFLEET_TEST_CLOSE_FAIL", "1");
        }
        let output = fleet_create(&cli, false);
        assert_eq!(code(&output), 1);
        let detail = stderr(&output);
        assert!(detail.contains("primary run failure"), "{detail}");
        if fail_close {
            assert!(
                detail.contains("cleanup failed for pane w1:p9:"),
                "{detail}"
            );
        }
        assert_killed_once(&cli, "w1:p9");
        assert_fleet_compensated(&cli);
    }
}

#[test]
fn herdr_unknown_split_never_guesses_a_pane_for_member_or_fleet_cleanup() {
    for member in [false, true] {
        let mut cli = herdr_fixture(member);
        cli.set_env("CAFLEET_TEST_UNKNOWN_PANE", "1");
        let output = if member {
            member_create(&cli, "prompt", false)
        } else {
            fleet_create(&cli, false)
        };
        assert_eq!(code(&output), 1);
        let detail = stderr(&output).to_lowercase();
        assert!(
            detail.contains("unknown") && detail.contains("unconfirmed"),
            "{detail}"
        );
        assert!(
            calls(&cli)
                .iter()
                .all(|line| !line.starts_with("pane close") && !line.starts_with("pane run"))
        );
        if member {
            assert_member_compensated(&cli);
        } else {
            assert_fleet_compensated(&cli);
        }
    }
}

#[test]
fn successful_member_text_and_json_leave_the_new_pane_and_member_active() {
    for json in [false, true] {
        let cli = fixture(true);
        let output = member_create(&cli, "prompt", json);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert!(stderr(&output).is_empty());
        if json {
            let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
            let registered = value["registered_at"].as_str().unwrap();
            let placed = value["placement"]["created_at"].as_str().unwrap();
            assert!(cafleet::time::parse_lenient(registered).is_ok());
            assert!(cafleet::time::parse_lenient(placed).is_ok());
            assert_eq!(
                stdout(&output),
                format!(
                    r#"{{"member_id":3,"name":"worker","registered_at":"{registered}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%9","coding_agent":"claude","created_at":"{placed}"}}}}"#
                ) + "\n"
            );
        } else {
            assert_eq!(stdout(&output), "3 worker backend=claude pane=%9\n");
        }
        assert_eq!(count(&cli, "compensation_events"), 0);
        assert!(
            calls(&cli)
                .iter()
                .all(|line| !line.starts_with("kill-pane"))
        );
        assert_no_exit_commands(&cli);
    }
}

#[test]
fn successful_fleet_text_and_json_do_not_run_creation_cleanup() {
    for json in [false, true] {
        let cli = fixture(false);
        let output = fleet_create(&cli, json);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        if json {
            let value: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
            let ts = value["created_at"].as_str().unwrap();
            assert!(cafleet::time::parse_lenient(ts).is_ok());
            assert_eq!(
                stdout(&output),
                format!(
                    r#"{{"fleet_id":1,"name":"fixture","created_at":"{ts}","director":{{"member_id":1,"name":"Director","description":"Root Director for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%0","coding_agent":"claude","created_at":"{ts}"}}}},"monitor":{{"member_id":2,"name":"monitor","description":"Monitor member for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%9","coding_agent":"claude","created_at":"{ts}"}}}}}}"#
                ) + "\n"
            );
        } else {
            assert_eq!(stdout(&output), "1 director=1 monitor=2\n");
        }
        assert_eq!(count(&cli, "members"), 2);
        assert_eq!(count(&cli, "member_placements"), 2);
        assert_eq!(count(&cli, "compensation_events"), 0);
        assert!(
            calls(&cli)
                .iter()
                .all(|line| !line.starts_with("kill-pane"))
        );
        assert_no_exit_commands(&cli);
    }
}
