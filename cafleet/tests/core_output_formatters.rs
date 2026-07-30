//! Step 3 contract tests: `cafleet::output` formatter layer (SPEC §6.4 *Exact
//! text layouts*) — golden multi-line, column-aligned, ANSI-free strings with
//! the single ASCII `-` absent glyph.
//!
//! Expected public API (inputs are `serde_json::Value` broker shapes):
//! - `format_message(message: &Value, full: bool) -> String`
//! - `format_indexed_list(items: &[Value], formatter: impl Fn(&Value) -> String,
//!    empty_msg: &str) -> String`
//! - `format_member_detail(member: &Value, full: bool) -> String`
//! - `format_fleet_create(data: &Value, full: bool) -> String`
//! - `format_member(data: &Value, full: bool) -> String`
//! - `format_member_list(members: &[Value]) -> String`

use cafleet::output::{
    format_fleet_create, format_indexed_list, format_member, format_member_detail,
    format_member_list, format_message,
};
use serde_json::{Value, json};

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
            format_message(&unicast(5, 2, 3, "hello"), false),
            format!("[5 | from:2 | {TS}]\nhello")
        );
    }

    #[test]
    fn compact_omits_the_body_line_when_text_is_empty() {
        assert_eq!(
            format_message(&unicast(5, 2, 3, ""), false),
            format!("[5 | from:2 | {TS}]")
        );
    }

    #[test]
    fn compact_appends_kind_and_origin_segments() {
        assert_eq!(
            format_message(&broadcast_summary(6, 2, 2), false),
            format!(
                "[6 | from:2 | {TS} | kind:broadcast_summary | origin:6]\nBroadcast sent to 2 recipients"
            )
        );
    }

    #[test]
    fn compact_unwraps_a_message_envelope() {
        let envelope = json!({"message": unicast(5, 2, 3, "hello"), "notification_sent": true});
        assert_eq!(
            format_message(&envelope, false),
            format!("[5 | from:2 | {TS}]\nhello")
        );
    }

    #[test]
    fn verbose_renders_the_labeled_block() {
        let expected = [
            "  id:    5",
            "  state: input_required",
            "  from:  2",
            "  to:    3",
            "  type:  unicast",
            "  text:  hello",
        ]
        .join("\n");
        assert_eq!(format_message(&unicast(5, 2, 3, "hello"), true), expected);
    }

    #[test]
    fn verbose_omits_to_when_null_and_keeps_type_always() {
        let expected = [
            "  id:    6",
            "  state: completed",
            "  from:  2",
            "  type:  broadcast_summary",
            "  text:  Broadcast sent to 3 recipients",
        ]
        .join("\n");
        assert_eq!(format_message(&broadcast_summary(6, 2, 3), true), expected);
    }

    #[test]
    fn verbose_omits_the_text_line_when_text_is_empty() {
        let expected = [
            "  id:    5",
            "  state: input_required",
            "  from:  2",
            "  to:    3",
            "  type:  unicast",
        ]
        .join("\n");
        assert_eq!(format_message(&unicast(5, 2, 3, ""), true), expected);
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
            format_indexed_list(&items, |m| format_message(m, false), "empty"),
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
            format_member_detail(&member(json!([]), Value::Null), false),
            "7 analyst active"
        );
    }

    #[test]
    fn verbose_renders_the_labeled_block_with_dash_skills_and_none_placement() {
        let expected = [
            "  member_id:   7",
            "  name:        analyst",
            "  description: Does analysis",
            "  status:      active",
            "  kind:        member",
            "  skills:      -",
            "  placement:   none",
        ]
        .join("\n");
        assert_eq!(
            format_member_detail(&member(json!([]), Value::Null), true),
            expected
        );
    }

    #[test]
    fn verbose_renders_skills_as_a_compact_json_array() {
        let out = format_member_detail(&member(json!(["python", "sql"]), Value::Null), true);
        assert!(out.contains("  skills:      [\"python\",\"sql\"]"));
    }

    #[test]
    fn verbose_truncates_the_description_to_60_codepoints() {
        let mut m = member(json!([]), Value::Null);
        m["description"] = json!("d".repeat(61));
        let out = format_member_detail(&m, true);
        let expected_line = format!("  description: {}…", "d".repeat(60));
        assert!(out.contains(&expected_line), "got: {out}");
        let mut m = member(json!([]), Value::Null);
        m["description"] = json!("d".repeat(60));
        let out = format_member_detail(&m, true);
        assert!(out.contains(&format!("  description: {}", "d".repeat(60))));
        assert!(!out.contains('…'));
    }

    #[test]
    fn verbose_renders_the_placement_block_with_dash_for_null_pane() {
        let placement = json!({
            "coding_agent": "claude",
            "mux_session": "main",
            "mux_window_id": "@1",
            "mux_pane_id": null,
            "backend": "tmux",
            "created_at": TS,
        });
        let expected_tail = [
            "  placement:",
            "    backend:    claude",
            "    session:    main",
            "    window_id:  @1",
            "    pane_id:    -",
            &format!("    created_at: {TS}"),
        ]
        .join("\n");
        let out = format_member_detail(&member(json!([]), placement), true);
        assert!(out.ends_with(&expected_tail), "got: {out}");
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
                    "mux_pane_id": "%1",
                },
            },
        })
    }

    #[test]
    fn compact_is_fleet_id_and_director() {
        assert_eq!(
            format_fleet_create(&fleet_create_result(json!("alpha")), false),
            "3 director=1"
        );
    }

    #[test]
    fn verbose_renders_the_six_line_block() {
        let expected = [
            "3",
            "1",
            "name:             alpha",
            &format!("created_at:       {TS}"),
            "director_name:    Director",
            "pane:             main:@1:%1",
        ]
        .join("\n");
        assert_eq!(
            format_fleet_create(&fleet_create_result(json!("alpha")), true),
            expected
        );
    }

    #[test]
    fn verbose_renders_a_null_name_as_empty() {
        let out = format_fleet_create(&fleet_create_result(Value::Null), true);
        assert!(out.contains("name:             \n"), "got: {out}");
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
            format_member(&member_create_result(json!("%5")), false),
            "9 worker backend=claude pane=%5"
        );
    }

    #[test]
    fn compact_renders_pending_for_a_null_pane() {
        assert_eq!(
            format_member(&member_create_result(Value::Null), false),
            "9 worker backend=claude pane=(pending)"
        );
    }

    #[test]
    fn verbose_renders_the_six_line_block() {
        let expected = [
            "Member registered and spawned.",
            "  member_id: 9",
            "  name:      worker",
            "  backend:   claude",
            "  pane_id:   %5",
            "  window_id: @2",
        ]
        .join("\n");
        assert_eq!(format_member(&member_create_result(json!("%5")), true), expected);
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
            row(1, "Director", "director", placed("claude", json!("%1")), json!(59)),
            row(2, "analyst", "member", Value::Null, Value::Null),
            row(3, "watch", "monitor", placed("codex", Value::Null), json!(60)),
            row(4, "w2", "member", placed("opencode", json!("%9")), json!(3599)),
            row(5, "w3", "member", placed("claude", json!("%10")), json!(3600)),
        ];
        let expected = [
            "5 members:",
            "  member_id  name           kind      backend   pane_id  idle",
            "  ---------  -------------  --------  --------  -------  ----",
            "  1          Director       director  claude    %1       59s",
            "  2          analyst        member    -         -        -",
            "  3          watch          monitor   codex     (pending)  1m",
            "  4          w2             member    opencode  %9       59m",
            "  5          w3             member    claude    %10      1h",
        ]
        .join("\n");
        assert_eq!(format_member_list(&members), expected);
    }
}
