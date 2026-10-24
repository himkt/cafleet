mod common;

use common::{Cli, code, stderr, stdout};

fn table_count(cli: &Cli, table: &str) -> i64 {
    cli.sqlite()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn assert_no_rows_persisted(cli: &Cli) {
    for table in ["fleets", "members", "member_placements"] {
        assert_eq!(table_count(cli, table), 0, "{table}: transaction rollback");
    }
}

fn create(cli: &Cli, backend: &str, json: bool) -> std::process::Output {
    let mut args = vec![
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        backend,
    ];
    if json {
        args.push("--json");
    }
    cli.run(&args)
}

fn assert_no_pane_writes(cli: &Cli) {
    assert!(
        cli.shim_calls()
            .iter()
            .all(|call| !call.starts_with("split-window")
                && !call.starts_with("send-keys")
                && !call.starts_with("kill-pane")),
        "{:?}",
        cli.shim_calls()
    );
}

#[test]
fn fleet_create_outside_any_multiplexer_preserves_the_registry() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run_outside_tmux(&[
        "fleet",
        "create",
        "--name",
        "alpha",
        "--coding-agent",
        "claude",
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output)
            .contains("Error: cafleet fleet create must be run inside a tmux or herdr session"),
        "{}",
        stderr(&output)
    );
    assert_no_rows_persisted(&cli);
}

#[test]
fn fleet_create_json_preserves_ordered_director_identity_and_placement() {
    let cli = Cli::new();
    cli.ready();
    let output = create(&cli, "claude", true);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let created = payload["created_at"].as_str().unwrap();
    assert_eq!(
        stdout(&output),
        format!(
            r#"{{"fleet_id":1,"name":"alpha","created_at":"{created}","director":{{"member_id":1,"name":"Director","description":"Root Director for this fleet","registered_at":"{created}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%0","coding_agent":"claude","created_at":"{created}"}}}}}}"#
        ) + "\n"
    );
    assert_eq!(table_count(&cli, "fleets"), 1);
    assert_eq!(table_count(&cli, "members"), 1);
    assert_eq!(table_count(&cli, "member_placements"), 1);
    assert_eq!(cli.sqlite().query_row(
        "SELECT f.director_member_id, m.status, p.mux_pane_id FROM fleets f JOIN members m ON m.member_id=f.director_member_id JOIN member_placements p ON p.member_id=m.member_id WHERE f.fleet_id=1",
        [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    ).unwrap(), (1, "active".into(), "%0".into()));
    assert_no_pane_writes(&cli);
}

#[test]
fn fleet_create_text_is_the_compact_registration_result() {
    let cli = Cli::new();
    cli.ready();
    let output = create(&cli, "claude", false);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stdout(&output), "1 director=1\n");
    assert_no_pane_writes(&cli);
}

#[test]
fn fleet_create_registers_each_backend_without_splitting() {
    for backend in ["claude", "codex", "opencode"] {
        let mut cli = Cli::new();
        cli.ready();
        cli.fail_subcommand = Some("split-window".into());
        let output = create(&cli, backend, true);
        assert_eq!(code(&output), 0, "{backend}: {}", stderr(&output));
        let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(payload["director"]["placement"]["coding_agent"], backend);
        assert_eq!(table_count(&cli, "members"), 1);
        assert_no_pane_writes(&cli);
    }
}

#[test]
fn fleet_create_rejects_an_unknown_backend_before_registration() {
    let cli = Cli::new();
    cli.ready();
    let output = create(&cli, "unknown-backend", false);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("unknown-backend"),
        "{}",
        stderr(&output)
    );
    assert_no_rows_persisted(&cli);
    assert_no_pane_writes(&cli);
}

#[test]
fn fleet_registration_failures_roll_back_every_stage_and_preserve_the_cause() {
    for (stage, trigger) in [
        ("fleet insert", "BEFORE INSERT ON fleets"),
        ("Director insert", "BEFORE INSERT ON members"),
        ("placement insert", "BEFORE INSERT ON member_placements"),
        (
            "Director backfill",
            "BEFORE UPDATE OF director_member_id ON fleets",
        ),
    ] {
        let cli = Cli::new();
        cli.ready();
        cli.sqlite().execute_batch(&format!(
            "CREATE TRIGGER fail_registration {trigger} BEGIN SELECT RAISE(ABORT, 'registration fixture failure'); END;"
        )).unwrap();
        let output = create(&cli, "claude", true);
        assert_eq!(code(&output), 1, "{stage}: {}", stderr(&output));
        assert!(
            stderr(&output).contains("registration fixture failure"),
            "{stage}: {}",
            stderr(&output)
        );
        assert!(stdout(&output).trim().is_empty());
        assert_no_rows_persisted(&cli);
        assert_no_pane_writes(&cli);
        cli.sqlite()
            .execute_batch("DROP TRIGGER fail_registration;")
            .unwrap();
        let again = create(&cli, "claude", false);
        assert_eq!(code(&again), 0, "{stage}: {}", stderr(&again));
        assert_eq!(stdout(&again), "1 director=1\n");
    }
}

