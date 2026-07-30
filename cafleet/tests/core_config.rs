//! Step 3 contract tests: `cafleet::config` — the six `CAFLEET_*` settings
//! (SPEC §7.1): exact env-var binding, documented defaults, default-only `~`
//! expansion, and loud failure on bad numerics.
//!
//! Expected public API:
//! - `struct Settings { database_url: String, broker_host: String,
//!    broker_port: u16, max_text_len: usize, multiplexer: Option<String>,
//!    monitor_stall_interval: u64 }`
//! - `Settings::from_lookup(lookup: impl Fn(&str) -> Option<String>)
//!    -> Result<Settings, CafleetError>` — reads each field from exactly its
//!   own `CAFLEET_*` name (no prefix magic). `Settings::from_env()` delegates
//!   with `std::env::var`; it is not tested here because env mutation is
//!   process-global and races parallel tests.

use cafleet::config::Settings;

#[test]
fn defaults_when_no_variable_is_set() {
    let s = Settings::from_lookup(|_| None).unwrap();
    assert_eq!(s.broker_host, "127.0.0.1");
    assert_eq!(s.broker_port, 8000);
    assert_eq!(s.max_text_len, 200);
    assert_eq!(s.multiplexer, None);
    assert_eq!(s.monitor_stall_interval, 240);
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
    let s = Settings::from_lookup(|name| {
        match name {
            "CAFLEET_DATABASE_URL" => Some("sqlite:///srv/db/registry.db".to_string()),
            "CAFLEET_BROKER_HOST" => Some("0.0.0.0".to_string()),
            "CAFLEET_BROKER_PORT" => Some("9001".to_string()),
            "CAFLEET_MAX_TEXT_LEN" => Some("50".to_string()),
            "CAFLEET_MULTIPLEXER" => Some("herdr".to_string()),
            "CAFLEET_MONITOR_STALL_INTERVAL" => Some("600".to_string()),
            _ => None,
        }
    })
    .unwrap();
    assert_eq!(s.database_url, "sqlite:///srv/db/registry.db");
    assert_eq!(s.broker_host, "0.0.0.0");
    assert_eq!(s.broker_port, 9001);
    assert_eq!(s.max_text_len, 50);
    assert_eq!(s.multiplexer, Some("herdr".to_string()));
    assert_eq!(s.monitor_stall_interval, 600);
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
fn non_integer_monitor_stall_interval_fails_loudly() {
    let result = Settings::from_lookup(|name| match name {
        "CAFLEET_MONITOR_STALL_INTERVAL" => Some("4m".to_string()),
        _ => None,
    });
    assert!(result.is_err());
}

#[test]
fn zero_monitor_stall_interval_is_valid() {
    let s = Settings::from_lookup(|name| match name {
        "CAFLEET_MONITOR_STALL_INTERVAL" => Some("0".to_string()),
        _ => None,
    })
    .unwrap();
    assert_eq!(s.monitor_stall_interval, 0);
}
