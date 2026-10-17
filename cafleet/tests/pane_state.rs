use cafleet::pane_state::{PaneState, classify};

const AGENT_CUES: [(&str, &str, &str, &str); 3] = [
    (
        "claude",
        "❯",
        " ❯ 1. Yes",
        "✻ Cogitating… (12s · esc to interrupt)",
    ),
    (
        "codex",
        "›",
        "› 1. Yes, proceed",
        "• Working (8s • esc to interrupt)",
    ),
    (
        "opencode",
        "┃",
        "┃  △ Permission required",
        "⬝⬝⬝⬝⬝⬝  esc interrupt",
    ),
];

fn assert_state(agent: &str, content: &str, expected: PaneState) {
    assert_eq!(
        classify(agent, content),
        expected,
        "{agent} capture:\n{content}"
    );
}

fn assert_held(agent: &str, content: &str) {
    match classify(agent, content) {
        PaneState::AwaitingUser
        | PaneState::Working
        | PaneState::Draft
        | PaneState::Unclassified => {}
        PaneState::Finished => panic!("unsafe {agent} capture classified Finished:\n{content}"),
    }
}

fn claude_box(rows: &[&str]) -> String {
    let mut lines = vec!["╭──────────────────────────────────────────╮".to_string()];
    lines.extend(rows.iter().map(|row| format!("│ {row:<40} │")));
    lines.push("╰──────────────────────────────────────────╯".to_string());
    lines.push("  ? for shortcuts".to_string());
    lines.join("\n")
}

fn claude_capture(state: &str) -> String {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/pane_state/claude-herdr.json"))
            .expect("the real Claude capture fixtures are valid JSON");
    fixtures
        .iter()
        .find(|fixture| fixture["state"] == state)
        .expect("the requested real Claude observation is recorded")["content"]
        .as_str()
        .expect("every real observation contains captured text")
        .to_string()
}

#[test]
fn real_claude_single_line_composer_is_draft() {
    assert_state("claude", &claude_capture("draft_single"), PaneState::Draft);
}

#[test]
fn real_claude_wrapped_composer_with_clipped_footer_holds() {
    assert_held("claude", &claude_capture("draft_wrapped"));
}

#[test]
fn real_claude_multiline_composer_with_clipped_footer_holds() {
    assert_held("claude", &claude_capture("draft_multiline"));
}

#[test]
fn real_claude_blank_first_row_with_entered_continuation_holds() {
    assert_held("claude", &claude_capture("draft_blank_first"));
}

#[test]
fn real_claude_active_indicator_overrides_empty_composer() {
    assert_state("claude", &claude_capture("working"), PaneState::Working);
}

#[test]
fn real_claude_changed_exit_footer_is_unclassified() {
    assert_state(
        "claude",
        &claude_capture("restored"),
        PaneState::Unclassified,
    );
}

#[test]
fn real_claude_whitespace_input_is_indistinguishable_from_empty_capture() {
    assert_eq!(claude_capture("draft_whitespace"), claude_capture("empty"));
}

#[test]
fn synthetic_claude_boxed_single_line_input_holds() {
    assert_held("claude", &claude_box(&["❯ draft reply typed so far"]));
}

#[test]
fn synthetic_claude_boxed_multiline_input_holds() {
    assert_held(
        "claude",
        &claude_box(&["❯ first line", "  second line", "  third line"]),
    );
}

#[test]
fn synthetic_claude_boxed_blank_first_row_with_continuation_holds() {
    assert_held(
        "claude",
        &claude_box(&["❯", "", "  keep this continuation"]),
    );
}

#[test]
fn synthetic_claude_boxed_wrapped_input_holds() {
    assert_held(
        "claude",
        &claude_box(&[
            "❯ a draft that reaches the right boundary",
            "  and continues on the following row",
        ]),
    );
}

#[test]
fn bare_prompt_glyphs_have_incomplete_layouts() {
    for (agent, prompt, _, _) in AGENT_CUES {
        for content in [prompt.to_string(), format!("{prompt} ")] {
            assert_state(agent, &content, PaneState::Unclassified);
        }
    }
}

#[test]
fn arbitrary_prompt_suffixes_hold() {
    for (agent, prompt, _, _) in AGENT_CUES {
        for suffix in ["keep my reply", "  ", "\t", "[image attached]"] {
            assert_held(agent, &format!("{prompt} {suffix}"));
        }
    }
}

