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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_no_variable_is_set() {
        let s = Settings::from_lookup(|_| None).unwrap();
        assert_eq!(s.broker_host, "127.0.0.1");
        assert_eq!(s.broker_port, 8000);
        assert_eq!(s.max_text_len, 200);
        assert_eq!(s.multiplexer, None);
        assert_eq!(s.monitor_wake_interval, 600);
    }

    #[test]
    fn default_database_url_points_at_expanded_home_cafleet_v6() {
        let s = Settings::from_lookup(|_| None).unwrap();
        assert!(s.database_url.starts_with("sqlite:///"));
        assert!(
            s.database_url
                .ends_with("/.local/share/cafleet/cafleet_v6.db")
        );
        assert!(
            !s.database_url.contains('~'),
            "the default URL expands the home directory at startup"
        );
    }

    #[test]
    fn each_field_binds_to_its_exact_env_var() {
        let s = Settings::from_lookup(|name| match name {
            "CAFLEET_DATABASE_URL" => Some("sqlite:///srv/db/registry.db".to_string()),
            "CAFLEET_BROKER_HOST" => Some("0.0.0.0".to_string()),
            "CAFLEET_BROKER_PORT" => Some("9001".to_string()),
            "CAFLEET_MAX_TEXT_LEN" => Some("50".to_string()),
            "CAFLEET_MULTIPLEXER" => Some("herdr".to_string()),
            "CAFLEET_MONITOR_WAKE_INTERVAL" => Some("900".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(s.database_url, "sqlite:///srv/db/registry.db");
        assert_eq!(s.broker_host, "0.0.0.0");
        assert_eq!(s.broker_port, 9001);
        assert_eq!(s.max_text_len, 50);
        assert_eq!(s.multiplexer, Some("herdr".to_string()));
        assert_eq!(s.monitor_wake_interval, 900);
    }

    #[test]
    fn user_database_url_is_passed_through_verbatim() {
        let s = Settings::from_lookup(|name| match name {
            "CAFLEET_DATABASE_URL" => Some("sqlite:///~/custom/path.db".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            s.database_url, "sqlite:///~/custom/path.db",
            "a user-supplied URL gets no ~ expansion and no rewriting"
        );
    }

    #[test]
    fn non_integer_broker_port_fails_loudly() {
        let result = Settings::from_lookup(|name| match name {
            "CAFLEET_BROKER_PORT" => Some("not-a-port".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn out_of_range_broker_port_fails_loudly() {
        let result = Settings::from_lookup(|name| match name {
            "CAFLEET_BROKER_PORT" => Some("65536".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn non_integer_max_text_len_fails_loudly() {
        let result = Settings::from_lookup(|name| match name {
            "CAFLEET_MAX_TEXT_LEN" => Some("twenty".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn negative_max_text_len_fails_loudly() {
        let result = Settings::from_lookup(|name| match name {
            "CAFLEET_MAX_TEXT_LEN" => Some("-1".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn non_integer_monitor_wake_interval_fails_loudly() {
        let result = Settings::from_lookup(|name| match name {
            "CAFLEET_MONITOR_WAKE_INTERVAL" => Some("10m".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn zero_monitor_wake_interval_is_valid() {
        let s = Settings::from_lookup(|name| match name {
            "CAFLEET_MONITOR_WAKE_INTERVAL" => Some("0".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(s.monitor_wake_interval, 0);
    }
}
