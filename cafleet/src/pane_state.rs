use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    Idle,
    AwaitingUser,
    Working,
    Draft,
    Unknown,
}

impl PaneState {
    pub fn hold_reason(self) -> Option<&'static str> {
        match self {
            Self::Idle => None,
            Self::AwaitingUser => Some("awaiting_user"),
            Self::Working => Some("working"),
            Self::Draft => Some("draft"),
            Self::Unknown => Some("unknown"),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Style {
    foreground: Option<u16>,
    background: Option<[u16; 3]>,
    dim: bool,
    ambiguous: bool,
    explicit: bool,
}

#[derive(Clone, Copy)]
enum Role {
    Decoration,
    Input,
    Unknown,
}

impl Style {
    fn role(self, coding_agent: &str) -> Role {
        if self.ambiguous {
            Role::Unknown
        } else if self.dim
            && (matches!(self.foreground, Some(240..=248))
                || (coding_agent == "codex"
                    && self.foreground.is_none()
                    && self.background == Some([53, 53, 53])))
        {
            Role::Decoration
        } else if self.explicit && matches!(self.foreground, None | Some(37 | 97)) && !self.dim {
            Role::Input
        } else {
            Role::Unknown
        }
    }

    fn apply(&mut self, parameters: &str) {
        let Ok(codes) = parameters
            .split(';')
            .map(|value| {
                if value.is_empty() {
                    Ok(0)
                } else {
                    value.parse::<u16>()
                }
            })
            .collect::<Result<Vec<_>, _>>()
        else {
            self.ambiguous = true;
            return;
        };
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0 => {
                    *self = Self {
                        explicit: true,
                        ..Self::default()
                    }
                }
                1 | 3 | 4 | 23 | 24 => {}
                2 => self.dim = true,
                22 => self.dim = false,
                30..=37 | 90..=97 => self.foreground = Some(codes[i]),
                39 => self.foreground = None,
                38 | 48 => match &codes[i + 1..] {
                    [5, index, ..] => {
                        if codes[i] == 38 && *index <= 255 {
                            self.foreground = Some(*index);
                        } else {
                            self.ambiguous = true;
                        }
                        i += 2;
                    }
                    [2, red, green, blue, ..] => {
                        let rgb = [*red, *green, *blue];
                        if codes[i] == 48 && rgb.iter().all(|channel| *channel <= 255) {
                            self.background = Some(rgb);
                        } else {
                            self.ambiguous = true;
                        }
                        i += 4;
                    }
                    _ => {
                        self.ambiguous = true;
                        return;
                    }
                },
                49 => self.background = None,
                _ => self.ambiguous = true,
            }
            self.explicit = true;
            i += 1;
        }
    }
}

struct Line {
    text: String,
    characters: Vec<(char, Role)>,
}

fn styled_lines(coding_agent: &str, content: &str) -> Vec<Line> {
    static CSI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[([0-?]*)([ -/]*)([@-~])").expect("valid CSI pattern"));
    let mut style = Style::default();
    content
        .lines()
        .map(|line| {
            let mut characters = Vec::new();
            let mut offset = 0;
            for capture in CSI.captures_iter(line) {
                let escape = capture
                    .get(0)
                    .expect("a CSI capture has its complete match");
                characters.extend(
                    line[offset..escape.start()]
                        .chars()
                        .map(|ch| (ch, style.role(coding_agent))),
                );
                if &capture[3] == "m" && capture[2].is_empty() {
                    style.apply(&capture[1]);
                } else {
                    style.ambiguous = true;
                }
                offset = escape.end();
            }
            characters.extend(
                line[offset..]
                    .chars()
                    .map(|ch| (ch, style.role(coding_agent))),
            );
            Line {
                text: characters.iter().map(|(ch, _)| ch).collect(),
                characters,
            }
        })
        .collect()
}

fn is_border(line: &str) -> bool {
    let text = line.trim();
    !text.is_empty()
        && text
            .chars()
            .all(|ch| matches!(ch, '─' | '━' | '│' | '┌' | '┐' | '└' | '┘'))
}

fn composer_state(lines: &[&Line], marker: char) -> PaneState {
    let mut draft = false;
    let mut unknown = false;
    for line in lines {
        let Some(start) = line.characters.iter().position(|(ch, _)| *ch == marker) else {
            return PaneState::Unknown;
        };
        for (ch, role) in &line.characters[start + 1..] {
            if ch.is_whitespace() {
                continue;
            }
            match role {
                Role::Decoration => {}
                Role::Input => draft = true,
                Role::Unknown => unknown = true,
            }
        }
    }
    if draft {
        PaneState::Draft
    } else if unknown {
        PaneState::Unknown
    } else {
        PaneState::Idle
    }
}

pub fn classify(coding_agent: &str, content: &str) -> PaneState {
    if !matches!(coding_agent, "claude" | "codex" | "opencode") {
        return PaneState::Unknown;
    }
    let lines = styled_lines(coding_agent, content);
    let lines: Vec<_> = lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .collect();
    let recent = &lines[lines.len().saturating_sub(20)..];
    if recent
        .iter()
        .any(|line| line.text.trim_start().starts_with("```"))
    {
        return PaneState::Unknown;
    }
    static APPROVAL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[\s│]*[❯›]\s+\d+\.\s|\[y/n\]|Permission required")
            .expect("valid approval pattern")
    });
    static WORKING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)esc\s+(to\s+)?interrupt").expect("valid work pattern"));
    if recent.iter().any(|line| APPROVAL.is_match(&line.text)) {
        return PaneState::AwaitingUser;
    }
    if recent
        .iter()
        .rev()
        .take(10)
        .any(|line| WORKING.is_match(&line.text))
    {
        return PaneState::Working;
    }
    let Some(last) = lines.last() else {
        return PaneState::Unknown;
    };
    match coding_agent {
        "claude" if last.text.trim() == "? for shortcuts" => {
            let Some(index) = lines
                .iter()
                .rposition(|line| line.text.trim_start().starts_with('❯'))
            else {
                return PaneState::Unknown;
            };
            if index == 0
                || index + 2 != lines.len() - 1
                || !is_border(&lines[index - 1].text)
                || !is_border(&lines[index + 1].text)
            {
                return PaneState::Unknown;
            }
            composer_state(&[lines[index]], '❯')
        }
        "codex" => {
            let Some(index) = lines
                .iter()
                .rposition(|line| line.text.trim_start().starts_with('›'))
            else {
                return PaneState::Unknown;
            };
            let footer = &lines[index + 1..];
            static CONTEXT_USED: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r"\bContext (?:100|[1-9]?\d)% used\b")
                    .expect("valid Codex context pattern")
            });
            if !(matches!(footer, [status] if status.text.contains("context left") || CONTEXT_USED.is_match(&status.text))
                || matches!(footer, [status, shortcuts] if CONTEXT_USED.is_match(&status.text)
                    && shortcuts.text.trim_start().starts_with("? for shortcuts")))
            {
                return PaneState::Unknown;
            }
            composer_state(&[lines[index]], '›')
        }
        "opencode" if last.text.trim_start().starts_with('┃') && last.text.contains("Build") => {
            let Some(start) = lines[..lines.len() - 1]
                .iter()
                .rposition(|line| !line.text.trim_start().starts_with('┃'))
            else {
                return PaneState::Unknown;
            };
            let composer = &lines[start + 1..lines.len() - 1];
            if composer.len() < 3 {
                return PaneState::Unknown;
            }
            composer_state(composer, '┃')
        }
        _ => PaneState::Unknown,
    }
}
