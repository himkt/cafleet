use crate::broker::MessageRecord;

const INLINE_BODY_LIMIT: usize = 4000;
const RESUME_CLAUSE: &str = "[cafleet] Resume your work if something was still running.";
const DISMISSED_PROMPT_NOTE: &str = "[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

fn fold_visible(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                result.push('⏎');
            }
            '\n' => result.push('⏎'),
            '\t' => result.push('␉'),
            '\u{00}'..='\u{1f}' | '\u{7f}' => result.push_str(&format!("\\x{:02x}", ch as u32)),
            _ => result.push(ch),
        }
    }
    result
}

pub(crate) fn fallback_preview_line(canonical: &str) -> String {
    format!("{canonical} {DISMISSED_PROMPT_NOTE}")
}

pub(crate) fn canonical_preview_line(message: &MessageRecord) -> String {
    let folded = fold_visible(&message.text);
    let body = if folded.chars().count() <= INLINE_BODY_LIMIT {
        folded
    } else {
        body_pointer(message)
    };
    format!(
        "[cafleet msg {} from {} {}] {body} {RESUME_CLAUSE}",
        message.message_id, message.from_member_id, message.created_at
    )
}

pub(crate) fn pointer_preview_line(message: &MessageRecord) -> String {
    format!(
        "[cafleet msg {} from {} {}] {} {RESUME_CLAUSE}",
        message.message_id,
        message.from_member_id,
        message.created_at,
        body_pointer(message)
    )
}

fn body_pointer(message: &MessageRecord) -> String {
    format!(
        "Body is {} characters; run {} to read it.",
        message.text.chars().count(),
        crate::cli::poll_command(message.to_member_id, None)
    )
}

#[cfg(test)]
mod tests {
    use crate::broker::MessageRecord;
    use crate::runtime::preview::*;

    const STAMP: &str = MessageRecord::FIXTURE_STAMP;
    const CLAUSES: &str = "[cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

    enum Body {
        Inline(&'static str),
        Repeated(&'static str, usize),
        Pointer(usize),
    }

    fn record(text: &str) -> MessageRecord {
        MessageRecord::fixture(57, 15, 16, text)
    }

    fn pointer(stored: usize) -> String {
        format!("Body is {stored} characters; run cafleet message poll 16 --json to read it.")
    }

    #[test]
    fn the_preview_line_folds_its_body_inline_up_to_the_limit_and_points_at_the_poll_beyond_it() {
        let cases: [(&str, String, Body); 6] = [
            (
                "the specified example",
                "ready (doc)\nsee the reviewer notes".into(),
                Body::Inline("ready (doc)⏎see the reviewer notes"),
            ),
            (
                "line breaks, tab and C0/DEL controls fold visibly",
                "a\r\nb\nc\rd\tx\x1b\x7f".into(),
                Body::Inline("a⏎b⏎c⏎d␉x\\x1b\\x7f"),
            ),
            (
                "4000 multibyte scalars stay inline",
                "é".repeat(4000),
                Body::Repeated("é", 4000),
            ),
            (
                "4001 multibyte scalars become the pointer",
                "é".repeat(4001),
                Body::Pointer(4001),
            ),
            (
                "8000 stored characters fold to 4000 and stay inline",
                "\r\n".repeat(4000),
                Body::Repeated("⏎", 4000),
            ),
            (
                "1001 stored controls fold to 4004 and the pointer counts the stored text",
                "\u{1}".repeat(1001),
                Body::Pointer(1001),
            ),
        ];
        for (name, text, body) in cases {
            let expected = match body {
                Body::Inline(body) => body.to_owned(),
                Body::Repeated(unit, count) => unit.repeat(count),
                Body::Pointer(stored) => pointer(stored),
            };
            let line = fallback_preview_line(&canonical_preview_line(&record(&text)));
            assert_eq!(
                line,
                format!("[cafleet msg 57 from 15 {STAMP}] {expected} {CLAUSES}"),
                "{name}"
            );
            assert!(crate::pane_lock::valid_input(&line), "{name}: {line:?}");
        }
        assert_eq!(
            pointer_preview_line(&record("短い本文")),
            format!(
                "[cafleet msg 57 from 15 {STAMP}] {} [cafleet] Resume your work if something was still running.",
                pointer(4)
            ),
            "the pointer line names the recipient's poll"
        );
    }
}
