use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::records::{MemberActivity, MemberKind, MemberRecord, Placement, RegisteredMember};
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc, parse_lenient};

#[derive(Debug, Clone)]
pub struct NewPlacement {
    pub backend: String,
    pub mux_session: String,
    pub mux_window_id: String,
    pub mux_pane_id: Option<String>,
    pub coding_agent: String,
}

pub(crate) fn db_err(e: rusqlite::Error) -> CafleetError {
    if let rusqlite::Error::FromSqlConversionFailure(_, _, source) = &e
        && let Some(error) = source.downcast_ref::<CafleetError>()
    {
        return error.clone();
    }
    CafleetError::App(format!("database error: {e}"))
}

pub fn register_member(
    conn: &mut Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    placement: Option<&NewPlacement>,
) -> Result<RegisteredMember, CafleetError> {
    crate::db::require_current(conn)?;
    let fleet = super::fleets::fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::Usage(format!("Fleet '{fleet_id}' not found.")))?;
    if fleet.deleted_at.is_some() {
        return Err(CafleetError::Usage(format!("fleet {fleet_id} is deleted")));
    }
    let now = format_utc(now_utc());
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    super::fleets::require_live_fleet(&tx, fleet_id)?;
    tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at)
         VALUES (?1, ?2, ?3, 'active', ?4)",
        params![fleet_id, name, description, now],
    )
    .map_err(db_err)?;
    let member_id = tx.last_insert_rowid();
    if let Some(p) = placement {
        tx.execute(
            "INSERT INTO member_placements
             (member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                member_id,
                p.mux_session,
                p.mux_window_id,
                p.mux_pane_id,
                p.backend,
                p.coding_agent,
                now
            ],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(RegisteredMember {
        member_id,
        name: name.into(),
        registered_at: now,
    })
}

pub fn get_member(
    conn: &Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<Option<MemberRecord>, CafleetError> {
    super::fleets::require_live_fleet(conn, fleet_id)?;
    conn.query_row(
        "SELECT m.name, m.description, m.status, m.registered_at,
                f.director_member_id=m.member_id,
                p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, p.created_at
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         LEFT JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.member_id=?1 AND m.fleet_id=?2 AND m.status='active'",
        params![member_id, fleet_id],
        |row| {
            let backend: Option<String> = row.get(5)?;
            Ok(MemberRecord {
                member_id, fleet_id, name: row.get(0)?, description: row.get(1)?,
                status: row.get(2)?, registered_at: row.get(3)?,
                kind: if row.get(4)? { MemberKind::Director } else { MemberKind::Member },
                placement: backend.map(|_| Placement::from_row(row, 5)).transpose()?,
            })
        },
    ).optional().map_err(db_err)
}

/// The fleet id of an active member row — the derivation behind every
/// positional `MEMBER_ID` subject (SPEC §6.3 *Positional subject ids*).
pub fn active_member_fleet(conn: &Connection, member_id: i64) -> Result<Option<i64>, CafleetError> {
    conn.query_row(
        "SELECT fleet_id FROM members WHERE member_id=?1 AND status='active'",
        [member_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

pub fn deregister_member(conn: &mut Connection, member_id: i64) -> Result<bool, CafleetError> {
    crate::db::require_current(conn)?;
    let is_root: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM fleets \
             WHERE director_member_id=?1 AND deleted_at IS NULL)",
            [member_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    if is_root {
        return Err(CafleetError::App(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead".to_string(),
        ));
    }
    let now = format_utc(now_utc());
    let tx = conn.transaction().map_err(db_err)?;
    let changed = tx
        .execute(
            "UPDATE members SET status='deregistered', deregistered_at=?1 \
             WHERE member_id=?2 AND status='active'",
            params![now, member_id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM member_placements WHERE member_id=?1",
        [member_id],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(true)
}

pub fn update_placement_pane_id(
    conn: &mut Connection,
    member_id: i64,
    pane_id: &str,
) -> Result<Option<Placement>, CafleetError> {
    crate::db::require_current(conn)?;
    let changed = conn
        .execute(
            "UPDATE member_placements SET mux_pane_id=?1 WHERE member_id=?2",
            params![pane_id, member_id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Ok(None);
    }
    conn.query_row(
        "SELECT backend, mux_session, mux_window_id, mux_pane_id, coding_agent, created_at \
         FROM member_placements WHERE member_id=?1",
        [member_id],
        |row| Placement::from_row(row, 0),
    )
    .optional()
    .map_err(db_err)
}

pub fn verify_member_fleet(
    conn: &Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<bool, CafleetError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM members WHERE member_id=?1 AND fleet_id=?2)",
        params![member_id, fleet_id],
        |row| row.get(0),
    )
    .map_err(db_err)
}

pub fn get_member_names(
    conn: &Connection,
    member_ids: &[i64],
) -> Result<BTreeMap<i64, String>, CafleetError> {
    let ids = member_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let ids = ids.into_iter().collect::<Vec<_>>();
    let mut names = BTreeMap::new();
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut statement = conn
            .prepare(&format!(
                "SELECT member_id, name FROM members WHERE member_id IN ({placeholders})"
            ))
            .map_err(db_err)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(chunk), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        names.extend(rows);
    }
    Ok(names)
}

fn roster_rows(
    conn: &Connection,
    fleet_id: i64,
    include_message_holders: bool,
    activity: bool,
) -> Result<Vec<MemberActivity>, CafleetError> {
    let activity_columns = if activity {
        ", (SELECT MAX(created_at) FROM messages WHERE from_member_id=m.member_id),
           (SELECT MAX(created_at) FROM messages WHERE to_member_id=m.member_id),
           (SELECT MAX(status_timestamp) FROM messages WHERE to_member_id=m.member_id AND status_state='completed')"
    } else {
        ""
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT m.member_id, m.name, m.status, f.director_member_id=m.member_id,
                p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, p.created_at,
                m.description, m.registered_at {activity_columns}
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         LEFT JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND (m.status='active' OR (?2 AND EXISTS(
               SELECT 1 FROM messages WHERE to_member_id=m.member_id)))
         ORDER BY m.member_id"
    )).map_err(db_err)?;
    stmt.query_map(params![fleet_id, include_message_holders], |row| {
        let backend: Option<String> = row.get(4)?;
        Ok(MemberActivity {
            member: MemberRecord {
                member_id: row.get(0)?,
                fleet_id,
                name: row.get(1)?,
                status: row.get(2)?,
                kind: if row.get(3)? {
                    MemberKind::Director
                } else {
                    MemberKind::Member
                },
                placement: backend.map(|_| Placement::from_row(row, 4)).transpose()?,
                description: row.get(10)?,
                registered_at: row.get(11)?,
            },
            last_sent: if activity { row.get(12)? } else { None },
            last_recv: if activity { row.get(13)? } else { None },
            last_ack: if activity { row.get(14)? } else { None },
            idle: None,
        })
    })
    .map_err(db_err)?
    .collect::<Result<Vec<_>, _>>()
    .map_err(db_err)
}

fn idle_seconds(row: &MemberActivity, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let latest = [&row.last_sent, &row.last_recv, &row.last_ack]
        .into_iter()
        .flatten()
        .max()?;
    let parsed = parse_lenient(latest).ok()?;
    Some((now - parsed).num_seconds().max(0))
}

pub fn list_members(conn: &Connection, fleet_id: i64) -> Result<Vec<MemberActivity>, CafleetError> {
    super::fleets::require_live_fleet(conn, fleet_id)?;
    let now = now_utc();
    let mut rows = roster_rows(conn, fleet_id, false, true)?;
    for row in &mut rows {
        row.idle = idle_seconds(row, now);
    }
    Ok(rows)
}

pub fn list_roster(
    conn: &Connection,
    fleet_id: i64,
    include_message_holders: bool,
) -> Result<Vec<MemberRecord>, CafleetError> {
    super::fleets::require_live_fleet(conn, fleet_id)?;
    Ok(roster_rows(conn, fleet_id, include_message_holders, false)?
        .into_iter()
        .map(|row| row.member)
        .collect())
}

pub(crate) fn capture_roster(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<MemberRecord>, CafleetError> {
    Ok(roster_rows(conn, fleet_id, false, false)?
        .into_iter()
        .map(|row| row.member)
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{create_fleet, migrated_conn, placement, register};
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn register_member_returns_the_registration_summary() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            Some(&placement(Some("%2"))),
        )
        .map(|record| crate::presentation::registered_member(&record))
        .unwrap();
        assert!(result["member_id"].as_i64().is_some());
        assert_eq!(result["name"], "analyst");
        assert!(crate::time::parse_lenient(result["registered_at"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn get_member_shape_is_pinned() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            Some(&placement(Some("%2"))),
        )
        .map(|record| crate::presentation::registered_member(&record))
        .unwrap();
        let member_id = result["member_id"].as_i64().unwrap();
        let ts = result["registered_at"].as_str().unwrap().to_string();

        let member = broker::get_member(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        let expected = format!(
            r#"{{"member_id":{member_id},"name":"analyst","description":"test member","status":"active","registered_at":"{ts}","kind":"member","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%2","coding_agent":"claude","created_at":"{ts}"}}}}"#
        );
        assert_eq!(format_json(&member), expected);
    }

    #[test]
    fn placementless_member_has_null_placement() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = broker::register_member(&mut conn, fleet_id, "ghost", "d", None)
            .unwrap()
            .member_id;
        let member = broker::get_member(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"], Value::Null);
        let ts = member["registered_at"].as_str().unwrap();
        assert_eq!(
            format_json(&member),
            format!(
                r#"{{"member_id":{member_id},"name":"ghost","description":"d","status":"active","registered_at":"{ts}","kind":"member","placement":null}}"#
            )
        );
    }

    #[test]
    fn register_member_unknown_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = broker::register_member(&mut conn, 999, "x", "d", None)
            .expect_err("unknown fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), "Fleet '999' not found.");
    }

    #[test]
    fn register_member_deleted_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        common::guarded_delete_fleet(&mut conn, fleet_id).unwrap();
        let err = broker::register_member(&mut conn, fleet_id, "x", "d", None)
            .expect_err("deleted fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), format!("fleet {fleet_id} is deleted"));
    }

    #[test]
    fn registration_requires_an_active_director_for_every_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "alpha");
        conn.execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [director],
        )
        .unwrap();
        let placed = placement(Some("%2"));
        for placement in [Some(&placed), None] {
            let error =
                broker::register_member(&mut conn, fleet, "worker", "test", placement).unwrap_err();
            assert!(matches!(error, CafleetError::App(_)));
            assert_eq!(
                error.message(),
                format!("fleet {fleet}'s root Director (member {director}) is not active.")
            );
        }
    }

    #[test]
    fn deregister_root_director_is_rejected() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let err = broker::deregister_member(&mut conn, director_id)
            .expect_err("the root Director must not be deregisterable");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            "cannot deregister the root Director; use 'cafleet fleet delete' instead"
        );
    }

    #[test]
    fn deregister_member_flips_cleans_and_is_not_repeatable() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));

        assert!(broker::deregister_member(&mut conn, member_id).unwrap());

        let (status, deregistered_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, deregistered_at FROM members WHERE member_id=?1",
                [member_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "deregistered");
        assert!(deregistered_at.is_some());
        let placements: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM member_placements WHERE member_id=?1",
                [member_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(placements, 0);

        assert!(!broker::deregister_member(&mut conn, member_id).unwrap());
    }

    #[test]
    fn get_member_is_active_only_and_fleet_scoped() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, _) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let member_id = register(&mut conn, fleet_a, "worker", Some("%2"));

        assert!(
            broker::get_member(&conn, member_id, fleet_b)
                .map(|record| record.as_ref().map(crate::presentation::member))
                .unwrap()
                .is_none()
        );
        broker::deregister_member(&mut conn, member_id).unwrap();
        assert!(
            broker::get_member(&conn, member_id, fleet_a)
                .map(|record| record.as_ref().map(crate::presentation::member))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn update_placement_pane_id_patches_the_pending_pane() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", None);

        let updated = broker::update_placement_pane_id(&mut conn, member_id, "%9")
            .map(|record| record.as_ref().map(crate::presentation::placement))
            .unwrap()
            .unwrap();
        assert_eq!(updated["mux_pane_id"], "%9");
        let member = broker::get_member(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"]["mux_pane_id"], "%9");

        let placementless = broker::register_member(&mut conn, fleet_id, "ghost", "d", None)
            .unwrap()
            .member_id;
        assert!(
            broker::update_placement_pane_id(&mut conn, placementless, "%1")
                .map(|record| record.as_ref().map(crate::presentation::placement))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn list_members_rows_carry_kind_placement_and_activity_proxies() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let ghost_id = broker::register_member(&mut conn, fleet_id, "ghost", "d", None)
            .unwrap()
            .member_id;

        let sent = common::send(&mut conn, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let rows = broker::list_members(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::member_activity)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            rows.len(),
            3,
            "Director, ordinary member and placementless member"
        );
        let by_id = |id: i64| rows.iter().find(|r| r["member_id"] == id).unwrap();

        let director = by_id(director_id);
        assert_eq!(director["kind"], "director");
        assert!(director["last_sent"].is_string());
        assert_eq!(director["last_recv"], Value::Null);
        let idle = director["idle"].as_i64().unwrap();
        assert!((0..=5).contains(&idle), "fresh activity, got idle {idle}");

        let worker = by_id(member_id);
        assert_eq!(worker["kind"], "member");
        assert!(worker["last_recv"].is_string());
        assert_eq!(worker["last_ack"], Value::Null);

        let ghost = by_id(ghost_id);
        assert_eq!(ghost["placement"], Value::Null);
        assert_eq!(ghost["last_sent"], Value::Null);
        assert_eq!(ghost["last_recv"], Value::Null);
        assert_eq!(ghost["idle"], Value::Null);

        broker::ack_message(&mut conn, message_id).unwrap();
        let rows = broker::list_members(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::member_activity)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        let worker = rows.iter().find(|r| r["member_id"] == member_id).unwrap();
        assert!(worker["last_ack"].is_string());
    }

    #[test]
    fn list_members_excludes_deregistered_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        broker::deregister_member(&mut conn, member_id).unwrap();
        let rows = broker::list_members(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::member_activity)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert!(rows.iter().all(|r| r["member_id"] != member_id));
    }

    #[test]
    fn member_kind_uses_the_fleet_director_back_reference_across_views() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%3"));
        let expected = [(director_id, "director"), (member_id, "member")];

        for (id, kind) in expected {
            let member = broker::get_member(&conn, id, fleet_id)
                .map(|record| record.as_ref().map(crate::presentation::member))
                .unwrap()
                .unwrap();
            assert_eq!(member["kind"], kind, "get_member kind for member {id}");
        }
        let rows = broker::list_members(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::member_activity)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        for (id, kind) in expected {
            let row = rows.iter().find(|r| r["member_id"] == id).unwrap();
            assert_eq!(row["kind"], kind, "list_members kind for member {id}");
        }
        let roster = broker::members::capture_roster(&conn, fleet_id).unwrap();
        for (id, kind) in expected {
            let row = roster.iter().find(|member| member.member_id == id).unwrap();
            assert_eq!(
                row.kind.as_str(),
                kind,
                "capture_roster kind for member {id}"
            );
        }
    }

    #[test]
    fn capture_roster_returns_active_fleet_members_with_pending_or_absent_placements() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let holder_id = register(&mut conn, fleet_id, "holder", Some("%2"));
        let silent_id = register(&mut conn, fleet_id, "silent", Some("%3"));
        common::send(&mut conn, director_id, holder_id, "hi");
        broker::deregister_member(&mut conn, holder_id).unwrap();
        broker::deregister_member(&mut conn, silent_id).unwrap();
        let placed_id = register(&mut conn, fleet_id, "placed", Some("%4"));
        let pending_id = register(&mut conn, fleet_id, "pending", None);
        let paneless_id = broker::register_member(&mut conn, fleet_id, "paneless", "test", None)
            .unwrap()
            .member_id;
        let (other_fleet, _) = create_fleet(&mut conn, "other");
        register(&mut conn, other_fleet, "other-worker", Some("%9"));

        let roster = broker::members::capture_roster(&conn, fleet_id).unwrap();
        assert_eq!(
            roster
                .iter()
                .map(|member| member.member_id)
                .collect::<Vec<_>>(),
            vec![director_id, placed_id, pending_id, paneless_id]
        );
        assert!(
            roster
                .iter()
                .all(|member| member.status == crate::broker::records::MemberStatus::Active)
        );
        assert_eq!(
            roster[1].placement.as_ref().unwrap().mux_pane_id.as_deref(),
            Some("%4")
        );
        assert_eq!(roster[2].placement.as_ref().unwrap().mux_pane_id, None);
        assert_eq!(roster[3].placement, None);
    }
}

