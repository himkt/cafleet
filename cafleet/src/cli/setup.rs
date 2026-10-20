//! `cafleet setup` — the single onboarding and schema-management entry point
//! (SPEC §6.3, §8): the refinery db half, then the offline embedded assets
//! half, failing independently.

use std::path::PathBuf;

use clap::Args;
use rusqlite::Connection;

use super::InvocationHooks;
use crate::assets::{TARGET_AGENTS, agent_paths, install_agent};
use crate::config::Settings;
use crate::diagnosis::{SchemaState, asset_table_exists};
use crate::error::CafleetError;

#[derive(Args)]
pub struct SetupArgs {
    /// Install the named agent's assets (space-delimited, repeatable; default: all agents).
    #[arg(long = "coding-agent", value_name = "AGENT", num_args = 1.., value_parser = ["claude", "codex", "opencode"])]
    coding_agent: Vec<String>,
}

pub fn run(
    settings: &Settings,
    args: SetupArgs,
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    let mut slot = None;
    let mut failed_halves = Vec::new();
    let db_result = db_half_in_slot(settings, &mut slot, hooks);
    if let Err(error) = db_result {
        println!("db half failed: {}", error.message());
        failed_halves.push("db");
    }
    let assets_result = (|| {
        if slot.is_none() {
            slot = Some((hooks.connect)(&settings.database_url)?);
        }
        assets_half(
            slot.as_mut().expect("assets connection opened"),
            &args.coding_agent,
            hooks.asset_env,
        )
    })();
    if let Err(error) = assets_result {
        println!("assets half failed: {}", error.message());
        failed_halves.push("assets");
    }
    if failed_halves.is_empty() {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "{} half failed",
            failed_halves.join(" and ")
        )))
    }
}

/// Retain ownership across the two independent setup halves.
fn db_half_in_slot(
    settings: &Settings,
    slot: &mut Option<Connection>,
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    let path = settings
        .database_url
        .strip_prefix("sqlite:///")
        .ok_or_else(|| {
            CafleetError::App(format!(
                "database URL must use the sqlite scheme (sqlite:///<path>); got '{}'",
                settings.database_url
            ))
        })?;
    if path.is_empty() {
        return Err(CafleetError::App(
            "database URL has no file path".to_string(),
        ));
    }
    let db_file = PathBuf::from(path);
    if let Some(parent) = db_file.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| CafleetError::App(format!("cannot create {}: {e}", parent.display())))?;
    }

    *slot = Some((hooks.connect)(&settings.database_url)?);
    let conn = slot.as_mut().expect("database connection opened");
    let head = crate::db::head_version();
    let schema = crate::diagnosis::classify_schema(conn, head);
    let recorded = match schema {
        SchemaState::Unreachable { cause } => return Err(cause),
        SchemaState::Unversioned => return Err(CafleetError::App(
            "DB has existing tables but no refinery_schema_history. Refusing to migrate an unversioned database.".into()
        )),
        SchemaState::Missing => None,
        SchemaState::Behind { recorded, .. } | SchemaState::Ahead { recorded, .. } => Some(recorded),
        SchemaState::Head { version } => Some(version),
    };
    if let Some(version) = recorded {
        if version > head {
            return Err(CafleetError::App(format!(
                "DB schema is at version {version} which is unknown to this version \
                 of cafleet. Refusing to downgrade automatically."
            )));
        }
        if version == head {
            println!("Already at head ({head}); nothing to do.");
            return Ok(());
        }
    }
    crate::db::migrate_to_head(conn)?;
    let after = crate::diagnosis::classify_schema(conn, head);
    super::helpers::schema_guard(&after)?;
    match recorded {
        None => println!(
            "Created {} and applied migrations to head ({head}).",
            db_file.display()
        ),
        Some(version) => println!("Upgraded from {version} to {head}."),
    }
    Ok(())
}

/// The assets half (SPEC §6.3): the explicit selector installs exactly the
/// named agents; the no-flag form installs all three. An install failure
/// aborts the loop; rows recorded before the failure remain.
fn assets_half(
    conn: &mut Connection,
    selected: &[String],
    env: crate::config_dir::EnvLookup<'_>,
) -> Result<(), CafleetError> {
    if !asset_table_exists(conn)? {
        return Err(CafleetError::App(
            "the database schema is missing or outdated; run 'cafleet setup' first".into(),
        ));
    }
    let home = PathBuf::from(
        std::env::var("HOME").map_err(|_| CafleetError::App("HOME is not set".into()))?,
    );
    for agent in TARGET_AGENTS {
        if !selected.is_empty() && !selected.iter().any(|value| value == agent) {
            continue;
        }
        let paths = agent_paths(env, &home, agent)?;
        install_agent(conn, agent, &paths, super::VERSION)?;
    }
    Ok(())
}

#[cfg(test)]
mod parser_contracts {
    use crate::cli::{CliArgs, Command};
    use clap::Parser;

    #[test]
    fn setup_selections_preserve_order_and_duplicates_across_flag_forms() {
        let cases: &[(&[&str], &[&str])] = &[
            (&["cafleet", "setup"], &[]),
            (&["cafleet", "setup", "--coding-agent", "codex"], &["codex"]),
            (
                &["cafleet", "setup", "--coding-agent", "claude", "codex"],
                &["claude", "codex"],
            ),
            (
                &[
                    "cafleet",
                    "setup",
                    "--coding-agent",
                    "claude",
                    "--coding-agent",
                    "codex",
                ],
                &["claude", "codex"],
            ),
            (
                &[
                    "cafleet",
                    "setup",
                    "--coding-agent",
                    "claude",
                    "--coding-agent",
                    "codex",
                    "--coding-agent",
                    "opencode",
                ],
                &["claude", "codex", "opencode"],
            ),
            (
                &[
                    "cafleet",
                    "setup",
                    "--coding-agent",
                    "claude",
                    "codex",
                    "--coding-agent",
                    "opencode",
                ],
                &["claude", "codex", "opencode"],
            ),
            (
                &[
                    "cafleet",
                    "setup",
                    "--coding-agent",
                    "opencode",
                    "claude",
                    "--coding-agent",
                    "claude",
                ],
                &["opencode", "claude", "claude"],
            ),
        ];
        for (argv, expected) in cases {
            let Command::Setup(args) = CliArgs::try_parse_from(*argv).unwrap().command else {
                panic!("{argv:?} must parse as setup");
            };
            assert_eq!(args.coding_agent, *expected, "{argv:?}");
        }
    }
}
