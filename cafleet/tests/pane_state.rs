use cafleet::delivery::DELIVERY_CAPTURE_LINES;
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

fn runtime_capture(agent: &str, state: &str) -> String {
    let source = match agent {
        "claude" => include_str!("fixtures/pane_state/claude-herdr-runtime.json"),
        "codex" => include_str!("fixtures/pane_state/codex-herdr-runtime.json"),
        "codex-monitor" => include_str!("fixtures/pane_state/codex-monitor-runtime.json"),
        "opencode" => include_str!("fixtures/pane_state/opencode-herdr-runtime.json"),
        "opencode-unobstructed" => include_str!("fixtures/pane_state/opencode-unobstructed.json"),
        _ => panic!("no runtime capture matrix for {agent}"),
    };
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(source).expect("the runtime capture fixtures are valid JSON");
    let fixture = fixtures
        .iter()
        .find(|fixture| fixture["state"] == state)
        .expect("the requested runtime observation is recorded");
    let mut lines = fixture["content"]
        .as_str()
        .expect("every runtime observation contains captured text")
        .lines()
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let window = usize::try_from(DELIVERY_CAPTURE_LINES).expect("capture line bound is positive");
    lines[lines.len().saturating_sub(window)..].join("\n")
}

#[test]
fn runtime_claude_complete_empty_composer_is_finished() {
    assert_state(
        "claude",
        &runtime_capture("claude", "empty"),
        PaneState::Finished,
    );
}

#[test]
fn runtime_claude_entered_composer_rows_are_draft() {
    for state in [
        "draft-single",
        "draft-wrapped",
        "draft-multiline",
        "draft-blank-first",
    ] {
        assert_state(
            "claude",
            &runtime_capture("claude", state),
            PaneState::Draft,
        );
    }
}

#[test]
fn runtime_claude_native_model_decision_menu_is_awaiting_user() {
    assert_state(
        "claude",
        &runtime_capture("claude", "decision-menu"),
        PaneState::AwaitingUser,
    );
}

#[test]
fn runtime_claude_unproven_new_message_overlay_holds_even_with_empty_composer() {
    for state in ["working", "final-empty"] {
        assert_state(
            "claude",
            &runtime_capture("claude", state),
            PaneState::Unclassified,
        );
    }
}

#[test]
fn runtime_claude_indistinguishable_whitespace_has_the_empty_capture_classification() {
    assert_eq!(
        classify("claude", &runtime_capture("claude", "draft-whitespace")),
        classify("claude", &runtime_capture("claude", "empty")),
    );
}

#[test]
fn runtime_codex_observed_composer_content_is_draft() {
    for state in [
        "draft-single",
        "draft-wrapped",
        "draft-multiline",
        "draft-blank-first",
        "draft-whitespace",
    ] {
        assert_state("codex", &runtime_capture("codex", state), PaneState::Draft);
        assert_state(
            "codex",
            &runtime_capture("codex", &format!("earlier-{state}")),
            PaneState::Draft,
        );
    }
}

#[test]
fn runtime_codex_complete_empty_and_restored_composers_are_finished() {
    for state in ["earlier-empty", "empty", "final-empty"] {
        assert_state(
            "codex",
            &runtime_capture("codex", state),
            PaneState::Finished,
        );
    }
}

#[test]
fn runtime_codex_native_model_selection_is_awaiting_user() {
    assert_state(
        "codex",
        &runtime_capture("codex", "decision-menu"),
        PaneState::AwaitingUser,
    );
}

#[test]
fn runtime_codex_current_active_indicator_overrides_empty_placeholder() {
    assert_state(
        "codex",
        &runtime_capture("codex", "working"),
        PaneState::Working,
    );
}

#[test]
fn runtime_codex_current_active_indicator_overrides_queued_composer_text() {
    let content = runtime_capture("codex", "working");
    let draft = content.replace("› Ask Codex to do anything", "› Preserve this queued reply");
    assert_state("codex", &draft, PaneState::Working);
}

