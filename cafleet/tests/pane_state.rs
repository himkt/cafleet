use cafleet::output::strip_ansi;
use cafleet::pane_state::{PaneState, classify};

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

const CAPTURED_CLAUDE_HERDR_EMPTY: &str =
    include_str!("fixtures/pane-readiness/claude-herdr-empty.ansi");
const CAPTURED_CLAUDE_TMUX_EMPTY: &str =
    include_str!("fixtures/pane-readiness/claude-tmux-empty.ansi");
const CAPTURED_CLAUDE_EMPTY: &[(&str, &str)] = &[
    (
        CAPTURED_CLAUDE_HERDR_EMPTY,
        "cafleet-evidence-claude-herdr-1105Z",
    ),
    (
        CAPTURED_CLAUDE_TMUX_EMPTY,
        "cafleet-evidence-claude-tmux-1105Z",
    ),
];

const CAPTURED_CLAUDE_HERDR_DRAFT: &str =
    include_str!("fixtures/pane-readiness/claude-herdr-draft.ansi");
const CAPTURED_CLAUDE_TMUX_EMPTY_AGENTS: &str =
    include_str!("fixtures/pane-readiness/claude-tmux-empty-agents.ansi");
const CAPTURED_CLAUDE_DRAFT_TEXT: &str = "cafleet verification draft 34";

const CAPTURED_CODEX_HINT: &str = "Ask Codex to do anything";
const CAPTURED_CODEX_PLACEHOLDER: &str =
    include_str!("fixtures/pane-readiness/codex-herdr-placeholder.ansi");
const CAPTURED_CODEX_DRAFT: &str = include_str!("fixtures/pane-readiness/codex-herdr-draft.ansi");

const CAPTURED_OPENCODE_HINT: &str = "Ask anything… \"Fix broken tests\"";
const CAPTURED_OPENCODE_PLACEHOLDER: &str =
    include_str!("fixtures/pane-readiness/opencode-herdr-placeholder.ansi");
const CAPTURED_OPENCODE_DRAFT: &str =
    include_str!("fixtures/pane-readiness/opencode-herdr-draft.ansi");
const CAPTURED_OPENCODE_HINT_STYLE: &str = "\x1b[38;2;128;128;128m\x1b[48;2;30;30;30m";
const CAPTURED_OPENCODE_INPUT_STYLE: &str = "\x1b[38;2;238;238;238m\x1b[48;2;30;30;30m";
const CAPTURED_OPENCODE_TMUX_PLACEHOLDER: &str =
    include_str!("fixtures/pane-readiness/opencode-tmux-placeholder.ansi");
const CAPTURED_OPENCODE_TMUX_DRAFT: &str =
    include_str!("fixtures/pane-readiness/opencode-tmux-draft.ansi");
const CAPTURED_OPENCODE_TMUX_HINT_STYLE: &str = "\x1b[38;2;128;128;128m";
const CAPTURED_OPENCODE_TMUX_INPUT_STYLE: &str = "\x1b[38;2;238;238;238m";

fn captured_claude_composer_index(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|line| strip_ansi(line).trim_start().starts_with('❯'))
        .expect("actual empty Claude frame has its live composer row")
}

fn captured_opencode_composer_text(capture: &str) -> String {
    strip_ansi(capture)
        .lines()
        .filter(|line| line.trim_start().starts_with('┃'))
        .nth(1)
        .expect("captured OpenCode fixture has its input row after the empty top row")
        .trim()
        .to_string()
}

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
fn captured_claude_herdr_empty_named_composer_is_idle_with_the_actual_agents_footer() {
    let visible = strip_ansi(CAPTURED_CLAUDE_HERDR_EMPTY);
    assert!(visible.contains("Claude Code v2.1.301"));
    assert!(visible.contains("cafleet-evidence-claude-herdr-1105Z"));
    assert!(visible.contains("⏵⏵ don't ask on (shift+tab to cycle) · ← for agents"));
    assert!(!visible.contains("? for shortcuts"));
    let lines: Vec<_> = visible.lines().map(str::to_owned).collect();
    let composer = captured_claude_composer_index(&lines);
    assert_eq!(lines[composer].trim(), "❯");
    assert!(lines[composer - 1].starts_with('─'));
    assert!(lines[composer - 1].ends_with("cafleet-evidence-claude-herdr-1105Z ─"));
    assert!(
        lines[composer + 1]
            .chars()
            .all(|character| character == '─')
    );
    assert_eq!(
        classify("claude", CAPTURED_CLAUDE_HERDR_EMPTY),
        PaneState::Idle
    );
}

