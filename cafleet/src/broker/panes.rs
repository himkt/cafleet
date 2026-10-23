use crate::error::CafleetError;
use rusqlite::{Connection, params};

pub(crate) fn pending_previews(
    conn: &Connection,
    fleet_id: i64,
    subject_id: i64,
) -> Result<Vec<(i64, i64)>, CafleetError> {
    let mut statement = conn
        .prepare(
            "SELECT m.member_id, g.message_id FROM members m
         JOIN fleets f ON f.fleet_id=m.fleet_id
         JOIN messages g ON g.to_member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND m.member_id!=f.director_member_id
           AND m.member_id!=?2 AND g.from_member_id!=m.member_id
           AND g.status_state='input_required' AND g.notified_at IS NULL
         ORDER BY m.member_id, g.created_at, g.message_id",
        )
        .map_err(super::members::db_err)?;
    statement
        .query_map(params![fleet_id, subject_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(super::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::members::db_err)
}
