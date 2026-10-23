use rusqlite::{Connection, OptionalExtension};

use crate::error::CafleetError;

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// The embedded `V<N>__<slug>.sql` chain (SPEC §8) as the refinery runner's
/// `(version, slug)` pairs, sorted ascending.
pub fn migration_chain() -> Vec<(u32, String)> {
    let mut chain: Vec<(u32, String)> = embedded::migrations::runner()
        .get_migrations()
        .iter()
        .map(|migration| {
            let version = u32::try_from(migration.version())
                .expect("embedded migration versions are positive");
            (version, migration.name().to_string())
        })
        .collect();
    chain.sort();
    chain
}

/// The file path a `sqlite:///<path>` database URL names.
pub fn database_path(database_url: &str) -> Result<&str, CafleetError> {
    database_url.strip_prefix("sqlite:///").ok_or_else(|| {
        CafleetError::App(format!(
            "database URL must use the sqlite scheme (sqlite:///<path>); got '{database_url}'"
        ))
    })
}

/// Open a connection to `database_url` (the `sqlite:///<path>` form only) and
/// apply the mandatory per-connection PRAGMAs (SPEC §6.1).
pub fn connect(database_url: &str) -> Result<Connection, CafleetError> {
    let path = database_path(database_url)?;
    let conn = Connection::open(path)
        .map_err(|e| CafleetError::App(format!("failed to open database at '{path}': {e}")))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .map_err(|e| CafleetError::App(format!("failed to apply connection PRAGMAs: {e}")))?;
    Ok(conn)
}

/// Apply the embedded migration chain to head (idempotent) and return the
/// head version.
pub fn migrate_to_head(conn: &mut Connection) -> Result<u32, CafleetError> {
    embedded::migrations::runner()
        .set_grouped(true)
        .run(conn)
        .map_err(|e| CafleetError::App(format!("migration failed: {e}")))?;
    Ok(head_version())
}

/// The embedded chain's head version.
pub fn head_version() -> u32 {
    migration_chain()
        .last()
        .expect("the embedded migration chain has at least the baseline")
        .0
}

pub(crate) fn require_current(conn: &Connection) -> Result<(), CafleetError> {
    crate::diagnosis::classify_schema(conn, head_version()).require_current()
}

pub const APPLICATION_ID: i64 = 1128351288;

