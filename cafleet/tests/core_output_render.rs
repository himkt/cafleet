//! Step 3 contract tests: `cafleet::output` render layer (SPEC §6.4) —
//! codepoint truncation, ANSI CSI strip + CR defrag, compact JSON with
//! pinned insertion-order keys, and the message wire projections.
//!
//! Expected public API (all over `serde_json::Value`; the crate must enable
//! serde_json's `preserve_order` feature so object key order is insertion
//! order):
//! - `strip_ansi(&str) -> String`
//! - `format_json(&Value) -> String` — compact separators, raw UTF-8
//! - `truncate_text(value: Option<&str>, full: bool, limit: usize)
//!    -> Option<String>` — callers pass the effective limit
//!   (`settings.max_text_len` default 200, or an explicit literal such as the
//!   member-description 60)
//! - `truncate_message_text(result: &mut Value, full: bool, max_text_len: usize)`
//!   — in-place, unwraps `{"message": …}` envelopes, walks lists
//! - `render_message(message: &Value, full: bool) -> Value` — compact
//!   projection `{id, from, ts, text[, kind][, origin]}`
//! - `render_messages_in_result(result: &Value, full: bool) -> Value` —
//!   non-mutating walker

use cafleet::output::{
    format_json, render_message, render_messages_in_result, strip_ansi, truncate_message_text,
    truncate_text,
};
use serde_json::{Value, json};

const TS: &str = "2026-07-30T09:00:00.000000+00:00";

fn unicast(id: i64, from: i64, text: &str) -> Value {
    json!({
        "message_id": id,
        "owner_member_id": 3,
        "from_member_id": from,
        "to_member_id": 3,
        "type": "unicast",
        "created_at": TS,
        "status_state": "input_required",
        "status_timestamp": TS,
        "origin_message_id": null,
        "text": text,
    })
}

mod strip_ansi_tests {
    use super::*;

    #[test]
    fn removes_csi_sequences() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b[38;5;196mX\x1b[0m"), "X");
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hclear"), "clear");
    }

    #[test]
    fn keeps_non_csi_escapes() {
        assert_eq!(strip_ansi("\x1b]0;title\x07 done"), "\x1b]0;title\x07 done");
        assert_eq!(strip_ansi("\x1b(Btext"), "\x1b(Btext");
    }

    #[test]
    fn cr_defrag_keeps_only_the_segment_after_the_last_cr() {
        assert_eq!(strip_ansi("prefix\rNEW"), "NEW");
        assert_eq!(strip_ansi("a\rb\rc"), "c");
    }

    #[test]
    fn cr_defrag_applies_per_line() {
        assert_eq!(strip_ansi("one\rONE\ntwo\rTWO"), "ONE\nTWO");
        assert_eq!(strip_ansi("plain\nspin\rdone"), "plain\ndone");
    }

    #[test]
    fn csi_strip_runs_before_cr_defrag() {
        assert_eq!(strip_ansi("\x1b[2Kspinner\rdone"), "done");
    }

    #[test]
    fn empty_and_plain_inputs_pass_through() {
        assert_eq!(strip_ansi(""), "");
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }
}

mod format_json_tests {
    use super::*;

