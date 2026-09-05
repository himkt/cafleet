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
    fn send_inline_preview(&self, _: &str, _: i64, _: i64, _: &str, _: &str) -> Result<(), String> {
        Err("pane notification suppressed in tests".to_string())
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

/// Fleet 1 with director 1 (`%0`), its bootstrap monitor member 2 (`%1`),
/// and two pane-bound workers (`%2`, `%3`).
fn seeded_fleet(conn: &mut rusqlite::Connection) -> (i64, i64, i64, i64) {
    let fleet = broker::create_fleet(
        conn,
        Some("web"),
        "main",
        "@1",
        "%0",
        "claude",
        "tmux",
        "monitor",
        "Monitor member for this fleet",
        |_, _, _| Ok("%1".to_string()),
    )
    .unwrap();
    let fleet_id = fleet["fleet_id"].as_i64().unwrap();
    let director_id = fleet["director"]["member_id"].as_i64().unwrap();
    let member_id = broker::register_member_record(
        conn,
        fleet_id,
        "worker",
        "d",
        &[],
        Some(&placed("%2")),
        false,
    )
    .map(|record| cafleet::presentation::registered_member(&record))
    .unwrap()["member_id"]
        .as_i64()
        .unwrap();
    let helper_id = broker::register_member_record(
        conn,
        fleet_id,
        "helper",
        "d",
        &[],
        Some(&placed("%3")),
        false,
    )
    .map(|record| cafleet::presentation::registered_member(&record))
    .unwrap()["member_id"]
        .as_i64()
        .unwrap();
    (fleet_id, director_id, member_id, helper_id)
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
    assert_eq!(
        fleets[0]["member_count"], 4,
        "director + bootstrap monitor + two workers"
    );
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
    assert_eq!(
        body, r#"{"detail":"X-Fleet-Id header required"}"#,
        "empty counts as missing"
    );

    let (status, body) = call(app.clone(), "GET", "/api/members", Some("abc"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id must be an integer"}"#);

    let (status, body) = call(app.clone(), "GET", "/api/members", Some(" "), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body, r#"{"detail":"X-Fleet-Id must be an integer"}"#,
        "whitespace-only passes the presence check and fails the parse"
    );

    let (status, body) = call(app.clone(), "GET", "/api/members", Some("999"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Fleet not found"}"#);

    let (status, _) = call(app, "GET", "/api/members", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_roster_wraps_members_with_the_three_value_kind_union() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, director_id, member_id, helper_id) = seeded_fleet(&mut conn);
    let monitor_id = broker::active_monitor_member_id(&conn, fleet_id)
        .unwrap()
        .expect("the bootstrap registers the monitor member");
    let holder_id =
        broker::register_member_record(&mut conn, fleet_id, "ghost", "d", &[], None, false)
            .map(|record| cafleet::presentation::registered_member(&record))
            .unwrap()["member_id"]
            .as_i64()
            .unwrap();
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
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
    assert_eq!(
        members.len(),
        5,
        "active rows + the deregistered message holder"
    );

    let expected_keys: std::collections::BTreeSet<&str> = [
        "member_id",
        "name",
        "description",
        "status",
        "registered_at",
        "kind",
        "placement",
    ]
    .into();
    for member in members {
        let row_keys: std::collections::BTreeSet<&str> = member
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            row_keys, expected_keys,
            "the full roster row key set is pinned (SPEC §6.4), got: {member}"
        );
    }

    let by_id = |id: i64| members.iter().find(|m| m["member_id"] == id).unwrap();
    let director = by_id(director_id);
    assert_eq!(director["kind"], "director");
    assert_eq!(
        director["description"], "Root Director for this fleet",
        "the SPA renders the description"
    );
    assert!(
        cafleet::time::parse_lenient(director["registered_at"].as_str().unwrap()).is_ok(),
        "the SPA sorts on registered_at"
    );
    let worker = by_id(member_id);
    assert_eq!(worker["kind"], "member");
    let helper = by_id(helper_id);
    assert_eq!(helper["kind"], "member");
    let monitor = by_id(monitor_id);
    assert_eq!(monitor["kind"], "monitor");
    let holder = by_id(holder_id);
    assert_eq!(holder["status"], "deregistered");
    assert_eq!(holder["placement"], Value::Null);
    assert_eq!(holder["kind"], "member");
}

#[tokio::test]
async fn inbox_and_sent_carry_the_formatted_message_wire_shape() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director_id, member_id, _) = seeded_fleet(&mut conn);
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
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

    let (status, body) = call(app, "GET", "/api/members/999/inbox", Some("1"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Member not found"}"#);
}

#[tokio::test]
async fn the_timeline_is_hard_capped_at_200() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director_id, member_id, _) = seeded_fleet(&mut conn);
    for i in 0..201 {
        broker::send_message_record(
            &mut conn,
            &NullNotifier,
            200,
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
    assert_eq!(
        status,
        StatusCode::OK,
        "persistence alone decides the response — a failed or skipped pane \
         notification never changes it"
    );
    let payload = parsed(&body);
    assert!(payload["message_id"].is_i64());
    assert_eq!(payload["status"], "input_required");
    assert_eq!(
        keys(&payload),
        ["message_id", "status"],
        "the unicast response gains no notification field"
    );

    let (status, body) = call(
        app.clone(),
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id": director_id, "to_member_id": "*", "text": "all"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&body)["status"],
        "completed",
        "the broadcast returns its summary"
    );

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
    let (fleet_id, director_id, member_id, helper_id) = seeded_fleet(&mut conn);
    let monitor_id = broker::active_monitor_member_id(&conn, fleet_id)
        .unwrap()
        .expect("the bootstrap registers the monitor member");
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
        director_id,
        &member_id.to_string(),
        "pending work",
    )
    .unwrap();
    let app = app(&url);

    let (status, body) = call(app.clone(), "GET", "/api/monitor", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["running"], false);
    assert_eq!(payload["pid"], Value::Null);
    assert_eq!(payload["tick_seconds"], Value::Null, "no row at all");
    assert_eq!(payload["wake_interval_seconds"], Value::Null);
    assert_eq!(payload["last_wake_at"], Value::Null);
    assert_eq!(payload["last_wake_age_seconds"], Value::Null);
    let rows = payload["members"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        2,
        "the members array re-sources to the wake roster"
    );
    assert!(
        !rows.iter().any(|row| row["member_id"] == director_id),
        "the root Director has no row, got: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row["member_id"] == monitor_id),
        "the monitor member has no row, got: {rows:?}"
    );
    let worker = rows.iter().find(|r| r["member_id"] == member_id).unwrap();
    assert_eq!(
        keys(worker),
        [
            "member_id",
            "name",
            "pending_count",
            "oldest_pending_ts",
            "oldest_pending_age_seconds",
        ],
        "the members element key order is pinned"
    );
    assert_eq!(worker["name"], "worker");
    assert_eq!(worker["pending_count"], 1);
    assert!(worker["oldest_pending_ts"].is_string());
    assert!(worker["oldest_pending_age_seconds"].is_i64());
    let helper = rows.iter().find(|r| r["member_id"] == helper_id).unwrap();
    assert_eq!(helper["pending_count"], 0);
    assert_eq!(helper["oldest_pending_ts"], Value::Null);
    assert_eq!(helper["oldest_pending_age_seconds"], Value::Null);

    let now = cafleet::time::now_utc();
    let pid = i64::from(std::process::id());
    broker::claim_monitor_runtime(
        &mut conn,
        fleet_id,
        pid,
        5,
        600,
        &cafleet::time::format_utc(now),
    )
    .unwrap();
    broker::record_monitor_wake(&mut conn, fleet_id, &cafleet::time::format_utc(now)).unwrap();
    let (status, body) = call(app.clone(), "GET", "/api/monitor", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(payload["running"], true);
    assert_eq!(payload["pid"], pid);
    assert_eq!(payload["tick_seconds"], 5);
    assert_eq!(payload["wake_interval_seconds"], 600);
    let age = payload["last_tick_age_seconds"].as_i64().unwrap();
    assert!((0..=5).contains(&age), "whole-second age, got {age}");
    assert!(payload["last_wake_at"].is_string());
    let wake_age = payload["last_wake_age_seconds"].as_i64().unwrap();
    assert!(
        (0..=5).contains(&wake_age),
        "whole-second wake age, got {wake_age}"
    );

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
    assert_eq!(
        payload["pid"],
        Value::Null,
        "a stale row never leaks its pid"
    );
    assert_eq!(payload["tick_seconds"], 5, "only tick_seconds survives");
    assert_eq!(
        payload["wake_interval_seconds"], 600,
        "the interval survives from the stale row, like tick_seconds"
    );
    assert_eq!(payload["last_tick_at"], Value::Null);
    assert_eq!(payload["started_at"], Value::Null);
    assert_eq!(
        payload["last_wake_at"],
        Value::Null,
        "last_wake_at is masked with the stale row"
    );
    assert_eq!(payload["last_wake_age_seconds"], Value::Null);
}

#[tokio::test]
async fn patch_monitor_updates_the_wake_interval_with_the_pinned_error_contract() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, _, _, _) = seeded_fleet(&mut conn);
    let app = app(&url);
    let valid = json!({"wake_interval_seconds": 300});

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/monitor",
        None,
        Some(valid.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id header required"}"#);

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/monitor",
        Some(""),
        Some(valid.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body, r#"{"detail":"X-Fleet-Id header required"}"#,
        "empty counts as missing"
    );

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/monitor",
        Some("abc"),
        Some(valid.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id must be an integer"}"#);

    // An unparsable body 422s before the fleet check — the unknown fleet 999
    // never reaches its 404, matching POST /api/messages/send.
    let request = Request::builder()
        .method("PATCH")
        .uri("/api/monitor")
        .header("X-Fleet-Id", "999")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("not json"))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let detail = parsed(core::str::from_utf8(&bytes).unwrap())["detail"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(detail.starts_with("invalid JSON body: "), "got: {detail}");

    // Rejected, not coerced: missing, float, stringified, negative, and
    // above-i64::MAX — all 422 before the unknown-fleet 404.
    for bad in [
        json!({}),
        json!({"wake_interval_seconds": 1.5}),
        json!({"wake_interval_seconds": "300"}),
        json!({"wake_interval_seconds": -1}),
        json!({"wake_interval_seconds": 9_223_372_036_854_775_808u64}),
    ] {
        let (status, body) = call(
            app.clone(),
            "PATCH",
            "/api/monitor",
            Some("999"),
            Some(bad.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "for {bad}");
        assert_eq!(
            body, r#"{"detail":"wake_interval_seconds must be a non-negative integer"}"#,
            "for {bad}"
        );
    }

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/monitor",
        Some("999"),
        Some(valid.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body, r#"{"detail":"Fleet not found"}"#,
        "a valid body against an unknown fleet 404s after validation"
    );

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/monitor",
        Some("1"),
        Some(valid.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body, r#"{"detail":"monitor has never run for this fleet"}"#,
        "a known fleet with no runtime row"
    );

    broker::claim_monitor_runtime(
        &mut conn,
        fleet_id,
        i64::from(std::process::id()),
        5,
        600,
        &cafleet::time::format_utc(cafleet::time::now_utc()),
    )
    .unwrap();
    let (status, body) = call(app.clone(), "PATCH", "/api/monitor", Some("1"), Some(valid)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, r#"{"wake_interval_seconds":300}"#,
        "the 200 payload is pinned"
    );

    let (status, body) = call(app.clone(), "GET", "/api/monitor", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&body)["wake_interval_seconds"],
        300,
        "GET reflects the PATCHed value"
    );

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/monitor",
        Some("1"),
        Some(json!({"wake_interval_seconds": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, r#"{"wake_interval_seconds":0}"#,
        "0 disables the wake"
    );

    let (status, body) = call(
        app,
        "PATCH",
        "/api/monitor",
        Some("1"),
        Some(json!({"wake_interval_seconds": i64::MAX})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, r#"{"wake_interval_seconds":9223372036854775807}"#,
        "i64::MAX is the inclusive domain ceiling"
    );
}

#[tokio::test]
async fn post_monitor_wake_requests_a_forced_wake_with_the_pinned_error_contract() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet_id, _, _, _) = seeded_fleet(&mut conn);
    let app = app(&url);

    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id header required"}"#);

    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some(""), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body, r#"{"detail":"X-Fleet-Id header required"}"#,
        "empty counts as missing"
    );

    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some("abc"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, r#"{"detail":"X-Fleet-Id must be an integer"}"#);

    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some("999"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Fleet not found"}"#);

    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some("1"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body, r#"{"detail":"monitor is not running for this fleet"}"#,
        "no runtime row — the fleet's monitor has never run"
    );

    let pid = i64::from(std::process::id());
    let now = cafleet::time::now_utc();
    broker::claim_monitor_runtime(
        &mut conn,
        fleet_id,
        pid,
        5,
        600,
        &cafleet::time::format_utc(now),
    )
    .unwrap();
    let stale = cafleet::time::format_utc(now - chrono::Duration::seconds(100));
    conn.execute(
        "UPDATE monitor_runtime SET last_tick_at=?1, started_at=?1 WHERE fleet_id=?2",
        rusqlite::params![stale, fleet_id],
    )
    .unwrap();
    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some("1"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body, r#"{"detail":"monitor is not running for this fleet"}"#,
        "a stale heartbeat reads as not running"
    );
    let row = broker::read_monitor_runtime_record(&conn, fleet_id)
        .map(|record| record.as_ref().map(cafleet::presentation::monitor_runtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        row["wake_requested_at"],
        Value::Null,
        "a refused request never stamps the row"
    );

    broker::claim_monitor_runtime(
        &mut conn,
        fleet_id,
        pid,
        5,
        600,
        &cafleet::time::format_utc(cafleet::time::now_utc()),
    )
    .unwrap();
    broker::clear_monitor_runtime(&mut conn, fleet_id, pid).unwrap();
    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some("1"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body, r#"{"detail":"monitor is not running for this fleet"}"#,
        "a cleared slot (pid NULL after a clean loop exit) is stopped"
    );

    broker::claim_monitor_runtime(
        &mut conn,
        fleet_id,
        pid,
        5,
        600,
        &cafleet::time::format_utc(cafleet::time::now_utc()),
    )
    .unwrap();
    let (status, body) = call(app.clone(), "POST", "/api/monitor/wake", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(
        keys(&payload),
        ["wake_requested_at"],
        "the 200 payload carries exactly the stamp"
    );
    let stamp = payload["wake_requested_at"]
        .as_str()
        .expect("a UTC ISO string");
    let row = broker::read_monitor_runtime_record(&conn, fleet_id)
        .map(|record| record.as_ref().map(cafleet::presentation::monitor_runtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        row["wake_requested_at"], stamp,
        "the 200 stamps the row with the returned timestamp"
    );

    let (status, body) = call(
        app,
        "POST",
        "/api/monitor/wake",
        Some("1"),
        Some(json!({"ignored": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        parsed(&body)["wake_requested_at"].is_string(),
        "any request body is ignored"
    );
}

#[tokio::test]
async fn the_spa_fallback_serves_index_except_for_reserved_prefixes() {
    let dir = TempDir::new().unwrap();
    let (url, _conn) = migrated(&dir);
    let app = app(&url);

    let (status, body) = call(app.clone(), "GET", "/", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<div id="),
        "the embedded SPA index, got: {body}"
    );

    let (status, fallback_body) = call(app.clone(), "GET", "/fleets/1/details", None, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "unknown paths fall back to the SPA entry"
    );
    assert_eq!(fallback_body, body, "the fallback serves index.html itself");

    let (status, body) = call(app.clone(), "GET", "/api/nope", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        !body.contains("<div id="),
        "an unmatched /api/* path is never swallowed by the SPA, got: {body}"
    );

    let (status, body) = call(app, "GET", "/ui/missing.js", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        !body.contains("<div id="),
        "reserved prefixes hard-404, got: {body}"
    );
}

#[tokio::test]
async fn timeline_broadcast_excludes_summary_through_two_delivery_ack_transitions() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director, _, helper) = seeded_fleet(&mut conn);
    broker::deregister_member(&mut conn, helper).unwrap();
    let router = app(&url);
    let (status, body) = call(
        router.clone(),
        "POST",
        "/api/messages/send",
        Some("1"),
        Some(json!({"from_member_id":director,"to_member_id":"*","text":"broadcast"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response = parsed(&body);
    let summary_id = response["message_id"].as_i64().unwrap();
    assert_eq!(
        response["status"], "completed",
        "broadcast response still represents the persisted summary"
    );
    let summary = broker::get_message_record(&conn, summary_id)
        .map(|record| cafleet::presentation::message_envelope(&record))
        .unwrap();
    assert_eq!(summary["message"]["type"], "broadcast_summary");
    assert!(summary["message"]["to_member_id"].is_null());
    for acked in 0..=2 {
        let (status, body) = call(router.clone(), "GET", "/api/timeline", Some("1"), None).await;
        assert_eq!(status, StatusCode::OK);
        let payload = parsed(&body);
        assert_eq!(keys(&payload), ["messages"]);
        let rows = payload["messages"].as_array().unwrap();
        assert_eq!(
            rows.len(),
            2,
            "completed summary is neither a recipient nor an ACK"
        );
        assert!(rows.iter().all(|r| r["type"] == "unicast"
            && r["to_member_id"].is_i64()
            && r["to_member_name"].is_string()
            && r["origin_message_id"] == summary_id));
        assert_eq!(
            rows.iter().filter(|r| r["status"] == "completed").count(),
            acked
        );
        assert_eq!(
            broker::get_message_record(&conn, summary_id)
                .map(|record| cafleet::presentation::message_envelope(&record))
                .unwrap(),
            summary
        );
        if let Some(row) = rows.iter().find(|r| r["status"] == "input_required") {
            broker::ack_message_record(&mut conn, row["message_id"].as_i64().unwrap())
                .map(|record| cafleet::presentation::message_envelope(&record))
                .unwrap();
        }
    }
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn timeline_empty_and_summary_only_fleets_have_the_empty_messages_envelope() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet, director, worker, helper) = seeded_fleet(&mut conn);
    let router = app(&url);
    let (status, body) = call(router.clone(), "GET", "/api/timeline", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"messages":[]}"#);
    let monitor = broker::active_monitor_member_id(&conn, fleet)
        .unwrap()
        .unwrap();
    for id in [worker, helper, monitor] {
        broker::deregister_member(&mut conn, id).unwrap();
    }
    let result =
        broker::broadcast_message_record(&mut conn, &NullNotifier, 200, director, "nobody")
            .map(|record| vec![cafleet::presentation::broadcast_outcome(&record)])
            .unwrap();
    assert_eq!(result[0]["recipients"], 0);
    let (status, body) = call(router, "GET", "/api/timeline", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"messages":[]}"#);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE type='broadcast_summary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn timeline_owner_fleet_scope_and_status_id_order_survive_wire_formatting() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director, worker, _) = seeded_fleet(&mut conn);
    let (_, foreign_director, foreign_worker, _) = seeded_fleet(&mut conn);
    for text in ["first", "second", "third"] {
        broker::send_message_record(
            &mut conn,
            &NullNotifier,
            200,
            director,
            &worker.to_string(),
            text,
        )
        .unwrap();
    }
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
        foreign_director,
        &foreign_worker.to_string(),
        "foreign endpoints, local owner",
    )
    .unwrap();
    conn.execute(
        "UPDATE messages SET owner_member_id=?1 WHERE message_id=4",
        [worker],
    )
    .unwrap();
    conn.execute(
        "UPDATE messages SET owner_member_id=?1 WHERE message_id=3",
        [foreign_worker],
    )
    .unwrap();
    conn.execute_batch("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00', created_at='2099-01-01T00:00:00+00:00';
        UPDATE messages SET status_timestamp='2026-02-01T00:00:00+00:00', created_at='2020-01-01T00:00:00+00:00' WHERE message_id IN (1,2);").unwrap();
    broker::deregister_member(&mut conn, worker).unwrap();
    let (status, body) = call(app(&url), "GET", "/api/timeline", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    let rows = payload["messages"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["message_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [2, 1, 4]
    );
    assert!(
        rows.iter()
            .all(|r| r["type"] == "unicast" && r["origin_message_id"].is_null())
    );
    assert_eq!(
        keys(&rows[0]),
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
            "body"
        ]
    );
    let (status, body) = call(app(&url), "GET", "/api/timeline", Some("2"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parsed(&body)["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["message_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [3]
    );
}

#[tokio::test]
async fn timeline_filters_before_200_row_cap_and_returns_only_fetched_broadcast_deliveries() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director, worker, helper) = seeded_fleet(&mut conn);
    broker::deregister_member(&mut conn, helper).unwrap();
    broker::broadcast_message_record(&mut conn, &NullNotifier, 200, director, "partial")
        .map(|record| vec![cafleet::presentation::broadcast_outcome(&record)])
        .unwrap();
    for _ in 0..199 {
        broker::send_message_record(
            &mut conn,
            &NullNotifier,
            200,
            director,
            &worker.to_string(),
            "single",
        )
        .unwrap();
    }
    conn.execute_batch("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00';
        UPDATE messages SET status_timestamp='2099-01-01T00:00:00+00:00' WHERE type='broadcast_summary';").unwrap();
    let (status, body) = call(app(&url), "GET", "/api/timeline", Some("1"), None).await;
    assert_eq!(status, StatusCode::OK);
    let payload = parsed(&body);
    assert_eq!(
        keys(&payload),
        ["messages"],
        "no group totals or pagination envelope are introduced"
    );
    let rows = payload["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 200);
    assert_eq!(
        rows.iter()
            .map(|r| r["message_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        (3..=202).rev().collect::<Vec<_>>()
    );
    let partial: Vec<_> = rows
        .iter()
        .filter(|r| r["origin_message_id"] == 1)
        .collect();
    assert_eq!(
        partial.len(),
        1,
        "the other recipient is outside the fetched row cap"
    );
    assert_eq!(partial[0]["status"], "input_required");
    assert_eq!(partial[0]["to_member_id"], worker);
    assert_eq!(
        broker::get_message_record(&conn, 1)
            .map(|record| cafleet::presentation::message_envelope(&record))
            .unwrap()["message"]["type"],
        "broadcast_summary"
    );
}

#[tokio::test]
async fn integrity_missing_sender_name_returns_500_without_a_panicked_task_detail() {
    missing_message_name_is_an_integrity_error(true).await;
}

#[tokio::test]
async fn integrity_missing_recipient_name_returns_500_without_a_panicked_task_detail() {
    missing_message_name_is_an_integrity_error(false).await;
}

async fn missing_message_name_is_an_integrity_error(sender: bool) {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director, worker, _) = seeded_fleet(&mut conn);
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
        director,
        &worker.to_string(),
        "integrity",
    )
    .unwrap();
    let absent = if sender { director } else { worker };
    // Break only this isolated database. Keep a valid owner so the read
    // reaches name resolution instead of being filtered out of the timeline.
    conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    if !sender {
        conn.execute("UPDATE messages SET owner_member_id=?1", [director])
            .unwrap();
    }
    conn.execute("DELETE FROM members WHERE member_id=?1", [absent])
        .unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let (status, body) = call(app(&url), "GET", "/api/timeline", Some("1"), None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let payload = parsed(&body);
    assert_eq!(keys(&payload), ["detail"]);
    let detail = payload["detail"].as_str().unwrap();
    assert!(
        !detail.contains("panicked") && !detail.contains("internal error: task"),
        "integrity errors must not originate from a panicked blocking task: {detail}"
    );
    assert!(!detail.trim().is_empty());
}

#[tokio::test]
async fn integrity_invalid_message_status_is_500_in_inbox_sent_and_timeline() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director, worker, _) = seeded_fleet(&mut conn);
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
        director,
        &worker.to_string(),
        "integrity",
    )
    .unwrap();
    conn.execute_batch(
        "PRAGMA ignore_check_constraints=ON; UPDATE messages SET status_state='corrupt-status';",
    )
    .unwrap();
    for path in [
        format!("/api/members/{worker}/inbox"),
        format!("/api/members/{director}/sent"),
        "/api/timeline".into(),
    ] {
        let (status, body) = call(app(&url), "GET", &path, Some("1"), None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}: {body}");
        let payload = parsed(&body);
        assert_eq!(keys(&payload), ["detail"]);
        let detail = payload["detail"].as_str().unwrap();
        assert!(
            !detail.trim().is_empty() && !detail.contains("panicked"),
            "{detail}"
        );
    }
}

#[tokio::test]
async fn integrity_invalid_member_status_is_500_instead_of_a_successful_roster_row() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (_, director, worker, _) = seeded_fleet(&mut conn);
    broker::send_message_record(
        &mut conn,
        &NullNotifier,
        200,
        director,
        &worker.to_string(),
        "holder",
    )
    .unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    conn.execute(
        "UPDATE members SET status='corrupt-status' WHERE member_id=?1",
        [worker],
    )
    .unwrap();
    let (status, body) = call(app(&url), "GET", "/api/members", Some("1"), None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let payload = parsed(&body);
    assert_eq!(keys(&payload), ["detail"]);
    let detail = payload["detail"].as_str().unwrap();
    assert!(
        !detail.trim().is_empty() && !detail.contains("panicked"),
        "{detail}"
    );
}

#[tokio::test]
async fn compatibility_stopped_monitor_wire_distinguishes_no_row_null_zero_and_positive_interval() {
    let dir = TempDir::new().unwrap();
    let (url, mut conn) = migrated(&dir);
    let (fleet, _, _, _) = seeded_fleet(&mut conn);
    for (row_exists, interval) in [
        (false, None),
        (true, None),
        (true, Some(0)),
        (true, Some(90)),
    ] {
        if row_exists {
            conn.execute(
                "INSERT OR IGNORE INTO monitor_runtime(fleet_id) VALUES (?1)",
                [fleet],
            )
            .unwrap();
            conn.execute("UPDATE monitor_runtime SET wake_interval_seconds=?1, last_wake_at='durable', wake_requested_at='pending' WHERE fleet_id=?2", rusqlite::params![interval,fleet]).unwrap();
        }
        let (status, body) = call(app(&url), "GET", "/api/monitor", Some("1"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let tick_json = if row_exists { "5" } else { "null" };
        let interval_json = serde_json::to_string(&interval).unwrap();
        assert_eq!(
            body,
            format!(
                r#"{{"running":false,"pid":null,"tick_seconds":{tick_json},"wake_interval_seconds":{interval_json},"last_tick_at":null,"last_tick_age_seconds":null,"started_at":null,"last_wake_at":null,"last_wake_age_seconds":null,"members":[{{"member_id":3,"name":"worker","pending_count":0,"oldest_pending_ts":null,"oldest_pending_age_seconds":null}},{{"member_id":4,"name":"helper","pending_count":0,"oldest_pending_ts":null,"oldest_pending_age_seconds":null}}]}}"#
            )
        );
        if row_exists {
            let raw = broker::read_monitor_runtime_record(&conn, fleet)
                .map(|record| record.as_ref().map(cafleet::presentation::monitor_runtime))
                .unwrap()
                .unwrap();
            assert_eq!(raw["last_wake_at"], "durable");
            assert_eq!(raw["wake_requested_at"], "pending");
        }
    }
}
