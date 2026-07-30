//! The admin WebUI HTTP app (SPEC §6.8): the 9 `/api` routes behind the
//! `X-Fleet-Id` header dependency, the wire renames (`status_state`→`status`,
//! `text`→`body`), the `{"detail": <string>}` error shape (422 included), and
//! the SPA fallback over the dist embedded at build time. Handlers bridge to
//! the synchronous broker via `spawn_blocking`.

use std::collections::BTreeSet;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::Response;
use axum::routing::{get, post};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::broker;
use crate::cli::helpers::CliNotifier;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::format_json;
use crate::time::now_utc;

const RESERVED_PREFIXES: [&str; 2] = ["ui", "api"];

#[derive(Clone)]
struct AppState {
    database_url: String,
}

/// Build the configured HTTP application: the `/api` router ahead of the SPA
/// fallback, both over the embedded dist and the given database.
pub fn create_app(database_url: &str) -> Result<Router, CafleetError> {
    let state = AppState {
        database_url: database_url.to_string(),
    };
    let api = Router::new()
        .route("/fleets", get(list_fleets))
        .route("/members", get(roster))
        .route("/monitor", get(monitor))
        .route(
            "/members/{member_id}/monitor",
            get(member_monitor).patch(patch_monitor),
        )
        .route("/members/{member_id}/inbox", get(inbox))
        .route("/members/{member_id}/sent", get(sent))
        .route("/timeline", get(timeline))
        .route("/messages/send", post(send))
        .with_state(state);
    Ok(Router::new().nest("/api", api).fallback(spa_fallback))
}

fn json_response(status: StatusCode, value: &Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(format_json(value).into())
        .expect("a JSON response always builds")
}

fn detail(status: StatusCode, message: &str) -> Response {
    json_response(status, &json!({"detail": message}))
}

fn broker_500(error: CafleetError) -> Response {
    detail(StatusCode::INTERNAL_SERVER_ERROR, error.message())
}

/// The sync-broker bridge: every handler body runs on the blocking pool with
/// its own connection.
async fn run_blocking(
    state: AppState,
    body: impl FnOnce(&mut Connection) -> Response + Send + 'static,
) -> Response {
    let task = tokio::task::spawn_blocking(move || {
        let mut conn = match crate::db::connect(&state.database_url) {
            Ok(conn) => conn,
            Err(error) => return broker_500(error),
        };
        body(&mut conn)
    });
    match task.await {
        Ok(response) => response,
        Err(error) => detail(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("internal error: {error}"),
        ),
    }
}

/// The `X-Fleet-Id` header dependency, resolution order pinned (SPEC §6.8):
/// missing/empty → 400; non-integer → 400; unknown fleet → 404 (checked in
/// the handler body against the DB).
fn fleet_header(headers: &HeaderMap) -> Result<i64, Box<Response>> {
    let raw = headers
        .get("X-Fleet-Id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if raw.is_empty() {
        return Err(Box::new(detail(
            StatusCode::BAD_REQUEST,
            "X-Fleet-Id header required",
        )));
    }
    raw.parse().map_err(|_| {
        Box::new(detail(
            StatusCode::BAD_REQUEST,
            "X-Fleet-Id must be an integer",
        ))
    })
}

fn require_fleet(conn: &Connection, fleet_id: i64) -> Result<(), Box<Response>> {
    match broker::get_fleet(conn, fleet_id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(Box::new(detail(StatusCode::NOT_FOUND, "Fleet not found"))),
        Err(error) => Err(Box::new(broker_500(error))),
    }
}

/// The monitor-config wire projection: `member_id` and `last_stall_check_at`
/// dropped, key order pinned.
fn monitor_projection(config: &Value) -> Value {
    json!({
        "interval_seconds": config["interval_seconds"],
        "last_ping_at": config["last_ping_at"],
        "enabled": config["enabled"],
    })
}

/// The `FormattedMessage` projection: names resolved by one bulk lookup with
/// direct keyed access — a missing id is a hard 500, never a silent fallback.
fn formatted_messages(conn: &Connection, rows: &[Value]) -> Result<Vec<Value>, CafleetError> {
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    for row in rows {
        ids.insert(
            row["from_member_id"]
                .as_i64()
                .expect("rows carry the sender"),
        );
        if let Some(to) = row["to_member_id"].as_i64() {
            ids.insert(to);
        }
    }
    let ids: Vec<i64> = ids.into_iter().collect();
    let names = broker::get_member_names(conn, &ids)?;
    let name_of = |id: i64| -> Value {
        json!(
            names
                .get(&id)
                .unwrap_or_else(|| panic!("member {id} has a name row"))
        )
    };
    Ok(rows
        .iter()
        .map(|row| {
            let to_name = row["to_member_id"].as_i64().map(name_of);
            json!({
                "message_id": row["message_id"],
                "from_member_id": row["from_member_id"],
                "from_member_name": name_of(row["from_member_id"].as_i64().expect("sender id")),
                "to_member_id": row["to_member_id"],
                "to_member_name": to_name,
                "type": row["type"],
                "status": row["status_state"],
                "created_at": row["created_at"],
                "status_timestamp": row["status_timestamp"],
                "origin_message_id": row["origin_message_id"],
                "body": row["text"],
            })
        })
        .collect())
}