#[test]
fn captured_claude_tmux_empty_named_composer_is_idle_with_the_actual_agents_and_effort_footer() {
    let visible = strip_ansi(CAPTURED_CLAUDE_TMUX_EMPTY);
    assert!(visible.contains("Claude Code v2.1.301"));
    assert!(visible.contains("cafleet-evidence-claude-tmux-1105Z"));
    assert!(visible.contains("⏵⏵ don't ask on (shift+tab to cycle) · ← for agents"));
    assert!(visible.contains("◐ medium · /effort"));
    assert!(!visible.contains("? for shortcuts"));
    let lines: Vec<_> = visible.lines().map(str::to_owned).collect();
    let composer = captured_claude_composer_index(&lines);
    assert_eq!(lines[composer].trim(), "❯");
    assert!(lines[composer - 1].ends_with("cafleet-evidence-claude-tmux-1105Z ─"));
    assert!(
        lines[composer + 1]
            .chars()
            .all(|character| character == '─')
    );
    assert!(CAPTURED_CLAUDE_TMUX_EMPTY.contains("❯\u{a0}\x1b[7m \x1b[0m"));
    assert_eq!(
        classify("claude", CAPTURED_CLAUDE_TMUX_EMPTY),
        PaneState::Idle
    );
}

#[test]
fn captured_claude_derived_named_borders_accept_changed_names_with_the_shortcuts_footer() {
    for &(capture, title) in CAPTURED_CLAUDE_EMPTY {
        let mut lines: Vec<_> = capture.lines().map(str::to_owned).collect();
        let composer = captured_claude_composer_index(&lines);
        assert_eq!(lines[composer - 1].matches(title).count(), 1);
        lines[composer - 1] = lines[composer - 1].replace(title, "independent-review-session-42");
        lines[composer + 2] = "? for shortcuts".to_string();
        assert_eq!(classify("claude", &lines.join("\n")), PaneState::Idle);
    }
}

#[test]
fn captured_claude_derived_actual_status_footers_accept_a_plain_upper_border() {
    for &(capture, _) in CAPTURED_CLAUDE_EMPTY {
        let mut lines: Vec<_> = capture.lines().map(str::to_owned).collect();
        let composer = captured_claude_composer_index(&lines);
        assert!(strip_ansi(&lines[composer + 2]).contains("don't ask on"));
        lines[composer - 1] = "────────────────────────────".to_string();
        assert_eq!(classify("claude", &lines.join("\n")), PaneState::Idle);
    }
}

#[test]
fn captured_claude_derived_missing_borders_history_and_ambiguous_input_remain_unknown() {
    for &(capture, _) in CAPTURED_CLAUDE_EMPTY {
        let lines: Vec<_> = capture.lines().map(str::to_owned).collect();
        let composer = captured_claude_composer_index(&lines);
        let mut no_upper_border = lines.clone();
        no_upper_border.remove(composer - 1);
        let mut no_lower_border = lines.clone();
        no_lower_border.remove(composer + 1);
        let mut ambiguous_input = lines.clone();
        ambiguous_input[composer] = "❯ \x1b[38;2;255;0;0mretained user input\x1b[0m".to_string();
        let mut unstyled_input: Vec<_> = strip_ansi(capture).lines().map(str::to_owned).collect();
        unstyled_input[composer] = "❯ retained user input".to_string();
        for content in [
            no_upper_border.join("\n"),
            no_lower_border.join("\n"),
            ambiguous_input.join("\n"),
            unstyled_input.join("\n"),
            lines[composer + 2].clone(),
            format!("Archived terminal output:\n```text\n{capture}\n```\nMore output follows."),
        ] {
            assert_eq!(classify("claude", &content), PaneState::Unknown);
        }
    }
}

