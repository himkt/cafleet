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