async fn list_fleets(State(state): State<AppState>) -> Response {
    run_blocking(state, |conn| match broker::list_fleets(conn) {
        Ok(fleets) => json_response(StatusCode::OK, &Value::Array(fleets)),
        Err(error) => broker_500(error),
    })
    .await
}

async fn roster(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        let rows = match broker::list_roster(conn, fleet_id, true) {
            Ok(rows) => rows,
            Err(error) => return broker_500(error),
        };
        let mut members = Vec::with_capacity(rows.len());
        for row in rows {
            let member_id = row["member_id"].as_i64().expect("roster rows carry the id");
            let monitor = match broker::get_monitor_config(conn, fleet_id, member_id) {
                Ok(config) => config
                    .map(|c| monitor_projection(&c))
                    .unwrap_or(Value::Null),
                Err(error) => return broker_500(error),
            };
            let mut member = row;
            member["monitor"] = monitor;
            members.push(member);
        }
        json_response(StatusCode::OK, &json!({"members": members}))
    })
    .await
}

async fn monitor(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        let now = now_utc();
        let mut payload = match broker::monitor_runtime_payload(conn, fleet_id, now) {
            Ok(payload) => payload,
            Err(error) => return broker_500(error),
        };
        let members = match broker::monitor_members_payload(conn, fleet_id, now) {
            Ok(members) => members,
            Err(error) => return broker_500(error),
        };
        payload["members"] = Value::Array(members);
        json_response(StatusCode::OK, &payload)
    })
    .await
}

async fn member_monitor(
    State(state): State<AppState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        match broker::get_monitor_config(conn, fleet_id, member_id) {
            Ok(Some(config)) => json_response(StatusCode::OK, &monitor_projection(&config)),
            Ok(None) => detail(StatusCode::NOT_FOUND, "Member not enrolled"),
            Err(error) => broker_500(error),
        }
    })
    .await
}

/// The optional-fields PATCH body; a `null` field counts as absent.
fn parse_patch_body(body: &Bytes) -> Result<(Option<i64>, Option<bool>), Box<Response>> {
    let invalid = |message: &str| Box::new(detail(StatusCode::UNPROCESSABLE_ENTITY, message));
    let payload: Value =
        serde_json::from_slice(body).map_err(|e| invalid(&format!("invalid JSON body: {e}")))?;
    if !payload.is_object() {
        return Err(invalid("the request body must be a JSON object"));
    }
    let interval_seconds = match payload.get("interval_seconds") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let interval = value
                .as_i64()
                .ok_or_else(|| invalid("interval_seconds must be an integer"))?;
            if interval < 1 {
                return Err(invalid("interval_seconds must be >= 1"));
            }
            Some(interval)
        }
    };
    let enabled = match payload.get("enabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(enabled)) => Some(*enabled),
        Some(_) => return Err(invalid("enabled must be a boolean")),
    };
    Ok((interval_seconds, enabled))
}

async fn patch_monitor(
    State(state): State<AppState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    let (interval_seconds, enabled) = match parse_patch_body(&body) {
        Ok(parsed) => parsed,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        match broker::get_monitor_config(conn, fleet_id, member_id) {
            Ok(Some(_)) => {}
            Ok(None) => return detail(StatusCode::NOT_FOUND, "Member not enrolled"),
            Err(error) => return broker_500(error),
        }
        // The pre-check passed; a deregistration between it and the update
        // (TOCTOU) collapses to the same 404, never a 500.
        match broker::update_monitor_config(conn, fleet_id, member_id, interval_seconds, enabled) {
            Ok(config) => json_response(StatusCode::OK, &monitor_projection(&config)),
            Err(CafleetError::App(_)) => detail(StatusCode::NOT_FOUND, "Member not enrolled"),
            Err(error) => broker_500(error),
        }
    })
    .await
}

async fn member_messages(state: AppState, fleet_id: i64, member_id: i64, sent: bool) -> Response {
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        match broker::verify_member_fleet(conn, member_id, fleet_id) {
            Ok(true) => {}
            Ok(false) => return detail(StatusCode::NOT_FOUND, "Member not found"),
            Err(error) => return broker_500(error),
        }
        let rows = if sent {
            broker::list_sent(conn, member_id)
        } else {
            broker::list_inbox(conn, member_id)
        };
        let rows = match rows {
            Ok(rows) => rows,
            Err(error) => return broker_500(error),
        };
        match formatted_messages(conn, &rows) {
            Ok(messages) => json_response(StatusCode::OK, &json!({"messages": messages})),
            Err(error) => broker_500(error),
        }
    })
    .await
}

