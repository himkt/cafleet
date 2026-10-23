use crate::error::CafleetError;
use rusqlite::Connection;

pub fn pending_preview_members(conn: &Connection, fleet_id: i64) -> Result<Vec<i64>, CafleetError> {
    super::fleets::require_live_fleet(conn, fleet_id)?;
    let mut statement = conn.prepare(
        "SELECT m.member_id FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         WHERE m.fleet_id=?1 AND m.status='active' AND m.member_id!=f.director_member_id
           AND EXISTS(SELECT 1 FROM messages g WHERE g.to_member_id=m.member_id
             AND g.from_member_id!=m.member_id AND g.status_state='input_required' AND g.notified_at IS NULL)
         ORDER BY m.member_id"
    ).map_err(super::members::db_err)?;
    statement
        .query_map([fleet_id], |row| row.get(0))
        .map_err(super::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::members::db_err)
}
