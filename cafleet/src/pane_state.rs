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

#[derive(Clone, Copy)]
enum Foreground {
    Basic(u16),
    Indexed(u16),
    Rgb([u16; 3]),
}

#[derive(Clone, Copy, Default)]
struct Style {
    foreground: Option<Foreground>,
    background: Option<[u16; 3]>,
    dim: bool,
    ambiguous: bool,
    explicit: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComposerProfile {
    Standard,
    Codex,
    OpenCodeRgb,
}

#[derive(Clone, Copy)]
enum Role {
    Decoration,
    Input,
    Unknown,
}

impl Style {
    fn role(self, profile: ComposerProfile) -> Role {
        if self.ambiguous {
            Role::Unknown
        } else if let Some(Foreground::Rgb(rgb)) = self.foreground {
            if profile == ComposerProfile::OpenCodeRgb
                && self.background == Some([30, 30, 30])
                && !self.dim
            {
                match rgb {
                    [128, 128, 128] => Role::Decoration,
                    [238, 238, 238] => Role::Input,
                    _ => Role::Unknown,
                }
            } else {
                Role::Unknown
            }
        } else if self.dim
            && (matches!(self.foreground, Some(Foreground::Indexed(240..=248)))
                || (profile == ComposerProfile::Codex
                    && self.foreground.is_none()
                    && self.background == Some([53, 53, 53])))
        {
            Role::Decoration
        } else if self.explicit
            && matches!(
                self.foreground,
                None | Some(Foreground::Basic(37 | 97) | Foreground::Indexed(37 | 97))
            )
            && !self.dim
        {
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
                30..=37 | 90..=97 => self.foreground = Some(Foreground::Basic(codes[i])),
                39 => self.foreground = None,
                38 | 48 => match &codes[i + 1..] {
                    [5, index, ..] => {
                        if codes[i] == 38 && *index <= 255 {
                            self.foreground = Some(Foreground::Indexed(*index));
                        } else {
                            self.ambiguous = true;
                        }
                        i += 2;
                    }
                    [2, red, green, blue, ..] => {
                        let rgb = [*red, *green, *blue];
                        if rgb.iter().all(|channel| *channel <= 255) {
                            if codes[i] == 38 {
                                self.foreground = Some(Foreground::Rgb(rgb));
                            } else {
                                self.background = Some(rgb);
                            }
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
    characters: Vec<(char, Style)>,
}

fn styled_lines(content: &str) -> Vec<Line> {
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
                characters.extend(line[offset..escape.start()].chars().map(|ch| (ch, style)));
                if &capture[3] == "m" && capture[2].is_empty() {
                    style.apply(&capture[1]);
                } else {
                    style.ambiguous = true;
                }
                offset = escape.end();
            }
            characters.extend(line[offset..].chars().map(|ch| (ch, style)));
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

fn composer_state(lines: &[&Line], marker: char, profile: ComposerProfile) -> PaneState {
    let mut draft = false;
    let mut unknown = false;
    for line in lines {
        let Some(start) = line.characters.iter().position(|(ch, _)| *ch == marker) else {
            return PaneState::Unknown;
        };
        for (ch, style) in &line.characters[start + 1..] {
            if ch.is_whitespace() {
                continue;
            }
            match style.role(profile) {
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

fn opencode_state(lines: &[&Line]) -> PaneState {
    let (metadata, profile) = if lines
        .last()
        .is_some_and(|line| line.text.trim_start().starts_with('┃'))
    {
        (lines.len() - 1, ComposerProfile::Standard)
    } else {
        let Some(shortcuts) = lines.iter().rposition(|line| {
            line.text
                .split_whitespace()
                .eq(["tab", "agents", "ctrl+p", "commands"])
        }) else {
            return PaneState::Unknown;
        };
        if shortcuts < 2 {
            return PaneState::Unknown;
        }
        let Some(border) = lines[shortcuts - 1].text.trim().strip_prefix('╹') else {
            return PaneState::Unknown;
        };
        if border.is_empty() || !border.chars().all(|ch| ch == '▀') {
            return PaneState::Unknown;
        }
        (shortcuts - 2, ComposerProfile::OpenCodeRgb)
    };
    let Some(footer) = lines[metadata].text.trim().strip_prefix('┃') else {
        return PaneState::Unknown;
    };
    if footer.trim().is_empty() {
        return PaneState::Unknown;
    }
    let start = lines[..metadata]
        .iter()
        .rposition(|line| !line.text.trim_start().starts_with('┃'))
        .map_or(0, |index| index + 1);
    let composer = &lines[start..metadata];
    if composer.len() < 3
        || composer[0].text.trim() != "┃"
        || composer[composer.len() - 1].text.trim() != "┃"
    {
        return PaneState::Unknown;
    }
    composer_state(composer, '┃', profile)
}

pub fn classify(coding_agent: &str, content: &str) -> PaneState {
    if !matches!(coding_agent, "claude" | "codex" | "opencode") {
        return PaneState::Unknown;
    }
    let lines = styled_lines(content);
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
            composer_state(&[lines[index]], '❯', ComposerProfile::Standard)
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
            composer_state(&[lines[index]], '›', ComposerProfile::Codex)
        }
        "opencode" => opencode_state(&lines),
        _ => PaneState::Unknown,
    }
}
