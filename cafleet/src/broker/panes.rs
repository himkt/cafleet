use crate::error::CafleetError;
use rusqlite::{Connection, params};

pub(crate) fn pending_previews(
    conn: &Connection,
    fleet_id: i64,
    subject_id: i64,
) -> Result<Vec<(i64, i64)>, CafleetError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT m.member_id, g.message_id FROM members m
         JOIN fleets f ON f.fleet_id=m.fleet_id
         JOIN member_placements p ON p.member_id=m.member_id AND p.mux_pane_id IS NOT NULL
         JOIN messages g ON g.to_member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND m.member_id!=?2
         AND m.member_id!=f.director_member_id AND {}
         ORDER BY m.member_id, g.created_at, g.message_id",
            super::messaging::PENDING_PREVIEW,
        ))
        .map_err(super::members::db_err)?;
    statement
        .query_map(params![fleet_id, subject_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(super::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::members::db_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;

    #[test]
    fn preview_rotation_selects_only_concrete_ordinary_placements() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "rotation");
        let pending = register(&mut conn, fleet, "pending", None);
        let placed = register(&mut conn, fleet, "placed", Some("%2"));
        broker::send_message(&mut conn, director, pending, "wait for placement").unwrap();
        let message = broker::send_message(&mut conn, director, placed, "deliver now").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, director).unwrap(),
            vec![(placed, message.message_id)]
        );
    }

    #[test]
    fn an_ordinary_subject_retries_ordinary_members_and_preserves_the_directors_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "director");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let observer = register(&mut conn, fleet, "observer", Some("%3"));
        let reply = broker::send_message(&mut conn, worker, director, "reply").unwrap();
        let assignment = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, observer).unwrap(),
            vec![(worker, assignment.message_id)]
        );
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    #[test]
    fn the_subjects_own_previews_are_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "subject");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        broker::send_message(&mut conn, worker, director, "reply").unwrap();
        let to_worker = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, director).unwrap(),
            vec![(worker, to_worker.message_id)]
        );
        assert!(pending_previews(&conn, fleet, worker).unwrap().is_empty());
    }

    #[test]
    fn retry_eligibility_follows_the_fleets_director_back_reference() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, former_director) = create_fleet_fixture(&mut conn, "new-root");
        let observer = register(&mut conn, fleet, "observer", Some("%2"));
        let director = register(&mut conn, fleet, "new-director", Some("%3"));
        conn.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            rusqlite::params![director, fleet],
        )
        .unwrap();
        let ordinary =
            broker::send_message(&mut conn, director, former_director, "ordinary assignment")
                .unwrap();
        let reply = broker::send_message(&mut conn, observer, director, "root reply").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, observer).unwrap(),
            vec![(former_director, ordinary.message_id)]
        );
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }
}
