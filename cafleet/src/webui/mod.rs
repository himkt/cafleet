//! The admin WebUI HTTP app (SPEC §6.8): the 9 `/api` routes behind the
//! `X-Fleet-Id` header dependency, the wire renames (`status_state`→`status`,
//! `text`→`body`), the `{"detail": <string>}` error shape (422 included), and
//! the SPA fallback over the dist embedded at build time. Handlers bridge to
//! the synchronous broker via `spawn_blocking`.

use std::collections::BTreeSet;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::Response;
use axum::routing::{get, post};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::format_json;
use crate::runtime::RuntimeNotifier;
use crate::time::{format_utc, now_utc};

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
        .route("/monitor", get(monitor).patch(patch_monitor))
        .route("/monitor/wake", post(post_monitor_wake))
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
    detail(StatusCode::INTERNAL_SERVER_ERROR, &error.message())
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
    match broker::fleets::fetch_fleet(conn, fleet_id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(Box::new(detail(StatusCode::NOT_FOUND, "Fleet not found"))),
        Err(error) => Err(Box::new(broker_500(error))),
    }
}

/// The `FormattedMessage` projection: names resolved by one bulk lookup with
/// direct keyed access — a missing id is a hard 500, never a silent fallback.
fn formatted_message_records(
    conn: &Connection,
    rows: &[broker::records::MessageRecord],
) -> Result<Vec<Value>, CafleetError> {
    let mut ids = BTreeSet::new();
    for row in rows {
        ids.insert(row.from_member_id);
        ids.extend(row.to_member_id);
    }
    let names = broker::get_member_names(conn, &ids.into_iter().collect::<Vec<_>>())?;
    let name_of = |id: i64| {
        names
            .get(&id)
            .ok_or_else(|| CafleetError::InvalidStoredValue {
                field: "message member name".into(),
                value: id.to_string(),
            })
    };
    rows.iter()
        .map(|row| {
            let from_name = name_of(row.from_member_id)?;
            let to_name = row.to_member_id.map(name_of).transpose()?;
            Ok(json!({
                "message_id":row.message_id,"from_member_id":row.from_member_id,
                "from_member_name":from_name,"to_member_id":row.to_member_id,
                "to_member_name":to_name,"type":row.kind.as_str(),"status":row.status.as_str(),
                "created_at":row.created_at,"status_timestamp":row.status_timestamp,
                "origin_message_id":row.origin_message_id,"body":row.text,
            }))
        })
        .collect()
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
        match broker::list_roster_records(conn, fleet_id, true) {
            Ok(members) => json_response(StatusCode::OK, &json!({"members": members.iter().map(crate::presentation::roster_member).collect::<Vec<_>>()})),
            Err(error) => broker_500(error),
        }
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
        let mut payload = match broker::monitor_runtime_view(conn, fleet_id, now) {
            Ok(payload) => crate::presentation::monitor_runtime_view(&payload),
            Err(error) => return broker_500(error),
        };
        let members = match broker::monitor_member_records(conn, fleet_id, now) {
            Ok(members) => members
                .iter()
                .map(crate::presentation::monitor_member)
                .collect(),
            Err(error) => return broker_500(error),
        };
        payload["members"] = Value::Array(members);
        json_response(StatusCode::OK, &payload)
    })
    .await
}

/// `wake_interval_seconds` must be a JSON integer in `0..=i64::MAX` — floats,
/// stringified integers, negatives, and numbers above `i64::MAX` are
/// rejected, not coerced (SPEC §6.8).
fn parse_patch_monitor_body(body: &Bytes) -> Result<i64, Box<Response>> {
    let invalid = |message: &str| Box::new(detail(StatusCode::UNPROCESSABLE_ENTITY, message));
    let payload: Value =
        serde_json::from_slice(body).map_err(|e| invalid(&format!("invalid JSON body: {e}")))?;
    payload["wake_interval_seconds"]
        .as_i64()
        .filter(|value| *value >= 0)
        .ok_or_else(|| invalid("wake_interval_seconds must be a non-negative integer"))
}

/// Body validation precedes the fleet check, matching `POST /api/messages/send`;
/// the row update reports a never-run fleet as its own 404.
async fn patch_monitor(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    let wake_interval = match parse_patch_monitor_body(&body) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        match broker::set_monitor_wake_interval(conn, fleet_id, wake_interval) {
            Ok(true) => json_response(
                StatusCode::OK,
                &json!({"wake_interval_seconds": wake_interval}),
            ),
            Ok(false) => detail(
                StatusCode::NOT_FOUND,
                "monitor has never run for this fleet",
            ),
            Err(error) => broker_500(error),
        }
    })
    .await
}

/// The 404 gate is liveness, not row existence as in `PATCH /api/monitor` —
/// a wake request needs a live consumer. The check-then-write pair is not
/// transactional; a row that vanishes in between (`request_monitor_wake`
/// returning `false` after the gate passed) yields the same 404. Any request
/// body is ignored.
async fn post_monitor_wake(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let fleet_id = match fleet_header(&headers) {
        Ok(fleet_id) => fleet_id,
        Err(response) => return *response,
    };
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        let not_running = || {
            detail(
                StatusCode::NOT_FOUND,
                "monitor is not running for this fleet",
            )
        };
        match broker::monitor_is_live(conn, fleet_id, now_utc()) {
            Ok(true) => {}
            Ok(false) => return not_running(),
            Err(error) => return broker_500(error),
        }
        let when = format_utc(now_utc());
        match broker::request_monitor_wake(conn, fleet_id, &when) {
            Ok(true) => json_response(StatusCode::OK, &json!({"wake_requested_at": when})),
            Ok(false) => not_running(),
            Err(error) => broker_500(error),
        }
    })
    .await
}

