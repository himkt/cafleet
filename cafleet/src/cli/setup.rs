//! `cafleet setup` — the single onboarding and schema-management entry point
//! (SPEC §6.3, §8): the refinery db half, then the offline embedded assets
//! half, failing independently.

use std::path::PathBuf;

use clap::Args;
use rusqlite::Connection;

use crate::assets::{TARGET_AGENTS, agent_paths, install_agent};
use crate::broker::asset_installs_table_exists;
use crate::config::Settings;
use crate::error::CafleetError;

#[derive(Args)]
pub struct SetupArgs {
    /// Install the named agent's assets (space-delimited, repeatable; default: all agents).
    #[arg(long = "coding-agent", value_name = "AGENT", num_args = 1.., value_parser = ["claude", "codex", "opencode"])]
    coding_agent: Vec<String>,
}

pub fn run(settings: &Settings, args: SetupArgs) -> Result<(), CafleetError> {
    let mut failed_halves: Vec<&str> = Vec::new();

    if let Err(error) = db_half(settings) {
        println!("db half failed: {}", error.message());
        failed_halves.push("db");
    }

    if let Err(error) = assets_half(settings, &args.coding_agent) {
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

/// The db-migration driver (SPEC §8): scheme validation, the two refusals,
/// and the three head-migration messages.
fn db_half(settings: &Settings) -> Result<(), CafleetError> {
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

    let mut conn = crate::db::connect(&settings.database_url)?;
    let head = crate::db::head_version();
    let recorded = recorded_version(&conn)?;
    if recorded.is_none() && has_foreign_tables(&conn)? {
        return Err(CafleetError::App(
            "DB has existing tables but no refinery_schema_history. \
             Refusing to migrate an unversioned database."
                .to_string(),
        ));
    }
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
    crate::db::migrate_to_head(&mut conn)?;
    match recorded {
        None => println!(
            "Created {} and applied migrations to head ({head}).",
            db_file.display()
        ),
        Some(version) => println!("Upgraded from {version} to {head}."),
    }
    Ok(())
}

/// The applied-migration high-water mark, `None` when the ledger is absent
/// or empty.
pub(super) fn recorded_version(conn: &Connection) -> Result<Option<u32>, CafleetError> {
    let ledger_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type='table' AND name='refinery_schema_history')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| CafleetError::App(format!("database error: {e}")))?;
    if !ledger_exists {
        return Ok(None);
    }
    conn.query_row(
        "SELECT MAX(version) FROM refinery_schema_history",
        [],
        |row| row.get::<_, Option<u32>>(0),
    )
    .map_err(|e| CafleetError::App(format!("database error: {e}")))
}

/// Whether any table outside the ledger (and SQLite's own bookkeeping)
/// exists.
pub(super) fn has_foreign_tables(conn: &Connection) -> Result<bool, CafleetError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' \
         AND name NOT LIKE 'sqlite_%' AND name != 'refinery_schema_history')",
        [],
        |row| row.get(0),
    )
    .map_err(|e| CafleetError::App(format!("database error: {e}")))
}

/// The assets half (SPEC §6.3): the explicit selector installs exactly the
/// named agents; the no-flag form installs all three. An install failure
/// aborts the loop; rows recorded before the failure remain.
fn assets_half(settings: &Settings, selected: &[String]) -> Result<(), CafleetError> {
    let mut conn = crate::db::connect(&settings.database_url)?;
    if !asset_installs_table_exists(&conn) {
        return Err(CafleetError::App(
            "the database schema is missing or outdated; run 'cafleet setup' first".to_string(),
        ));
    }
    let home = PathBuf::from(
        std::env::var("HOME").map_err(|_| CafleetError::App("HOME is not set".to_string()))?,
    );
    let env = |name: &str| std::env::var(name).ok();

    for agent in TARGET_AGENTS {
        if !selected.is_empty() && !selected.iter().any(|s| s == agent) {
            continue;
        }
        let paths = agent_paths(&env, &home, agent)?;
        install_agent(&mut conn, agent, &paths, super::VERSION)?;
    }
    Ok(())
}
