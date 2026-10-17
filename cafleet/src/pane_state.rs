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
    let composer = match coding_agent {
        "claude" => claude_composer(&lines),
        "codex" | "opencode" => None,
        _ => unreachable!("the coding agent has a cue table"),
    };
    let transcript_end = match &composer {
        Some(composer) => composer.top,
        None => lines.len(),
    };
    let Some(mut cue_lines) = unquoted_lines(&lines[..transcript_end]) else {
        return PaneState::Unclassified;
    };
    cue_lines.resize(lines.len(), "");

    if matches_in_region(&cues.awaiting_user, &cue_lines, AWAITING_REGION_LINES) {
        PaneState::AwaitingUser
    } else if matches_in_region(&cues.working, &cue_lines, WORKING_REGION_LINES) {
        PaneState::Working
    } else if cue_lines
        .iter()
        .any(|line| line.contains(" new message (click)"))
    {
        PaneState::Unclassified
    } else {
        match composer {
            Some(composer) => composer.state(),
            None => PaneState::Unclassified,
        }
    }
}

fn unquoted_lines<'a>(lines: &[&'a str]) -> Option<Vec<&'a str>> {
    let mut quoted = false;
    let mut result = Vec::with_capacity(lines.len());
    for line in lines {
        if line.trim().starts_with("```") {
            quoted = !quoted;
            result.push("");
        } else {
            result.push(if quoted { "" } else { line });
        }
    }
    if quoted { None } else { Some(result) }
}

static CLAUDE_MODEL_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^  Opus 5\.5 \| 📂 .+$").expect("the model footer pattern is valid")
});
static CLAUDE_USAGE_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^  \$ [0-9.]+ \| ○ [0-9.]+K/1M \([0-9]+%\)$")
        .expect("the usage footer pattern is valid")
});

struct ClaudeComposer<'a> {
    top: usize,
    rows: &'a [&'a str],
    idle_footer: bool,
}

impl ClaudeComposer<'_> {
    fn state(&self) -> PaneState {
        let (first, continuation) = self
            .rows
            .split_first()
            .expect("a recognized composer contains its prompt row");
        if !matches!(*first, "❯" | "❯\u{a0}") || continuation.iter().any(|line| !line.is_empty())
        {
            PaneState::Draft
        } else if self.idle_footer {
            PaneState::Finished
        } else {
            PaneState::Unclassified
        }
    }
}

fn claude_composer<'a>(lines: &'a [&'a str]) -> Option<ClaudeComposer<'a>> {
    let idle_footer = matches!(
        *lines.last()?,
        "  ⏵⏵ don't ask on (shift+tab to cycle)"
            | "  ⏵⏵ don't ask on (shift+tab to cycle) · ← for agents"
    );
    let footer_start = lines.len().checked_sub(if idle_footer { 3 } else { 2 })?;
    if !CLAUDE_MODEL_FOOTER.is_match(lines[footer_start])
        || !CLAUDE_USAGE_FOOTER.is_match(lines[footer_start + 1])
    {
        return None;
    }
    let bottom = footer_start.checked_sub(1)?;
    let border = lines[bottom];
    if border.chars().count() < 3 || !border.chars().all(|ch| ch == '─') {
        return None;
    }
    let top = lines[..bottom].iter().rposition(|line| {
        let Some((rule, title)) = line.split_once(' ') else {
            return false;
        };
        rule.chars().count() >= 3
            && rule.chars().all(|ch| ch == '─')
            && title.ends_with(" ─")
            && line.chars().count() == border.chars().count()
    })?;
    let rows = &lines[top + 1..bottom];
    let (first, continuation) = rows.split_first()?;
    if !first.starts_with('❯')
        || continuation
            .iter()
            .any(|line| !line.is_empty() && !line.starts_with("  "))
    {
        return None;
    }
    Some(ClaudeComposer {
        top,
        rows,
        idle_footer,
    })
}

fn matches_in_region(cues: &[Regex], lines: &[&str], region_lines: usize) -> bool {
    let region = &lines[lines.len().saturating_sub(region_lines)..];
    region
        .iter()
        .any(|line| cues.iter().any(|cue| cue.is_match(line)))
}
