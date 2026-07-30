//! Step 8 CLI contract tests: the `server` subcommand's settings-derived
//! defaults, shown in `--help` (SPEC §6.8 *cafleet server launcher*, A7).

mod common;

use std::process::Command;

use common::{Cli, code, stdout, text};

#[test]
fn server_help_shows_the_settings_derived_defaults() {
    let cli = Cli::new();
    let output = cli.run(&["server", "--help"]);
    assert_eq!(code(&output), 0);
    let out = stdout(&output);
    assert!(out.contains("--host"), "got: {out}");
    assert!(out.contains("--port"), "got: {out}");
    assert!(out.contains("127.0.0.1"), "the settings default host, got: {out}");
    assert!(out.contains("8000"), "the settings default port, got: {out}");
}

#[test]
fn server_help_defaults_follow_the_broker_env_vars() {
    let output = Command::new(env!("CARGO_BIN_EXE_cafleet"))
        .args(["server", "--help"])
        .env_clear()
        .env("CAFLEET_BROKER_HOST", "0.0.0.0")
        .env("CAFLEET_BROKER_PORT", "9005")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let out = text(&output.stdout);
    assert!(out.contains("0.0.0.0"), "CAFLEET_BROKER_HOST is honored, got: {out}");
    assert!(out.contains("9005"), "CAFLEET_BROKER_PORT is honored, got: {out}");
}
