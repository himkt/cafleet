use serde_json::Value;

use crate::output::render::render_message;

/// Render a scalar JSON value the way it appears in text output: strings
/// bare (no quotes), everything else in its JSON form.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `[<id> | from:<from> | <ts>[ | kind:<kind>][ | origin:<id>]]` + the body
/// line (omitted when empty) — the only text form; the full envelope is
/// `--json` (SPEC §6.4).
pub fn format_message(message: &Value) -> String {
    let message = match message.get("message") {
        Some(inner) if inner.is_object() => inner,
        _ => message,
    };
    let rendered = render_message(message);
    let mut line1 = format!(
        "[{} | from:{} | {}",
        scalar(&rendered["id"]),
        scalar(&rendered["from"]),
        scalar(&rendered["ts"]),
    );
    if let Some(kind) = rendered.get("kind") {
        line1.push_str(&format!(" | kind:{}", scalar(kind)));
    }
    if let Some(origin) = rendered.get("origin") {
        line1.push_str(&format!(" | origin:{}", scalar(origin)));
    }
    line1.push(']');
    let body = rendered.get("text").map(scalar).unwrap_or_default();
    if body.is_empty() {
        return line1;
    }
    format!("{line1}\n{body}")
}

/// Join formatted items with a single blank line between them; items are not
/// numbered (members reference messages by `message_id`, not list index).
pub fn format_indexed_list(
    items: &[Value],
    formatter: impl Fn(&Value) -> String,
    empty_msg: &str,
) -> String {
    if items.is_empty() {
        return empty_msg.to_string();
    }
    items.iter().map(formatter).collect::<Vec<_>>().join("\n\n")
}

/// `<id> <name> <status>` — the only text form; the detailed view is the
/// member JSON presenter output, emitted by `--json` (SPEC §6.4).
pub fn format_member_detail(member: &Value) -> String {
    format!(
        "{} {} {}",
        scalar(&member["member_id"]),
        scalar(&member["name"]),
        scalar(&member["status"]),
    )
}

/// `<fleet_id> director=<id> monitor=<id>` — the only text form (SPEC §6.4).
pub fn format_fleet_create(data: &Value) -> String {
    format!(
        "{} director={} monitor={}",
        scalar(&data["fleet_id"]),
        scalar(&data["director"]["member_id"]),
        scalar(&data["monitor"]["member_id"]),
    )
}

/// `<id> <name> backend=<coding_agent> pane=<pane_id>` — the only text form
/// (SPEC §6.4).
pub fn format_member(data: &Value) -> String {
    let placement = &data["placement"];
    let pane = if placement["mux_pane_id"].is_null() {
        "(pending)".to_string()
    } else {
        scalar(&placement["mux_pane_id"])
    };
    format!(
        "{} {} backend={} pane={pane}",
        scalar(&data["member_id"]),
        scalar(&data["name"]),
        scalar(&placement["coding_agent"]),
    )
}

fn format_idle(value: &Value) -> String {
    match value.as_i64() {
        None => "-".to_string(),
        Some(seconds) if seconds < 60 => format!("{seconds}s"),
        Some(seconds) if seconds < 3600 => format!("{}m", seconds / 60),
        Some(seconds) => format!("{}h", seconds / 3600),
    }
}