#[test]
fn captured_claude_derived_work_and_approval_outrank_both_actual_empty_composers() {
    let fixture = COMPOSERS
        .iter()
        .find(|fixture| fixture.agent == "claude")
        .unwrap();
    for &(capture, _) in CAPTURED_CLAUDE_EMPTY {
        let working = format!("{capture}\n{}", fixture.working);
        assert_eq!(classify("claude", &working), PaneState::Working);
        let approval = format!("{working}\n{}", fixture.approval);
        assert_eq!(classify("claude", &approval), PaneState::AwaitingUser);
    }
}

#[test]
fn captured_claude_herdr_actual_typed_input_is_a_draft_with_the_observed_shorter_footer() {
    let before: Vec<_> = CAPTURED_CLAUDE_HERDR_EMPTY
        .lines()
        .map(str::to_owned)
        .collect();
    let after: Vec<_> = CAPTURED_CLAUDE_HERDR_DRAFT
        .lines()
        .map(str::to_owned)
        .collect();
    let composer = captured_claude_composer_index(&after);
    assert_eq!(captured_claude_composer_index(&before), composer);
    assert_eq!(before.len(), after.len());
    assert_eq!(before[..composer], after[..composer]);
    assert_eq!(before[composer + 1], after[composer + 1]);
    assert_eq!(strip_ansi(&before[composer]).trim(), "❯");
    assert_eq!(
        strip_ansi(&after[composer]).trim(),
        format!("❯\u{a0}{CAPTURED_CLAUDE_DRAFT_TEXT}")
    );
    assert_eq!(
        CAPTURED_CLAUDE_HERDR_DRAFT
            .matches(CAPTURED_CLAUDE_DRAFT_TEXT)
            .count(),
        1
    );
    assert_eq!(
        after[composer],
        format!("❯\u{a0}{CAPTURED_CLAUDE_DRAFT_TEXT}\x1b[0m\x1b[7m \x1b[0m")
    );
    assert!(strip_ansi(&before[composer + 2]).contains("← for agents"));
    assert_eq!(
        strip_ansi(&after[composer + 2]).trim(),
        "⏵⏵ don't ask on (shift+tab to cycle)"
    );
    assert_ne!(
        strip_ansi(CAPTURED_CLAUDE_HERDR_EMPTY),
        strip_ansi(CAPTURED_CLAUDE_HERDR_DRAFT)
    );
    assert_eq!(
        classify("claude", CAPTURED_CLAUDE_HERDR_DRAFT),
        PaneState::Draft
    );
}

#[test]
fn captured_claude_tmux_actual_empty_agents_footer_is_idle_without_effort_metadata() {
    let visible = strip_ansi(CAPTURED_CLAUDE_TMUX_EMPTY_AGENTS);
    let lines: Vec<_> = visible.lines().map(str::to_owned).collect();
    let composer = captured_claude_composer_index(&lines);
    assert_eq!(lines[composer].trim(), "❯");
    assert!(lines[composer - 1].ends_with("cafleet-evidence-claude-tmux-1105Z ─"));
    assert_eq!(
        lines[composer + 2].trim(),
        "⏵⏵ don't ask on (shift+tab to cycle) · ← for agents"
    );
    assert!(visible.contains("1 more notice hidden"));
    assert!(!visible.contains("/effort"));
    assert!(strip_ansi(CAPTURED_CLAUDE_TMUX_EMPTY).contains("◐ medium · /effort"));
    assert_eq!(
        classify("claude", CAPTURED_CLAUDE_TMUX_EMPTY_AGENTS),
        PaneState::Idle
    );
}

#[test]
fn captured_claude_derived_empty_permission_footer_without_agents_remains_idle() {
    assert_eq!(
        CAPTURED_CLAUDE_HERDR_DRAFT
            .matches(CAPTURED_CLAUDE_DRAFT_TEXT)
            .count(),
        1
    );
    let content = CAPTURED_CLAUDE_HERDR_DRAFT.replacen(CAPTURED_CLAUDE_DRAFT_TEXT, "", 1);
    let visible = strip_ansi(&content);
    let lines: Vec<_> = visible.lines().map(str::to_owned).collect();
    let composer = captured_claude_composer_index(&lines);
    assert_eq!(lines[composer].trim(), "❯");
    assert_eq!(
        lines[composer + 2].trim(),
        "⏵⏵ don't ask on (shift+tab to cycle)"
    );
    assert_eq!(classify("claude", &content), PaneState::Idle);
}

