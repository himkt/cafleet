use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

const TRUNCATION_SUFFIX: char = '…';

static ANSI_ESCAPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("the CSI pattern is valid"));

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
