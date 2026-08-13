//! Assets-install version recording (SPEC §5.2 *AssetInstalls*). The
//! colocated tests pin the contract; see [`super::test_support`] for the API.

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::members::db_err;
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc};

pub fn asset_installs_table_exists(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type='table' AND name='asset_installs')",
        [],
        |row| row.get(0),
    )
    .expect("sqlite_master is always queryable")
}

pub fn list_asset_installs(conn: &Connection) -> Result<Vec<Value>, CafleetError> {
    let mut stmt = conn
        .prepare(
            "SELECT coding_agent, path, cafleet_version, installed_at \
             FROM asset_installs ORDER BY coding_agent, path",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "coding_agent": row.get::<_, String>(0)?,
                "path": row.get::<_, String>(1)?,
                "cafleet_version": row.get::<_, String>(2)?,
                "installed_at": row.get::<_, String>(3)?,
            }))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn record_asset_install(
    conn: &mut Connection,
    coding_agent: &str,
    path: &str,
    cafleet_version: &str,
) -> Result<(), CafleetError> {
    let now = format_utc(now_utc());
    conn.execute(
        "INSERT INTO asset_installs (coding_agent, path, cafleet_version, installed_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(coding_agent, path) DO UPDATE SET \
             cafleet_version=excluded.cafleet_version, installed_at=excluded.installed_at",
        params![coding_agent, path, cafleet_version, now],
    )
    .map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support::migrated_conn;

    #[test]
    fn table_exists_reflects_the_schema_state() {
        let dir = TempDir::new().unwrap();
        let url = format!("sqlite:///{}", dir.path().join("bare.db").display());
        let conn = crate::db::connect(&url).unwrap();
        assert!(!broker::asset_installs_table_exists(&conn));

        let migrated = migrated_conn(&dir);
        assert!(broker::asset_installs_table_exists(&migrated));
    }

    #[test]
    fn record_and_list_orders_rows_by_coding_agent_then_path() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        assert!(broker::list_asset_installs(&conn).unwrap().is_empty());

        broker::record_asset_install(&mut conn, "codex", "/b/.codex", "0.22.0").unwrap();
        broker::record_asset_install(&mut conn, "codex", "/a/.codex", "0.22.0").unwrap();
        broker::record_asset_install(&mut conn, "claude", "/b/.claude", "0.22.0").unwrap();

        let rows = broker::list_asset_installs(&conn).unwrap();
        let keys: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| {
                (
                    row["coding_agent"].as_str().unwrap(),
                    row["path"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("claude", "/b/.claude"),
                ("codex", "/a/.codex"),
                ("codex", "/b/.codex"),
            ],
            "ascending (coding_agent, path)"
        );
        assert_eq!(rows[0]["cafleet_version"], "0.22.0");
        let installed_at = rows[0]["installed_at"].as_str().unwrap();
        assert_eq!(installed_at.len(), 32, "canonical fixed-width timestamp");
        assert!(crate::time::parse_lenient(installed_at).is_ok());
    }

    #[test]
    fn record_asset_install_upserts_on_the_composite_key() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        broker::record_asset_install(&mut conn, "claude", "/x/.claude", "0.21.0").unwrap();
        broker::record_asset_install(&mut conn, "claude", "/x/.claude", "0.22.0").unwrap();

        let rows = broker::list_asset_installs(&conn).unwrap();
        assert_eq!(rows.len(), 1, "same (coding_agent, path) upserts in place");
        assert_eq!(rows[0]["cafleet_version"], "0.22.0");
    }

    #[test]
    fn the_same_agent_records_distinct_rows_at_distinct_paths() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        broker::record_asset_install(&mut conn, "claude", "/old/.claude", "0.21.0").unwrap();
        broker::record_asset_install(&mut conn, "claude", "/new/.claude", "0.22.0").unwrap();

        let rows = broker::list_asset_installs(&conn).unwrap();
        assert_eq!(rows.len(), 2, "a new path is a new row, not an upsert");
    }
}
