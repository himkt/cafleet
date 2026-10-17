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

static CLAUDE_VIEWPORT_OVERLAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Jump to bottom \(click\)|^─{3,} [0-9]+ new messages?(?: \(click\))? ─{3,}$")
        .expect("the Claude viewport overlay pattern is valid")
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
        "codex" => codex_composer(&lines),
        "opencode" => opencode_composer(&lines),
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
    } else if cue_lines.iter().any(|line| {
        line.contains(" new message (click)")
            || (coding_agent == "claude" && CLAUDE_VIEWPORT_OVERLAY.is_match(line.trim()))
    }) || (coding_agent == "opencode"
        && cue_lines
            .iter()
            .any(|line| OPENCODE_MENU_OVERLAY.is_match(line)))
    {
        PaneState::Unclassified
    } else {
        match composer {
            Some(composer) => composer.state,
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
    Regex::new(r"^  \S[^|\r\n]* \| 📂 \S[^\r\n]*$").expect("the model footer pattern is valid")
});
static CLAUDE_USAGE_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^  \$ [0-9]+(?:\.[0-9]+)? \| ○ [0-9]+(?:\.[0-9]+)?[KM]?/[0-9]+(?:\.[0-9]+)?[KM]? \([0-9]+%\)$",
    )
        .expect("the usage footer pattern is valid")
});

struct Composer {
    top: usize,
    state: PaneState,
}

fn claude_composer(lines: &[&str]) -> Option<Composer> {
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
    let state = if !matches!(*first, "❯" | "❯\u{a0}") || !continuation.is_empty() {
        PaneState::Draft
    } else if idle_footer {
        PaneState::Finished
    } else {
        PaneState::Unclassified
    };
    Some(Composer { top, state })
}

static CODEX_MODEL_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^  \S+ \S+ · \S[^·\r\n]* · \S[^·\r\n]* · Context [0-9]+% used · Fast (on|off)$")
        .expect("the Codex model footer pattern is valid")
});

fn codex_composer(lines: &[&str]) -> Option<Composer> {
    let footer = *lines.last()?;
    let has_hint = matches!(footer, "  ? for shortcuts" | "  esc latest");
    let model_index = lines.len().checked_sub(if has_hint { 2 } else { 1 })?;
    if !CODEX_MODEL_FOOTER.is_match(lines[model_index]) {
        return None;
    }
    let composer_end = model_index.checked_sub(1)?;
    if !lines[composer_end].is_empty() {
        return None;
    }
    let top = lines[..composer_end]
        .iter()
        .rposition(|line| line.starts_with('›'))?;
    if top == 0 || !lines[top - 1].is_empty() {
        return None;
    }
    let (first, continuation) = lines[top..composer_end].split_first()?;
    if !(*first == "›" || first.starts_with("› "))
        || continuation
            .iter()
            .any(|line| !line.is_empty() && !line.starts_with("  "))
    {
        return None;
    }
    let state = if *first == "› Ask Codex to do anything"
        && continuation.is_empty()
        && footer == "  ? for shortcuts"
    {
        PaneState::Finished
    } else {
        PaneState::Draft
    };
    Some(Composer { top, state })
}

static OPENCODE_MENU_OVERLAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r" {3,}\S.* {2,}ctrl\+x [a-z](?: {2,}|$)")
        .expect("the OpenCode menu overlay pattern is valid")
});
static OPENCODE_IDLE_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^   (?:/|~/)[^\r\n]* {2,}[0-9]+(?:\.[0-9]+)?[KM]? \([0-9]+%\)  ctrl\+p commands$")
        .expect("the OpenCode idle footer pattern is valid")
});
static OPENCODE_MODEL_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^  ┃  \S[^·\r\n]* · \S[^\r\n]* OpenCode$")
        .expect("the OpenCode model row pattern is valid")
});

fn opencode_composer(lines: &[&str]) -> Option<Composer> {
    if !OPENCODE_IDLE_FOOTER.is_match(lines.last()?) {
        return None;
    }
    let border_index = lines.len().checked_sub(2)?;
    let rule = lines[border_index].strip_prefix("  ╹")?;
    if rule.chars().count() < 3 || !rule.chars().all(|ch| ch == '▀') {
        return None;
    }
    let model_index = border_index.checked_sub(1)?;
    if !OPENCODE_MODEL_ROW.is_match(lines[model_index]) {
        return None;
    }
    let mut top = model_index;
    while top > 0 && lines[top - 1].starts_with("  ┃") {
        top -= 1;
    }
    if top == 0 || !lines[top - 1].is_empty() {
        return None;
    }
    let rows = &lines[top..model_index];
    if rows.len() < 3 || rows.first()? != &"  ┃" || rows.last()? != &"  ┃" {
        return None;
    }
    let mut state = if rows.len() == 3 {
        PaneState::Finished
    } else {
        PaneState::Draft
    };
    for row in rows {
        let suffix = row.strip_prefix("  ┃")?;
        if !suffix.is_empty() {
            suffix.strip_prefix("  ")?;
            state = PaneState::Draft;
        }
    }
    Some(Composer { top, state })
}

fn matches_in_region(cues: &[Regex], lines: &[&str], region_lines: usize) -> bool {
    let region = &lines[lines.len().saturating_sub(region_lines)..];
    region
        .iter()
        .any(|line| cues.iter().any(|cue| cue.is_match(line)))
}
