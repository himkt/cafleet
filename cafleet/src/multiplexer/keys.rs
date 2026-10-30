use crate::error::CafleetError;

const NAMED_KEYS: &[(&str, &str, &str)] = &[
    ("Space", "Space", "space"),
    ("Escape", "Escape", "esc"),
    ("Enter", "Enter", "enter"),
    ("Tab", "Tab", "tab"),
    ("Backspace", "BSpace", "backspace"),
    ("Up", "Up", "up"),
    ("Down", "Down", "down"),
    ("Left", "Left", "left"),
    ("Right", "Right", "right"),
];

#[derive(Debug)]
enum PressKey {
    Literal(u8),
    Named(&'static (&'static str, &'static str, &'static str)),
    Function(u8),
    Control(u8),
    ShiftTab,
}

#[derive(Debug)]
pub(crate) struct PressKeys(Vec<PressKey>);

impl PressKeys {
    pub(crate) fn parse(keys: &[String]) -> Result<Self, CafleetError> {
        if !(1..=64).contains(&keys.len()) {
            return Err(CafleetError::Usage(
                "press requires between 1 and 64 keys".into(),
            ));
        }
        let mut parsed = Vec::with_capacity(keys.len());
        for (index, token) in keys.iter().enumerate() {
            if token.len() > 32 {
                return Err(CafleetError::Usage(format!(
                    "press key at position {} exceeds 32 UTF-8 bytes",
                    index + 1
                )));
            }
            let key = if token.len() == 1 && (b'!'..=b'~').contains(&token.as_bytes()[0]) {
                Some(PressKey::Literal(token.as_bytes()[0]))
            } else if let Some(named) = NAMED_KEYS
                .iter()
                .find(|named| token.eq_ignore_ascii_case(named.0))
            {
                Some(PressKey::Named(named))
            } else if token.eq_ignore_ascii_case("Shift+Tab") {
                Some(PressKey::ShiftTab)
            } else if token.len() == 6
                && token
                    .get(..5)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Ctrl+"))
                && token.as_bytes()[5].is_ascii_alphabetic()
            {
                Some(PressKey::Control(token.as_bytes()[5].to_ascii_lowercase()))
            } else {
                (1..=12)
                    .find(|n| token.eq_ignore_ascii_case(&format!("F{n}")))
                    .map(PressKey::Function)
            };
            parsed.push(key.ok_or_else(|| {
                CafleetError::Usage(format!(
                    "invalid press key at position {}: {}",
                    index + 1,
                    serde_json::to_string(token).expect("strings serialize")
                ))
            })?);
        }
        Ok(Self(parsed))
    }

    pub(crate) fn canonical(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|key| match key {
                PressKey::Literal(byte) => char::from(*byte).to_string(),
                PressKey::Named(named) => named.0.into(),
                PressKey::Function(n) => format!("F{n}"),
                PressKey::Control(byte) => format!("Ctrl+{}", char::from(*byte)),
                PressKey::ShiftTab => "Shift+Tab".into(),
            })
            .collect()
    }

    pub(super) fn tmux_arguments(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|key| match key {
                PressKey::Literal(byte) => format!("0x{byte:02x}"),
                PressKey::Named(named) => named.1.into(),
                PressKey::Function(n) => format!("F{n}"),
                PressKey::Control(byte) => format!("C-{}", char::from(*byte)),
                PressKey::ShiftTab => "BTab".into(),
            })
            .collect()
    }

    pub(super) fn herdr_arguments(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|key| match key {
                PressKey::Literal(b'+') => "plus".into(),
                PressKey::Literal(b'-') => "minus".into(),
                PressKey::Literal(b';') => "semicolon".into(),
                PressKey::Literal(byte) => char::from(*byte).to_string(),
                PressKey::Named(named) => named.2.into(),
                PressKey::Function(n) => format!("f{n}"),
                PressKey::Control(byte) => format!("ctrl+{}", char::from(*byte)),
                PressKey::ShiftTab => "shift+tab".into(),
            })
            .collect()
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;
    pub(crate) fn parse(tokens: &[&str]) -> PressKeys {
        PressKeys::parse(
            &tokens
                .iter()
                .map(|token| (*token).into())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::multiplexer::test_support::argv;

    fn usage(tokens: &[String], expected: &str) {
        let failure = match PressKeys::parse(tokens) {
            Ok(_) => panic!("the key vector is outside the common grammar"),
            Err(failure) => failure,
        };
        assert!(matches!(&failure, CafleetError::Usage(message) if message == expected));
        assert_eq!(failure.exit_code(), 2);
    }
    #[test]
    fn press_keys_validate_the_complete_common_grammar_and_byte_count_bounds() {
        let keys = parse(&[
            "A",
            "a",
            "+",
            "-",
            ";",
            "!",
            "~",
            "sPaCe",
            "ESCAPE",
            "enter",
            "Tab",
            "Backspace",
            "up",
            "down",
            "left",
            "right",
            "f1",
            "F12",
            "cTrL+A",
            "CTRL+Z",
            "sHiFt+tAb",
        ]);
        assert_eq!(
            keys.canonical(),
            argv(&[
                "A",
                "a",
                "+",
                "-",
                ";",
                "!",
                "~",
                "Space",
                "Escape",
                "Enter",
                "Tab",
                "Backspace",
                "Up",
                "Down",
                "Left",
                "Right",
                "F1",
                "F12",
                "Ctrl+a",
                "Ctrl+z",
                "Shift+Tab",
            ])
        );
        assert_eq!(
            PressKeys::parse(&vec!["a".into(); 64])
                .unwrap()
                .canonical()
                .len(),
            64
        );
        usage(&[], "press requires between 1 and 64 keys");
        usage(
            &vec!["a".into(); 65],
            "press requires between 1 and 64 keys",
        );
        let long = "é".repeat(17);
        usage(
            &["Enter".into(), long],
            "press key at position 2 exceeds 32 UTF-8 bytes",
        );
        for invalid in ["F0", "F13", "Ctrl+1", " ", "é"] {
            usage(
                &["a".into(), invalid.into()],
                &format!(
                    "invalid press key at position 2: {}",
                    serde_json::to_string(invalid).unwrap()
                ),
            );
        }
    }
}
