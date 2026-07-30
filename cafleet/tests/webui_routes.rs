//! Step 8 contract tests: the WebUI HTTP app, in-process via
//! `tower::ServiceExt` (SPEC §6.8) — the 9 routes, the `X-Fleet-Id` header
//! dependency, the wire renames, the 422 `{"detail": <string>}` body, and the
//! SPA fallback over the embedded dist.
//!
//! Expected public API:
//! - `cafleet::webui::create_app(database_url: &str)
//!    -> Result<axum::Router, CafleetError>`
//!
//! The dev-dependency versions (`axum`, `tower`) must match the crate's own —
//! the oneshot calls type-check against the app's router.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cafleet::broker::{self, InlinePreviewSender, NewPlacement};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct NullNotifier;

impl InlinePreviewSender for NullNotifier {
    fn send_inline_preview(&self, _: &str, _: i64, _: i64, _: &str, _: &str) -> bool {
        false
    }
}

fn migrated(dir: &TempDir) -> (String, rusqlite::Connection) {
    let url = format!("sqlite:///{}", dir.path().join("webui.db").display());
    let mut conn = cafleet::db::connect(&url).unwrap();
    cafleet::db::migrate_to_head(&mut conn).unwrap();
    (url, conn)
}

fn app(url: &str) -> axum::Router {
    cafleet::webui::create_app(url).unwrap()
}

fn placed(pane: &str) -> NewPlacement {
    NewPlacement {
        backend: "tmux".to_string(),
        mux_session: "main".to_string(),
        mux_window_id: "@1".to_string(),
        mux_pane_id: Some(pane.to_string()),
        coding_agent: "claude".to_string(),
    }
}

/// Fleet 1 with director 1 (`%0`), an enrolled worker (`%2`), and a
/// monitoring member (`%3`).
fn seeded_fleet(conn: &mut rusqlite::Connection) -> (i64, i64, i64, i64) {
    let fleet = broker::create_fleet(conn, Some("web"), "main", "@1", "%0", "claude", "tmux")
        .unwrap();
    let fleet_id = fleet["fleet_id"].as_i64().unwrap();
    let director_id = fleet["director"]["member_id"].as_i64().unwrap();
    let member_id = broker::register_member(
        conn,
        fleet_id,
        "worker",
        "d",
        &[],
        Some(&placed("%2")),
        None,
    )
    .unwrap()["member_id"]
        .as_i64()
        .unwrap();
    let monitor_id = broker::register_member(
        conn,
        fleet_id,
        "watch",
        "d",
        &[],
        Some(&placed("%3")),
        Some("monitoring-member"),
    )
    .unwrap()["member_id"]
        .as_i64()
        .unwrap();
    (fleet_id, director_id, member_id, monitor_id)
}

async fn call(
    app: axum::Router,
    method: &str,
    path: &str,
    fleet_header: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(fleet) = fleet_header {
        builder = builder.header("X-Fleet-Id", fleet);
    }
    let request = match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn parsed(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("non-JSON body {body:?}: {e}"))
}

fn keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .expect("a JSON object")
        .keys()
        .cloned()
        .collect()
}

#[tokio::test]
async fn get_fleets_is_an_unscoped_bare_array() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    seeded_fleet(&mut conn);

    let (status, body) = call(app(&url), "GET", "/api/fleets", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    let fleets = payload.as_array().expect("a bare array, not a wrapper");
    assert_eq!(fleets.len(), 1);
    assert_eq!(fleets[0]["fleet_id"], 1);
    assert_eq!(fleets[0]["name"], "web");
    assert_eq!(fleets[0]["member_count"], 3);
}

