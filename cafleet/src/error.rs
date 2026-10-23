use std::{borrow::Cow, fmt};

#[derive(Debug, Clone)]
pub enum CafleetError {
    Usage(String),
    App(String),
    Value(String),
    InvalidStoredValue { field: String, value: String },
}

impl CafleetError {
    /// Append a compensation diagnostic without changing the primary exit category.
    pub(crate) fn with_cleanup(self, diagnostic: impl std::fmt::Display) -> Self {
        let message = format!("{self}\n{diagnostic}");
        match self {
            Self::Usage(_) => Self::Usage(message),
            Self::Value(_) => Self::Value(message),
            Self::App(_) | Self::InvalidStoredValue { .. } => Self::App(message),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            CafleetError::Usage(_) => 2,
            CafleetError::App(_)
            | CafleetError::Value(_)
            | CafleetError::InvalidStoredValue { .. } => 1,
        }
    }

    pub fn message(&self) -> Cow<'_, str> {
        match self {
            CafleetError::Usage(message)
            | CafleetError::App(message)
            | CafleetError::Value(message) => Cow::Borrowed(message),
            CafleetError::InvalidStoredValue { field, value } => {
                Cow::Owned(format!("invalid stored value for {field}: {value}"))
            }
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