#[test]
fn placeholder_text_alone_has_no_proven_empty_layout() {
    for (agent, content) in [
        ("codex", "› Ask Codex to do anything"),
        ("codex", "▌ Find and fix a bug in @filename"),
        ("opencode", "┃  Ask anything..."),
        ("claude", "❯ Try asking for help"),
    ] {
        assert_state(agent, content, PaneState::Unclassified);
    }
}

#[test]
fn blank_first_row_and_nonempty_continuation_hold_for_every_agent() {
    for (agent, prompt, _, _) in AGENT_CUES {
        assert_held(agent, &format!("{prompt}\n\n  preserve this draft\n"));
    }
}

#[test]
fn multiline_and_wrapped_input_hold_for_every_agent() {
    for (agent, prompt, _, _) in AGENT_CUES {
        for continuation in ["second line", "  wrapped continuation", "\n  last row"] {
            assert_held(agent, &format!("{prompt} first line\n{continuation}"));
        }
    }
}

#[test]
fn historical_prompt_followed_by_transcript_is_unclassified() {
    for (agent, prompt, _, _) in AGENT_CUES {
        assert_state(
            agent,
            &format!("{prompt}\nThe transcript continues here.\nTool output is incomplete."),
            PaneState::Unclassified,
        );
    }
}

#[test]
fn blank_lines_preserve_the_separation_from_historical_prompts() {
    for (agent, prompt, _, _) in AGENT_CUES {
        assert_state(
            agent,
            &format!("{prompt}\n\n\n\n\nTranscript output\n\n\n"),
            PaneState::Unclassified,
        );
    }
}

#[test]
fn missing_claude_composer_boundary_holds() {
    for content in [
        "│ ❯                                      │\n╰──────────────────────────────────────────╯\n  ? for shortcuts",
        "╭──────────────────────────────────────────╮\n│ ❯                                      │",
        "────────────────────────────────────────────\n❯",
    ] {
        assert_state("claude", content, PaneState::Unclassified);
    }
}

#[test]
fn unexpected_footer_after_composer_holds() {
    for (agent, prompt, _, _) in AGENT_CUES {
        assert_state(
            agent,
            &format!("{prompt}\n[unrecognized application overlay]"),
            PaneState::Unclassified,
        );
    }
}

#[test]
fn attachments_and_composer_decorations_hold() {
    for (agent, prompt, _, _) in AGENT_CUES {
        for decoration in ["[Image #1]", "[Pasted text #1 +20 lines]", "📎 file.txt"] {
            assert_held(agent, &format!("{decoration}\n{prompt}\n{decoration}"));
        }
    }
}

#[test]
fn capture_truncated_inside_a_long_composer_holds() {
    for (agent, prompt, _, _) in AGENT_CUES {
        let rows = std::iter::repeat_n("  continuation row", 100).collect::<Vec<_>>();
        assert_held(agent, &format!("{prompt}\n{}", rows.join("\n")));
    }
}

#[test]
fn active_turn_overrides_queued_draft_input() {
    for (agent, prompt, _, working) in AGENT_CUES {
        assert_state(
            agent,
            &format!("{working}\n{prompt} queued operator reply"),
            PaneState::Working,
        );
    }
}

#[test]
fn permission_interface_overrides_working_and_draft() {
    for (agent, prompt, permission, working) in AGENT_CUES {
        assert_state(
            agent,
            &format!("{working}\n{prompt} queued operator reply\n{permission}"),
            PaneState::AwaitingUser,
        );
    }
}

#[test]
fn current_permission_interfaces_hold() {
    for (agent, _, permission, _) in AGENT_CUES {
        assert_state(agent, permission, PaneState::AwaitingUser);
    }
    assert_state(
        "codex",
        "Allow Codex to run this command outside the sandbox? [y/n]",
        PaneState::AwaitingUser,
    );
}

#[test]
fn empty_or_quiet_capture_is_unclassified() {
    for (agent, _, _, _) in AGENT_CUES {
        for content in ["", "\n\n", "   \n\t", "Reading a file...\nTool output"] {
            assert_state(agent, content, PaneState::Unclassified);
        }
    }
}

#[test]
fn unknown_agent_is_unclassified_even_with_recognized_cues() {
    for agent in ["aider", "", "Claude"] {
        for (_, prompt, permission, working) in AGENT_CUES {
            for content in [prompt, permission, working] {
                assert_state(agent, content, PaneState::Unclassified);
            }
        }
    }
}
