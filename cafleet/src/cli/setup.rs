//! `cafleet setup` — the single onboarding and schema-management entry point
//! (SPEC §6.3, §8): the refinery db half, then the offline embedded assets
//! half, failing independently.

use std::{collections::BTreeMap, path::PathBuf};

use clap::Args;
use rusqlite::Connection;

use super::{InvocationEvent, InvocationHooks, InvocationPhase, SchemaPoint, inspect_schema};
use crate::assets::{TARGET_AGENTS, agent_paths, install_agent};
use crate::config::Settings;
#[cfg(test)]
use crate::diagnosis::recorded_version;
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
    let db_result = db_half_in_slot(settings, &mut slot, hooks, || {});
    (hooks.observe)(InvocationEvent::Finished {
        phase: InvocationPhase::SetupDatabase,
        conn: slot.as_ref(),
        result: &db_result,
    });
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
    (hooks.observe)(InvocationEvent::Finished {
        phase: InvocationPhase::SetupAssets,
        conn: slot.as_ref(),
        result: &assets_result,
    });
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

// Preserve the existing per-call migration-race seam for its unit tests.
#[cfg(test)]
fn db_half_with_after_diagnosis(
    settings: &Settings,
    after_diagnosis: impl FnOnce(),
) -> Result<(), CafleetError> {
    db_half_in_slot(
        settings,
        &mut None,
        &InvocationHooks {
            connect: &crate::db::connect,
            observe: &|_| {},
            asset_env: &|name| std::env::var(name).ok(),
        },
        after_diagnosis,
    )
}

