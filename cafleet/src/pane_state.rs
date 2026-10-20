use std::sync::LazyLock;

use regex::Regex;

const AWAITING_REGION_LINES: usize = 20;
const WORKING_REGION_LINES: usize = 10;
const FINISHED_REGION_LINES: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    AwaitingUser,
    Working,
    Finished,
    Unclassified,
}

struct CueTable {
    awaiting_user: Vec<Regex>,
    working: Vec<Regex>,
    finished: Vec<Regex>,
}

impl CueTable {
    fn new(awaiting_user: &[&str], working: &[&str], finished: &[&str]) -> Self {
        Self {
            awaiting_user: compile(awaiting_user),
            working: compile(working),
            finished: compile(finished),
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
        &["esc to interrupt"],
        &[r"^[\s│]*[>❯](\s.*)?$"],
    )
});

static CODEX_CUES: LazyLock<CueTable> = LazyLock::new(|| {
    CueTable::new(
        &[r"^\s*›\s+\d+\.\s", r"\[y/n\]"],
        &["esc to interrupt"],
        &[r"^\s*[›▌](\s.*)?$"],
    )
});

static OPENCODE_CUES: LazyLock<CueTable> = LazyLock::new(|| {
    CueTable::new(
        &["Permission required"],
        &[r"esc\s+(to\s+)?interrupt"],
        &[r"^\s*┃(\s.*)?$"],
    )
});

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
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();

    if matches_in_region(&cues.awaiting_user, &lines, AWAITING_REGION_LINES) {
        PaneState::AwaitingUser
    } else if matches_in_region(&cues.working, &lines, WORKING_REGION_LINES) {
        PaneState::Working
    } else if matches_in_region(&cues.finished, &lines, FINISHED_REGION_LINES) {
        PaneState::Finished
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