#[test]
fn captured_claude_derived_actual_draft_role_follows_input_style_across_text_changes() {
    assert_eq!(
        CAPTURED_CLAUDE_HERDR_DRAFT
            .matches(CAPTURED_CLAUDE_DRAFT_TEXT)
            .count(),
        1
    );
    for text in [
        "Ask Claude to do anything",
        "! printf example",
        "/help",
        "An unseen suggestion",
    ] {
        let content = CAPTURED_CLAUDE_HERDR_DRAFT.replacen(CAPTURED_CLAUDE_DRAFT_TEXT, text, 1);
        assert_eq!(
            classify("claude", &content),
            PaneState::Draft,
            "derived text {text:?}"
        );
    }
}

#[test]
fn captured_claude_derived_optional_footers_keep_ambiguous_input_and_incomplete_blocks_unknown() {
    let lines: Vec<_> = CAPTURED_CLAUDE_HERDR_DRAFT
        .lines()
        .map(str::to_owned)
        .collect();
    let composer = captured_claude_composer_index(&lines);
    let mut no_upper = lines.clone();
    no_upper.remove(composer - 1);
    let mut no_lower = lines.clone();
    no_lower.remove(composer + 1);
    let mut unknown_footer = lines.clone();
    unknown_footer[composer + 2] = "unrecognized session status".to_string();
    for foreground in [
        "\x1b[38;2;255;0;0m",
        "\x1b[38;2;0;0;2m",
        "\x1b[7m",
        "\x1b[999m",
    ] {
        let content = CAPTURED_CLAUDE_HERDR_DRAFT.replacen(
            CAPTURED_CLAUDE_DRAFT_TEXT,
            &format!("{foreground}{CAPTURED_CLAUDE_DRAFT_TEXT}"),
            1,
        );
        assert_eq!(classify("claude", &content), PaneState::Unknown);
    }
    for content in [
        no_upper.join("\n"),
        no_lower.join("\n"),
        unknown_footer.join("\n"),
        strip_ansi(CAPTURED_CLAUDE_HERDR_DRAFT),
        format!(
            "Archived terminal output:\n```text\n{CAPTURED_CLAUDE_HERDR_DRAFT}\n```\nMore output follows."
        ),
    ] {
        assert_eq!(classify("claude", &content), PaneState::Unknown);
    }
}

#[test]
fn captured_claude_derived_work_and_approval_outrank_the_actual_draft_and_current_empty_frame() {
    let fixture = COMPOSERS
        .iter()
        .find(|fixture| fixture.agent == "claude")
        .unwrap();
    for capture in [
        CAPTURED_CLAUDE_HERDR_DRAFT,
        CAPTURED_CLAUDE_TMUX_EMPTY_AGENTS,
    ] {
        let working = format!("{capture}\n{}", fixture.working);
        assert_eq!(classify("claude", &working), PaneState::Working);
        let approval = format!("{working}\n{}", fixture.approval);
        assert_eq!(classify("claude", &approval), PaneState::AwaitingUser);
    }
}

#[test]
fn captured_opencode_placeholder_is_idle_with_the_real_cafleet_and_trailing_footer_layout() {
    let placeholder = strip_ansi(CAPTURED_OPENCODE_PLACEHOLDER);
    assert_eq!(placeholder, strip_ansi(CAPTURED_OPENCODE_DRAFT));
    assert_eq!(
        captured_opencode_composer_text(CAPTURED_OPENCODE_PLACEHOLDER),
        format!("┃  {CAPTURED_OPENCODE_HINT}")
    );
    for content in [
        "Cafleet · Big Pickle OpenCode",
        "tab agents  ctrl+p commands",
        "Run /connect to add an AI provider and start coding",
        "workspace:feat/0000184-goal-native-",
        "1.19.1",
    ] {
        assert!(
            placeholder.contains(content),
            "missing captured layout: {content}"
        );
    }
    assert_eq!(
        placeholder
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap()
            .trim(),
        "agent-multiplexer"
    );
    let hint = format!("{CAPTURED_OPENCODE_HINT_STYLE}{CAPTURED_OPENCODE_HINT}");
    let input = format!("{CAPTURED_OPENCODE_INPUT_STYLE}{CAPTURED_OPENCODE_HINT}");
    assert_eq!(CAPTURED_OPENCODE_PLACEHOLDER.matches(&hint).count(), 1);
    assert_eq!(
        CAPTURED_OPENCODE_PLACEHOLDER.replacen(&hint, &input, 1),
        CAPTURED_OPENCODE_DRAFT
    );
    assert_eq!(
        classify("opencode", CAPTURED_OPENCODE_PLACEHOLDER),
        PaneState::Idle
    );
}