    #[test]
    fn emits_compact_separators() {
        let data = json!({"a":1,"b":[1,2],"c":{"d":"e"}});
        assert_eq!(format_json(&data), r#"{"a":1,"b":[1,2],"c":{"d":"e"}}"#);
    }

    #[test]
    fn keeps_non_ascii_raw() {
        let data = json!({"t":"ellipsis …"});
        assert_eq!(format_json(&data), "{\"t\":\"ellipsis …\"}");
    }

    #[test]
    fn preserves_insertion_order_of_keys() {
        let data = json!({"z":1,"a":2,"m":3});
        assert_eq!(format_json(&data), r#"{"z":1,"a":2,"m":3}"#);
    }
}

mod truncate_text_tests {
    use super::*;

    #[test]
    fn short_value_passes_through() {
        assert_eq!(
            truncate_text(Some("hello"), false, 200),
            Some("hello".to_string())
        );
    }

    #[test]
    fn value_exactly_at_the_limit_passes_through() {
        assert_eq!(
            truncate_text(Some("aaaa"), false, 4),
            Some("aaaa".to_string())
        );
    }

    #[test]
    fn over_limit_value_keeps_limit_codepoints_plus_ellipsis() {
        assert_eq!(
            truncate_text(Some("hello"), false, 4),
            Some("hell…".to_string())
        );
    }

    #[test]
    fn truncation_counts_codepoints_not_bytes() {
        let value = "あ".repeat(5);
        assert_eq!(
            truncate_text(Some(&value), false, 3),
            Some("あああ…".to_string())
        );
    }

    #[test]
    fn full_returns_the_value_unchanged() {
        let value = "x".repeat(500);
        assert_eq!(truncate_text(Some(&value), true, 4), Some(value));
    }

    #[test]
    fn none_returns_none() {
        assert_eq!(truncate_text(None, false, 4), None);
    }

    #[test]
    fn zero_limit_truncates_to_the_ellipsis_alone() {
        assert_eq!(truncate_text(Some("x"), false, 0), Some("…".to_string()));
    }
}

mod truncate_message_text_tests {
    use super::*;

    #[test]
    fn truncates_a_bare_message_dict_in_place() {
        let mut message = unicast(1, 2, &"a".repeat(10));
        truncate_message_text(&mut message, false, 5);
        assert_eq!(message["text"], "aaaaa…");
    }

    #[test]
    fn truncates_inside_a_message_envelope() {
        let mut result = json!({"message": unicast(1, 2, &"b".repeat(10)), "notification_sent": true});
        truncate_message_text(&mut result, false, 5);
        assert_eq!(result["message"]["text"], "bbbbb…");
        assert_eq!(result["notification_sent"], true);
    }

    #[test]
    fn truncates_every_item_of_a_list() {
        let mut result = json!([
            {"message": unicast(1, 2, &"c".repeat(10))},
            unicast(2, 3, &"d".repeat(10)),
        ]);
        truncate_message_text(&mut result, false, 5);
        assert_eq!(result[0]["message"]["text"], "ccccc…");
        assert_eq!(result[1]["text"], "ddddd…");
    }

    #[test]
    fn full_leaves_the_result_untouched() {
        let mut result = json!([unicast(1, 2, &"e".repeat(10))]);
        let before = result.clone();
        truncate_message_text(&mut result, true, 5);
        assert_eq!(result, before);
    }

    #[test]
    fn non_message_items_are_left_untouched() {
        let mut result = json!(["just a string", {"recipients": 2}]);
        let before = result.clone();
        truncate_message_text(&mut result, false, 5);
        assert_eq!(result, before);
    }
}

mod render_message_tests {
    use super::*;

    #[test]
    fn compact_projection_has_the_pinned_key_order() {
        let rendered = render_message(&unicast(5, 2, "hi"), false);
        assert_eq!(
            format_json(&rendered),
            format!(r#"{{"id":5,"from":2,"ts":"{TS}","text":"hi"}}"#)
        );
    }

    #[test]
    fn non_unicast_type_surfaces_kind_and_truthy_origin() {
        let mut summary = unicast(6, 2, "Broadcast sent to 2 recipients");
        summary["type"] = json!("broadcast_summary");
        summary["status_state"] = json!("completed");
        summary["to_member_id"] = Value::Null;
        summary["origin_message_id"] = json!(6);
        let rendered = render_message(&summary, false);
        assert_eq!(
            format_json(&rendered),
            format!(
                r#"{{"id":6,"from":2,"ts":"{TS}","text":"Broadcast sent to 2 recipients","kind":"broadcast_summary","origin":6}}"#
            )
        );
    }

    #[test]
    fn unicast_delivery_with_origin_keeps_origin_but_suppresses_kind() {
        let mut delivery = unicast(7, 2, "fanout");
        delivery["origin_message_id"] = json!(6);
        let rendered = render_message(&delivery, false);
        assert_eq!(
            format_json(&rendered),
            format!(r#"{{"id":7,"from":2,"ts":"{TS}","text":"fanout","origin":6}}"#)
        );
    }

    #[test]
    fn falsy_origin_is_suppressed() {
        let mut message = unicast(8, 2, "x");
        message["origin_message_id"] = json!(0);
        let rendered = render_message(&message, false);
        assert!(rendered.get("origin").is_none());
        let message = unicast(9, 2, "x");
        let rendered = render_message(&message, false);
        assert!(rendered.get("origin").is_none());
    }

    #[test]
    fn full_returns_the_message_unchanged() {
        let message = unicast(10, 2, "verbatim");
        assert_eq!(render_message(&message, true), message);
    }
}

mod render_messages_in_result_tests {
    use super::*;

    #[test]
    fn full_returns_the_result_unchanged() {
        let result = json!([{"message": unicast(1, 2, "a")}]);
        assert_eq!(render_messages_in_result(&result, true), result);
    }

    #[test]
    fn projects_each_bare_message_of_a_list() {
        let result = json!([unicast(1, 2, "a"), unicast(2, 3, "b")]);
        let rendered = render_messages_in_result(&result, false);
        assert_eq!(rendered[0], render_message(&unicast(1, 2, "a"), false));
        assert_eq!(rendered[1], render_message(&unicast(2, 3, "b"), false));
    }

    #[test]
    fn projects_the_message_inside_an_envelope_and_keeps_siblings() {
        let result = json!({"message": unicast(1, 2, "a"), "recipients": 2, "delivered": 1});
        let rendered = render_messages_in_result(&result, false);
        assert_eq!(rendered["message"], render_message(&unicast(1, 2, "a"), false));
        assert_eq!(rendered["recipients"], 2);
        assert_eq!(rendered["delivered"], 1);
    }

    #[test]
    fn does_not_mutate_the_input() {
        let result = json!([{"message": unicast(1, 2, "a")}]);
        let before = result.clone();
        let _ = render_messages_in_result(&result, false);
        assert_eq!(result, before);
    }

    #[test]
    fn non_message_values_pass_through_unchanged() {
        let scalar = json!("plain");
        assert_eq!(render_messages_in_result(&scalar, false), scalar);
        let unrelated = json!({"member_id": 4, "name": "analyst"});
        assert_eq!(render_messages_in_result(&unrelated, false), unrelated);
        let envelope_without_id = json!({"message": {"note": "no message_id"}});
        assert_eq!(
            render_messages_in_result(&envelope_without_id, false),
            envelope_without_id
        );
    }
}
