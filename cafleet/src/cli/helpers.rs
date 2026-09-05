//! Shared CLI plumbing: the schema-version and stale-assets guards, the
//! positional-`TEXT` / `--file` body reader, multiplexer resolution, and the
//! JSON-vs-text emit fork.

use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::Value;

use crate::broker::{asset_installs_table_exists, list_asset_installs};
use crate::config::Settings;
use crate::config_dir::{claude_config_dir, codex_home, opencode_preset_base};
use crate::error::CafleetError;
use crate::output::format_json;
pub use crate::runtime::resolve_mux;
use crate::runtime::system::read_stdin;

pub fn connect(settings: &Settings) -> Result<Connection, CafleetError> {
    crate::db::connect(&settings.database_url)
}

/// Each agent's recorded-path identity (SPEC §6.3 *Config-dir resolution*),
/// in the fixed `claude`, `codex`, `opencode` order.
fn identity_paths() -> Result<[(&'static str, String); 3], CafleetError> {
    let home = PathBuf::from(
        std::env::var("HOME").map_err(|_| CafleetError::App("HOME is not set".to_string()))?,
    );
    let env = |name: &str| std::env::var(name).ok();
    Ok([
        ("claude", claude_config_dir(&env, &home)?.path),
        ("codex", codex_home(&env, &home)?.path),
        ("opencode", opencode_preset_base(&env, &home)?.path),
    ]
    .map(|(agent, path)| (agent, path.display().to_string())))
}

/// The schema-version guard (SPEC §6.3): classifies the database against the
/// embedded head before any non-setup command body runs — ahead of the
/// stale-assets guard, so no missing or outdated schema state reaches
/// `asset_installs`. Connection-level failures keep their own errors.
pub fn schema_guard(settings: &Settings) -> Result<(), CafleetError> {
    let conn = connect(settings)?;
    let head = crate::db::head_version();
    match super::setup::recorded_version(&conn)? {
        Some(recorded) if recorded == head => Ok(()),
        Some(recorded) if recorded < head => Err(CafleetError::App(format!(
            "database schema is outdated (schema {recorded}, head {head}); run 'cafleet setup'"
        ))),
        Some(recorded) => Err(CafleetError::App(format!(
            "database schema {recorded} is newer than this cafleet (head {head}); upgrade cafleet"
        ))),
        None if super::setup::has_foreign_tables(&conn)? => Err(CafleetError::App(
            "database has tables but no schema history — not a cafleet database?".to_string(),
        )),
        None => Err(CafleetError::App(
            "no cafleet database; run 'cafleet setup'".to_string(),
        )),
    }
}

/// The stale-assets guard (SPEC §6.3): resolves each agent's identity path
/// and validates only the recorded rows at those paths before any
/// fleet-scoped subcommand body runs; superseded rows at other paths are
/// ignored.
pub fn stale_assets_guard(settings: &Settings) -> Result<(), CafleetError> {
    let identities = identity_paths()?;
    let recorded = match connect(settings) {
        Ok(conn) if asset_installs_table_exists(&conn) => list_asset_installs(&conn)?,
        _ => Vec::new(),
    };
    let current: Vec<(&str, &Value)> = identities
        .iter()
        .filter_map(|(agent, path)| {
            recorded
                .iter()
                .find(|row| row["coding_agent"] == *agent && row["path"] == path.as_str())
                .map(|row| (*agent, row))
        })
        .collect();
    if current.is_empty() {
        return Err(CafleetError::App(
            "no assets install is recorded at the resolved paths; \
             run 'cafleet setup' to install"
                .to_string(),
        ));
    }
    let stale: Vec<String> = current
        .iter()
        .filter(|(_, row)| row["cafleet_version"] != super::VERSION)
        .map(|(agent, row)| {
            format!(
                "{agent}={}",
                row["cafleet_version"]
                    .as_str()
                    .expect("rows carry the version")
            )
        })
        .collect();
    if stale.is_empty() {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "stale assets detected ({}; CLI {}); run 'cafleet setup' to reinstall",
            stale.join(", "),
            super::VERSION
        )))
    }
}

