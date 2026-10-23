use cafleet::output::strip_ansi;
use cafleet::pane_state::classify;

struct ComposerFixture {
    agent: &'static str,
    placeholder: &'static str,
    draft: &'static str,
    working: &'static str,
    approval: &'static str,
}

const COMPOSERS: &[ComposerFixture] = &[
    ComposerFixture {
        agent: "claude",
        placeholder: include_str!("fixtures/pane-readiness/claude-placeholder.ansi"),
        draft: include_str!("fixtures/pane-readiness/claude-draft.ansi"),
        working: "✻ Cogitating… (12s · esc to interrupt)",
        approval: " ❯ 1. Yes, proceed",
    },
    ComposerFixture {
        agent: "codex",
        placeholder: include_str!("fixtures/pane-readiness/codex-placeholder.ansi"),
        draft: include_str!("fixtures/pane-readiness/codex-draft.ansi"),
        working: "• Working (8s • esc to interrupt)",
        approval: "› 1. Yes, proceed",
    },
    ComposerFixture {
        agent: "opencode",
        placeholder: include_str!("fixtures/pane-readiness/opencode-placeholder.ansi"),
        draft: include_str!("fixtures/pane-readiness/opencode-draft.ansi"),
        working: "⬝⬝⬝⬝⬝⬝  esc interrupt",
        approval: "┃  △ Permission required",
    },
];

const CAPTURED_CODEX_HINT: &str = "Ask Codex to do anything";
const CAPTURED_CODEX_PLACEHOLDER: &str =
    include_str!("fixtures/pane-readiness/codex-herdr-placeholder.ansi");
const CAPTURED_CODEX_DRAFT: &str = include_str!("fixtures/pane-readiness/codex-herdr-draft.ansi");

fn captured_codex_composer_text(capture: &str) -> String {
    strip_ansi(capture)
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('›'))
        .expect("captured Codex fixture has a composer")
        .trim()
        .to_string()
}

fn assert_state(agent: &str, content: &str, expected: &str) {
    assert_eq!(
        format!("{:?}", classify(agent, content)),
        expected,
        "{agent} capture: {content:?}"
    );
}

fn hint(fixture: &ComposerFixture) -> String {
    format!("Investigate a new task {}", fixture.agent)
}

#[test]
fn affirmative_empty_composers_are_idle() {
    for fixture in COMPOSERS {
        let empty = fixture.placeholder.replace(&hint(fixture), "");
        assert_state(fixture.agent, &empty, "Idle");
    }
}

#[test]
fn styled_placeholder_and_identical_user_input_have_distinct_readiness() {
    for fixture in COMPOSERS {
        assert_eq!(strip_ansi(fixture.placeholder), strip_ansi(fixture.draft));
        assert!(fixture.placeholder.contains('\u{1b}'));
        assert_state(fixture.agent, fixture.placeholder, "Idle");
        assert_state(fixture.agent, fixture.draft, "Draft");
    }
}

#[test]
fn decoration_recognition_handles_rotating_hint_text() {
    for fixture in COMPOSERS {
        for replacement in [
            "Review the invoices for next week",
            "Try the new deployment workflow",
            "調査を続ける",
            "A hint that is absent from every built-in text list",
        ] {
            let content = fixture.placeholder.replace(&hint(fixture), replacement);
            assert_state(fixture.agent, &content, "Idle");
        }
    }
}

#[test]
fn user_drafts_keep_their_meaning_when_their_text_resembles_a_placeholder() {
    for fixture in COMPOSERS {
        for text in [
            "Ask anything...",
            "Ask Codex to do anything",
            "! printf example",
            "/help",
        ] {
            let content = fixture.draft.replace(&hint(fixture), text);
            assert_state(fixture.agent, &content, "Draft");
        }
    }
}

#[test]
fn working_evidence_outranks_an_empty_or_placeholder_composer() {
    for fixture in COMPOSERS {
        for composer in [
            fixture.placeholder.to_string(),
            fixture.placeholder.replace(&hint(fixture), ""),
        ] {
            assert_state(
                fixture.agent,
                &format!("{}\n{composer}", fixture.working),
                "Working",
            );
        }
    }
}

#[test]
fn approval_evidence_outranks_idle_and_active_work() {
    for fixture in COMPOSERS {
        let content = format!(
            "{}\n{}\n{}",
            fixture.working, fixture.approval, fixture.placeholder
        );
        assert_state(fixture.agent, &content, "AwaitingUser");
    }
}

#[test]
fn nonempty_unstyled_composers_have_ambiguous_input_roles() {
    for fixture in COMPOSERS {
        assert_state(fixture.agent, &strip_ansi(fixture.placeholder), "Unknown");
    }
}

#[test]
fn grey_text_outside_a_composer_does_not_establish_idle() {
    for fixture in COMPOSERS {
        assert_state(
            fixture.agent,
            "\x1b[2;38;5;244mTry another task when ready\x1b[0m\nBuild completed.",
            "Unknown",
        );
    }
}

#[test]
fn missing_and_truncated_composers_are_unknown() {
    for fixture in COMPOSERS {
        for capture in [
            "",
            "\n   \n",
            "assistant output only",
            "\x1b[2;38;5;244m",
            "─".repeat(40).as_str(),
        ] {
            assert_state(fixture.agent, capture, "Unknown");
        }
    }
}

#[test]
fn ambiguous_sgr_roles_are_unknown() {
    for fixture in COMPOSERS {
        for style in ["\x1b[31m", "\x1b[7m", "\x1b[999m"] {
            let content = fixture.placeholder.replace("\x1b[2;38;5;244m", style);
            assert_state(fixture.agent, &content, "Unknown");
        }
    }
}

