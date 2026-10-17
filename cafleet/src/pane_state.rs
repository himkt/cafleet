use std::sync::LazyLock;

use regex::Regex;

const AWAITING_REGION_LINES: usize = 20;
const WORKING_REGION_LINES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    AwaitingUser,
    Working,
    Draft,
    Finished,
    Unclassified,
}

struct CueTable {
    awaiting_user: Vec<Regex>,
    working: Vec<Regex>,
}

impl CueTable {
    fn new(awaiting_user: &[&str], working: &[&str]) -> Self {
        Self {
            awaiting_user: compile(awaiting_user),
            working: compile(working),
        }
    }
}

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|pattern| Regex::new(pattern).expect("the cue pattern is valid"))
        .collect()
}

static CLAUDE_CUES: LazyLock<CueTable> = LazyLock::new(|| {
    CueTable::new(
        &[r"^[\s│]*❯\s+\d+\.\s"],
        &["esc to interrupt", r"^[✻✽✶✳✢·] .+… \(\d+s ·"],
    )
});

static CODEX_CUES: LazyLock<CueTable> =
    LazyLock::new(|| CueTable::new(&[r"^\s*›\s+\d+\.\s", r"\[y/n\]"], &["esc to interrupt"]));

static OPENCODE_CUES: LazyLock<CueTable> =
    LazyLock::new(|| CueTable::new(&["Permission required"], &[r"esc\s+(to\s+)?interrupt"]));

fn cue_table(coding_agent: &str) -> Option<&'static CueTable> {
    match coding_agent {
        "claude" => Some(&CLAUDE_CUES),
        "codex" => Some(&CODEX_CUES),
        "opencode" => Some(&OPENCODE_CUES),
        _ => None,
    }
}

pub fn classify(coding_agent: &str, content: &str) -> PaneState {
    let Some(cues) = cue_table(coding_agent) else {
        return PaneState::Unclassified;
    };
    let lines: Vec<&str> = content.lines().collect();

    if matches_in_region(&cues.awaiting_user, &lines, AWAITING_REGION_LINES) {
        PaneState::AwaitingUser
    } else if matches_in_region(&cues.working, &lines, WORKING_REGION_LINES) {
        PaneState::Working
    } else {
        match coding_agent {
            "claude" => classify_claude_composer(&lines),
            "codex" | "opencode" => PaneState::Unclassified,
            _ => unreachable!("the coding agent has a cue table"),
        }
    }
}

static CLAUDE_MODEL_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^  Opus 5\.5 \| 📂 .+$").expect("the model footer pattern is valid")
});
static CLAUDE_USAGE_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^  \$ [0-9.]+ \| ○ [0-9.]+K/1M \([0-9]+%\)$")
        .expect("the usage footer pattern is valid")
});

fn classify_claude_composer(lines: &[&str]) -> PaneState {
    let Some(footer_start) = lines.len().checked_sub(2) else {
        return PaneState::Unclassified;
    };
    if !CLAUDE_MODEL_FOOTER.is_match(lines[footer_start])
        || !CLAUDE_USAGE_FOOTER.is_match(lines[footer_start + 1])
    {
        return PaneState::Unclassified;
    }
    let Some(bottom) = footer_start.checked_sub(1) else {
        return PaneState::Unclassified;
    };
    let border = lines[bottom];
    if border.chars().count() < 3 || !border.chars().all(|ch| ch == '─') {
        return PaneState::Unclassified;
    }
    let Some(top) = lines[..bottom].iter().rposition(|line| {
        let Some((rule, title)) = line.split_once(' ') else {
            return false;
        };
        rule.chars().count() >= 3
            && rule.chars().all(|ch| ch == '─')
            && title.ends_with(" ─")
            && line.chars().count() == border.chars().count()
    }) else {
        return PaneState::Unclassified;
    };
    let Some((first, continuation)) = lines[top + 1..bottom].split_first() else {
        return PaneState::Unclassified;
    };
    if !first.starts_with('❯')
        || continuation
            .iter()
            .any(|line| !line.is_empty() && !line.starts_with("  "))
    {
        return PaneState::Unclassified;
    }
    if !matches!(*first, "❯" | "❯\u{a0}") || continuation.iter().any(|line| !line.is_empty()) {
        PaneState::Draft
    } else {
        PaneState::Unclassified
    }
}

fn matches_in_region(cues: &[Regex], lines: &[&str], region_lines: usize) -> bool {
    let region = &lines[lines.len().saturating_sub(region_lines)..];
    region
        .iter()
        .any(|line| cues.iter().any(|cue| cue.is_match(line)))
}