#[tokio::test]
async fn the_fleet_header_dependency_resolves_in_the_pinned_order() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    seeded_fleet(&mut conn);
    let app = app(&url);

    let (status, body) = call(app.clone(), "GET", "/api/members", None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id header required"}"#);

    let (status, body) = call(app.clone(), "GET", "/api/members", Some(""), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id header required"}"#, "empty counts as missing");

    let (status, body) = call(app.clone(), "GET", "/api/members", Some("abc"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id must be an integer"}"#);

    let (status, body) = call(app.clone(), "GET", "/api/members", Some(" "), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        r#"{"detail":"X-Fleet-Id must be an integer"}"#,
        "whitespace-only passes the presence check and fails the parse"
    );

    let (status, body) = call(app.clone(), "GET", "/api/members", Some("999"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Fleet not found"}"#);

    let (status, _) = call(app, "GET", "/api/members", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_roster_wraps_members_and_projects_the_monitor_config() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, director_id, member_id, monitor_id) = seeded_fleet(&mut conn);
    let holder_id = broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None)
        .unwrap()["member_id"]
        .as_i64()
        .unwrap();
    broker::send_message(
        &mut conn,
        &NullNotifier,
        200,
        fleet_id,
        director_id,
        &holder_id.to_string(),
        "audit trail",
    )
    .unwrap();
    broker::deregister_member(&mut conn, holder_id).unwrap();

    let (status, body) = call(app(&url), "GET", "/api/members", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    let members = payload["members"].as_array().expect("wrapped in members");
    assert_eq!(members.len(), 4, "active rows + the deregistered message holder");

    let by_id = |id: i64| members.iter().find(|m| m["member_id"] == id).unwrap();
    let director = by_id(director_id);
    assert_eq!(director["kind"], "director");
    assert_eq!(
        director["monitor"],
        json!({"interval_seconds": 180, "last_ping_at": null, "enabled": true}),
        "the projection drops member_id and last_stall_check_at"
    );
    let worker = by_id(member_id);
    assert_eq!(worker["kind"], "member");
    assert_eq!(worker["monitor"]["interval_seconds"], 720);
    let monitor = by_id(monitor_id);
    assert_eq!(monitor["kind"], "monitor");
    assert_eq!(monitor["monitor"], Value::Null, "the watcher is unenrolled");
    let holder = by_id(holder_id);
    assert_eq!(holder["status"], "deregistered");
    assert_eq!(holder["placement"], Value::Null);
    assert_eq!(holder["monitor"], Value::Null);
}

#[tokio::test]
async fn member_monitor_get_returns_the_exact_projection_or_404() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, _, member_id, monitor_id) = seeded_fleet(&mut conn);
    let app = app(&url);

    let (status, body) = call(
        app.clone(),
        "GET",
        &format!("/api/members/{member_id}/monitor"),
        Some("1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        r#"{"interval_seconds":720,"last_ping_at":null,"enabled":true}"#,
        "the projected config, key order pinned"
    );

    let (status, body) = call(
        app,
        "GET",
        &format!("/api/members/{monitor_id}/monitor"),
        Some("1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Member not enrolled"}"#);
}

#[tokio::test]
async fn patch_monitor_applies_partial_updates_and_validates_the_interval() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, _, member_id, monitor_id) = seeded_fleet(&mut conn);
    let app = app(&url);
    let path = format!("/api/members/{member_id}/monitor");

    let (status, body) = call(
        app.clone(),
        "PATCH",
        &path,
        Some("1"),
        Some(json!({"interval_seconds": 300})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["interval_seconds"], 300);
    assert_eq!(payload["enabled"], true, "the unspecified field is untouched");

    let (status, body) = call(
        app.clone(),
        "PATCH",
        &path,
        Some("1"),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["interval_seconds"], 300);
    assert_eq!(payload["enabled"], false);

    let (status, _) = call(app.clone(), "PATCH", &path, Some("1"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "a null/null patch is a valid no-op");

    let (status, body) = call(
        app.clone(),
        "PATCH",
        &path,
        Some("1"),
        Some(json!({"interval_seconds": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let payload = parsed(&body);
    assert!(
        payload["detail"].is_string(),
        "every request-validation failure is 422 {{\"detail\": <string>}}, got: {body}"
    );

    let (status, body) = call(
        app,
        "PATCH",
        &format!("/api/members/{monitor_id}/monitor"),
        Some("1"),
        Some(json!({"interval_seconds": 300})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Member not enrolled"}"#);
}

#[tokio::test]
async fn inbox_and_sent_carry_the_formatted_message_wire_shape() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, director_id, member_id, _) = seeded_fleet(&mut conn);
    broker::send_message(
        &mut conn,
        &NullNotifier,
        200,
        fleet_id,
        director_id,
        &member_id.to_string(),
        "hello wire",
    )
    .unwrap();
    let app = app(&url);

    let (status, body) = call(
        app.clone(),
        "GET",
        &format!("/api/members/{member_id}/inbox"),
        Some("1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    let messages = payload["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(
        keys(message),
        [
            "message_id",
            "from_member_id",
            "from_member_name",
            "to_member_id",
            "to_member_name",
            "type",
            "status",
            "created_at",
            "status_timestamp",
            "origin_message_id",
            "body",
        ],
        "the FormattedMessage key order is pinned"
    );
    assert_eq!(message["from_member_name"], "Director");
    assert_eq!(message["to_member_name"], "worker");
    assert_eq!(message["status"], "input_required", "status_state → status");
    assert_eq!(message["body"], "hello wire", "text → body");

    let (status, body) = call(
        app.clone(),
        "GET",
        &format!("/api/members/{director_id}/sent"),
        Some("1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&body)["messages"].as_array().unwrap().len(), 1);

    let (status, body) = call(
        app,
        "GET",
        "/api/members/999/inbox",
        Some("1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Member not found"}"#);
}

#[tokio::test]
async fn the_timeline_is_hard_capped_at_200() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, director_id, member_id, _) = seeded_fleet(&mut conn);
    for i in 0..201 {
        broker::send_message(
            &mut conn,
            &NullNotifier,
            200,
            fleet_id,
            director_id,
            &member_id.to_string(),
            &format!("message {i}"),
        )
        .unwrap();
    }

    let (status, body) = call(app(&url), "GET", "/api/timeline", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    let messages = payload["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 200, "the 200 most recent");
    assert_eq!(messages[0]["body"], "message 200", "newest first");
}

#[tokio::test]
async fn post_send_handles_unicast_broadcast_and_the_error_surfaces() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director_id, member_id, _) = seeded_fleet(&mut conn);
    let app = app(&url);

    let (status, body) = call(
        app.clone(),
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id": director_id, "to_member_id": member_id, "text": "hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert!(payload["message_id"].is_i64());
    assert_eq!(payload["status"], "input_required");

    let (status, body) = call(
        app.clone(),
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id": director_id, "to_member_id": "*", "text": "all"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&body)["status"], "completed", "the broadcast returns its summary");

    let (status, body) = call(
        app.clone(),
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id": director_id, "to_member_id": "5", "text": "hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        parsed(&body)["detail"].is_string(),
        "a stringified integer recipient is rejected, not coerced: {body}"
    );

    let (status, body) = call(
        app.clone(),
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id": 999, "to_member_id": member_id, "text": "hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"from_member not in fleet"}"#);

    let (status, body) = call(
        app,
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id": director_id, "to_member_id": 999, "text": "hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Member not found"}"#);
}

#[tokio::test]
async fn the_monitor_endpoint_reports_and_masks_the_runtime() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, _, _, _) = seeded_fleet(&mut conn);
    let app = app(&url);

    let (status, body) = call(app.clone(), "GET", "/api/monitor", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["running"], false);
    assert_eq!(payload["pid"], Value::Null);
    assert_eq!(payload["tick_seconds"], Value::Null, "no row at all");
    assert_eq!(
        payload["members"].as_array().unwrap().len(),
        2,
        "the watched set rides along"
    );

    let now = cafleet::time::now_utc();
    let pid = i64::from(std::process::id());
    broker::claim_monitor_runtime(&mut conn, fleet_id, pid, 5, &cafleet::time::format_utc(now))
        .unwrap();
    let (status, body) = call(app.clone(), "GET", "/api/monitor", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["running"], true);
    assert_eq!(payload["pid"], pid);
    assert_eq!(payload["tick_seconds"], 5);
    let age = payload["last_tick_age_seconds"].as_i64().unwrap();
    assert!((0..=5).contains(&age), "whole-second age, got {age}");

    let stale = cafleet::time::format_utc(now - chrono::Duration::seconds(100));
    conn.execute(
        "UPDATE monitor_runtime SET last_tick_at=?1, started_at=?1 WHERE fleet_id=?2",
        rusqlite::params![stale, fleet_id],
    )
    .unwrap();
    let (status, body) = call(app, "GET", "/api/monitor", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["running"], false);
    assert_eq!(payload["pid"], Value::Null, "a stale row never leaks its pid");
    assert_eq!(payload["tick_seconds"], 5, "only tick_seconds survives");
    assert_eq!(payload["last_tick_at"], Value::Null);
    assert_eq!(payload["started_at"], Value::Null);
}

#[tokio::test]
async fn the_spa_fallback_serves_index_except_for_reserved_prefixes() {
    let dir = TempDir::new().unwrap();
    let (url, _conn) = migrated(&dir);
    let app = app(&url);

    let (status, body) = call(app.clone(), "GET", "/", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<div id="), "the embedded SPA index, got: {body}");

    let (status, fallback_body) = call(app.clone(), "GET", "/fleets/1/details", None, None).await;
    assert_eq!(status, StatusCode::OK, "unknown paths fall back to the SPA entry");
    assert_eq!(fallback_body, body, "the fallback serves index.html itself");

    let (status, body) = call(app.clone(), "GET", "/api/nope", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        !body.contains("<div id="),
        "an unmatched /api/* path is never swallowed by the SPA, got: {body}"
    );

    let (status, body) = call(app, "GET", "/ui/missing.js", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("<div id="), "reserved prefixes hard-404, got: {body}");
}
