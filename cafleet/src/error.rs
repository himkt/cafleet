use std::fmt;

#[derive(Debug)]
pub enum CafleetError {
    Usage(String),
    App(String),
    /// A broker value error (bad input / missing row), translated by callers:
    /// the CLI wraps to exit 1, the WebUI maps to an HTTP status.
    Value(String),
    /// A broker permission error (e.g. a non-recipient ack), translated by
    /// callers like [`CafleetError::Value`].
    Permission(String),
}

impl CafleetError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CafleetError::Usage(_) => 2,
            CafleetError::App(_) | CafleetError::Value(_) | CafleetError::Permission(_) => 1,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            CafleetError::Usage(message)
            | CafleetError::App(message)
            | CafleetError::Value(message)
            | CafleetError::Permission(message) => message,
        }
    }
}

impl fmt::Display for CafleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
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
    fn missing_fleet_id_is_an_application_error_with_the_pinned_string() {
        let err = missing_fleet_id();
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(err.exit_code(), 1);
        assert_eq!(
            err.message(),
            "--fleet-id <int> is required for this subcommand. \
             Create a fleet with 'cafleet fleet create' and pass its id."
        );
    }
}
