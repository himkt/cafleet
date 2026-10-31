use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

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
            let line = line.strip_suffix('\r').unwrap_or(line);
            line.rsplit('\r').next().expect("rsplit yields a segment")
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
    use super::*;

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
            ("text\r\nnext", "text\nnext"),
            ("text\r\n", "text\n"),
            ("old\rnew\r\nnext", "new\nnext"),
            ("first\r\nold\rnew", "first\nnew"),
            ("\r\n", "\n"),
            ("a\r\n\r\nb", "a\n\nb"),
            (
                "\x1b[1m Allow this read?\x1b[0m\r\n ❯ 1. Yes\r\n   2. No\r\n Esc to cancel",
                " Allow this read?\n ❯ 1. Yes\n   2. No\n Esc to cancel",
            ),
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
