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
