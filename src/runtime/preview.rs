use crate::broker::MessageRecord;
use crate::coding_agent::catalog::LiteralFraming;
use crate::coding_agent::geometry::Headroom;
use crate::coding_agent::geometry::LiveComposer;

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

/// The compact preview holding the fold of the body's first `prefix` stored scalars.
pub(crate) fn compact_preview_line(message: &MessageRecord, prefix: usize) -> String {
    let excerpt = message.text.chars().take(prefix).collect::<String>();
    let more = message.text.chars().count() - prefix;
    format!(
        "[cafleet msg {} from {} {}] {}… ({more} more characters; run {}) {RESUME_CLAUSE}",
        message.message_id,
        message.from_member_id,
        message.created_at,
        fold_visible(&excerpt),
        crate::cli::poll_command(message.to_member_id, None)
    )
}

pub(crate) fn cue_preview_line(message: &MessageRecord) -> String {
    format!(
        "[cafleet msg {}] run {}",
        message.message_id,
        crate::cli::poll_command(message.to_member_id, None)
    )
}

pub(crate) fn minimal_preview_line(message: &MessageRecord) -> String {
    format!("[cafleet msg {}]", message.message_id)
}

/// A preview form, longest first; selection takes the first that fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rung {
    Full,
    Compact,
    Pointer,
    Cue,
    Minimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RungSelection {
    Selected {
        rung: Rung,
        literal: String,
    },
    /// The composer verifies empty but no rung fits it.
    ComposerTooSmall,
    /// The screen selects no single `composer.text` rule, so no geometry describes it.
    ComposerGeometryUnavailable,
}

/// The first rung of the ladder whose literal fits `live` and that `allowed` admits. A message
/// typed before (`typings` above 0) takes only the pointer, cue and minimal rungs.
pub(crate) fn select_rung(
    message: &MessageRecord,
    typings: u8,
    live: Option<&LiveComposer>,
    framing: LiteralFraming,
    allowed: &dyn Fn(&str) -> bool,
) -> RungSelection {
    let Some(live) = live else {
        return RungSelection::ComposerGeometryUnavailable;
    };
    let fits = |literal: &str, headroom: Headroom| {
        allowed(literal) && live.fits(literal, framing, headroom)
    };
    let selected = |rung, literal| RungSelection::Selected { rung, literal };
    if typings == 0 {
        let full = canonical_preview_line(message);
        if fits(&full, Headroom::Transient) {
            return selected(Rung::Full, full);
        }
        if fold_visible(&message.text).chars().count() <= INLINE_BODY_LIMIT
            && let Some(literal) =
                longest_compact(message, |literal| fits(literal, Headroom::Transient))
        {
            return selected(Rung::Compact, literal);
        }
    }
    [
        (Rung::Pointer, pointer_preview_line(message)),
        (Rung::Cue, cue_preview_line(message)),
        (Rung::Minimal, minimal_preview_line(message)),
    ]
    .into_iter()
    .find(|(_, literal)| fits(literal, Headroom::Zero))
    .map_or(RungSelection::ComposerTooSmall, |(rung, literal)| {
        selected(rung, literal)
    })
}

