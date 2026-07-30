use std::fmt;

#[derive(Debug)]
pub enum CafleetError {
    Usage(String),
    App(String),
}

impl CafleetError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CafleetError::Usage(_) => 2,
            CafleetError::App(_) => 1,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            CafleetError::Usage(message) | CafleetError::App(message) => message,
        }
    }
}

impl fmt::Display for CafleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CafleetError {}

pub fn missing_fleet_id() -> CafleetError {
    CafleetError::App(
        "--fleet-id <int> is required for this subcommand. \
         Create a fleet with 'cafleet fleet create' and pass its id."
            .to_string(),
    )
}
