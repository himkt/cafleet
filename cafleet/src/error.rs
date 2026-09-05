use std::{borrow::Cow, fmt};

#[derive(Debug)]
pub enum CafleetError {
    Usage(String),
    App(String),
    /// A broker value error (bad input / missing row), translated by callers:
    /// the CLI wraps to exit 1, the WebUI maps to an HTTP status.
    Value(String),
    ActiveMonitorExists {
        fleet_id: i64,
        member_id: i64,
    },
}

impl CafleetError {
    /// Append a compensation diagnostic without changing the primary exit category.
    pub(crate) fn with_cleanup(self, diagnostic: impl std::fmt::Display) -> Self {
        let message = format!("{self}\n{diagnostic}");
        match self {
            Self::Usage(_) => Self::Usage(message),
            Self::Value(_) => Self::Value(message),
            Self::App(_) | Self::ActiveMonitorExists { .. } => Self::App(message),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            CafleetError::Usage(_) => 2,
            CafleetError::App(_)
            | CafleetError::Value(_)
            | CafleetError::ActiveMonitorExists { .. } => 1,
        }
    }

    pub fn message(&self) -> Cow<'_, str> {
        match self {
            CafleetError::Usage(message)
            | CafleetError::App(message)
            | CafleetError::Value(message) => Cow::Borrowed(message),
            CafleetError::ActiveMonitorExists {
                fleet_id,
                member_id,
            } => Cow::Owned(format!(
                "fleet {fleet_id} already has an active monitor member (member {member_id})"
            )),
        }
    }
}

impl fmt::Display for CafleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for CafleetError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_error_maps_to_exit_code_2() {
        assert_eq!(CafleetError::Usage("bad flag".to_string()).exit_code(), 2);
    }

    #[test]
    fn app_error_maps_to_exit_code_1() {
        assert_eq!(
            CafleetError::App("runtime conflict".to_string()).exit_code(),
            1
        );
    }

    #[test]
    fn message_returns_the_carried_string() {
        assert_eq!(CafleetError::Usage("u".to_string()).message(), "u");
        assert_eq!(CafleetError::App("a".to_string()).message(), "a");
    }

    #[test]
    fn active_monitor_conflict_preserves_message_display_and_exit_code() {
        let error = CafleetError::ActiveMonitorExists {
            fleet_id: 17,
            member_id: 42,
        };
        let expected = "fleet 17 already has an active monitor member (member 42)";
        assert_eq!(error.message(), expected);
        assert_eq!(error.to_string(), expected);
        assert_eq!(error.exit_code(), 1);
    }
}