/// The compact literal with the longest stored prefix of at least one scalar that `fits`,
/// cut outside every CRLF so each fold unit lies wholly inside or outside the excerpt.
fn longest_compact(message: &MessageRecord, fits: impl Fn(&str) -> bool) -> Option<String> {
    let scalars = message.text.chars().collect::<Vec<_>>();
    (1..=scalars.len())
        .rev()
        .filter(|prefix| !(scalars[prefix - 1] == '\r' && scalars.get(*prefix) == Some(&'\n')))
        .map(|prefix| compact_preview_line(message, prefix))
        .find(|literal| fits(literal))
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

    #[test]
    fn compact_selection_keeps_the_longest_prefix_across_a_word_wrap_gap() {
        use crate::coding_agent::CodingAgent;
        use crate::coding_agent::catalog::ProfileCatalog;

        let geometry = ProfileCatalog::bundled()
            .composer_geometry(CodingAgent::Codex, "codex-composer-text")
            .unwrap();
        let message = MessageRecord::fixture(1, 2, 3, &"a ".repeat(120));
        let fits =
            |literal: &str| geometry.fit_rows(literal, 120, LiteralFraming::Plain) == Some(2);
        assert!(fits(&compact_preview_line(&message, 60)));
        assert!(!fits(&compact_preview_line(&message, 61)));
        assert_eq!(
            longest_compact(&message, fits),
            Some(compact_preview_line(&message, 62))
        );
    }

    #[test]
    fn claude_compact_fit_leaves_room_at_the_measured_wrap_boundary() {
        use crate::coding_agent::CodingAgent;
        use crate::coding_agent::catalog::ProfileCatalog;
        use crate::coding_agent::detector::Detector;
        use crate::runtime::TerminalSize;
        use crate::runtime::TerminalState;

        let detector = Detector::new(ProfileCatalog::bundled(), CodingAgent::Claude, None);
        let mut terminal = TerminalState::new(TerminalSize {
            rows: 12,
            cols: 120,
        })
        .unwrap();
        terminal
            .feed("\x1b[?1049h\x1b[8;1H───\r\n❯\u{a0}\r\n───\x1b[9;3H".as_bytes())
            .unwrap();
        let live = detector
            .composer_geometry(&terminal.snapshot())
            .unwrap()
            .unwrap();
        let body = (1..=86)
            .map(|word| format!("w{word:03} "))
            .collect::<String>();
        let mut message = MessageRecord::fixture(13, 1, 2, &body);
        message.created_at = "2026-11-28T07:07:00.972130+00:00".into();
        let framing = LiteralFraming::Plain;
        assert!(!live.fits(
            &compact_preview_line(&message, 175),
            framing,
            Headroom::Transient
        ));
        assert!(matches!(
            select_rung(&message, 0, Some(&live), framing, &|_| true),
            RungSelection::Selected { rung: Rung::Compact, literal }
                if live.fits(&literal, framing, Headroom::Transient)
        ));
    }

    #[test]
    fn codex_full_preview_fits_at_its_native_composer_height_limit() {
        use crate::coding_agent::CodingAgent;
        use crate::coding_agent::catalog::ProfileCatalog;
        use crate::coding_agent::detector::Detector;
        use crate::runtime::{TerminalSize, TerminalState};

        let detector = Detector::new(ProfileCatalog::bundled(), CodingAgent::Codex, None);
        let body = (1..=181)
            .map(|word| format!("w{word:03} "))
            .collect::<String>();
        let message = MessageRecord::fixture(123, 456, 789, &body);
        for (rows, expected) in [(21, Rung::Compact), (23, Rung::Compact), (24, Rung::Full)] {
            let mut terminal = TerminalState::new(TerminalSize { rows, cols: 80 }).unwrap();
            let prompt = rows - 2;
            terminal
                .feed(
                    format!(
                        "\x1b[?1049h\x1b[{prompt};1H› Ask Codex to do anything\x1b[{prompt};3H"
                    )
                    .as_bytes(),
                )
                .unwrap();
            let live = detector
                .composer_geometry(&terminal.snapshot())
                .unwrap()
                .unwrap();
            let selected = select_rung(&message, 0, Some(&live), LiteralFraming::Plain, &|_| true);
            assert!(
                matches!(selected, RungSelection::Selected { rung, .. } if rung == expected),
                "{rows} rows"
            );
        }
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

    /// The §4 compact form of the record from 15 to 16 holding `excerpt` with `more` stored
    /// characters left out.
    fn compact(excerpt: &str, more: usize) -> String {
        format!(
            "[cafleet msg 57 from 15 {STAMP}] {excerpt}… ({more} more characters; run cafleet message poll 16 --json) [cafleet] Resume your work if something was still running."
        )
    }

    mod ladder_lines {
        use crate::runtime::preview::tests::*;

        #[test]
        fn the_cue_and_minimal_lines_name_the_message_and_the_recipient_poll() {
            let message = record("a body the short rungs leave out");
            assert_eq!(
                cue_preview_line(&message),
                "[cafleet msg 57] run cafleet message poll 16 --json"
            );
            assert_eq!(minimal_preview_line(&message), "[cafleet msg 57]");
        }

        #[test]
        fn the_compact_line_folds_a_stored_prefix_and_counts_the_stored_rest() {
            for (name, body, prefix, excerpt, more) in [
                ("an ASCII prefix", "abcdef", 3, "abc", 3),
                ("a cut after a stored CRLF", "ab\r\ncd", 4, "ab⏎", 2),
                ("a cut before a stored CRLF", "ab\r\ncd", 2, "ab", 4),
                (
                    "a cut after a control character",
                    "ab\x01cd",
                    3,
                    "ab\\x01",
                    2,
                ),
                ("a cut before a control character", "ab\x01cd", 2, "ab", 3),
                ("a cut after a tab", "a\tb", 2, "a␉", 1),
                ("multibyte scalars count once", "ééé", 1, "é", 2),
            ] {
                assert_eq!(
                    compact_preview_line(&record(body), prefix),
                    compact(excerpt, more),
                    "{name}"
                );
            }
        }
    }

    mod rung_selection {
        use crate::coding_agent::catalog::LiteralFraming;
        use crate::coding_agent::geometry::LiveComposer;
        use crate::coding_agent::geometry::test_support::live_composer;
        use crate::runtime::preview::tests::*;
        use serde_json::json;

        const RESERVED: usize = 1;
        const TRANSIENT: usize = 2;

        /// Full-width char-wrapped text rows above one reserved row, with two transient rows of
        /// headroom for the full and compact rungs.
        fn composer(rows: u16, cols: u16) -> LiveComposer {
            live_composer(
                rows,
                cols,
                json!({
                    "margin_columns":0,"first_row_margin_columns":0,"text_window":null,
                    "reserved_rows":RESERVED,"transient_rows":TRANSIENT,"wrap_slack_rows":0,"carried_space":false,"fit_wrap":"char",
                    "collapsed_paste":null,"min_rows":2,"min_columns":10
                }),
                "char",
            )
        }

        /// The smallest pane height at which the ASCII `literal` fits `cols` columns with
        /// `headroom` rows.
        fn rows_for(literal: &str, cols: u16, headroom: usize) -> u16 {
            let text = literal.chars().count().div_ceil(usize::from(cols));
            u16::try_from(text + RESERVED + headroom).unwrap()
        }

        fn select_limited(
            message: &MessageRecord,
            typings: u8,
            rows: u16,
            cols: u16,
            allowed: &dyn Fn(&str) -> bool,
        ) -> RungSelection {
            select_rung(
                message,
                typings,
                Some(&composer(rows, cols)),
                LiteralFraming::Plain,
                allowed,
            )
        }

        fn select(message: &MessageRecord, typings: u8, rows: u16, cols: u16) -> RungSelection {
            select_limited(message, typings, rows, cols, &|_| true)
        }

        fn selected(selection: RungSelection, name: &str) -> (Rung, String) {
            match selection {
                RungSelection::Selected { rung, literal } => (rung, literal),
                other => panic!("{name}: a rung is selected, not {other:?}"),
            }
        }

        fn words(count: usize) -> String {
            (1..=count)
                .map(|n| format!("w{n:03}"))
                .collect::<Vec<_>>()
                .join(" ")
        }

        /// The §4 compact literal with the longest stored ASCII prefix of `body` whose literal
        /// holds at most `capacity` scalars.
        fn longest_compact(body: &str, capacity: usize) -> String {
            let stored = body.chars().count();
            (1..=stored)
                .rev()
                .map(|prefix| compact(&body[..prefix], stored - prefix))
                .find(|literal| literal.chars().count() <= capacity)
                .expect("a one-scalar excerpt fits")
        }

        #[test]
        fn the_full_rung_is_chosen_from_its_boundary_height_with_headroom() {
            let message = record(&words(40));
            let full = canonical_preview_line(&message);
            for cols in [80, 120] {
                let rows = rows_for(&full, cols, TRANSIENT);
                assert_eq!(
                    selected(select(&message, 0, rows, cols), "the boundary"),
                    (Rung::Full, full.clone()),
                    "{cols} columns at the full boundary"
                );
                let (rung, _) = selected(select(&message, 0, rows - 1, cols), "one row less");
                assert_eq!(
                    rung,
                    Rung::Compact,
                    "{cols} columns one row below the boundary"
                );
            }
        }

        #[test]
        fn the_compact_rung_takes_the_longest_excerpt_that_fits_with_headroom() {
            let body = words(100);
            let message = record(&body);
            let cols = 80;
            let rows = 7;
            assert!(
                rows < rows_for(&canonical_preview_line(&message), cols, TRANSIENT),
                "the full rung does not fit"
            );
            let capacity = (usize::from(rows) - RESERVED - TRANSIENT) * usize::from(cols);
            assert_eq!(
                selected(select(&message, 0, rows, cols), "a 7-row pane"),
                (Rung::Compact, longest_compact(&body, capacity)),
                "the excerpt fills four 80-column rows"
            );
            let shortest = compact(&body[..1], body.len() - 1);
            let boundary = rows_for(&shortest, cols, TRANSIENT);
            let (rung, _) = selected(select(&message, 0, boundary, cols), "the compact boundary");
            assert_eq!(rung, Rung::Compact, "a one-scalar excerpt at its boundary");
            assert!(
                boundary > rows_for(&pointer_preview_line(&message), cols, 0),
                "the pointer fits one row below the compact boundary"
            );
            assert_eq!(
                selected(select(&message, 0, boundary - 1, cols), "below compact"),
                (Rung::Pointer, pointer_preview_line(&message)),
                "one row below the compact boundary"
            );
        }

        #[test]
        fn the_pointer_cue_and_minimal_rungs_fit_without_headroom() {
            let message = record(&words(100));
            let pointer = pointer_preview_line(&message);
            let rows = rows_for(&pointer, 80, 0);
            let shortest = compact("w", words(100).len() - 1);
            assert!(
                rows < rows_for(&shortest, 80, TRANSIENT),
                "no compact excerpt fits at the pointer boundary"
            );
            assert_eq!(
                selected(select(&message, 0, rows, 80), "the pointer boundary"),
                (Rung::Pointer, pointer),
                "the pointer at its boundary"
            );
            let cue = cue_preview_line(&message);
            let minimal = minimal_preview_line(&message);
            for (name, rows, cols, rung, literal) in [
                (
                    "one row below the pointer boundary",
                    rows - 1,
                    80,
                    Rung::Cue,
                    cue.clone(),
                ),
                (
                    "the cue boundary at 20 columns",
                    rows_for(&cue, 20, 0),
                    20,
                    Rung::Cue,
                    cue.clone(),
                ),
                (
                    "one row below the cue boundary",
                    rows_for(&cue, 20, 0) - 1,
                    20,
                    Rung::Minimal,
                    minimal.clone(),
                ),
                (
                    "the minimal boundary at 10 columns",
                    rows_for(&minimal, 10, 0),
                    10,
                    Rung::Minimal,
                    minimal.clone(),
                ),
            ] {
                assert_eq!(
                    selected(select(&message, 0, rows, cols), name),
                    (rung, literal),
                    "{name}"
                );
            }
            assert_eq!(
                select(&message, 0, rows_for(&minimal, 10, 0) - 1, 10),
                RungSelection::ComposerTooSmall,
                "no rung fits one row below the minimal boundary"
            );
        }

        #[test]
        fn a_second_typing_skips_the_full_and_compact_rungs() {
            let message = record(&words(40));
            let rows = rows_for(&canonical_preview_line(&message), 80, TRANSIENT) + 4;
            let (rung, _) = selected(select(&message, 0, rows, 80), "the first typing");
            assert_eq!(rung, Rung::Full, "the first typing");
            assert_eq!(
                selected(select(&message, 1, rows, 80), "the second typing"),
                (Rung::Pointer, pointer_preview_line(&message)),
                "the second typing"
            );
            assert_eq!(
                selected(select(&message, 1, 2, 80), "a one-row pane"),
                (Rung::Cue, cue_preview_line(&message)),
                "the second typing in a one-text-row pane"
            );
        }

        #[test]
        fn a_body_folding_past_the_inline_limit_has_no_compact_rung() {
            for (name, body) in [
                ("4001 stored scalars", "é".repeat(4001)),
                (
                    "1001 controls folding to 4004 scalars",
                    "\u{1}".repeat(1001),
                ),
            ] {
                let message = record(&body);
                let full = canonical_preview_line(&message);
                assert_eq!(
                    full,
                    pointer_preview_line(&message),
                    "{name}: the full rung carries the body pointer"
                );
                let boundary = rows_for(&full, 80, TRANSIENT);
                assert_eq!(
                    selected(select(&message, 0, boundary, 80), name),
                    (Rung::Full, full.clone()),
                    "{name}: at the full boundary"
                );
                assert_eq!(
                    selected(select(&message, 0, boundary - 1, 80), name),
                    (Rung::Pointer, full),
                    "{name}: below the full boundary the pointer fits without headroom"
                );
            }
        }

        #[test]
        fn the_plain_limit_bounds_every_rung_it_allows() {
            let body = words(100);
            let message = record(&body);
            let limit = |max: usize| move |literal: &str| literal.chars().count() <= max;
            assert_eq!(
                selected(
                    select_limited(&message, 0, 60, 120, &limit(300)),
                    "a 300-scalar limit"
                ),
                (Rung::Compact, longest_compact(&body, 300)),
                "a 300-scalar limit"
            );
            assert_eq!(
                selected(
                    select_limited(&message, 0, 60, 120, &limit(60)),
                    "a 60-scalar limit"
                ),
                (Rung::Cue, cue_preview_line(&message)),
                "a 60-scalar limit"
            );
            assert_eq!(
                selected(
                    select_limited(&message, 0, 60, 120, &limit(20)),
                    "a 20-scalar limit"
                ),
                (Rung::Minimal, minimal_preview_line(&message)),
                "a 20-scalar limit"
            );
        }

        #[test]
        fn the_excerpt_cut_keeps_each_fold_unit_whole_and_counts_stored_scalars() {
            for (name, unit, folded) in [("a CRLF", "\r\n", "⏎"), ("a control", "\x01", "\\x01")]
            {
                let body = format!("{}{unit}{}", "x".repeat(100), "y".repeat(300));
                let stored = body.chars().count();
                let message = record(&body);
                let inside = compact(
                    &format!("{}{folded}", "x".repeat(100)),
                    stored - 100 - unit.chars().count(),
                );
                let outside = compact(&"x".repeat(100), stored - 100);
                let fits = inside.chars().count();
                for (side, max, expected) in [
                    ("inside", fits, inside.clone()),
                    ("outside", fits - 1, outside),
                ] {
                    let allowed = move |literal: &str| literal.chars().count() <= max;
                    assert_eq!(
                        selected(
                            select_limited(&message, 0, 60, 200, &allowed),
                            &format!("{name} {side}")
                        ),
                        (Rung::Compact, expected),
                        "{name} {side} the excerpt"
                    );
                }
            }
        }

        #[test]
        fn selection_without_live_geometry_is_unavailable() {
            assert_eq!(
                select_rung(&record("body"), 0, None, LiteralFraming::Plain, &|_| true),
                RungSelection::ComposerGeometryUnavailable
            );
        }
    }
}
