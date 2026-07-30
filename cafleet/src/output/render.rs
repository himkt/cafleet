use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

const TRUNCATION_SUFFIX: char = '…';

static ANSI_ESCAPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("the CSI pattern is valid"));

/// The A8 visual-blank test (SPEC §6.5): a line is blank when it is
/// whitespace-only after CSI stripping. Emptiness check only — the caller
/// keeps the line's original bytes.
pub fn is_visually_blank(line: &str) -> bool {
    ANSI_ESCAPE_RE.replace_all(line, "").trim().is_empty()
}

/// Strip ANSI CSI escape sequences and collapse `\r`-rewritten line segments:
/// only the segment after the last `\r` of each line survives, so the
/// captured buffer matches what an operator sees.
pub fn strip_ansi(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let cleaned = ANSI_ESCAPE_RE.replace_all(text, "");
    cleaned
        .split('\n')
        .map(|line| {
            line.rsplit('\r')
                .next()
                .expect("rsplit yields at least one segment")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compact separators, raw UTF-8, insertion-order keys.
pub fn format_json(data: &Value) -> String {
    serde_json::to_string(data).expect("a JSON value always serializes")
}

/// Truncate `value` to `limit` codepoints + the `…` suffix. Callers pass the
/// effective limit (`settings.max_text_len`, or an explicit literal such as
/// the member-description 60). `full` returns the value unchanged.
pub fn truncate_text(value: Option<&str>, full: bool, limit: usize) -> Option<String> {
    let value = value?;
    if full || value.chars().count() <= limit {
        return Some(value.to_string());
    }
    let mut truncated: String = value.chars().take(limit).collect();
    truncated.push(TRUNCATION_SUFFIX);
    Some(truncated)
}

/// In-place body truncation over a broker result: unwraps `{"message": …}`
/// envelopes, walks lists, and leaves non-message items untouched.
pub fn truncate_message_text(result: &mut Value, full: bool, max_text_len: usize) {
    if full {
        return;
    }
    match result {
        Value::Array(items) => {
            for item in items {
                truncate_item(item, max_text_len);
            }
        }
        other => truncate_item(other, max_text_len),
    }
}

fn truncate_item(item: &mut Value, limit: usize) {
    let Some(obj) = item.as_object_mut() else {
        return;
    };
    let target = if obj.contains_key("message") {
        match obj.get_mut("message").and_then(Value::as_object_mut) {
            Some(inner) => inner,
            None => return,
        }
    } else {
        obj
    };
    let truncated = match target.get("text") {
        Some(Value::String(text)) => truncate_text(Some(text), false, limit),
        _ => None,
    };
    if let Some(text) = truncated {
        target.insert("text".to_string(), Value::String(text));
    }
}

/// Project a typed-column message to the compact rendered shape
/// `{id, from, ts, text[, kind][, origin]}`; `full` returns it unchanged.
pub fn render_message(message: &Value, full: bool) -> Value {
    if full {
        return message.clone();
    }
    let field = |name: &str| -> Value {
        message
            .get(name)
            .unwrap_or_else(|| panic!("message dict carries '{name}'"))
            .clone()
    };
    let mut out = serde_json::Map::new();
    out.insert("id".to_string(), field("message_id"));
    out.insert("from".to_string(), field("from_member_id"));
    out.insert("ts".to_string(), field("status_timestamp"));
    out.insert("text".to_string(), field("text"));
    let kind = field("type");
    if kind != "unicast" {
        out.insert("kind".to_string(), kind);
    }
    let origin = &message["origin_message_id"];
    if is_truthy(origin) {
        out.insert("origin".to_string(), origin.clone());
    }
    Value::Object(out)
}

pub(crate) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Apply [`render_message`] to every message dict in a broker result
/// structure without mutating the input.
pub fn render_messages_in_result(result: &Value, full: bool) -> Value {
    if full {
        return result.clone();
    }
    match result {
        Value::Array(items) => Value::Array(items.iter().map(render_item).collect()),
        other => render_item(other),
    }
}

fn render_item(item: &Value) -> Value {
    let Some(obj) = item.as_object() else {
        return item.clone();
    };
    if let Some(inner) = obj.get("message")
        && inner
            .as_object()
            .is_some_and(|m| m.contains_key("message_id"))
    {
        let mut new = obj.clone();
        new.insert("message".to_string(), render_message(inner, false));
        return Value::Object(new);
    }
    if obj.contains_key("message_id") {
        return render_message(item, false);
    }
    item.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            let mut result =
                json!({"message": unicast(1, 2, &"b".repeat(10)), "notification_sent": true});
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
            assert_eq!(
                rendered["message"],
                render_message(&unicast(1, 2, "a"), false)
            );
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
}
