//! Step 3 contract tests: `cafleet::error` — the two-tier error model
//! (SPEC §7.2) and the missing `--fleet-id` post-parse error (SPEC §6.3).
//!
//! Expected public API:
//! - `enum CafleetError { Usage(String), App(String) }`
//! - `CafleetError::exit_code(&self) -> i32` — `Usage` → 2, `App` → 1
//! - `CafleetError::message(&self) -> &str` — the bare message; the CLI's
//!   top-level printer prepends `Error: ` and writes it to stderr
//! - `missing_fleet_id() -> CafleetError` — the shared `--fleet-id` callback
//!   error (application class, exit 1)

use cafleet::error::{CafleetError, missing_fleet_id};

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
