//! Shared CLI plumbing: the schema-version and stale-assets guards, the
//! positional-`TEXT` / `--file` body reader, multiplexer resolution, and the
//! JSON-vs-text emit fork.

use serde_json::Value;

use crate::error::CafleetError;
use crate::output::format_json;
pub use crate::runtime::resolve_mux;
use crate::runtime::system::read_stdin;

/// Preserve the CLI guard's wording while sharing schema classification.
pub(crate) fn schema_guard(schema: &crate::diagnosis::SchemaState) -> Result<(), CafleetError> {
    use crate::diagnosis::SchemaState;
    match schema {
        SchemaState::Head { .. } => Ok(()),
        SchemaState::Behind { recorded, head } => Err(CafleetError::App(format!(
            "database schema is outdated (schema {recorded}, head {head}); run 'cafleet setup'"
        ))),
        SchemaState::Ahead { recorded, head } => Err(CafleetError::App(format!(
            "database schema {recorded} is newer than this cafleet (head {head}); upgrade cafleet"
        ))),
        SchemaState::Unversioned => Err(CafleetError::App(
            "database has tables but no schema history — not a cafleet database?".into(),
        )),
        SchemaState::Missing => Err(CafleetError::App(
            "no cafleet database; run 'cafleet setup'".into(),
        )),
        SchemaState::Unreachable { cause } => Err(cause.clone()),
    }
}

pub(crate) fn stale_assets_guard(
    assets: &crate::diagnosis::AssetReport,
    cli_version: &str,
) -> Result<(), CafleetError> {
    use crate::diagnosis::AssetState;
    for agent in &assets.agents {
        if let AssetState::PathError { cause, .. } = &agent.state {
            return Err(cause.clone());
        }
    }
    let mut current = 0;
    let mut stale = Vec::new();
    for agent in &assets.agents {
        match &agent.state {
            AssetState::PathError { cause, .. } => return Err(cause.clone()),
            AssetState::Incomplete { recovery, .. } => {
                return Err(CafleetError::App(recovery.diagnostic()));
            }
            AssetState::Current { .. } => current += 1,
            AssetState::Stale { install, .. } => {
                current += 1;
                stale.push(format!(
                    "{}={}",
                    agent.coding_agent, install.cafleet_version
                ));
            }
            AssetState::NotInstalled { .. } => {}
        }
    }
    if current == 0 {
        return Err(CafleetError::App(
            "no assets install is recorded at the resolved paths; run 'cafleet setup' to install"
                .into(),
        ));
    }
    if !stale.is_empty() {
        return Err(CafleetError::App(format!(
            "stale assets detected ({}; CLI {cli_version}); run 'cafleet setup' to reinstall",
            stale.join(", ")
        )));
    }
    Ok(())
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
