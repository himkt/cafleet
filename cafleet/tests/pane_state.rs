use cafleet::pane_state::{PaneState, classify};

const AWAITING_REGION_LINES: usize = 20;
const WORKING_REGION_LINES: usize = 10;
const FINISHED_REGION_LINES: usize = 6;

struct CueLines {
    agent: &'static str,
    awaiting_user: &'static str,
    working: &'static str,
    finished: &'static str,
}

const CUE_LINES: [CueLines; 3] = [
    CueLines {
        agent: "claude",
        awaiting_user: " ❯ 1. Yes",
        working: "✻ Cogitating… (12s · esc to interrupt)",
        finished: "> ",
    },
    CueLines {
        agent: "codex",
        awaiting_user: "› 1. Yes, proceed",
        working: "• Working (8s • esc to interrupt)",
        finished: "› Ask Codex to do anything",
    },
    CueLines {
        agent: "opencode",
        awaiting_user: "┃  △ Permission required",
        working: "⬝⬝⬝⬝⬝⬝  esc interrupt",
        finished: "┃  Ask anything...",
    },
];

fn label(state: PaneState) -> &'static str {
    match state {
        PaneState::AwaitingUser => "awaiting_user",
        PaneState::Working => "working",
        PaneState::Finished => "finished",
        PaneState::Unclassified => "unclassified",
    }
}

fn assert_state(agent: &str, content: &str, expected: &str) {
    assert_eq!(
        label(classify(agent, content)),
        expected,
        "{agent} capture:\n{content}"
    );
}

fn capture(lines: &[&str]) -> String {
    lines.join("\n")
}

fn transcript(lines: usize) -> Vec<String> {
    (1..=lines)
        .map(|index| format!("transcript line {index}"))
        .collect()
}

