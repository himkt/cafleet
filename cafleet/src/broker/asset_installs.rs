use rusqlite::{Connection, params};

use super::members::db_err;
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc};

pub fn record_asset_install(
    conn: &mut Connection,
    coding_agent: &str,
    path: &str,
    cafleet_version: &str,
) -> Result<(), CafleetError> {
    let now = format_utc(now_utc());
    conn.execute(
        "INSERT INTO asset_installs (coding_agent, path, cafleet_version, installed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(coding_agent, path) DO UPDATE SET
             cafleet_version=excluded.cafleet_version, installed_at=excluded.installed_at",
        params![coding_agent, path, cafleet_version, now],
    )
    .map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_record_upsert_changes_only_the_selected_agent_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&dir);
        for (agent, path) in [("codex", "/a"), ("codex", "/b"), ("claude", "/a")] {
            record_asset_install(&mut conn, agent, path, "old").unwrap();
        }
        conn.execute("UPDATE asset_installs SET installed_at='old-time'", [])
            .unwrap();
        record_asset_install(&mut conn, "codex", "/a", "new").unwrap();
        let rows = conn.prepare("SELECT coding_agent, path, cafleet_version, installed_at FROM asset_installs ORDER BY coding_agent, path")
            .unwrap().query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)))
            .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            (
                "claude".into(),
                "/a".into(),
                "old".into(),
                "old-time".into()
            )
        );
        assert_eq!(
            (&rows[1].0, &rows[1].1, &rows[1].2),
            (&"codex".into(), &"/a".into(), &"new".into())
        );
        assert!(crate::time::parse_lenient(&rows[1].3).is_ok());
        assert_eq!(
            rows[2],
            ("codex".into(), "/b".into(), "old".into(), "old-time".into())
        );
    }
}
