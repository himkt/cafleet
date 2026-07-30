use crate::error::CafleetError;

#[derive(Debug, Clone)]
pub struct Settings {
    pub database_url: String,
    pub broker_host: String,
    pub broker_port: u16,
    pub max_text_len: usize,
    pub multiplexer: Option<String>,
    pub monitor_stall_interval: u64,
}

impl Settings {
    /// Read each field from exactly its own `CAFLEET_*` name (no prefix
    /// magic). Bad numerics fail loudly; the default database URL is the only
    /// place `~` (the home directory) is expanded.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Settings, CafleetError> {
        let database_url = match lookup("CAFLEET_DATABASE_URL") {
            Some(value) => value,
            None => {
                let home = std::env::var("HOME").map_err(|_| {
                    CafleetError::App(
                        "HOME is not set; cannot derive the default database path".to_string(),
                    )
                })?;
                format!("sqlite:///{home}/.local/share/cafleet/cafleet_v6.db")
            }
        };
        Ok(Settings {
            database_url,
            broker_host: lookup("CAFLEET_BROKER_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            broker_port: parse_numeric(&lookup, "CAFLEET_BROKER_PORT", 8000, "a TCP port")?,
            max_text_len: parse_numeric(
                &lookup,
                "CAFLEET_MAX_TEXT_LEN",
                200,
                "a non-negative integer",
            )?,
            multiplexer: lookup("CAFLEET_MULTIPLEXER"),
            monitor_stall_interval: parse_numeric(
                &lookup,
                "CAFLEET_MONITOR_STALL_INTERVAL",
                240,
                "a non-negative integer",
            )?,
        })
    }

    pub fn from_env() -> Result<Settings, CafleetError> {
        Settings::from_lookup(|name| std::env::var(name).ok())
    }
}

fn parse_numeric<T: std::str::FromStr>(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &str,
    default: T,
    expectation: &str,
) -> Result<T, CafleetError> {
    match lookup(name) {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| CafleetError::App(format!("{name} must be {expectation} (got '{raw}')"))),
    }
}
