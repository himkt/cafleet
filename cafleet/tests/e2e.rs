mod common;

use common::{Cli, code, stderr, stdout};

#[test]
fn registration_spawn_and_point_to_point_message_lifecycle() {
    let mut cli = Cli::new();
    cli.install();
    let fleet = cli.run(&[
        "fleet",
        "create",
        "--name",
        "e2e",
        "--coding-agent",
        "claude",
        "--json",
    ]);
    assert_eq!(code(&fleet), 0, "{}", stderr(&fleet));
    let created: serde_json::Value = serde_json::from_str(&stdout(&fleet)).unwrap();
    let fleet_id = created["fleet_id"].as_i64().unwrap();
    let director = created["director"]["member_id"].as_i64().unwrap();
    assert_eq!(cli.shim_count("split-window"), 0);
    cli.set_env("CAFLEET_TEST_NEXT_PANE", "%8");
    let worker = cli.create_member(fleet_id, "worker");
    cli.set_pane_capture(
        "%8",
        "Ready.\n────────────\n❯ \x1b[0m\n────────────\n? for shortcuts",
    );
    let sent = cli.run(&[
        "message",
        "send",
        "--from-member-id",
        &director.to_string(),
        "--to-member-id",
        &worker.to_string(),
        "e2e task",
        "--json",
    ]);
    assert_eq!(code(&sent), 0, "{}", stderr(&sent));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&sent)).unwrap();
    let id = payload["message"]["message_id"].as_i64().unwrap();
    assert_eq!(payload["notification_sent"], true);
    let polled = cli.run(&["message", "poll", &worker.to_string(), "--json"]);
    assert_eq!(code(&polled), 0, "{}", stderr(&polled));
    let inbox: serde_json::Value = serde_json::from_str(&stdout(&polled)).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0], payload["message"]);
    let acked = cli.run(&["message", "ack", &id.to_string(), "--json"]);
    assert_eq!(code(&acked), 0, "{}", stderr(&acked));
    let ack: serde_json::Value = serde_json::from_str(&stdout(&acked)).unwrap();
    assert_eq!(ack["message"]["status_state"], "completed");
    let empty = cli.run(&["message", "poll", &worker.to_string(), "--json"]);
    assert_eq!(code(&empty), 0);
    assert_eq!(stdout(&empty).trim(), "[]");
    let capture = cli.run(&["fleet", "capture", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&capture), 0, "{}", stderr(&capture));
    let panes: serde_json::Value = serde_json::from_str(&stdout(&capture)).unwrap();
    assert_eq!(panes[0]["member_id"], director);
    assert_eq!(panes[1]["member_id"], worker);
    let deleted = cli.run(&["fleet", "delete", &fleet_id.to_string(), "--json"]);
    assert_eq!(code(&deleted), 0, "{}", stderr(&deleted));
    assert_eq!(
        cli.sqlite()
            .query_row("SELECT count(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