#[cfg(test)]
mod step6_behavior_regressions {
    use super::*;
    use crate::broker::records::MemberStatus;

    fn activity(sent: Option<&str>, received: Option<&str>, ack: Option<&str>) -> MemberActivity {
        MemberActivity {
            member: MemberRecord {
                member_id: 1,
                fleet_id: 1,
                name: "worker".into(),
                description: String::new(),
                registered_at: String::new(),
                status: MemberStatus::Active,
                kind: MemberKind::Member,
                placement: None,
            },
            last_sent: sent.map(str::to_owned),
            last_recv: received.map(str::to_owned),
            last_ack: ack.map(str::to_owned),
            idle: None,
        }
    }

    #[test]
    fn step6_idle_clamps_future_activity_to_zero() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        for row in [
            activity(Some("2026-01-01T00:01:00Z"), None, None),
            activity(None, Some("2026-01-01T00:01:00Z"), None),
            activity(None, None, Some("2026-01-01T00:01:00Z")),
        ] {
            assert_eq!(idle_seconds(&row, now), Some(0));
        }
    }

    #[test]
    fn step6_idle_null_and_invalid_maximum_do_not_fall_back_to_older_valid_times() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(idle_seconds(&activity(None, None, None), now), None);
        for row in [
            activity(Some("z-invalid"), Some("2025-12-31T23:59:50Z"), None),
            activity(None, Some("z-invalid"), Some("2025-12-31T23:59:50Z")),
        ] {
            assert_eq!(idle_seconds(&row, now), None);
        }
    }

    #[test]
    fn step6_idle_selects_raw_lexicographic_maximum_before_parsing_offsets() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        let row = activity(
            Some("2026-01-01T08:00:00+09:00"),
            Some("2025-12-31T23:59:50Z"),
            None,
        );
        assert_eq!(idle_seconds(&row, now), Some(3600));
    }

    #[test]
    fn step6_idle_preserves_fractional_second_truncation() {
        let now = parse_lenient("2026-01-01T00:00:01.900000+00:00").unwrap();
        let row = activity(Some("2026-01-01T00:00:00.950000+00:00"), None, None);
        assert_eq!(idle_seconds(&row, now), Some(0));
        let row = activity(None, None, Some("2025-12-31T23:59:59.950000+00:00"));
        assert_eq!(idle_seconds(&row, now), Some(1));
    }
}