/// The `member list` table: one row per active registry entry, `-` cells for
/// a placementless row, `(pending)` for a placed row with no pane yet.
pub fn format_member_list(members: &[Value]) -> String {
    if members.is_empty() {
        return "0 members.".to_string();
    }
    let plural = if members.len() > 1 { "s" } else { "" };
    let mut lines = vec![
        format!("{} member{plural}:", members.len()),
        "  member_id  name           kind      backend   pane_id  idle".to_string(),
        "  ---------  -------------  --------  --------  -------  ----".to_string(),
    ];
    for member in members {
        let placement = &member["placement"];
        let (backend, pane) = if placement.is_null() {
            ("-".to_string(), "-".to_string())
        } else {
            let pane = if placement["mux_pane_id"].is_null() {
                "(pending)".to_string()
            } else {
                scalar(&placement["mux_pane_id"])
            };
            (scalar(&placement["coding_agent"]), pane)
        };
        lines.push(format!(
            "  {:<9}  {:<13}  {:<8}  {backend:<8}  {pane:<7}  {}",
            scalar(&member["member_id"]),
            scalar(&member["name"]),
            scalar(&member["kind"]),
            format_idle(&member["idle"]),
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TS: &str = "2026-07-30T09:00:00.000000+00:00";

    fn unicast(id: i64, from: i64, to: i64, text: &str) -> Value {
        json!({
            "message_id": id,
            "owner_member_id": to,
            "from_member_id": from,
            "to_member_id": to,
            "type": "unicast",
            "created_at": TS,
            "status_state": "input_required",
            "status_timestamp": TS,
            "origin_message_id": null,
            "text": text,
        })
    }

    fn broadcast_summary(id: i64, from: i64, n: usize) -> Value {
        json!({
            "message_id": id,
            "owner_member_id": from,
            "from_member_id": from,
            "to_member_id": null,
            "type": "broadcast_summary",
            "created_at": TS,
            "status_state": "completed",
            "status_timestamp": TS,
            "origin_message_id": id,
            "text": format!("Broadcast sent to {n} recipients"),
        })
    }

    mod format_message_tests {
        use super::*;

        #[test]
        fn compact_renders_the_bracket_line_plus_body() {
            assert_eq!(
                format_message(&unicast(5, 2, 3, "hello")),
                format!("[5 | from:2 | {TS}]\nhello")
            );
        }

        #[test]
        fn compact_omits_the_body_line_when_text_is_empty() {
            assert_eq!(
                format_message(&unicast(5, 2, 3, "")),
                format!("[5 | from:2 | {TS}]")
            );
        }

        #[test]
        fn compact_appends_kind_and_origin_segments() {
            assert_eq!(
                format_message(&broadcast_summary(6, 2, 2)),
                format!(
                    "[6 | from:2 | {TS} | kind:broadcast_summary | origin:6]\nBroadcast sent to 2 recipients"
                )
            );
        }

        #[test]
        fn compact_unwraps_a_message_envelope() {
            let envelope = json!({"message": unicast(5, 2, 3, "hello"), "notification_sent": true});
            assert_eq!(
                format_message(&envelope),
                format!("[5 | from:2 | {TS}]\nhello")
            );
        }
    }

    mod format_indexed_list_tests {
        use super::*;

        #[test]
        fn empty_list_returns_the_empty_message() {
            let items: Vec<Value> = vec![];
            assert_eq!(
                format_indexed_list(&items, |_| String::new(), "No messages found."),
                "No messages found."
            );
        }

        #[test]
        fn items_are_joined_with_one_blank_line_and_not_numbered() {
            let items = vec![unicast(1, 2, 3, "a"), unicast(2, 2, 3, "b")];
            assert_eq!(
                format_indexed_list(&items, format_message, "empty"),
                format!("[1 | from:2 | {TS}]\na\n\n[2 | from:2 | {TS}]\nb")
            );
        }
    }

    mod format_member_detail_tests {
        use super::*;

        fn member(skills: Value, placement: Value) -> Value {
            json!({
                "member_id": 7,
                "name": "analyst",
                "description": "Does analysis",
                "status": "active",
                "registered_at": TS,
                "kind": "member",
                "skills": skills,
                "placement": placement,
            })
        }

        #[test]
        fn compact_is_id_name_status() {
            assert_eq!(
                format_member_detail(&member(json!([]), Value::Null)),
                "7 analyst active"
            );
        }

        #[test]
        fn compact_ignores_skills_and_placement() {
            let placement = json!({
                "coding_agent": "claude",
                "mux_session": "main",
                "mux_window_id": "@1",
                "mux_pane_id": null,
                "backend": "tmux",
                "created_at": TS,
            });
            assert_eq!(
                format_member_detail(&member(json!(["python", "sql"]), placement)),
                "7 analyst active",
                "the detailed view is --json only"
            );
        }
    }

    mod format_fleet_create_tests {
        use super::*;

        fn fleet_create_result(name: Value) -> Value {
            json!({
                "fleet_id": 3,
                "name": name,
                "created_at": TS,
                "director": {
                    "member_id": 1,
                    "name": "Director",
                    "placement": {
                        "mux_session": "main",
                        "mux_window_id": "@1",
                        "mux_pane_id": "%0",
                    },
                },
                "monitor": {
                    "member_id": 2,
                    "name": "monitor",
                    "placement": {
                        "mux_session": "main",
                        "mux_window_id": "@1",
                        "mux_pane_id": "%1",
                    },
                },
            })
        }

        #[test]
        fn compact_is_fleet_id_director_and_monitor() {
            assert_eq!(
                format_fleet_create(&fleet_create_result(json!("alpha"))),
                "3 director=1 monitor=2"
            );
        }

        #[test]
        fn compact_ignores_the_name_and_placement() {
            assert_eq!(
                format_fleet_create(&fleet_create_result(Value::Null)),
                "3 director=1 monitor=2",
                "the detailed view is --json only"
            );
        }
    }

    mod format_member_tests {
        use super::*;

        fn member_create_result(pane: Value) -> Value {
            json!({
                "member_id": 9,
                "name": "worker",
                "placement": {
                    "coding_agent": "claude",
                    "mux_pane_id": pane,
                    "mux_window_id": "@2",
                },
            })
        }

        #[test]
        fn compact_is_id_name_backend_pane() {
            assert_eq!(
                format_member(&member_create_result(json!("%5"))),
                "9 worker backend=claude pane=%5"
            );
        }

        #[test]
        fn compact_renders_pending_for_a_null_pane() {
            assert_eq!(
                format_member(&member_create_result(Value::Null)),
                "9 worker backend=claude pane=(pending)"
            );
        }
    }

    mod format_member_list_tests {
        use super::*;

        fn row(id: i64, name: &str, kind: &str, placement: Value, idle: Value) -> Value {
            json!({
                "member_id": id,
                "name": name,
                "kind": kind,
                "placement": placement,
                "last_sent": null,
                "last_recv": null,
                "last_ack": null,
                "idle": idle,
            })
        }

        fn placed(agent: &str, pane: Value) -> Value {
            json!({"coding_agent": agent, "mux_pane_id": pane})
        }

        #[test]
        fn empty_list_renders_zero_members() {
            let members: Vec<Value> = vec![];
            assert_eq!(format_member_list(&members), "0 members.");
        }

        #[test]
        fn single_member_uses_the_singular_header() {
            let members = vec![row(
                1,
                "Director",
                "director",
                placed("claude", json!("%1")),
                json!(59),
            )];
            let expected = [
                "1 member:",
                "  member_id  name           kind      backend   pane_id  idle",
                "  ---------  -------------  --------  --------  -------  ----",
                "  1          Director       director  claude    %1       59s",
            ]
            .join("\n");
            assert_eq!(format_member_list(&members), expected);
        }

        #[test]
        fn table_renders_columns_dashes_pending_and_humanized_idle() {
            let members = vec![
                row(
                    1,
                    "Director",
                    "director",
                    placed("claude", json!("%1")),
                    json!(59),
                ),
                row(2, "analyst", "member", Value::Null, Value::Null),
                row(
                    3,
                    "helper",
                    "member",
                    placed("codex", Value::Null),
                    json!(60),
                ),
                row(
                    4,
                    "w2",
                    "member",
                    placed("opencode", json!("%9")),
                    json!(3599),
                ),
                row(
                    5,
                    "w3",
                    "member",
                    placed("claude", json!("%10")),
                    json!(3600),
                ),
                row(
                    6,
                    "monitor",
                    "monitor",
                    placed("claude", json!("%2")),
                    json!(5),
                ),
            ];
            let expected = [
                "6 members:",
                "  member_id  name           kind      backend   pane_id  idle",
                "  ---------  -------------  --------  --------  -------  ----",
                "  1          Director       director  claude    %1       59s",
                "  2          analyst        member    -         -        -",
                "  3          helper         member    codex     (pending)  1m",
                "  4          w2             member    opencode  %9       59m",
                "  5          w3             member    claude    %10      1h",
                "  6          monitor        monitor   claude    %2       5s",
            ]
            .join("\n");
            assert_eq!(format_member_list(&members), expected);
        }
    }
}