fn history_options(raw_query: Option<&str>) -> Result<broker::HistoryOptions, CafleetError> {
    let invalid = || CafleetError::Value(broker::HISTORY_LIMIT_ERROR.to_string());
    let mut options = broker::HistoryOptions::default();
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        if key != "limit" {
            continue;
        }
        if options.limit.is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        options.limit = Some(value.parse().map_err(|_| invalid())?);
    }
    options.validate()
}

async fn member_messages(
    state: AppState,
    fleet_id: i64,
    member_id: i64,
    sent: bool,
    raw_query: Option<String>,
) -> Response {
    run_blocking(state, move |conn| {
        if let Err(response) = require_fleet(conn, fleet_id) {
            return *response;
        }
        match broker::verify_member_fleet(conn, member_id, fleet_id) {
            Ok(true) => {}
            Ok(false) => return detail(StatusCode::NOT_FOUND, "Member not found"),
            Err(error) => return broker_500(error),
        }
        // Query validation follows the existing fleet and membership guards.
        let options = match history_options(raw_query.as_deref()) {
            Ok(options) => options,
            Err(error) => return detail(StatusCode::UNPROCESSABLE_ENTITY, &error.message()),
        };
        let rows = if sent {
            broker::list_sent_records_with_options(conn, member_id, options)
        } else {
            broker::list_inbox_records_with_options(conn, member_id, options)
        };
        let rows = match rows {
            Ok(rows) => rows,
            Err(error) => return broker_500(error),
        };
        match formatted_message_records(conn, &rows) {
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
    RawQuery(raw_query): RawQuery,
) -> Response {
    match fleet_header(&headers) {
        Ok(fleet_id) => member_messages(state, fleet_id, member_id, false, raw_query).await,
        Err(response) => *response,
    }
}

async fn sent(
    State(state): State<AppState>,
    Path(member_id): Path<i64>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Response {
    match fleet_header(&headers) {
        Ok(fleet_id) => member_messages(state, fleet_id, member_id, true, raw_query).await,
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
        let rows = match broker::list_timeline_records(conn, fleet_id, 200) {
            Ok(rows) => rows,
            Err(error) => return broker_500(error),
        };
        match formatted_message_records(conn, &rows) {
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
        match broker::get_member_record(conn, from, fleet_id) {
            Ok(Some(_)) => {}
            Ok(None) => return detail(StatusCode::BAD_REQUEST, "from_member not in fleet"),
            Err(error) => return broker_500(error),
        }
        let settings = match Settings::from_env() {
            Ok(settings) => settings,
            Err(error) => return broker_500(error),
        };
        let notifier = RuntimeNotifier::new(&settings);
        let message = match recipient {
            SendRecipient::Broadcast => {
                match broker::broadcast_message_record(
                    conn,
                    &notifier,
                    settings.max_text_len,
                    from,
                    &text,
                ) {
                    Ok(result) => result.message,
                    Err(error) => return broker_500(error),
                }
            }
            SendRecipient::Member(to) => {
                match broker::get_member_record(conn, to, fleet_id) {
                    Ok(Some(_)) => {}
                    Ok(None) => return detail(StatusCode::NOT_FOUND, "Member not found"),
                    Err(error) => return broker_500(error),
                }
                match broker::send_message_record(
                    conn,
                    &notifier,
                    settings.max_text_len,
                    from,
                    &to.to_string(),
                    &text,
                ) {
                    // Persistence alone decides the response; the outcome's
                    // notification_error is intentionally ignored (SPEC §6.8).
                    Ok(outcome) => outcome.message,
                    Err(error) => return broker_500(error),
                }
            }
        };
        json_response(
            StatusCode::OK,
            &json!({
                "message_id": message.message_id,
                "status": message.status.as_str(),
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

#[cfg(test)]
mod integrity_regressions {
    use super::*;
    use crate::broker::test_support as common;

    #[test]
    fn required_message_names_return_errors_without_unwinding_in_the_presenter() {
        for sender_missing in [false, true] {
            let dir = tempfile::Builder::new()
                .prefix(".missing-name-")
                .tempdir_in(env!("CARGO_MANIFEST_DIR"))
                .unwrap();
            let mut conn = common::migrated_conn(&dir);
            let (fleet, director) = common::create_fleet(&mut conn, "integrity");
            let worker = common::register(&mut conn, fleet, "worker", None);
            let sent = common::send(
                &mut conn,
                &common::FakeNotifier::succeeding(),
                director,
                worker,
                "work",
            );
            let message =
                broker::get_message_record(&conn, sent["message"]["message_id"].as_i64().unwrap())
                    .unwrap();
            conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
            conn.execute(
                "DELETE FROM members WHERE member_id=?1",
                [if sender_missing { director } else { worker }],
            )
            .unwrap();
            conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                formatted_message_records(&conn, &[message])
            }));
            let error = result
                .expect("the presenter must return an integrity error, never unwind")
                .expect_err("missing required names cannot produce successful partial rows");
            assert!(!error.to_string().trim().is_empty());
        }
    }
}
