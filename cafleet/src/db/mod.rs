use rusqlite::{Connection, OptionalExtension};

use crate::error::CafleetError;

mod embedded {
    refinery::embed_migrations!("migrations");
}

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

pub fn connect(database_url: &str) -> Result<Connection, CafleetError> {
    connect_with_flags(database_url, rusqlite::OpenFlags::default())
}

pub(crate) fn connect_read_only(database_url: &str) -> Result<Connection, CafleetError> {
    connect_with_flags(database_url, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn connect_with_flags(
    database_url: &str,
    flags: rusqlite::OpenFlags,
) -> Result<Connection, CafleetError> {
    let path = database_path(database_url)?;
    let conn = Connection::open_with_flags(path, flags)
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

const COLUMNS: &str =
    "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid";
const FOREIGN_KEYS: &str = "SELECT \"table\", \"from\", \"to\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list(?1) ORDER BY \"from\", \"to\"";
const INDEX_COLUMNS: &str = "SELECT seqno, cid, name FROM pragma_index_info(?1) ORDER BY seqno";
type Metadata = Vec<Vec<rusqlite::types::Value>>;

struct SchemaIndex {
    name: String,
    unique: i64,
    partial: i64,
    columns: Metadata,
}

struct SchemaTable {
    name: String,
    columns: Metadata,
    foreign_keys: Metadata,
    indexes: Vec<SchemaIndex>,
}

impl SchemaTable {
    fn matches(&self, conn: &Connection) -> Result<bool, CafleetError> {
        if metadata_rows(conn, COLUMNS, &self.name)? != self.columns
            || metadata_rows(conn, FOREIGN_KEYS, &self.name)? != self.foreign_keys
        {
            return Ok(false);
        }
        for index in &self.indexes {
            let definition: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT \"unique\", partial FROM pragma_index_list(?1) WHERE name=?2",
                    rusqlite::params![self.name, index.name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(crate::broker::members::db_err)?;
            if definition != Some((index.unique, index.partial))
                || metadata_rows(conn, INDEX_COLUMNS, &index.name)? != index.columns
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

struct SchemaShape(Vec<SchemaTable>);

impl SchemaShape {
    fn matches(&self, conn: &Connection) -> Result<bool, CafleetError> {
        for table in &self.0 {
            if !table.matches(conn)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn read_schema(conn: &Connection) -> Result<SchemaShape, CafleetError> {
    let mut tables = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).map_err(crate::broker::members::db_err)?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(crate::broker::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::broker::members::db_err)?;
    let mut schema = Vec::new();
    for name in names {
        let mut indexes = conn
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
        let indexes = indexes
            .into_iter()
            .map(|(name, unique, partial)| {
                Ok(SchemaIndex {
                    columns: metadata_rows(conn, INDEX_COLUMNS, &name)?,
                    name,
                    unique,
                    partial,
                })
            })
            .collect::<Result<Vec<_>, CafleetError>>()?;
        schema.push(SchemaTable {
            columns: metadata_rows(conn, COLUMNS, &name)?,
            foreign_keys: metadata_rows(conn, FOREIGN_KEYS, &name)?,
            name,
            indexes,
        });
    }
    Ok(SchemaShape(schema))
}

fn reference_schema(version: u32) -> Result<SchemaShape, CafleetError> {
    let mut expected = Connection::open_in_memory().map_err(crate::broker::members::db_err)?;
    embedded::migrations::runner()
        .set_grouped(true)
        .set_target(refinery::Target::Version(
            i32::try_from(version).expect("supported version fits i32"),
        ))
        .run(&mut expected)
        .map_err(|error| CafleetError::App(format!("reference migration failed: {error}")))?;
    read_schema(&expected)
}

pub(crate) fn schema_matches_version(
    conn: &Connection,
    version: u32,
) -> Result<bool, CafleetError> {
    if version == 0 {
        return Ok(false);
    }
    let head = head_version();
    let reference = reference_schema(version.min(head))?;
    let snapshot = if conn.is_autocommit() {
        Some(
            conn.unchecked_transaction()
                .map_err(crate::broker::members::db_err)?,
        )
    } else {
        None
    };
    let matches = if version > head {
        reference
            .0
            .iter()
            .find(|table| table.name == "refinery_schema_history")
            .expect("refinery migrations create their ledger")
            .matches(conn)?
    } else {
        reference.matches(conn)?
    };
    if let Some(snapshot) = snapshot {
        snapshot.commit().map_err(crate::broker::members::db_err)?;
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn require_current(conn: &Connection) -> Result<(), CafleetError> {
        crate::diagnosis::classify_schema(conn, head_version()).require_current()
    }

    fn at_version(version: u32) -> Connection {
        let mut conn = connect("sqlite:///:memory:").unwrap();
        embedded::migrations::runner()
            .set_grouped(true)
            .set_target(refinery::Target::Version(i32::try_from(version).unwrap()))
            .run(&mut conn)
            .unwrap();
        conn
    }

    fn rows(conn: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
        let mut statement = conn.prepare(sql).unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn application_rows(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        [
            "SELECT * FROM fleets ORDER BY fleet_id",
            "SELECT * FROM members ORDER BY member_id",
            "SELECT * FROM member_placements ORDER BY member_id",
            "SELECT message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at FROM messages ORDER BY message_id",
            "SELECT * FROM asset_installs ORDER BY coding_agent, path",
            "SELECT * FROM sqlite_sequence ORDER BY name",
        ].map(|sql| rows(conn, sql)).into()
    }

    #[test]
    fn supported_upgrades_and_repeated_migration_preserve_all_application_rows() {
        for (version, _) in migration_chain() {
            let mut conn = at_version(version);
            conn.execute_batch(
                "INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'preserved', 'created');
                 INSERT INTO members(member_id, fleet_id, name, description, status, registered_at, deregistered_at) VALUES
                    (1, 1, 'Director', 'director assignment', 'active', 'registered', NULL),
                    (2, 1, 'worker', 'worker assignment', 'active', 'registered', NULL),
                    (3, 1, 'departed', 'finished assignment', 'deregistered', 'registered', 'departed');
                 INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) VALUES
                    (1, 'main', '@1', '%0', 'tmux', 'claude', 'placed'),
                    (2, 'main', '@1', NULL, 'tmux', 'codex', 'placed');
                 UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;
                 INSERT INTO messages(message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at) VALUES
                    (7, 1, 2, 'created-7', 'input_required', 'status-7', 'pending unicode 世界', NULL),
                    (8, 1, 2, 'created-8', 'input_required', 'status-8', 'notified', 'notified-8'),
                    (9, 2, 1, 'created-9', 'completed', 'status-9', 'completed', NULL);
                 INSERT INTO asset_installs VALUES('claude', '/preserved/config', '0.29.0', 'installed');"
            ).unwrap();
            if version == 2 {
                conn.execute(
                    "UPDATE messages SET preview_started_at='claimed' WHERE message_id=7",
                    [],
                )
                .unwrap();
            }
            assert!(schema_matches_version(&conn, version).unwrap());
            let state = crate::diagnosis::classify_schema(&conn, head_version());
            if version < head_version() {
                assert!(
                    matches!(state, crate::diagnosis::SchemaState::Behind { recorded, .. } if recorded == version)
                );
                assert!(state.require_current().is_err());
            } else {
                state.require_current().unwrap();
            }
            let before = application_rows(&conn);
            let ledger = rows(
                &conn,
                "SELECT * FROM refinery_schema_history ORDER BY version",
            );
            for _ in 0..2 {
                assert_eq!(migrate_to_head(&mut conn).unwrap(), head_version());
                assert_eq!(application_rows(&conn), before, "migration from {version}");
                let after = rows(
                    &conn,
                    "SELECT * FROM refinery_schema_history ORDER BY version",
                );
                assert_eq!(&after[..ledger.len()], ledger);
                require_current(&conn).unwrap();
            }
        }
    }

    #[test]
    fn selected_reference_rechecks_ddl_and_accepts_auxiliary_objects() {
        let conn = at_version(head_version());
        conn.execute_batch(
            "CREATE TABLE operator_data(value TEXT);
             CREATE INDEX operator_messages ON messages(text);
             CREATE TRIGGER operator_message_insert AFTER INSERT ON messages BEGIN
                INSERT INTO operator_data VALUES(NEW.text);
             END;",
        )
        .unwrap();
        assert!(schema_matches_version(&conn, head_version()).unwrap());
        conn.execute_batch("ALTER TABLE messages RENAME COLUMN text TO broken")
            .unwrap();
        assert!(!schema_matches_version(&conn, head_version()).unwrap());
        conn.execute_batch("ALTER TABLE messages RENAME COLUMN broken TO text")
            .unwrap();
        assert!(schema_matches_version(&conn, head_version()).unwrap());
        conn.execute_batch("CREATE TEMP TABLE messages(wrong TEXT)")
            .unwrap();
        assert!(!schema_matches_version(&conn, head_version()).unwrap());
    }

    #[test]
    fn required_schema_components_and_identity_are_refused() {
        for damage in [
            "DROP INDEX idx_messages_to_member_status_ts",
            "DROP INDEX idx_messages_to_member_status_ts; CREATE INDEX idx_messages_to_member_status_ts ON messages(from_member_id, status_timestamp)",
            "DROP TABLE asset_installs",
            "ALTER TABLE members RENAME COLUMN status TO wrong",
            "ALTER TABLE refinery_schema_history RENAME COLUMN checksum TO wrong",
            "PRAGMA application_id=1",
        ] {
            let conn = at_version(head_version());
            conn.execute_batch(damage).unwrap();
            assert!(require_current(&conn).is_err(), "{damage}");
        }
    }

    #[test]
    fn structural_reference_checks_required_keys_types_defaults_and_foreign_keys() {
        for (table, original, replacement) in [
            (
                "asset_installs",
                "PRIMARY KEY (coding_agent, path)",
                "PRIMARY KEY (coding_agent)",
            ),
            ("members", "name TEXT NOT NULL", "name TEXT"),
            ("messages", "text TEXT NOT NULL", "text INTEGER NOT NULL"),
            ("member_placements", "DEFAULT 'tmux'", "DEFAULT 'herdr'"),
            ("messages", "ON DELETE RESTRICT", "ON DELETE CASCADE"),
        ] {
            let conn = at_version(head_version());
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(ddl.contains(original));
            let version: i64 = conn
                .query_row("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap();
            conn.execute_batch("PRAGMA writable_schema=ON").unwrap();
            conn.execute(
                "UPDATE sqlite_schema SET sql=?1 WHERE name=?2",
                rusqlite::params![ddl.replace(original, replacement), table],
            )
            .unwrap();
            conn.pragma_update(None, "schema_version", version + 1)
                .unwrap();
            conn.execute_batch("PRAGMA writable_schema=OFF").unwrap();
            assert!(
                !schema_matches_version(&conn, head_version()).unwrap(),
                "{table}: {original}"
            );
        }
    }

    #[test]
    fn ahead_versions_validate_the_ledger_without_requiring_the_head_application_shape() {
        let conn = at_version(head_version());
        conn.execute_batch("ALTER TABLE messages ADD COLUMN future_value TEXT")
            .unwrap();
        conn.execute("UPDATE refinery_schema_history SET version=?1 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)", [head_version() + 1]).unwrap();
        assert!(schema_matches_version(&conn, head_version() + 1).unwrap());
        assert!(matches!(
            crate::diagnosis::classify_schema(&conn, head_version()),
            crate::diagnosis::SchemaState::Ahead { .. }
        ));
        conn.execute_batch("ALTER TABLE refinery_schema_history RENAME COLUMN checksum TO wrong")
            .unwrap();
        assert!(!schema_matches_version(&conn, head_version() + 1).unwrap());
        assert!(!schema_matches_version(&conn, 0).unwrap());
    }

    #[test]
    fn connection_pragmas_and_foreign_keys_protect_real_writes() {
        let mut conn = connect("sqlite:///:memory:").unwrap();
        assert_eq!(
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            5000
        );
        migrate_to_head(&mut conn).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            APPLICATION_ID
        );
        assert!(conn.execute("INSERT INTO members(fleet_id, name, description, status, registered_at) VALUES(404, 'orphan', '', 'active', '')", []).is_err());
        conn.execute_batch("INSERT INTO fleets(fleet_id, created_at) VALUES(1, 'created'); INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES(1, 1, 'valid', '', 'active', 'registered');").unwrap();
        for (from, to) in [(404, 1), (1, 404)] {
            assert!(conn.execute("INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text) VALUES(?1, ?2, '', 'input_required', '', '')", rusqlite::params![from, to]).is_err());
        }
        for url in ["postgresql:///registry", "mysql://localhost/registry"] {
            assert!(connect(url).is_err());
        }
    }

    #[test]
    fn failed_baseline_initialization_rolls_back_identity_tables_and_ledger() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA max_page_count=1").unwrap();
        assert!(migrate_to_head(&mut conn).is_err());
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            rows(
                &conn,
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
            .is_empty()
        );
    }

    #[test]
    fn migration_chain_is_contiguous_from_one_with_a_single_baseline() {
        let chain = migration_chain();
        assert_eq!(
            chain
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            (1..=u32::try_from(chain.len()).unwrap()).collect::<Vec<_>>()
        );
        assert_eq!(
            chain.iter().filter(|(_, name)| name == "baseline").count(),
            1
        );
        assert_eq!(chain.first(), Some(&(1, "baseline".into())));
        assert!(chain.iter().all(|(_, name)| !name.is_empty()));
    }
}
