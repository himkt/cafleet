//! Schema compatibility facts shared by invocation, setup and diagnostics.

use rusqlite::Connection;

use crate::error::CafleetError;

#[derive(Debug)]
pub(crate) enum SchemaState {
    Missing,
    Incompatible { path: String },
    Behind { recorded: u32, head: u32 },
    Head { version: u32 },
    Ahead { recorded: u32, head: u32 },
    Unreachable { cause: CafleetError },
}

impl SchemaState {
    pub(crate) fn require_current(&self) -> Result<(), CafleetError> {
        match self {
            SchemaState::Head { .. } => Ok(()),
            SchemaState::Behind { recorded, head } => Err(CafleetError::App(format!(
                "database schema is outdated (schema {recorded}, head {head}); run 'cafleet setup'"
            ))),
            SchemaState::Ahead { recorded, head } => Err(CafleetError::App(format!(
                "database schema {recorded} is newer than this cafleet (head {head}); upgrade cafleet"
            ))),
            SchemaState::Incompatible { path } => Err(crate::diagnosis::incompatible_format(path)),
            SchemaState::Missing => Err(CafleetError::App(
                "no cafleet database; run 'cafleet setup'".into(),
            )),
            SchemaState::Unreachable { cause } => Err(cause.clone()),
        }
    }
}

fn recorded_version(conn: &Connection) -> Result<Option<u32>, CafleetError> {
    let ledger_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='refinery_schema_history')",
        [], |row| row.get(0),
    )?;
    if !ledger_exists {
        return Ok(None);
    }
    conn.query_row(
        "SELECT MAX(version) FROM refinery_schema_history",
        [],
        |row| row.get(0),
    )
    .map_err(crate::error::CafleetError::from)
}

pub(crate) fn incompatible_format(path: &str) -> CafleetError {
    CafleetError::App(format!(
        "database format is incompatible at '{path}'; choose an empty database path and run 'cafleet setup'."
    ))
}

pub(crate) fn classify_schema(conn: &Connection, head: u32) -> SchemaState {
    let incompatible = || SchemaState::Incompatible {
        path: conn.path().unwrap_or(":memory:").to_owned(),
    };
    let inspect = || -> Result<SchemaState, CafleetError> {
        let identity: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        let populated: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
            [], |row| row.get(0)
        )?;
        if identity == 0 && !populated {
            return Ok(SchemaState::Missing);
        }
        if identity != crate::db::APPLICATION_ID {
            return Ok(incompatible());
        }
        let Some(recorded) = recorded_version(conn)? else {
            return Ok(incompatible());
        };
        if !crate::db::schema_matches_version(conn, recorded)? {
            return Ok(incompatible());
        }
        if recorded > head {
            return Ok(SchemaState::Ahead { recorded, head });
        }
        Ok(if recorded < head {
            SchemaState::Behind { recorded, head }
        } else {
            SchemaState::Head { version: recorded }
        })
    };
    match inspect() {
        Ok(state) => state,
        Err(cause) => SchemaState::Unreachable { cause },
    }
}

pub(crate) fn classify_selected_schema(
    conn: &Connection,
    head: u32,
    database_url: &str,
) -> SchemaState {
    match classify_schema(conn, head) {
        SchemaState::Incompatible { .. } => match crate::db::database_path(database_url) {
            Ok(path) => SchemaState::Incompatible { path: path.into() },
            Err(cause) => SchemaState::Unreachable { cause },
        },
        state => state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_preserves_missing_behind_ahead_identity_and_sql_causes() {
        let mut conn = crate::db::connect("sqlite:///:memory:").unwrap();
        let head = crate::db::head_version();
        assert!(matches!(classify_schema(&conn, head), SchemaState::Missing));
        crate::db::migrate_to_head(&mut conn).unwrap();
        assert!(
            matches!(classify_schema(&conn, head), SchemaState::Head { version } if version == head)
        );
        conn.execute_batch("ALTER TABLE messages ADD COLUMN future_value TEXT")
            .unwrap();
        conn.execute("UPDATE refinery_schema_history SET version=?1 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)", [head + 1]).unwrap();
        assert!(matches!(
            classify_schema(&conn, head),
            SchemaState::Ahead { .. }
        ));
        conn.execute_batch("PRAGMA application_id=1").unwrap();
        assert!(matches!(
            classify_schema(&conn, head),
            SchemaState::Incompatible { .. }
        ));
        conn.execute_batch("PRAGMA application_id=1128351288; ALTER TABLE refinery_schema_history RENAME COLUMN version TO missing_version").unwrap();
        assert!(matches!(
            classify_schema(&conn, head),
            SchemaState::Unreachable { .. }
        ));
    }
}
