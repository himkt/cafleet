pub mod asset_installs;
pub mod fleets;
pub mod members;
pub mod messaging;
pub mod panes;
pub mod records;

pub use asset_installs::*;
pub use fleets::*;
pub use members::*;
pub use messaging::*;

#[cfg(test)]
pub(crate) mod tests {
    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::{self as broker, NewPlacement};

    pub fn migrated_conn(dir: &TempDir) -> Connection {
        let url = format!("sqlite:///{}", dir.path().join("broker.db").display());
        let mut conn = crate::db::connect(&url).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        conn
    }

    pub fn placement(pane: Option<&str>) -> NewPlacement {
        NewPlacement {
            backend: "tmux".into(),
            mux_session: "main".into(),
            mux_window_id: "@1".into(),
            mux_pane_id: pane.map(str::to_owned),
            coding_agent: "claude".into(),
        }
    }

    pub fn create_fleet(conn: &mut Connection, name: &str) -> (i64, i64) {
        let fleet =
            broker::create_fleet(conn, Some(name), "main", "@1", "%0", "claude", "tmux").unwrap();
        (fleet.fleet_id, fleet.director.member_id)
    }

    pub fn register(conn: &mut Connection, fleet: i64, name: &str, pane: Option<&str>) -> i64 {
        broker::register_member(conn, fleet, name, "assignment", Some(&placement(pane)))
            .unwrap()
            .member_id
    }

    pub fn guarded_delete_fleet(
        conn: &mut Connection,
        fleet: i64,
    ) -> Result<super::records::DeletedFleet, crate::error::CafleetError> {
        let path = conn
            .path()
            .expect("fleet deletion has a file-backed fixture");
        assert!(
            !path.is_empty(),
            "fleet deletion has a concrete database path"
        );
        let url = format!("sqlite:///{path}");
        let settings = crate::config::Settings::from_lookup(|name| {
            (name == "CAFLEET_DATABASE_URL").then(|| url.clone())
        })
        .unwrap();
        crate::input::delete_fleet(conn, &settings, fleet, crate::input::Deadline::new())
    }
}
