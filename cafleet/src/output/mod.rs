pub(crate) use crate::output::text::format_fleet_create;
pub(crate) use crate::output::text::format_fleet_delete;
pub(crate) use crate::output::text::format_fleet_list;
pub(crate) use crate::output::text::format_fleet_show;
pub(crate) use crate::output::text::format_input_operation;
pub(crate) use crate::output::text::format_member;
pub(crate) use crate::output::text::format_member_delete;
pub(crate) use crate::output::text::format_member_detail;
pub(crate) use crate::output::text::format_member_exec;
pub(crate) use crate::output::text::format_member_list;
pub(crate) use crate::output::text::format_member_press;
pub(crate) use crate::output::text::format_member_prompt;
pub(crate) use crate::output::text::format_message;
pub(crate) use crate::output::text::format_message_acknowledged;
pub(crate) use crate::output::text::format_message_sent;
pub(crate) use crate::output::text::format_message_wait;
pub(crate) use crate::output::text::format_messages;
pub(crate) use crate::output::text::format_permission_inspection;
pub(crate) use crate::output::text::format_permission_response_result;
pub(crate) use crate::output::text::format_runtime_event;
pub(crate) use crate::output::text::format_runtime_recovery;
pub(crate) use crate::output::text::format_runtime_snapshot;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

pub(crate) mod json;
mod text;

static ANSI_ESCAPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("the CSI pattern is valid"));

pub(crate) fn is_visually_blank(line: &str) -> bool {
    ANSI_ESCAPE_RE.replace_all(line, "").trim().is_empty()
}

pub(crate) fn strip_ansi(text: &str) -> String {
    let cleaned = ANSI_ESCAPE_RE.replace_all(text, "");
    cleaned
        .split('\n')
        .map(|line| {
            line.trim_end_matches('\r')
                .rsplit('\r')
                .next()
                .expect("rsplit yields a segment")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn format_json(data: &Value) -> String {
    serde_json::to_string(data).expect("a JSON value always serializes")
}

pub(crate) fn truncate_text(value: &str, limit: usize) -> String {
    match value.char_indices().nth(limit) {
        Some((byte, _)) => format!("{}…", &value[..byte]),
        None => value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use crate::output::*;

    #[test]
    fn capture_cleanup_removes_csi_and_keeps_the_final_carriage_return_segment() {
        for (raw, expected) in [
            ("", ""),
            ("plain\ntext", "plain\ntext"),
            ("\x1b[31mred\x1b[0m", "red"),
            ("\x1b[38;5;196mX\x1b[0m", "X"),
            ("\x1b[2J\x1b[Hclear", "clear"),
            ("one\rONE\ntwo\rTWO", "ONE\nTWO"),
            ("a\rb\rc", "c"),
            ("\x1b[2Kspinner\rdone", "done"),
            ("\x1b]0;title\x07 done", "\x1b]0;title\x07 done"),
            ("one\r\ntwo\r\nthree", "one\ntwo\nthree"),
            ("row1\r\nrow2\r", "row1\nrow2"),
            ("row\r", "row"),
            ("row\r\r\n", "row\n"),
            ("spin\rDONE\r\nnext", "DONE\nnext"),
            ("kept\x1b[0m\r\n", "kept\n"),
        ] {
            assert_eq!(strip_ansi(raw), expected);
        }
        assert!(is_visually_blank("\x1b[31m \t\x1b[0m"));
        assert!(!is_visually_blank("\x1b[31mX\x1b[0m"));
    }

    #[test]
    fn text_limit_counts_unicode_codepoints_and_preserves_exact_limits() {
        for (text, limit, expected) in [
            ("", 0, ""),
            ("a", 0, "…"),
            ("abc", 3, "abc"),
            ("界🙂abc", 2, "界🙂…"),
            ("hello", 4, "hell…"),
            ("a", 20, "a"),
        ] {
            assert_eq!(truncate_text(text, limit), expected);
        }
    }
}