fn cue_above(cue: &str, lines_below: usize) -> String {
    std::iter::once(cue.to_string())
        .chain(transcript(lines_below))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn each_agent_classifies_a_pending_prompt_as_awaiting_user() {
    for (agent, lines) in [
        (
            "claude",
            vec![
                "⏺ Bash(mise //cafleet:test)",
                "",
                "╭──────────────────────────────────────────╮",
                "│ Bash command                             │",
                "│                                          │",
                "│   mise //cafleet:test                    │",
                "│                                          │",
                "│ Do you want to proceed?                  │",
                "│ ❯ 1. Yes                                 │",
                "│   2. No, and tell Claude what to do      │",
                "╰──────────────────────────────────────────╯",
            ],
        ),
        (
            "claude",
            vec![
                " Which backend should the reviewer use?",
                "",
                " ❯ 1. claude",
                "      Everyday frontier",
                "   2. codex",
                "      Strongest reviewer",
                "   3. Other",
            ],
        ),
        (
            "codex",
            vec![
                "  Would you like to run the following command?",
                "",
                "  $ mise //cafleet:test",
                "",
                "› 1. Yes, proceed",
                "  2. Yes, and don't ask again for this command",
                "  3. No, and tell Codex what to do differently",
            ],
        ),
        (
            "codex",
            vec!["  Allow Codex to run mise //cafleet:test outside the sandbox? [y/n]"],
        ),
        (
            "opencode",
            vec![
                "┃",
                "┃  △ Permission required",
                "┃",
                "┃  $ mise //cafleet:test",
                "┃",
                "┃  Allow once   Allow always   Reject",
            ],
        ),
    ] {
        assert_state(agent, &capture(&lines), "awaiting_user");
    }
}

#[test]
fn each_agent_classifies_an_active_turn_as_working() {
    for (agent, lines) in [
        (
            "claude",
            vec![
                "⏺ Reading the design document.",
                "",
                "✻ Cogitating… (12s · ↓ 1.2k tokens · esc to interrupt)",
                "",
                "────────────────────────────────────────────",
                "> ",
                "────────────────────────────────────────────",
                "  ⏵⏵ don't ask on (shift+tab to cycle)",
            ],
        ),
        (
            "codex",
            vec![
                "• Exploring the repository",
                "",
                "• Working (8s • esc to interrupt)",
                "",
                "› Ask Codex to do anything",
                "",
                "  gpt-6.1-sol · 82% context left",
            ],
        ),
        (
            "opencode",
            vec![
                "  Reading the design document",
                "",
                "  ┃",
                "  ┃  Ask anything...",
                "  ┃",
                "  ⬝⬝⬝⬝⬝⬝  esc interrupt",
            ],
        ),
        (
            "opencode",
            vec![
                "  ~ Writing the migration...",
                "  ■■■⬝⬝⬝  esc to interrupt",
            ],
        ),
    ] {
        assert_state(agent, &capture(&lines), "working");
    }
}

#[test]
fn each_agent_classifies_a_composer_at_rest_as_finished() {
    for (agent, lines) in [
        (
            "claude",
            vec![
                "⏺ The tests pass.",
                "",
                "────────────────────────────────────────────",
                "> ",
                "────────────────────────────────────────────",
                "  ⏵⏵ don't ask on (shift+tab to cycle)",
            ],
        ),
        (
            "claude",
            vec![
                "╭──────────────────────────────────────────╮",
                "│ ❯ draft reply typed so far               │",
                "╰──────────────────────────────────────────╯",
                "  ? for shortcuts",
            ],
        ),
        (
            "codex",
            vec![
                "• The migration is in place.",
                "",
                "› Ask Codex to do anything",
                "",
                "  gpt-6.1-sol · 82% context left",
            ],
        ),
        (
            "codex",
            vec!["• Done.", "", "▌ Find and fix a bug in @filename"],
        ),
        (
            "opencode",
            vec![
                "  The migration is in place.",
                "",
                "  ┃",
                "  ┃  Ask anything...",
                "  ┃",
                "  ┃  Build  MiMo-V2.5 Free  OpenCode Zen",
            ],
        ),
    ] {
        assert_state(agent, &capture(&lines), "finished");
    }
}

#[test]
fn each_agent_classifies_a_quiet_pane_without_a_composer_as_unclassified() {
    for (agent, lines) in [
        (
            "claude",
            vec![
                "⏺ Bash(cargo build)",
                "  ⎿  Compiling cafleet v0.25.0",
                "     Building 120/340",
            ],
        ),
        (
            "codex",
            vec![
                "• Ran mise //cafleet:lint",
                "  └ Finished `dev` profile in 4.2s",
            ],
        ),
        (
            "opencode",
            vec![
                "  ~ Preparing the write...",
                "  → Read cafleet/src/lib.rs",
            ],
        ),
    ] {
        assert_state(agent, &capture(&lines), "unclassified");
    }
}

#[test]
fn awaiting_user_outranks_working_and_finished_when_cues_co_occur() {
    for cues in &CUE_LINES {
        for lines in [
            vec![cues.working, cues.awaiting_user],
            vec![cues.awaiting_user, cues.finished],
            vec![cues.working, cues.awaiting_user, cues.finished],
        ] {
            assert_state(cues.agent, &capture(&lines), "awaiting_user");
        }
    }
}

#[test]
fn working_outranks_finished_when_cues_co_occur() {
    for cues in &CUE_LINES {
        for lines in [
            [cues.working, cues.finished],
            [cues.finished, cues.working],
        ] {
            assert_state(cues.agent, &capture(&lines), "working");
        }
    }
}

#[test]
fn awaiting_user_cue_matches_only_within_its_region() {
    for cues in &CUE_LINES {
        assert_state(
            cues.agent,
            &cue_above(cues.awaiting_user, AWAITING_REGION_LINES - 1),
            "awaiting_user",
        );
        assert_state(
            cues.agent,
            &cue_above(cues.awaiting_user, AWAITING_REGION_LINES),
            "unclassified",
        );
    }
}

#[test]
fn working_cue_matches_only_within_its_region() {
    for cues in &CUE_LINES {
        assert_state(
            cues.agent,
            &cue_above(cues.working, WORKING_REGION_LINES - 1),
            "working",
        );
        assert_state(
            cues.agent,
            &cue_above(cues.working, WORKING_REGION_LINES),
            "unclassified",
        );
    }
}

#[test]
fn finished_cue_matches_only_within_its_region() {
    for cues in &CUE_LINES {
        assert_state(
            cues.agent,
            &cue_above(cues.finished, FINISHED_REGION_LINES - 1),
            "finished",
        );
        assert_state(
            cues.agent,
            &cue_above(cues.finished, FINISHED_REGION_LINES),
            "unclassified",
        );
    }
}

#[test]
fn blank_lines_do_not_count_toward_a_region() {
    for cues in &CUE_LINES {
        let mut lines = vec![cues.finished.to_string()];
        for line in transcript(FINISHED_REGION_LINES - 1) {
            lines.extend(vec![String::new(); 8]);
            lines.push(line);
        }
        lines.extend(vec![String::new(); 8]);
        assert_state(cues.agent, &lines.join("\n"), "finished");
    }
}

#[test]
fn cue_text_above_every_region_leaves_a_composer_at_rest_finished() {
    for cues in &CUE_LINES {
        let mut lines = vec![cues.awaiting_user.to_string(), cues.working.to_string()];
        lines.extend(transcript(AWAITING_REGION_LINES));
        lines.push(cues.finished.to_string());
        assert_state(cues.agent, &lines.join("\n"), "finished");
    }
}

#[test]
fn unknown_coding_agent_is_unclassified() {
    for cues in &CUE_LINES {
        for line in [cues.awaiting_user, cues.working, cues.finished] {
            assert_state("aider", line, "unclassified");
            assert_state("", line, "unclassified");
        }
    }
}