#[test]
fn runtime_codex_incomplete_model_footer_holds() {
    let content = runtime_capture("codex", "empty");
    let truncated = content
        .lines()
        .filter(|line| !line.contains("GPT-6.1-Sol medium"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_state("codex", &truncated, PaneState::Unclassified);
}

#[test]
fn runtime_codex_unrecognized_footer_holds() {
    let content = format!(
        "{}\n[Unrecognized footer]",
        runtime_capture("codex", "empty")
    );
    assert_state("codex", &content, PaneState::Unclassified);
}

#[test]
fn runtime_codex_unproven_attachment_decoration_holds() {
    let content = runtime_capture("codex", "empty");
    let decorated = content.replace(
        "› Ask Codex to do anything",
        "› Ask Codex to do anything\n  [Image #1]",
    );
    assert_held("codex", &decorated);
}

#[test]
fn runtime_codex_historical_quoted_activity_is_outside_current_interface() {
    let content = runtime_capture("codex", "empty");
    let quoted = content.replace(
        "› Ask Codex to do anything",
        "Transcript example:\n```text\nesc to interrupt\n```\n\n› Ask Codex to do anything",
    );
    assert_state("codex", &quoted, PaneState::Finished);
}

#[test]
fn runtime_codex_live_monitor_role_footer_is_finished() {
    assert_state(
        "codex",
        &runtime_capture("codex-monitor", "monitor-empty"),
        PaneState::Finished,
    );
}

#[test]
fn recognized_codex_footer_accepts_model_effort_path_branch_and_usage_data() {
    let content = runtime_capture("codex", "empty");
    let footer = content
        .lines()
        .find(|line| line.contains(" · Context "))
        .expect("the evidenced Codex footer records context usage");
    for replacement in [
        "  GPT-6-Luna low · /workspace/monitor · main · Context 20% used · Fast off",
        "  GPT-6-Astra high · ~/project with spaces · fix/notifications · Context 0% used · Fast on",
        "  GPT-6.1-Sol xhigh · /tmp/worktree · HEAD · Context 100% used · Fast off",
    ] {
        assert_state(
            "codex",
            &content.replace(footer, replacement),
            PaneState::Finished,
        );
    }
}

#[test]
fn recognized_claude_footer_accepts_model_path_cost_and_context_data() {
    let content = runtime_capture("claude", "empty");
    let model = content
        .lines()
        .find(|line| line.starts_with("  Opus 5.5 | 📂 "))
        .expect("the evidenced Claude layout records model and path");
    let usage = content
        .lines()
        .find(|line| line.starts_with("  $ "))
        .expect("the evidenced Claude layout records cost and context usage");
    for (model_value, usage_value) in [
        (
            "  Sonnet 5.5 | 📂 /workspace/another-project",
            "  $ 12.3000 | ○ 1.2K/200K (1%)",
        ),
        (
            "  Haiku 5.5 | 📂 ~/project with spaces",
            "  $ 0.0000 | ○ 150K/1M (15%)",
        ),
        (
            "  Opus 5.5 | 📂 /tmp/worktree",
            "  $ 3.1250 | ○ 1.2M/2M (60%)",
        ),
    ] {
        let variant = content
            .replace(model, model_value)
            .replace(usage, usage_value);
        assert_state("claude", &variant, PaneState::Finished);
    }
}

#[test]
fn runtime_opencode_empty_with_overlapping_sidebar_holds() {
    for state in ["empty", "final-empty"] {
        assert_held("opencode", &runtime_capture("opencode", state));
    }
}

#[test]
fn runtime_opencode_single_line_draft_with_sidebar_holds() {
    assert_held("opencode", &runtime_capture("opencode", "draft-single"));
}

#[test]
fn runtime_opencode_wrapped_and_multiline_composer_content_holds() {
    for state in ["draft-wrapped", "draft-multiline"] {
        assert_held("opencode", &runtime_capture("opencode", state));
    }
}

#[test]
fn runtime_opencode_blank_first_row_with_continuation_holds() {
    assert_held(
        "opencode",
        &runtime_capture("opencode", "draft-blank-first"),
    );
}

#[test]
fn runtime_opencode_clipped_native_command_palette_holds() {
    assert_held("opencode", &runtime_capture("opencode", "decision-menu"));
}

#[test]
fn runtime_opencode_active_generation_overrides_empty_composer() {
    assert_state(
        "opencode",
        &runtime_capture("opencode", "working"),
        PaneState::Working,
    );
}

#[test]
fn runtime_opencode_entered_whitespace_is_indistinguishable_from_empty() {
    assert_eq!(
        runtime_capture("opencode", "draft-whitespace"),
        runtime_capture("opencode", "empty")
    );
}

#[test]
fn runtime_opencode_unobstructed_empty_and_restored_composers_are_finished() {
    for state in [
        "empty",
        "restored-after-draft-single",
        "restored-after-draft-wrapped",
        "restored-after-draft-multiline",
        "restored-after-draft-blank-first",
        "restored-after-draft-whitespace",
        "decision-canceled",
        "final-empty",
    ] {
        assert_state(
            "opencode",
            &runtime_capture("opencode-unobstructed", state),
            PaneState::Finished,
        );
    }
}

#[test]
fn runtime_opencode_unobstructed_single_line_composer_is_draft() {
    assert_state(
        "opencode",
        &runtime_capture("opencode-unobstructed", "draft-single"),
        PaneState::Draft,
    );
}

#[test]
fn runtime_opencode_unobstructed_wrapped_composer_is_draft() {
    assert_state(
        "opencode",
        &runtime_capture("opencode-unobstructed", "draft-wrapped"),
        PaneState::Draft,
    );
}

#[test]
fn runtime_opencode_unobstructed_multiline_composer_is_draft() {
    assert_state(
        "opencode",
        &runtime_capture("opencode-unobstructed", "draft-multiline"),
        PaneState::Draft,
    );
}

#[test]
fn runtime_opencode_unobstructed_blank_first_row_with_continuation_is_draft() {
    assert_state(
        "opencode",
        &runtime_capture("opencode-unobstructed", "draft-blank-first"),
        PaneState::Draft,
    );
}

#[test]
fn runtime_opencode_unobstructed_indistinguishable_whitespace_matches_empty() {
    let empty = runtime_capture("opencode-unobstructed", "empty");
    let whitespace = runtime_capture("opencode-unobstructed", "draft-whitespace");
    assert_eq!(whitespace, empty);
    assert_eq!(
        classify("opencode", &whitespace),
        classify("opencode", &empty)
    );
}

#[test]
fn runtime_opencode_unobstructed_clipped_command_menu_holds() {
    assert_held(
        "opencode",
        &runtime_capture("opencode-unobstructed", "decision-menu"),
    );
}

#[test]
fn runtime_opencode_unobstructed_active_footer_overrides_empty_composer() {
    assert_state(
        "opencode",
        &runtime_capture("opencode-unobstructed", "working"),
        PaneState::Working,
    );
}

#[test]
fn runtime_opencode_unobstructed_active_footer_overrides_queued_draft() {
    let content = runtime_capture("opencode-unobstructed", "working");
    let current_composer = "  ┃\n  ┃\n  ┃\n  ┃  Cafleet · Big Pickle OpenCode";
    assert!(content.contains(current_composer));
    let queued = content.replace(
        current_composer,
        "  ┃\n  ┃  Preserve this queued reply\n  ┃\n  ┃  Cafleet · Big Pickle OpenCode",
    );
    assert_state("opencode", &queued, PaneState::Working);
}

#[test]
fn runtime_opencode_unobstructed_missing_footer_holds() {
    let content = runtime_capture("opencode-unobstructed", "empty");
    let truncated = content
        .lines()
        .filter(|line| !line.contains("ctrl+p commands"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_held("opencode", &truncated);
}

#[test]
fn runtime_opencode_unobstructed_unknown_footer_holds() {
    let content = format!(
        "{}\n[Unrecognized application overlay]",
        runtime_capture("opencode-unobstructed", "empty")
    );
    assert_held("opencode", &content);
}

#[test]
fn runtime_opencode_unobstructed_truncated_composer_holds() {
    let content = runtime_capture("opencode-unobstructed", "draft-blank-first");
    let continuation = content
        .lines()
        .position(|line| line.contains("Capture evidence after blank first line."))
        .expect("the observed draft has a nonempty continuation");
    let truncated = content
        .lines()
        .skip(continuation)
        .collect::<Vec<_>>()
        .join("\n");
    assert_held("opencode", &truncated);
}

#[test]
fn recognized_opencode_footer_accepts_model_path_and_context_data() {
    let content = runtime_capture("opencode-unobstructed", "empty");
    let footer = content
        .lines()
        .find(|line| line.contains("ctrl+p commands"))
        .expect("the observed idle footer has its commands shortcut");
    let variant = content
        .replace(
            "Cafleet · Big Pickle OpenCode",
            "Cafleet · Another Model OpenCode",
        )
        .replace(
            footer,
            "   /workspace/another-project                 1.2K (1%)  ctrl+p commands",
        );
    assert_state("opencode", &variant, PaneState::Finished);
}

#[test]
fn runtime_claude_unknown_footer_holds() {
    let content = format!(
        "{}\n[Unrecognized footer]",
        runtime_capture("claude", "empty")
    );
    assert_state("claude", &content, PaneState::Unclassified);
}

#[test]
fn runtime_claude_missing_upper_composer_boundary_holds() {
    let content = runtime_capture("claude", "empty");
    let lines = content.lines().collect::<Vec<_>>();
    let current_prompt = lines
        .iter()
        .rposition(|line| line.starts_with('❯'))
        .expect("the captured composer has its prompt row");
    let truncated = content
        .lines()
        .skip(current_prompt)
        .collect::<Vec<_>>()
        .join("\n");
    assert_state("claude", &truncated, PaneState::Unclassified);
}

#[test]
fn runtime_claude_observable_entered_spaces_are_draft() {
    let content = runtime_capture("claude", "empty");
    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    let current = lines
        .iter()
        .rposition(|line| line.starts_with('❯'))
        .expect("the captured composer has its prompt row");
    lines[current] = "❯   ".to_string();
    assert_state("claude", &lines.join("\n"), PaneState::Draft);
}

#[test]
fn runtime_claude_quoted_historical_permission_is_outside_current_interface() {
    let content = runtime_capture("claude", "empty");
    let mut lines = content.lines().collect::<Vec<_>>();
    let top = lines
        .iter()
        .rposition(|line| line.contains("CaptureProbe-Claude ─"))
        .expect("the captured composer has its named upper border");
    lines.splice(
        top..top,
        ["Transcript example:", "```text", " ❯ 1. Yes", "```"],
    );
    assert_state("claude", &lines.join("\n"), PaneState::Finished);
}

#[test]
fn runtime_claude_quoted_historical_activity_is_outside_current_interface() {
    let content = runtime_capture("claude", "empty");
    let mut lines = content.lines().collect::<Vec<_>>();
    let top = lines
        .iter()
        .rposition(|line| line.contains("CaptureProbe-Claude ─"))
        .expect("the captured composer has its named upper border");
    lines.splice(
        top..top,
        ["Transcript example:", "```text", "esc to interrupt", "```"],
    );
    assert_state("claude", &lines.join("\n"), PaneState::Finished);
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