#[test]
fn ordinary_member_creation_is_available_immediately_after_registration() {
    let cli = Cli::new();
    cli.ready();
    let output = create(&cli, "claude", false);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let member = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        "1",
        "--name",
        "worker",
        "--description",
        "test",
        "FLEET {fleet_id} ME {member_id} DIRECTOR {director_member_id} AGENT {coding_agent}",
    ]);
    assert_eq!(code(&member), 0, "{}", stderr(&member));
    assert_eq!(stdout(&member), "2 worker backend=claude pane=%7\n");
    assert_eq!(table_count(&cli, "members"), 2);
    let calls = cli.shim_calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.starts_with("split-window"))
            .count(),
        1
    );
    assert!(
        calls
            .iter()
            .any(|call| call.contains("FLEET 1 ME 2 DIRECTOR 1 AGENT claude"))
    );
}

#[test]
fn fleet_list_reports_empty_then_the_created_fleet() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("No fleets found."),
        "got: {}",
        stdout(&output)
    );

    let (fleet_id, _) = cli.seed_fleet("alpha");
    let output = cli.run(&["fleet", "list"]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(out.contains("FLEET_ID"), "the header row, got: {out}");
    assert!(out.contains("alpha"), "got: {out}");

    let json_output = cli.run(&["fleet", "list", "--json"]);
    let payload: serde_json::Value = serde_json::from_str(stdout(&json_output).trim()).unwrap();
    assert_eq!(payload[0]["fleet_id"], fleet_id);
    assert_eq!(
        payload[0]["member_count"], 1,
        "registration creates the Director"
    );
}

#[test]
fn fleet_show_takes_the_positional_subject_and_returns_soft_deleted_rows() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let output = cli.run(&["fleet", "show", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("testfleet"), "got: {out}");

    cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    let output = cli.run(&["fleet", "show", &fleet_id.to_string()]);
    assert_eq!(
        code(&output),
        0,
        "soft-deleted rows are returned intentionally"
    );
    assert!(
        stdout(&output).contains("deleted_at:"),
        "got: {}",
        stdout(&output)
    );
}

#[test]
fn fleet_show_missing_is_the_pinned_application_error() {
    let cli = Cli::new();
    cli.ready();
    let output = cli.run(&["fleet", "show", "999"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("Error: fleet '999' not found."),
        "got: {}",
        stderr(&output)
    );
}

#[test]
fn fleet_delete_reports_the_count_and_is_idempotent() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let output = cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("Deleted fleet 1. Deregistered 1 members."),
        "got: {}",
        stdout(&output)
    );

    let again = cli.run(&["fleet", "delete", &fleet_id.to_string()]);
    assert_eq!(code(&again), 0);
    assert!(
        stdout(&again).contains("Deleted fleet 1. Deregistered 0 members."),
        "got: {}",
        stdout(&again)
    );
}

#[test]
fn fleet_delete_json_reports_the_deregistered_count() {
    let cli = Cli::new();
    let (fleet_id, _) = cli.seeded_fleet();
    let output = cli.run(&["fleet", "delete", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(payload["deregistered_count"], 1);
}

#[test]
fn fleet_delete_deregisters_members_preserves_messages_and_leaves_panes_open() {
    let cli = Cli::new();
    let (fleet, director) = cli.seeded_fleet();
    let member = cli.seed_member(fleet, "worker");
    let message = cli.seed_message(director, member, "retained message");
    let before = cli.shim_calls().len();
    let output = cli.run(&["fleet", "delete", &fleet.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["deregistered_count"], 2);
    assert_eq!(table_count(&cli, "member_placements"), 0);
    assert_eq!(
        cli.sqlite()
            .query_row(
                "SELECT COUNT(*) FROM members WHERE status='deregistered'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        cli.sqlite()
            .query_row(
                "SELECT text FROM messages WHERE message_id=?1",
                [message],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "retained message"
    );
    assert!(
        cli.shim_calls()[before..]
            .iter()
            .all(|call| !call.starts_with("kill-pane") && !call.starts_with("send-keys"))
    );
}