fn metadata_rows(
    conn: &Connection,
    sql: &str,
    name: &str,
) -> Result<Vec<Vec<rusqlite::types::Value>>, CafleetError> {
    let mut statement = conn.prepare(sql).map_err(crate::broker::members::db_err)?;
    let rows = statement
        .query_map([name], |row| {
            (0..row.as_ref().column_count())
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(crate::broker::members::db_err)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(crate::broker::members::db_err)
}

pub(crate) fn current_schema_matches(conn: &Connection) -> Result<bool, CafleetError> {
    let mut expected = Connection::open_in_memory().map_err(crate::broker::members::db_err)?;
    migrate_to_head(&mut expected)?;
    let mut tables = expected.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).map_err(crate::broker::members::db_err)?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(crate::broker::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::broker::members::db_err)?;
    for name in names {
        for sql in [
            "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid",
            "SELECT \"table\", \"from\", \"to\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list(?1) ORDER BY \"from\", \"to\"",
        ] {
            if metadata_rows(conn, sql, &name)? != metadata_rows(&expected, sql, &name)? {
                return Ok(false);
            }
        }
        let mut indexes = expected
            .prepare("SELECT name, \"unique\", partial FROM pragma_index_list(?1)")
            .map_err(crate::broker::members::db_err)?;
        let indexes = indexes
            .query_map([&name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(crate::broker::members::db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::broker::members::db_err)?;
        for (index, unique, partial) in indexes {
            let definition: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT \"unique\", partial FROM pragma_index_list(?1) WHERE name=?2",
                    rusqlite::params![name, index],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(crate::broker::members::db_err)?;
            if definition != Some((unique, partial))
                || metadata_rows(
                    conn,
                    "SELECT seqno, cid, name FROM pragma_index_info(?1) ORDER BY seqno",
                    &index,
                )? != metadata_rows(
                    &expected,
                    "SELECT seqno, cid, name FROM pragma_index_info(?1) ORDER BY seqno",
                    &index,
                )?
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use tempfile::TempDir;

    use super::*;

    fn temp_db_url(dir: &TempDir) -> String {
        format!("sqlite:///{}", dir.path().join("core_db_test.db").display())
    }

    fn migrated_conn(dir: &TempDir) -> Connection {
        let mut conn = connect(&temp_db_url(dir)).unwrap();
        migrate_to_head(&mut conn).unwrap();
        conn
    }

    fn table_sql(conn: &Connection, table: &str) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn column_info(conn: &Connection, table: &str, column: &str) -> (i64, Option<String>) {
        conn.query_row(
            &format!(
                "SELECT \"notnull\", dflt_value FROM pragma_table_info('{table}') WHERE name=?1"
            ),
            [column],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    fn fk_rule(conn: &Connection, table: &str, from_column: &str) -> (String, String) {
        conn.query_row(
            &format!(
                "SELECT \"table\", on_delete FROM pragma_foreign_key_list('{table}') WHERE \"from\"=?1"
            ),
            [from_column],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    const APP_TABLES: [&str; 5] = [
        "members",
        "fleets",
        "asset_installs",
        "member_placements",
        "messages",
    ];

    #[test]
    fn connect_applies_the_foreign_keys_pragma() {
        let dir = TempDir::new().unwrap();
        let conn = connect(&temp_db_url(&dir)).unwrap();
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn connect_applies_the_busy_timeout_pragma() {
        let dir = TempDir::new().unwrap();
        let conn = connect(&temp_db_url(&dir)).unwrap();
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);
    }

    #[test]
    fn connect_rejects_non_sqlite_schemes() {
        assert!(connect("postgresql:///registry").is_err());
        assert!(connect("mysql://localhost/registry").is_err());
    }

    #[test]
    fn migrate_reaches_head_version_1_and_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let mut conn = connect(&temp_db_url(&dir)).unwrap();
        assert_eq!(migrate_to_head(&mut conn).unwrap(), 1);
        assert_eq!(migrate_to_head(&mut conn).unwrap(), 1);
    }

    #[test]
    fn baseline_sets_the_current_application_identity() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let identity: i64 = conn
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap();
        assert_eq!(identity, 1128351288);
    }

    #[test]
    fn baseline_columns_match_the_current_storage_contract() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        for (table, expected) in [
            (
                "members",
                vec![
                    ("member_id", "INTEGER", 1, 1),
                    ("fleet_id", "INTEGER", 1, 0),
                    ("name", "TEXT", 1, 0),
                    ("description", "TEXT", 1, 0),
                    ("status", "TEXT", 1, 0),
                    ("registered_at", "TEXT", 1, 0),
                    ("deregistered_at", "TEXT", 0, 0),
                ],
            ),
            (
                "fleets",
                vec![
                    ("fleet_id", "INTEGER", 1, 1),
                    ("name", "TEXT", 0, 0),
                    ("created_at", "TEXT", 1, 0),
                    ("deleted_at", "TEXT", 0, 0),
                    ("director_member_id", "INTEGER", 0, 0),
                ],
            ),
            (
                "asset_installs",
                vec![
                    ("coding_agent", "TEXT", 1, 1),
                    ("path", "TEXT", 1, 2),
                    ("cafleet_version", "TEXT", 1, 0),
                    ("installed_at", "TEXT", 1, 0),
                ],
            ),
            (
                "member_placements",
                vec![
                    ("member_id", "INTEGER", 1, 1),
                    ("mux_session", "TEXT", 1, 0),
                    ("mux_window_id", "TEXT", 1, 0),
                    ("mux_pane_id", "TEXT", 0, 0),
                    ("backend", "TEXT", 1, 0),
                    ("coding_agent", "TEXT", 1, 0),
                    ("created_at", "TEXT", 1, 0),
                ],
            ),
            (
                "messages",
                vec![
                    ("message_id", "INTEGER", 1, 1),
                    ("from_member_id", "INTEGER", 1, 0),
                    ("to_member_id", "INTEGER", 1, 0),
                    ("created_at", "TEXT", 1, 0),
                    ("status_state", "TEXT", 1, 0),
                    ("status_timestamp", "TEXT", 1, 0),
                    ("text", "TEXT", 1, 0),
                    ("notified_at", "TEXT", 0, 0),
                ],
            ),
        ] {
            let mut statement = conn
                .prepare(&format!("PRAGMA table_info('{table}')"))
                .unwrap();
            let actual = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(5)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            let expected = expected
                .into_iter()
                .map(|(name, kind, required, pk)| (name.to_owned(), kind.to_owned(), required, pk))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{table}");
        }
    }

    #[test]
    fn failed_baseline_initialization_rolls_back_identity_tables_and_ledger() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA max_page_count=1").unwrap();
        assert!(migrate_to_head(&mut conn).is_err());
        assert!(conn.is_autocommit());
        let identity: i64 = conn
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap();
        let tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!((identity, tables), (0, 0));
    }

    #[test]
    fn baseline_creates_exactly_the_head_tables() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap();
        let names: BTreeSet<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: BTreeSet<String> = APP_TABLES
            .iter()
            .chain(std::iter::once(&"refinery_schema_history"))
            .map(|s| s.to_string())
            .collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn generated_id_tables_use_autoincrement() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        for table in ["fleets", "members", "messages"] {
            assert!(
                table_sql(&conn, table).contains("AUTOINCREMENT"),
                "{table} must AUTOINCREMENT"
            );
        }
        for table in ["asset_installs", "member_placements"] {
            assert!(
                !table_sql(&conn, table).contains("AUTOINCREMENT"),
                "{table} must not AUTOINCREMENT"
            );
        }
    }

    #[test]
    fn application_indexes_match_the_current_query_keys() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let placeholders = APP_TABLES.map(|t| format!("'{t}'")).join(",");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT name FROM sqlite_master WHERE type='index' \
                 AND name NOT LIKE 'sqlite_%' AND tbl_name IN ({placeholders})"
            ))
            .unwrap();
        let names: BTreeSet<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: BTreeSet<String> = [
            "idx_members_fleet_status",
            "idx_messages_to_member_status_ts",
            "idx_messages_from_member_status_ts",
        ]
        .map(String::from)
        .into();
        assert_eq!(names, expected);
    }

    #[test]
    fn ddl_defaults_match_the_head_schema() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        assert_eq!(
            column_info(&conn, "member_placements", "backend").1,
            Some("'tmux'".to_string())
        );
        let (notnull, default) = column_info(&conn, "member_placements", "coding_agent");
        assert_eq!(
            (notnull, default),
            (1, None),
            "coding_agent: NOT NULL, no DDL default"
        );
    }

    #[test]
    fn nullability_matches_the_head_schema() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        assert_eq!(column_info(&conn, "messages", "from_member_id").0, 1);
        assert_eq!(column_info(&conn, "messages", "to_member_id").0, 1);
        assert_eq!(column_info(&conn, "messages", "notified_at").0, 0);
        assert_eq!(column_info(&conn, "messages", "text").0, 1);
        assert_eq!(column_info(&conn, "fleets", "director_member_id").0, 0);
        assert_eq!(column_info(&conn, "fleets", "name").0, 0);
        assert_eq!(column_info(&conn, "members", "deregistered_at").0, 0);
        assert_eq!(column_info(&conn, "member_placements", "mux_pane_id").0, 0);
    }

    #[test]
    fn foreign_key_on_delete_rules_match_the_head_schema() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        assert_eq!(
            fk_rule(&conn, "members", "fleet_id"),
            ("fleets".to_string(), "RESTRICT".to_string())
        );
        assert_eq!(
            fk_rule(&conn, "fleets", "director_member_id"),
            ("members".to_string(), "RESTRICT".to_string())
        );
        assert_eq!(
            fk_rule(&conn, "member_placements", "member_id"),
            ("members".to_string(), "CASCADE".to_string())
        );
        assert_eq!(
            fk_rule(&conn, "messages", "from_member_id"),
            ("members".to_string(), "RESTRICT".to_string())
        );
        assert_eq!(
            fk_rule(&conn, "messages", "to_member_id"),
            ("members".to_string(), "RESTRICT".to_string())
        );
    }

    // A CHECK constraint is a `CHECK` token on identifier boundaries (not a
    // substring of a column name) followed by an opening parenthesis.
    fn has_check_constraint(sql: &str) -> bool {
        let upper = sql.to_uppercase();
        let bytes = upper.as_bytes();
        let mut start = 0;
        while let Some(pos) = upper[start..].find("CHECK") {
            let idx = start + pos;
            let boundary_before = idx == 0 || {
                let c = bytes[idx - 1] as char;
                !(c.is_ascii_alphanumeric() || c == '_')
            };
            let boundary_after = upper[idx + "CHECK".len()..].trim_start().starts_with('(');
            if boundary_before && boundary_after {
                return true;
            }
            start = idx + "CHECK".len();
        }
        false
    }

    #[test]
    fn no_check_constraints_at_head() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        for table in APP_TABLES {
            assert!(
                !has_check_constraint(&table_sql(&conn, table)),
                "{table} must carry no CHECK constraint"
            );
        }
    }

    #[test]
    fn asset_installs_has_the_composite_primary_key_at_head() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let mut stmt = conn
            .prepare(
                "SELECT name, \"notnull\", pk FROM pragma_table_info('asset_installs') ORDER BY cid",
            )
            .unwrap();
        let columns: Vec<(String, i64, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            columns,
            vec![
                ("coding_agent".to_string(), 1, 1),
                ("path".to_string(), 1, 2),
                ("cafleet_version".to_string(), 1, 0),
                ("installed_at".to_string(), 1, 0),
            ],
            "columns in order, all NOT NULL, PRIMARY KEY (coding_agent, path)"
        );
    }

    #[test]
    fn foreign_keys_are_enforced_on_the_migrated_connection() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let result = conn.execute(
            "INSERT INTO members (fleet_id, name, description, status, registered_at) \
             VALUES (999, 'x', 'y', 'active', '2026-07-30T00:00:00.000000+00:00')",
            [],
        );
        let error = result.unwrap_err();
        assert!(
            matches!(&error, rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY),
            "{error}"
        );
    }

    #[test]
    fn message_foreign_keys_reject_missing_senders_and_recipients() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        conn.execute_batch(
            "INSERT INTO fleets(fleet_id, created_at) VALUES (1, '2026-10-23T00:00:00Z');
             INSERT INTO members(member_id, fleet_id, name, description, status, registered_at)
             VALUES (1, 1, 'sender', 'test', 'active', '2026-10-23T00:00:00Z');",
        )
        .unwrap();
        for (sender, recipient) in [(999, 1), (1, 999)] {
            let error = conn.execute(
                "INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text)
                 VALUES (?1, ?2, '2026-10-23T00:00:00Z', 'input_required', '2026-10-23T00:00:00Z', 'test')",
                [sender, recipient],
            ).unwrap_err();
            assert!(
                matches!(&error, rusqlite::Error::SqliteFailure(code, _)
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY),
                "{error}"
            );
        }
    }

    #[test]
    fn deleting_an_unreferenced_member_cascades_its_placement() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        conn.execute_batch(
            "INSERT INTO fleets(fleet_id, created_at) VALUES (1, '2026-10-23T00:00:00Z');
             INSERT INTO members(member_id, fleet_id, name, description, status, registered_at)
             VALUES (1, 1, 'member', 'test', 'active', '2026-10-23T00:00:00Z');
             INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at)
             VALUES (1, 'main', '@1', '%1', 'tmux', 'codex', '2026-10-23T00:00:00Z');
             DELETE FROM members WHERE member_id=1;"
        ).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM member_placements", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn refinery_ledger_records_the_baseline() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let mut stmt = conn
            .prepare("SELECT version FROM refinery_schema_history ORDER BY version")
            .unwrap();
        let versions: Vec<i64> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(versions, vec![1]);
    }

    // The chain guard reads the refinery-embedded listing, not the filesystem:
    // `migration_chain()` returns the `(version, name)` pairs of the embedded
    // runner's migrations, sorted ascending.
    #[test]
    fn migration_chain_is_contiguous_from_1_with_exactly_one_baseline_and_head_1() {
        let chain = migration_chain();
        let versions: Vec<u32> = chain.iter().map(|(version, _)| *version).collect();
        let contiguous: Vec<u32> = (1..=versions.len() as u32).collect();
        assert_eq!(versions, contiguous, "chain must be contiguous from 1");
        assert_eq!(
            chain.iter().filter(|(_, name)| name == "baseline").count(),
            1,
            "the chain must contain exactly one baseline"
        );
        assert_eq!(
            chain
                .first()
                .map(|(version, name)| (*version, name.as_str())),
            Some((1, "baseline")),
            "the single baseline heads the chain"
        );
        assert!(
            chain.iter().all(|(_, name)| !name.is_empty()),
            "every migration carries a slug"
        );
        assert_eq!(
            chain
                .last()
                .map(|(version, name)| (*version, name.as_str())),
            Some((1, "baseline")),
            "expected head is V1__baseline"
        );
    }
}