#[test]
fn captured_opencode_identical_text_in_the_actual_input_role_is_a_draft() {
    assert_eq!(
        captured_opencode_composer_text(CAPTURED_OPENCODE_DRAFT),
        format!("┃  {CAPTURED_OPENCODE_HINT}")
    );
    assert!(CAPTURED_OPENCODE_DRAFT.contains(&format!(
        "{CAPTURED_OPENCODE_INPUT_STYLE}{CAPTURED_OPENCODE_HINT}"
    )));
    assert_eq!(
        classify("opencode", CAPTURED_OPENCODE_DRAFT),
        PaneState::Draft
    );
}

#[test]
fn captured_opencode_derived_hints_and_agent_footer_values_preserve_their_input_roles() {
    for (capture, expected) in [
        (CAPTURED_OPENCODE_PLACEHOLDER, PaneState::Idle),
        (CAPTURED_OPENCODE_DRAFT, PaneState::Draft),
    ] {
        assert_eq!(capture.matches(CAPTURED_OPENCODE_HINT).count(), 1);
        for hint in [
            "Review a new task",
            "! printf example",
            "/help",
            "An unseen rotating suggestion",
        ] {
            for alternate_metadata in [false, true] {
                let mut content = capture.replace(CAPTURED_OPENCODE_HINT, hint);
                if alternate_metadata {
                    content = content
                        .replace("Cafleet", "Planner")
                        .replace("agent-multiplexer", "fixture-workspace");
                }
                assert_eq!(
                    classify("opencode", &content),
                    expected,
                    "derived hint {hint:?}, alternate metadata {alternate_metadata}"
                );
            }
        }
    }
}

#[test]
fn captured_opencode_derived_adapter_window_keeps_the_composer_and_complete_trailing_layout() {
    for (capture, expected) in [
        (CAPTURED_OPENCODE_PLACEHOLDER, PaneState::Idle),
        (CAPTURED_OPENCODE_DRAFT, PaneState::Draft),
    ] {
        let lines: Vec<_> = capture.lines().collect();
        let window = lines[lines.len() - 80..].join("\n");
        let visible = strip_ansi(&window);
        assert!(visible.contains(CAPTURED_OPENCODE_HINT));
        assert!(visible.contains("Cafleet · Big Pickle OpenCode"));
        assert!(visible.contains("agent-multiplexer"));
        assert_eq!(classify("opencode", &window), expected);
    }
}

#[test]
fn captured_opencode_derived_ambiguous_composer_styles_remain_unknown() {
    let hint = format!("{CAPTURED_OPENCODE_HINT_STYLE}{CAPTURED_OPENCODE_HINT}");
    for foreground in [
        "\x1b[38;2;0;0;2m",
        "\x1b[38;2;0;2;0m",
        "\x1b[38;2;2;0;0m",
        "\x1b[38;2;2;2;2m",
        "\x1b[38;2;255;0;0m",
        "\x1b[7;38;2;128;128;128m",
        "\x1b[999;38;2;128;128;128m",
    ] {
        let replacement = format!("{foreground}\x1b[48;2;30;30;30m{CAPTURED_OPENCODE_HINT}");
        let content = CAPTURED_OPENCODE_PLACEHOLDER.replacen(&hint, &replacement, 1);
        assert_eq!(
            classify("opencode", &content),
            PaneState::Unknown,
            "derived foreground {foreground:?}"
        );
    }
}