/// Render `path` with a `~` abbreviation when it sits under `home`.
pub(crate) fn tilde(path: &str, home: &std::path::Path) -> String {
    match std::path::Path::new(path).strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.to_string(),
    }
}

/// Whitespace per the empty-text contract: Unicode whitespace plus
/// U+001C–U+001F (SPEC §6.4 parity with `str.isspace`).
fn is_blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// The shared positional-`TEXT` / file-flag body reader (SPEC §6.3 *text-body
/// input*): `-` = stdin, raw-bytes UTF-8 decode with no newline translation,
/// uniform empty-body rejection. Exactly one source is present — clap's
/// required argument group enforces it at parse time. Error strings name the
/// file flag the caller exposes (`--file`, `--monitor-file`) via `flag`.
pub fn resolve_body(
    inline: Option<&str>,
    file: Option<&str>,
    flag: &str,
) -> Result<String, CafleetError> {
    match (inline, file) {
        (Some(text), None) => {
            if is_blank(text) {
                Err(CafleetError::Usage("text may not be empty.".to_string()))
            } else {
                Ok(text.to_string())
            }
        }
        (None, Some("-")) => {
            let bytes = read_stdin().map_err(|e| CafleetError::App(format!("{flag} -: {e}")))?;
            let body = String::from_utf8(bytes)
                .map_err(|_| CafleetError::App(format!("{flag} -: file is not valid UTF-8.")))?;
            if is_blank(&body) {
                Err(CafleetError::App(format!("{flag} -: stdin is empty.")))
            } else {
                Ok(body)
            }
        }
        (None, Some(path)) => {
            let bytes = std::fs::read(path).map_err(|e| {
                let message = match e.kind() {
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory => {
                        format!("{flag} {path}: file does not exist or is not a regular file.")
                    }
                    _ => format!("{flag} {path}: file is not readable."),
                };
                CafleetError::App(message)
            })?;
            let body = String::from_utf8(bytes).map_err(|_| {
                CafleetError::App(format!("{flag} {path}: file is not valid UTF-8."))
            })?;
            if is_blank(&body) {
                Err(CafleetError::App(format!("{flag} {path}: file is empty.")))
            } else {
                Ok(body)
            }
        }
        (Some(_), Some(_)) | (None, None) => {
            unreachable!("clap's required argument group supplies exactly one body source")
        }
    }
}

/// The JSON-vs-text emit fork shared by every one-shot subcommand.
pub fn emit(json: bool, payload: &Value, text: impl FnOnce() -> String) {
    if json {
        println!("{}", format_json(payload));
    } else {
        println!("{}", text());
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::InlinePreviewSender;
    use crate::config::Settings;
    use crate::runtime::RuntimeNotifier;

    fn settings(multiplexer: Option<&str>) -> Settings {
        Settings {
            database_url: "sqlite:///unused.db".to_string(),
            broker_host: "127.0.0.1".to_string(),
            broker_port: 8000,
            max_text_len: 200,
            multiplexer: multiplexer.map(str::to_string),
            monitor_wake_interval: 60,
        }
    }

    #[test]
    fn cli_notifier_construction_is_infallible_and_defers_the_resolution_error() {
        let notifier = RuntimeNotifier::new(&settings(Some("bogus")));
        let expected = "CAFLEET_MULTIPLEXER='bogus' is not a supported multiplexer \
                        (expected one of: herdr, tmux)";

        let err = notifier
            .send_inline_preview("%1", 7, 2, "2026-07-30T09:00:00.000000+00:00", "hi")
            .unwrap_err();
        assert_eq!(err, expected, "the retained resolve_mux error, verbatim");

        let err = notifier
            .send_inline_preview("%1", 8, 2, "2026-07-30T09:00:00.000000+00:00", "again")
            .unwrap_err();
        assert_eq!(
            err, expected,
            "the retained error survives repeated attempts"
        );
    }
}
