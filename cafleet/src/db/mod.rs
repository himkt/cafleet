use include_dir::{Dir, include_dir};
use rusqlite::Connection;

use crate::error::CafleetError;

/// The embedded `V<N>__<slug>.sql` chain (SPEC §8).
pub static MIGRATIONS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations");

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// Open a connection to `database_url` (the `sqlite:///<path>` form only) and
/// apply the mandatory per-connection PRAGMAs (SPEC §6.1).
pub fn connect(database_url: &str) -> Result<Connection, CafleetError> {
    let path = database_url.strip_prefix("sqlite:///").ok_or_else(|| {
        CafleetError::App(format!(
            "database URL must use the sqlite scheme (sqlite:///<path>); got '{database_url}'"
        ))
    })?;
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
        .run(conn)
        .map_err(|e| CafleetError::App(format!("migration failed: {e}")))?;
    Ok(head_version())
}

fn head_version() -> u32 {
    MIGRATIONS
        .files()
        .filter_map(|file| {
            let name = file.path().file_name()?.to_str()?;
            let (version, _slug) = name.strip_prefix('V')?.split_once("__")?;
            version.parse().ok()
        })
        .max()
        .expect("the embedded migration chain has at least the baseline")
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

    const APP_TABLES: [&str; 7] = [
        "members",
        "fleets",
        "asset_installs",
        "member_placements",
        "monitor_config",
        "monitor_runtime",
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
        assert_eq!(names, expected, "no alembic_version, nothing extra");
    }

    #[test]
    fn autoincrement_on_exactly_fleets_members_messages() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        for table in ["fleets", "members", "messages"] {
            assert!(
                table_sql(&conn, table).contains("AUTOINCREMENT"),
                "{table} must AUTOINCREMENT"
            );
        }
        for table in [
            "asset_installs",
            "member_placements",
            "monitor_config",
            "monitor_runtime",
        ] {
            assert!(
                !table_sql(&conn, table).contains("AUTOINCREMENT"),
                "{table} must not AUTOINCREMENT"
            );
        }
    }

    #[test]
    fn the_three_head_indexes_exist_and_nothing_more() {
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
            "idx_messages_owner_member_status_ts",
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
            column_info(&conn, "monitor_config", "interval_seconds").1,
            Some("60".to_string())
        );
        assert_eq!(
            column_info(&conn, "monitor_config", "enabled").1,
            Some("1".to_string())
        );
        assert_eq!(
            column_info(&conn, "monitor_runtime", "tick_seconds").1,
            Some("5".to_string())
        );
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
        assert_eq!(column_info(&conn, "messages", "to_member_id").0, 0);
        assert_eq!(column_info(&conn, "messages", "origin_message_id").0, 0);
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
            fk_rule(&conn, "monitor_config", "member_id"),
            ("members".to_string(), "CASCADE".to_string())
        );
        assert_eq!(
            fk_rule(&conn, "monitor_runtime", "fleet_id"),
            ("fleets".to_string(), "RESTRICT".to_string())
        );
        assert_eq!(
            fk_rule(&conn, "messages", "owner_member_id"),
            ("members".to_string(), "RESTRICT".to_string())
        );
    }

    // A CHECK constraint is a `CHECK` token (not part of an identifier such as
    // `last_stall_check_at`) followed by an opening parenthesis.
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
    fn foreign_keys_are_enforced_on_the_migrated_connection() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        let result = conn.execute(
            "INSERT INTO members (fleet_id, name, description, status, registered_at, member_card_json) \
             VALUES (999, 'x', 'y', 'active', '2026-07-30T00:00:00.000000+00:00', '{}')",
            [],
        );
        assert!(result.is_err(), "an FK to a missing fleet must be rejected");
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

    #[test]
    fn migration_chain_is_contiguous_from_1_with_exactly_one_baseline_and_head_1() {
        let mut versions: Vec<u32> = MIGRATIONS
            .files()
            .map(|file| {
                let name = file.path().file_name().unwrap().to_str().unwrap();
                assert!(name.ends_with(".sql"), "non-SQL migration file: {name}");
                let rest = name
                    .strip_prefix('V')
                    .unwrap_or_else(|| panic!("migration {name} must start with V"));
                let (version, slug) = rest
                    .split_once("__")
                    .unwrap_or_else(|| panic!("migration {name} must match V<N>__<slug>.sql"));
                assert!(
                    slug.strip_suffix(".sql").is_some_and(|s| !s.is_empty()),
                    "migration {name} must carry a slug"
                );
                version
                    .parse()
                    .unwrap_or_else(|_| panic!("migration {name} must carry a numeric version"))
            })
            .collect();
        versions.sort_unstable();
        let contiguous: Vec<u32> = (1..=versions.len() as u32).collect();
        assert_eq!(versions, contiguous, "chain must be contiguous from 1");
        assert!(
            MIGRATIONS.get_file("V1__baseline.sql").is_some(),
            "the single baseline must be V1__baseline.sql"
        );
        assert_eq!(*versions.last().unwrap(), 1, "expected head version is 1");
    }
}