#[test]
fn captured_opencode_derived_unstyled_truncated_and_footer_only_frames_remain_unknown() {
    let lines: Vec<_> = CAPTURED_OPENCODE_PLACEHOLDER.lines().collect();
    for content in [
        strip_ansi(CAPTURED_OPENCODE_PLACEHOLDER),
        lines[..42].join("\n"),
        lines[lines.len() - 10..].join("\n"),
    ] {
        assert_eq!(classify("opencode", &content), PaneState::Unknown);
    }
}

#[test]
fn captured_opencode_derived_work_and_approval_evidence_outrank_both_input_roles() {
    let fixture = COMPOSERS
        .iter()
        .find(|fixture| fixture.agent == "opencode")
        .unwrap();
    for capture in [CAPTURED_OPENCODE_PLACEHOLDER, CAPTURED_OPENCODE_DRAFT] {
        let working = format!("{capture}\n{}", fixture.working);
        assert_eq!(classify("opencode", &working), PaneState::Working);
        let approval = format!("{working}\n{}", fixture.approval);
        assert_eq!(classify("opencode", &approval), PaneState::AwaitingUser);
    }
}

#[test]
fn captured_opencode_derived_historical_quotes_remain_unknown() {
    for capture in [CAPTURED_OPENCODE_PLACEHOLDER, CAPTURED_OPENCODE_DRAFT] {
        let transcript =
            format!("Archived terminal output:\n```text\n{capture}\n```\nMore output follows.");
        assert_eq!(classify("opencode", &transcript), PaneState::Unknown);
    }
}

#[test]
fn captured_opencode_tmux_placeholder_is_idle_with_carried_sgr_and_the_complete_footer() {
    let visible = strip_ansi(CAPTURED_OPENCODE_TMUX_PLACEHOLDER);
    assert_eq!(visible, strip_ansi(CAPTURED_OPENCODE_TMUX_DRAFT));
    assert_eq!(
        captured_opencode_composer_text(CAPTURED_OPENCODE_TMUX_PLACEHOLDER),
        format!("┃  {CAPTURED_OPENCODE_HINT}")
    );
    for content in [
        "Cafleet · Big Pickle OpenCode",
        "tab agents  ctrl+p commands",
        "Run /connect to add an AI provider and start coding",
        "workspace:feat/0000184-goal-native-agent-multiplexer",
    ] {
        assert!(
            visible.contains(content),
            "missing captured layout: {content}"
        );
    }
    let footer = visible
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap();
    assert!(footer.contains("workspace:feat/0000184-goal-native-agent-multiplexer"));
    assert!(footer.ends_with("1.19.1"));
    let hint = format!("{CAPTURED_OPENCODE_TMUX_HINT_STYLE}{CAPTURED_OPENCODE_HINT}");
    let input = format!("{CAPTURED_OPENCODE_TMUX_INPUT_STYLE}{CAPTURED_OPENCODE_HINT}");
    assert!(CAPTURED_OPENCODE_TMUX_PLACEHOLDER.contains(&format!("\x1b[48;2;30;30;30m  {hint}")));
    assert_eq!(CAPTURED_OPENCODE_TMUX_PLACEHOLDER.matches(&hint).count(), 1);
    assert_eq!(
        CAPTURED_OPENCODE_TMUX_PLACEHOLDER.replacen(&hint, &input, 1),
        CAPTURED_OPENCODE_TMUX_DRAFT
    );
    assert_eq!(
        classify("opencode", CAPTURED_OPENCODE_TMUX_PLACEHOLDER),
        PaneState::Idle
    );
}

#[test]
fn captured_opencode_tmux_identical_text_with_the_actual_input_foreground_is_a_draft() {
    assert_eq!(
        captured_opencode_composer_text(CAPTURED_OPENCODE_TMUX_DRAFT),
        format!("┃  {CAPTURED_OPENCODE_HINT}")
    );
    assert!(CAPTURED_OPENCODE_TMUX_DRAFT.contains(&format!(
        "\x1b[48;2;30;30;30m  {CAPTURED_OPENCODE_TMUX_INPUT_STYLE}{CAPTURED_OPENCODE_HINT}"
    )));
    assert_eq!(
        classify("opencode", CAPTURED_OPENCODE_TMUX_DRAFT),
        PaneState::Draft
    );
}