async fn inbox(
    State(state): State<AppState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    match fleet_header(&headers) {
        Ok(fleet_id) => member_messages(state, fleet_id, member_id, false).await,
        Err(response) => *response,
    }
}

async fn sent(
    State(state): State<AppState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    match fleet_header(&headers) {
        Ok(fleet_id) => member_messages(state, fleet_id, member_id, true).await,
        Err(response) => *response,
    }
}

async fn timeline(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        let rows = match broker::list_timeline(conn, fleet_id, 200) {
            Ok(rows) => rows,
            Err(error) => return broker_500(error),
        };
        match formatted_messages(conn, &rows) {
            Ok(messages) => json_response(StatusCode::OK, &json!({"messages": messages})),
            Err(error) => broker_500(error),
        }
    })
    .await
}

/// `to_member_id` deserializes as a JSON integer or the exact string `"*"` —
/// a stringified integer is rejected, not coerced.
enum SendRecipient {
    Member(i64),
    Broadcast,
}

fn parse_send_body(body: &Bytes) -> Result<(i64, SendRecipient, String), Box<Response>> {
    let invalid = |message: &str| Box::new(detail(StatusCode::UNPROCESSABLE_ENTITY, message));
    let payload: Value =
        serde_json::from_slice(body).map_err(|e| invalid(&format!("invalid JSON body: {e}")))?;
    let from = payload["from_member_id"]
        .as_i64()
        .ok_or_else(|| invalid("from_member_id must be an integer"))?;
    let recipient = match &payload["to_member_id"] {
        Value::Number(number) => SendRecipient::Member(
            number
                .as_i64()
                .ok_or_else(|| invalid("to_member_id must be an integer or \"*\""))?,
        ),
        Value::String(star) if star == "*" => SendRecipient::Broadcast,
        _ => return Err(invalid("to_member_id must be an integer or \"*\"")),
    };
    let text = payload["text"]
        .as_str()
        .ok_or_else(|| invalid("text must be a string"))?
        .to_string();
    Ok((from, recipient, text))
}

async fn send(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    let (from, recipient, text) = match parse_send_body(&body) {
        Ok(parsed) => parsed,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        match broker::get_member(conn, from, fleet_id) {
            Ok(Some(_)) => {}
            Ok(None) => return detail(StatusCode::BAD_REQUEST, "from_member not in fleet"),
            Err(error) => return broker_500(error),
        }
        let settings = match Settings::from_env() {
            Ok(settings) => settings,
            Err(error) => return broker_500(error),
        };
        let notifier = CliNotifier::new(&settings);
        let message = match recipient {
            SendRecipient::Broadcast => {
                match broker::broadcast_message(
                    conn,
                    &notifier,
                    settings.max_text_len,
                    fleet_id,
                    from,
                    &text,
                ) {
                    Ok(result) => result[0]["message"].clone(),
                    Err(error) => return broker_500(error),
                }
            }
            SendRecipient::Member(to) => {
                match broker::get_member(conn, to, fleet_id) {
                    Ok(Some(_)) => {}
                    Ok(None) => return detail(StatusCode::NOT_FOUND, "Member not found"),
                    Err(error) => return broker_500(error),
                }
                match broker::send_message(
                    conn,
                    &notifier,
                    settings.max_text_len,
                    fleet_id,
                    from,
                    &to.to_string(),
                    &text,
                ) {
                    Ok(result) => result["message"].clone(),
                    Err(error) => return broker_500(error),
                }
            }
        };
        json_response(
            StatusCode::OK,
            &json!({
                "message_id": message["message_id"],
                "status": message["status_state"],
            }),
        )
    })
    .await
}

fn mime_for(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or("");
    crate::embedded::CONTENT_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or("application/octet-stream", |(_, mime)| *mime)
}

fn embedded_file(path: &str) -> Option<Response> {
    let bytes = crate::embedded::lookup(crate::embedded::WEBUI_DIST, path)?;
    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime_for(path))
            .body(bytes.to_vec().into())
            .expect("an embedded-asset response always builds"),
    )
}

/// The SPA static server over the embedded dist: assets as-is; the reserved
/// first segments (`ui`, `api`) hard-404; everything else serves the SPA
/// entry.
async fn spa_fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(response) = embedded_file(path) {
        return response;
    }
    let first_segment = path.split('/').next().unwrap_or("");
    if first_segment == "api" {
        return detail(StatusCode::NOT_FOUND, "Not Found");
    }
    if RESERVED_PREFIXES.contains(&first_segment) {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("Not Found".into())
            .expect("a plain 404 always builds");
    }
    embedded_file("index.html").expect("the dist is embedded at build time")
}