#[test]
fn transcript_quotes_do_not_become_a_live_composer() {
    for fixture in COMPOSERS {
        let transcript = format!(
            "Archived terminal output:\n```text\n{}\n```\nMore output follows.",
            fixture.placeholder
        );
        assert_state(fixture.agent, &transcript, "Unknown");
    }
}

#[test]
fn an_unknown_backend_has_unknown_readiness() {
    for fixture in COMPOSERS {
        for backend in ["", "unrecognized-agent"] {
            assert_state(backend, fixture.placeholder, "Unknown");
        }
    }
}

#[test]
fn captured_codex_dim_placeholder_is_idle_for_the_real_identical_text_pair() {
    let placeholder = strip_ansi(CAPTURED_CODEX_PLACEHOLDER);
    let draft = strip_ansi(CAPTURED_CODEX_DRAFT);
    assert_eq!(
        captured_codex_composer_text(CAPTURED_CODEX_PLACEHOLDER),
        captured_codex_composer_text(CAPTURED_CODEX_DRAFT)
    );
    assert_eq!(
        captured_codex_composer_text(CAPTURED_CODEX_PLACEHOLDER),
        "› Ask Codex to do anything"
    );
    assert!(
        CAPTURED_CODEX_PLACEHOLDER
            .contains("\u{1b}[2m\u{1b}[48;2;53;53;53mAsk Codex to do anything")
    );
    assert!(placeholder.contains("Context 10% used"));
    assert!(placeholder.contains("? for shortcuts"));
    assert!(!draft.contains("? for shortcuts"));
    assert_ne!(placeholder, draft);
    assert_state("codex", CAPTURED_CODEX_PLACEHOLDER, "Idle");
}

#[test]
fn captured_codex_identical_text_without_dim_is_a_user_draft() {
    assert!(
        CAPTURED_CODEX_DRAFT.contains("\u{1b}[0m\u{1b}[48;2;53;53;53m Ask Codex to do anything")
    );
    assert_state("codex", CAPTURED_CODEX_DRAFT, "Draft");
}

#[test]
fn captured_codex_shortcuts_footer_is_optional_for_both_composer_roles() {
    let shortcut = CAPTURED_CODEX_PLACEHOLDER
        .lines()
        .find(|line| strip_ansi(line).contains("? for shortcuts"))
        .unwrap();
    let without_shortcuts = CAPTURED_CODEX_PLACEHOLDER
        .lines()
        .filter(|line| !strip_ansi(line).contains("? for shortcuts"))
        .collect::<Vec<_>>()
        .join("\n");
    let with_shortcuts = format!("{CAPTURED_CODEX_DRAFT}{shortcut}\n");
    assert_state("codex", &without_shortcuts, "Idle");
    assert_state("codex", &with_shortcuts, "Draft");
}

#[test]
fn captured_codex_roles_follow_style_across_hint_and_context_changes() {
    for (capture, expected) in [
        (CAPTURED_CODEX_PLACEHOLDER, "Idle"),
        (CAPTURED_CODEX_DRAFT, "Draft"),
    ] {
        for hint in [
            "Review next week's invoices",
            "! printf example",
            "A previously unseen rotating hint",
        ] {
            for context in [0, 100] {
                let content = capture
                    .replace(CAPTURED_CODEX_HINT, hint)
                    .replace("Context 10% used", &format!("Context {context}% used"));
                assert_state("codex", &content, expected);
            }
        }
    }
}

#[test]
fn captured_codex_work_and_approval_outrank_both_composer_roles() {
    for capture in [CAPTURED_CODEX_PLACEHOLDER, CAPTURED_CODEX_DRAFT] {
        let working = format!("• Working (8s • esc to interrupt)\n{capture}");
        assert_state("codex", &working, "Working");
        let approval = format!("› 1. Yes, proceed\n{working}");
        assert_state("codex", &approval, "AwaitingUser");
    }
}

#[test]
fn captured_codex_unstyled_and_ambiguous_composer_roles_are_unknown() {
    for capture in [CAPTURED_CODEX_PLACEHOLDER, CAPTURED_CODEX_DRAFT] {
        assert_state("codex", &strip_ansi(capture), "Unknown");
    }
    for style in ["\u{1b}[31m", "\u{1b}[7m", "\u{1b}[999m"] {
        let capture = CAPTURED_CODEX_PLACEHOLDER.replace(
            "\u{1b}[2m\u{1b}[48;2;53;53;53mAsk Codex",
            &format!("{style}\u{1b}[48;2;53;53;53mAsk Codex"),
        );
        assert_state("codex", &capture, "Unknown");
    }
}

#[test]
fn captured_codex_unknown_rgb_foreground_channels_preserve_ambiguity() {
    for foreground in [
        "38;2;0;0;2",
        "38;2;2;0;0",
        "38;2;0;2;0",
        "38;2;0;0;0",
        "38;2;2;2;2",
    ] {
        let capture = CAPTURED_CODEX_PLACEHOLDER.replace(
            "\u{1b}[2m\u{1b}[48;2;53;53;53mAsk Codex",
            &format!("\u{1b}[{foreground}m\u{1b}[48;2;53;53;53mAsk Codex"),
        );
        assert_state("codex", &capture, "Unknown");
    }
}

#[test]
fn captured_codex_warning_footer_preserves_the_proven_placeholder_role() {
    let capture = include_str!("fixtures/pane-readiness/codex-herdr-placeholder-warning.ansi");
    let visible = strip_ansi(capture);
    assert!(visible.contains("Context 47% used"));
    assert!(visible.contains("? for shortcuts"));
    assert!(visible.contains("1 warning"));
    assert_state("codex", capture, "Idle");
}