#[test]
fn captured_opencode_tmux_derived_adapter_windows_preserve_roles_and_trailing_layout() {
    for (capture, expected) in [
        (CAPTURED_OPENCODE_TMUX_PLACEHOLDER, PaneState::Idle),
        (CAPTURED_OPENCODE_TMUX_DRAFT, PaneState::Draft),
    ] {
        let lines: Vec<_> = capture.lines().collect();
        let start = lines.len().checked_sub(80).unwrap();
        let window = lines[start..].join("\n");
        let visible = strip_ansi(&window);
        assert!(visible.contains(CAPTURED_OPENCODE_HINT));
        assert!(visible.contains("Cafleet · Big Pickle OpenCode"));
        assert!(visible.contains("tab agents  ctrl+p commands"));
        assert!(visible.contains("workspace:feat/0000184-goal-native-agent-multiplexer"));
        assert!(visible.contains("1.19.1"));
        assert_eq!(classify("opencode", &window), expected);
    }
}

#[test]
fn captured_opencode_tmux_derived_hint_and_metadata_changes_preserve_carried_roles() {
    for (capture, expected) in [
        (CAPTURED_OPENCODE_TMUX_PLACEHOLDER, PaneState::Idle),
        (CAPTURED_OPENCODE_TMUX_DRAFT, PaneState::Draft),
    ] {
        assert_eq!(capture.matches(CAPTURED_OPENCODE_HINT).count(), 1);
        for hint in ["An unseen rotating suggestion", "! printf example", "/help"] {
            let content = capture
                .replace(CAPTURED_OPENCODE_HINT, hint)
                .replace("Cafleet", "Planner")
                .replace("agent-multiplexer", "fixture-workspace");
            assert_eq!(
                classify("opencode", &content),
                expected,
                "derived hint {hint:?}"
            );
        }
    }
}

#[test]
fn captured_opencode_tmux_derived_ambiguous_or_incomplete_roles_remain_unknown() {
    for (capture, style) in [
        (
            CAPTURED_OPENCODE_TMUX_PLACEHOLDER,
            CAPTURED_OPENCODE_TMUX_HINT_STYLE,
        ),
        (
            CAPTURED_OPENCODE_TMUX_DRAFT,
            CAPTURED_OPENCODE_TMUX_INPUT_STYLE,
        ),
    ] {
        let hint = format!("{style}{CAPTURED_OPENCODE_HINT}");
        assert_eq!(capture.matches(&hint).count(), 1);
        for foreground in [
            "\x1b[38;2;0;0;2m",
            "\x1b[38;2;0;2;0m",
            "\x1b[38;2;2;0;0m",
            "\x1b[38;2;255;0;0m",
            "\x1b[7;38;2;128;128;128m",
            "\x1b[999;38;2;128;128;128m",
        ] {
            let content =
                capture.replacen(&hint, &format!("{foreground}{CAPTURED_OPENCODE_HINT}"), 1);
            assert_eq!(
                classify("opencode", &content),
                PaneState::Unknown,
                "derived foreground {foreground:?}"
            );
        }
        let input_background = format!("\x1b[48;2;30;30;30m  {hint}");
        assert_eq!(capture.matches(&input_background).count(), 1);
        let lines: Vec<_> = capture.lines().collect();
        for content in [
            capture.replacen(&input_background, &format!("  {hint}"), 1),
            strip_ansi(capture),
            lines[..42].join("\n"),
            lines[lines.len() - 10..].join("\n"),
            format!("Archived terminal output:\n```text\n{capture}\n```\nMore output follows."),
        ] {
            assert_eq!(classify("opencode", &content), PaneState::Unknown);
        }
    }
}

#[test]
fn captured_opencode_tmux_derived_work_and_approval_outrank_both_input_roles() {
    let fixture = COMPOSERS
        .iter()
        .find(|fixture| fixture.agent == "opencode")
        .unwrap();
    for capture in [
        CAPTURED_OPENCODE_TMUX_PLACEHOLDER,
        CAPTURED_OPENCODE_TMUX_DRAFT,
    ] {
        let working = format!("{capture}\n{}", fixture.working);
        assert_eq!(classify("opencode", &working), PaneState::Working);
        let approval = format!("{working}\n{}", fixture.approval);
        assert_eq!(classify("opencode", &approval), PaneState::AwaitingUser);
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