/// Retain ownership across the two independent setup halves.
fn db_half_in_slot(
    settings: &Settings,
    slot: &mut Option<Connection>,
    hooks: &InvocationHooks<'_>,
    after_diagnosis: impl FnOnce(),
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
    let schema = inspect_schema(conn, SchemaPoint::SetupBefore, hooks);
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
    if let Some(diagnostic) = duplicate_monitor_diagnostic(conn)? {
        return Err(CafleetError::App(diagnostic));
    }
    after_diagnosis();
    if let Err(original) = crate::db::migrate_to_head(conn) {
        // The diagnostic and migration do not hold a shared transaction. The
        // index remains authoritative if a writer committed between them.
        if let Ok(Some(diagnostic)) = duplicate_monitor_diagnostic(conn) {
            return Err(CafleetError::App(diagnostic));
        }
        return Err(original);
    }
    let after = inspect_schema(conn, SchemaPoint::SetupAfter, hooks);
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

fn duplicate_monitor_diagnostic(conn: &Connection) -> Result<Option<String>, CafleetError> {
    let db_error = |error| CafleetError::App(format!("database error: {error}"));
    let members_exist: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='members')",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if !members_exist {
        return Ok(None);
    }
    let mut statement = conn
        .prepare(
            "SELECT fleet_id, member_id FROM members WHERE status='active' \
             AND json_extract(member_card_json, '$.cafleet.kind')='monitor' \
             ORDER BY fleet_id, member_id",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(db_error)?;
    let mut fleets: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for row in rows {
        let (fleet_id, member_id) = row.map_err(db_error)?;
        fleets.entry(fleet_id).or_default().push(member_id);
    }
    let conflicts: Vec<String> = fleets
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(fleet, members)| {
            let ids = members
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("fleet {fleet}: members {ids}")
        })
        .collect();
    Ok((!conflicts.is_empty()).then(|| {
        format!(
            "active monitor duplicates prevent migration: {}",
            conflicts.join("; ")
        )
    }))
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
            "the database schema is missing or outdated; run 'cafleet setup' first".to_string(),
        ));
    }
    let home = PathBuf::from(
        std::env::var("HOME").map_err(|_| CafleetError::App("HOME is not set".to_string()))?,
    );

    for agent in TARGET_AGENTS {
        if !selected.is_empty() && !selected.iter().any(|s| s == agent) {
            continue;
        }
        let paths = agent_paths(env, &home, agent)?;
        install_agent(conn, agent, &paths, super::VERSION)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::types::Value;
    use std::cell::Cell;
    use tempfile::TempDir;

    mod embedded {
        refinery::embed_migrations!("migrations");
    }

    struct Fixture {
        _dir: TempDir,
        settings: Settings,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .prefix(".setup-race-")
                .tempdir_in(env!("CARGO_MANIFEST_DIR"))
                .unwrap();
            let url = format!("sqlite:///{}", dir.path().join("fixture.db").display());
            let settings =
                Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone()))
                    .unwrap();
            Self {
                _dir: dir,
                settings,
            }
        }

        fn connect(&self) -> Connection {
            crate::db::connect(&self.settings.database_url).unwrap()
        }

        fn seed_version(&self, version: i32) -> Connection {
            let mut conn = self.connect();
            embedded::migrations::runner()
                .set_target(refinery::Target::Version(version))
                .run(&mut conn)
                .unwrap();
            conn
        }
    }

    fn rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut statement = conn.prepare(sql).unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| (0..count).map(|i| row.get(i)).collect())
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn snapshot(conn: &Connection) -> Vec<Vec<Vec<Value>>> {
        let mut data = vec![rows(
            conn,
            "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name",
        )];
        for table in [
            "refinery_schema_history",
            "fleets",
            "members",
            "member_placements",
            "messages",
            "monitor_runtime",
            "asset_installs",
            "sqlite_sequence",
        ] {
            data.push(rows(conn, &format!("SELECT * FROM {table} ORDER BY 1")));
        }
        data
    }

    fn seed_clean_monitors(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO fleets(fleet_id, name, created_at) VALUES (20, 'twenty', 'fixture'), (3, 'three', 'fixture');
             INSERT INTO members(member_id, fleet_id, name, description, status, registered_at, member_card_json) VALUES
                (30, 20, 'm30', '', 'active', 'fixture', '{\"cafleet\":{\"kind\":\"monitor\"}}'),
                (2, 3, 'm2', '', 'active', 'fixture', '{\"cafleet\":{\"kind\":\"monitor\"}}');"
        ).unwrap();
    }

    fn insert_competing_monitors(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO members(member_id, fleet_id, name, description, status, registered_at, member_card_json) VALUES
                (90, 20, 'm90', '', 'active', 'fixture', '{\"cafleet\":{\"kind\":\"monitor\"}}'),
                (8, 3, 'm8', '', 'active', 'fixture', '{\"cafleet\":{\"kind\":\"monitor\"}}');"
        ).unwrap();
    }

    const DUPLICATES: &str = "active monitor duplicates prevent migration: fleet 3: members 2, 8; fleet 20: members 30, 90";

    #[test]
    fn setup_rediagnoses_competing_commit_and_preserves_pending_schema_and_data() {
        for version in [5, 7] {
            let fixture = Fixture::new();
            let observer = fixture.seed_version(version);
            seed_clean_monitors(&observer);
            assert!(duplicate_monitor_diagnostic(&observer).unwrap().is_none());
            let mut after_competing_commit = None;
            let mut calls = 0;
            let error = db_half_with_after_diagnosis(&fixture.settings, || {
                calls += 1;
                let writer = fixture.connect();
                insert_competing_monitors(&writer);
                assert!(writer.is_autocommit());
                after_competing_commit = Some(snapshot(&writer));
            })
            .unwrap_err();
            assert_eq!(calls, 1);
            assert_eq!(error.message(), DUPLICATES);
            assert_eq!(error.exit_code(), 1);
            assert_eq!(snapshot(&observer), after_competing_commit.unwrap());
            assert_eq!(recorded_version(&observer).unwrap(), Some(version as u32));
        }
    }

    #[test]
    fn setup_skips_callback_and_duplicate_query_for_unversioned_database() {
        let fixture = Fixture::new();
        let conn = fixture.connect();
        conn.execute_batch("CREATE TABLE members (deliberately_invalid TEXT)")
            .unwrap();
        let called = Cell::new(false);
        let error =
            db_half_with_after_diagnosis(&fixture.settings, || called.set(true)).unwrap_err();
        assert!(!called.get());
        assert!(
            error
                .message()
                .contains("Refusing to migrate an unversioned database")
        );
        assert_eq!(recorded_version(&conn).unwrap(), None);
    }

    #[test]
    fn setup_skips_callback_and_duplicate_diagnosis_for_ahead_schema() {
        let fixture = Fixture::new();
        let conn = fixture.seed_version(7);
        seed_clean_monitors(&conn);
        insert_competing_monitors(&conn);
        conn.execute_batch(
            "INSERT INTO refinery_schema_history VALUES(99, 'future', 'fixture', '0')",
        )
        .unwrap();
        let before = snapshot(&conn);
        let called = Cell::new(false);
        let error =
            db_half_with_after_diagnosis(&fixture.settings, || called.set(true)).unwrap_err();
        assert!(!called.get());
        assert!(error.message().contains("version 99 which is unknown"));
        assert!(!error.message().contains("duplicates"));
        assert_eq!(snapshot(&conn), before);
    }

    #[test]
    fn setup_skips_callback_for_head_schema_and_preserves_everything() {
        let fixture = Fixture::new();
        let conn = fixture.seed_version(8);
        seed_clean_monitors(&conn);
        let before = snapshot(&conn);
        let called = Cell::new(false);
        db_half_with_after_diagnosis(&fixture.settings, || called.set(true)).unwrap();
        assert!(!called.get());
        assert_eq!(snapshot(&conn), before);
    }

    #[test]
    fn setup_skips_callback_when_duplicates_exist_before_migration() {
        let fixture = Fixture::new();
        let conn = fixture.seed_version(7);
        seed_clean_monitors(&conn);
        insert_competing_monitors(&conn);
        let before = snapshot(&conn);
        let called = Cell::new(false);
        let error =
            db_half_with_after_diagnosis(&fixture.settings, || called.set(true)).unwrap_err();
        assert!(!called.get());
        assert_eq!(error.message(), DUPLICATES);
        assert_eq!(snapshot(&conn), before);
    }

    #[test]
    fn setup_calls_callback_once_for_clean_pending_migrations() {
        let fixture = Fixture::new();
        let conn = fixture.seed_version(7);
        seed_clean_monitors(&conn);
        let members = rows(&conn, "SELECT * FROM members ORDER BY member_id");
        let mut calls = 0;
        db_half_with_after_diagnosis(&fixture.settings, || calls += 1).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(recorded_version(&conn).unwrap(), Some(8));
        assert_eq!(
            rows(&conn, "SELECT * FROM members ORDER BY member_id"),
            members
        );
    }

    #[test]
    fn setup_migrates_a_fresh_database_without_requiring_a_members_table() {
        let fixture = Fixture::new();
        let mut calls = 0;
        db_half_with_after_diagnosis(&fixture.settings, || calls += 1).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(recorded_version(&fixture.connect()).unwrap(), Some(8));
    }

    #[test]
    fn setup_preserves_unrelated_migration_error_after_clean_rediagnosis() {
        let fixture = Fixture::new();
        let observer = fixture.seed_version(5);
        seed_clean_monitors(&observer);
        let mut after_interference = None;
        let error = db_half_with_after_diagnosis(&fixture.settings, || {
            let writer = fixture.connect();
            writer
                .execute_batch(
                    "CREATE TABLE idx_members_one_active_monitor_per_fleet (fixture TEXT)",
                )
                .unwrap();
            after_interference = Some(snapshot(&writer));
        })
        .unwrap_err();
        assert!(error.message().starts_with("migration failed:"));
        assert!(
            error
                .message()
                .contains("idx_members_one_active_monitor_per_fleet")
        );
        assert!(!error.message().contains("duplicates"));
        assert_eq!(snapshot(&observer), after_interference.unwrap());
    }

    #[test]
    fn setup_preserves_original_migration_error_when_rediagnosis_also_fails() {
        let fixture = Fixture::new();
        let observer = fixture.seed_version(7);
        let mut after_interference = None;
        let error = db_half_with_after_diagnosis(&fixture.settings, || {
            let writer = fixture.connect();
            writer
                .execute_batch("ALTER TABLE members RENAME COLUMN member_card_json TO broken_card")
                .unwrap();
            after_interference = Some(snapshot(&writer));
        })
        .unwrap_err();
        assert!(error.message().starts_with("migration failed:"), "{error}");
        assert!(error.message().contains("member_card_json"), "{error}");
        assert!(duplicate_monitor_diagnostic(&observer).is_err());
        assert_eq!(snapshot(&observer), after_interference.unwrap());
    }
}
