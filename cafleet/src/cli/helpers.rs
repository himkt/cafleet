//! Shared CLI plumbing: the stale-assets guard, the positional-`TEXT` /
//! `--file` body reader, multiplexer resolution, and the JSON-vs-text emit
//! fork.

use std::rc::Rc;

use rusqlite::Connection;
use serde_json::Value;

use super::system::{SystemRunner, read_stdin};
use crate::broker::{InlinePreviewSender, asset_installs_table_exists, list_asset_installs};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::{AnyMultiplexer, Multiplexer, MultiplexerError, resolve_multiplexer};
use crate::output::format_json;

pub fn connect(settings: &Settings) -> Result<Connection, CafleetError> {
    crate::db::connect(&settings.database_url)
}

/// The stale-assets guard (SPEC §6.3): validates the recorded installs before
/// any fleet-scoped subcommand body runs.
pub fn stale_assets_guard(settings: &Settings) -> Result<(), CafleetError> {
    let recorded = match connect(settings) {
        Ok(conn) if asset_installs_table_exists(&conn) => list_asset_installs(&conn)?,
        _ => Vec::new(),
    };
    if recorded.is_empty() {
        return Err(CafleetError::App(
            "no assets install is recorded; run 'cafleet setup' first".to_string(),
        ));
    }
    let stale: Vec<String> = recorded
        .iter()
        .filter(|row| row["cafleet_version"] != super::VERSION)
        .map(|row| {
            format!(
                "{}={}",
                row["coding_agent"].as_str().expect("rows carry the agent"),
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

/// Whitespace per the empty-text contract: Unicode whitespace plus
/// U+001C–U+001F (SPEC §6.4 parity with `str.isspace`).
fn is_blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// The shared positional-`TEXT` / `--file` body reader (SPEC §6.3 *text-body
/// input*): `-` = stdin, raw-bytes UTF-8 decode with no newline translation,
/// uniform empty-body rejection. Exactly one source is present — clap's
/// required argument group enforces it at parse time.
pub fn resolve_body(inline: Option<&str>, file: Option<&str>) -> Result<String, CafleetError> {
    match (inline, file) {
        (Some(text), None) => {
            if is_blank(text) {
                Err(CafleetError::Usage("text may not be empty.".to_string()))
            } else {
                Ok(text.to_string())
            }
        }
        (None, Some("-")) => {
            let bytes = read_stdin().map_err(|e| CafleetError::App(format!("--file -: {e}")))?;
            let body = String::from_utf8(bytes)
                .map_err(|_| CafleetError::App("--file -: file is not valid UTF-8.".to_string()))?;
            if is_blank(&body) {
                Err(CafleetError::App("--file -: stdin is empty.".to_string()))
            } else {
                Ok(body)
            }
        }
        (None, Some(path)) => {
            let bytes = std::fs::read(path).map_err(|e| {
                let message = match e.kind() {
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory => {
                        format!("--file {path}: file does not exist or is not a regular file.")
                    }
                    _ => format!("--file {path}: file is not readable."),
                };
                CafleetError::App(message)
            })?;
            let body = String::from_utf8(bytes).map_err(|_| {
                CafleetError::App(format!("--file {path}: file is not valid UTF-8."))
            })?;
            if is_blank(&body) {
                Err(CafleetError::App(format!("--file {path}: file is empty.")))
            } else {
                Ok(body)
            }
        }
        (Some(_), Some(_)) | (None, None) => {
            unreachable!("clap's required argument group supplies exactly one body source")
        }
    }
}

/// The environment snapshot the backends read presence variables from.
fn env_snapshot() -> std::collections::HashMap<String, String> {
    std::env::vars().collect()
}

pub fn resolve_mux(settings: &Settings) -> Result<AnyMultiplexer, MultiplexerError> {
    resolve_multiplexer(
        settings.multiplexer.as_deref(),
        env_snapshot(),
        Rc::new(SystemRunner),
    )
}

/// The broker-side preview notifier: the resolved multiplexer when one
/// exists, else a silent non-delivery (best-effort by contract).
pub struct CliNotifier {
    mux: Option<AnyMultiplexer>,
}

impl CliNotifier {
    pub fn new(settings: &Settings) -> Self {
        CliNotifier {
            mux: resolve_mux(settings).ok(),
        }
    }
}

impl InlinePreviewSender for CliNotifier {
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> bool {
        self.mux.as_ref().is_some_and(|mux| {
            mux.send_inline_preview(target_pane_id, message_id, sender_id, ts, text)
        })
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
