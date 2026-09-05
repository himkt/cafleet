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
}
