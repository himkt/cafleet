use crate::error::CafleetError;
use rusqlite::{Connection, params};

pub(crate) fn pending_previews(
    conn: &Connection,
    fleet_id: i64,
    subject_id: i64,
) -> Result<Vec<(i64, i64, bool)>, CafleetError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT m.member_id, g.message_id, g.preview_started_at IS NOT NULL FROM members m
         JOIN member_placements p ON p.member_id=m.member_id AND p.mux_pane_id IS NOT NULL
         JOIN messages g ON g.to_member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND m.member_id!=?2 AND {}
         ORDER BY m.member_id, g.created_at, g.message_id",
            super::messaging::PENDING_PREVIEW,
        ))
        .map_err(super::members::db_err)?;
    statement
        .query_map(params![fleet_id, subject_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(super::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::members::db_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{self, test_support as common};

    #[test]
    fn preview_rotation_selects_only_concrete_ordinary_placements() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "rotation");
        let pending = common::register(&mut conn, fleet, "pending", None);
        let placed = common::register(&mut conn, fleet, "placed", Some("%2"));
        broker::send_message(
            &mut conn,
            director,
            &pending.to_string(),
            "wait for placement",
        )
        .unwrap();
        let message =
            broker::send_message(&mut conn, director, &placed.to_string(), "deliver now").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, director).unwrap(),
            vec![(placed, message.message_id, false)]
        );
    }

    #[test]
    fn an_ordinary_subject_retries_ordinary_members_and_preserves_the_directors_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "director");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let observer = common::register(&mut conn, fleet, "observer", Some("%3"));
        let reply =
            broker::send_message(&mut conn, worker, &director.to_string(), "reply").unwrap();
        let assignment =
            broker::send_message(&mut conn, director, &worker.to_string(), "assignment").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, observer).unwrap(),
            vec![(worker, assignment.message_id, false)]
        );
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    #[test]
    fn the_subjects_own_previews_are_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "subject");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        broker::send_message(&mut conn, worker, &director.to_string(), "reply").unwrap();
        let to_worker =
            broker::send_message(&mut conn, director, &worker.to_string(), "assignment").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, director).unwrap(),
            vec![(worker, to_worker.message_id, false)]
        );
        assert!(pending_previews(&conn, fleet, worker).unwrap().is_empty());
    }

    #[test]
    fn retry_eligibility_follows_the_fleets_director_back_reference() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, former_director) = common::create_fleet(&mut conn, "new-root");
        let observer = common::register(&mut conn, fleet, "observer", Some("%2"));
        let director = common::register(&mut conn, fleet, "new-director", Some("%3"));
        conn.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            rusqlite::params![director, fleet],
        )
        .unwrap();
        let ordinary = broker::send_message(
            &mut conn,
            director,
            &former_director.to_string(),
            "ordinary assignment",
        )
        .unwrap();
        let reply =
            broker::send_message(&mut conn, observer, &director.to_string(), "root reply").unwrap();
        assert_eq!(
            pending_previews(&conn, fleet, observer).unwrap(),
            vec![(former_director, ordinary.message_id, false)]
        );
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }
}
