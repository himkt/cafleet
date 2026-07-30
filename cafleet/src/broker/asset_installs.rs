//! Assets-install version recording (SPEC §5.2 *AssetInstalls*). The
//! colocated tests pin the contract; see [`super::test_support`] for the API.

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
    fn record_and_list_orders_rows_by_coding_agent() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        assert!(broker::list_asset_installs(&conn).unwrap().is_empty());

        broker::record_asset_install(&mut conn, "codex", "0.22.0").unwrap();
        broker::record_asset_install(&mut conn, "claude", "0.22.0").unwrap();

        let rows = broker::list_asset_installs(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["coding_agent"], "claude");
        assert_eq!(rows[1]["coding_agent"], "codex");
        assert_eq!(rows[0]["cafleet_version"], "0.22.0");
        let installed_at = rows[0]["installed_at"].as_str().unwrap();
        assert_eq!(installed_at.len(), 32, "canonical fixed-width timestamp");
        assert!(crate::time::parse_lenient(installed_at).is_ok());
    }

    #[test]
    fn record_asset_install_upserts_the_existing_row() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        broker::record_asset_install(&mut conn, "claude", "0.21.0").unwrap();
        broker::record_asset_install(&mut conn, "claude", "0.22.0").unwrap();

        let rows = broker::list_asset_installs(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["cafleet_version"], "0.22.0");
    }
}
