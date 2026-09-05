//! Member registry, placement, roster, and activity proxies (SPEC §6.2
//! *Members*), including the monitor-member card marker
//! (`$.cafleet.kind = 'monitor'`) behind the three-value member kind. The
//! colocated tests pin the contract; see [`super::test_support`] for the API.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use super::records::{
    MemberActivity, MemberKind, MemberRecord, MemberStatus, Placement, RegisteredMember,
};
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

pub(crate) fn member_card(
    name: &str,
    description: &str,
    skills: &[Value],
    monitor: bool,
) -> String {
    let mut card = json!({"name": name, "description": description, "skills": skills});
    if monitor {
        card["cafleet"] = json!({"kind": "monitor"});
    }
    card.to_string()
}

/// The three-value derivation over the SQL-supplied `is_director` flag plus
/// the member card's `$.cafleet.kind` marker (SPEC §5.4); `director` wins, so
/// a root Director can never read as `monitor` regardless of its card.
fn member_kind(is_director: bool, is_monitor: bool) -> MemberKind {
    if is_director {
        MemberKind::Director
    } else if is_monitor {
        MemberKind::Monitor
    } else {
        MemberKind::Member
    }
}

const IS_MONITOR_COLUMN: &str =
    "COALESCE(json_extract(m.member_card_json, '$.cafleet.kind')='monitor', 0)";

fn skills_from_card(card_json: &str) -> Vec<Value> {
    let card: Value = serde_json::from_str(card_json).unwrap_or(Value::Null);
    match card.get("skills") {
        Some(Value::Array(skills)) => skills.clone(),
        _ => Vec::new(),
    }
}

pub fn register_member_record(
    conn: &mut Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    skills: &[Value],
    placement: Option<&NewPlacement>,
    monitor: bool,
) -> Result<RegisteredMember, CafleetError> {
    let fleet = super::fleets::fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::Usage(format!("Fleet '{fleet_id}' not found.")))?;
    if fleet.deleted_at.is_some() {
        return Err(CafleetError::Usage(format!("fleet {fleet_id} is deleted")));
    }
    if placement.is_some()
        && let Some(director_id) = fleet.director_member_id
    {
        let director_active: bool = conn
            .query_row(
                "SELECT status='active' FROM members WHERE member_id=?1",
                [director_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if !director_active {
            return Err(CafleetError::App(format!(
                "fleet {fleet_id}'s root Director (member {director_id}) is not active."
            )));
        }
    }

    let now = format_utc(now_utc());
    let card = member_card(name, description, skills, monitor);
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    if monitor && let Some(member_id) = active_monitor_member_id(&tx, fleet_id)? {
        return Err(CafleetError::ActiveMonitorExists {
            fleet_id,
            member_id,
        });
    }
    let inserted = tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at, member_card_json) \
         VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
        params![fleet_id, name, description, now, card],
    );
    if let Err(error) = inserted {
        // SQLite reports column names for this (non-expression) index. Match
        // its extended code and exact column, then require a monitor conflict
        // in this same writer transaction. Other constraints retain their cause.
        if monitor
            && matches!(&error, rusqlite::Error::SqliteFailure(code, Some(message))
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    && message == "UNIQUE constraint failed: members.fleet_id")
            && let Ok(Some(member_id)) = active_monitor_member_id(&tx, fleet_id)
        {
            return Err(CafleetError::ActiveMonitorExists {
                fleet_id,
                member_id,
            });
        }
        return Err(db_err(error));
    }
    let member_id = tx.last_insert_rowid();
    if let Some(p) = placement {
        tx.execute(
            "INSERT INTO member_placements \
             (member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) \
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

pub fn get_member_record(
    conn: &Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<Option<MemberRecord>, CafleetError> {
    let row = conn
        .query_row(
            &format!(
                "SELECT m.name, m.description, m.status, m.registered_at, m.member_card_json, \
                        EXISTS(SELECT 1 FROM fleets f \
                               WHERE f.fleet_id=m.fleet_id AND f.director_member_id=m.member_id), \
                        p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, \
                        p.created_at, {IS_MONITOR_COLUMN} \
                 FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id \
                 WHERE m.member_id=?1 AND m.fleet_id=?2 AND m.status='active'"
            ),
            params![member_id, fleet_id],
            |row| {
                let backend: Option<String> = row.get(6)?;
                Ok(MemberRecord {
                    member_id,
                    fleet_id,
                    name: row.get(0)?,
                    description: row.get(1)?,
                    status: row.get(2)?,
                    registered_at: row.get(3)?,
                    kind: member_kind(row.get(5)?, row.get(12)?),
                    skills: skills_from_card(&row.get::<_, String>(4)?),
                    placement: backend.map(|_| Placement::from_row(row, 6)).transpose()?,
                })
            },
        )
        .optional()
        .map_err(db_err)?;
    Ok(row)
}

/// The fleet's single `status='active'` member whose card carries the
/// monitor marker (`$.cafleet.kind = 'monitor'`), or `None`. Consumed by the
/// CLI's two `member create` monitor-role guards and the tick's monitor-pane
/// resolution.
pub fn active_monitor_member_id(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<i64>, CafleetError> {
    conn.query_row(
        "SELECT member_id FROM members \
         WHERE fleet_id=?1 AND status='active' \
           AND json_extract(member_card_json, '$.cafleet.kind')='monitor'",
        [fleet_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
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

pub fn update_placement_record(
    conn: &mut Connection,
    member_id: i64,
    pane_id: &str,
) -> Result<Option<Placement>, CafleetError> {
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
    let mut names = BTreeMap::new();
    for &member_id in member_ids {
        let name: Option<String> = conn
            .query_row(
                "SELECT name FROM members WHERE member_id=?1",
                [member_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)?;
        if let Some(name) = name {
            names.insert(member_id, name);
        }
    }
    Ok(names)
}

fn roster_rows(
    conn: &Connection,
    fleet_id: i64,
    include_message_holders: bool,
) -> Result<Vec<MemberActivity>, CafleetError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT m.member_id, m.name, m.status, \
                    EXISTS(SELECT 1 FROM fleets f \
                           WHERE f.fleet_id=m.fleet_id AND f.director_member_id=m.member_id), \
                    p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, \
                    p.created_at, \
                    (SELECT MAX(created_at) FROM messages WHERE from_member_id=m.member_id), \
                    (SELECT MAX(created_at) FROM messages \
                     WHERE owner_member_id=m.member_id AND type='unicast'), \
                    (SELECT MAX(status_timestamp) FROM messages \
                     WHERE owner_member_id=m.member_id AND type='unicast' \
                       AND status_state='completed'), \
                    m.description, m.registered_at, {IS_MONITOR_COLUMN}, m.member_card_json \
             FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND (m.status='active' OR (?2 AND EXISTS( \
                   SELECT 1 FROM messages WHERE owner_member_id=m.member_id))) \
             ORDER BY m.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![fleet_id, include_message_holders], |row| {
            let backend: Option<String> = row.get(4)?;
            Ok(MemberActivity {
                member: MemberRecord {
                    member_id: row.get(0)?,
                    fleet_id,
                    name: row.get(1)?,
                    status: row.get::<_, MemberStatus>(2)?,
                    kind: member_kind(row.get(3)?, row.get(15)?),
                    placement: backend.map(|_| Placement::from_row(row, 4)).transpose()?,
                    description: row.get(13)?,
                    registered_at: row.get(14)?,
                    skills: skills_from_card(&row.get::<_, String>(16)?),
                },
                last_sent: row.get(10)?,
                last_recv: row.get(11)?,
                last_ack: row.get(12)?,
                idle: None,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn idle_seconds(row: &MemberActivity, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let latest = [&row.last_sent, &row.last_recv, &row.last_ack]
        .into_iter()
        .flatten()
        .max()?;
    let parsed = parse_lenient(latest).ok()?;
    Some((now - parsed).num_seconds())
}

pub fn list_member_records(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<MemberActivity>, CafleetError> {
    let now = now_utc();
    let mut rows = roster_rows(conn, fleet_id, false)?;
    for row in &mut rows {
        row.idle = idle_seconds(row, now);
    }
    Ok(rows)
}

pub fn list_roster_records(
    conn: &Connection,
    fleet_id: i64,
    include_message_holders: bool,
) -> Result<Vec<MemberRecord>, CafleetError> {
    Ok(roster_rows(conn, fleet_id, include_message_holders)?
        .into_iter()
        .map(|row| row.member)
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, bootstrap_monitor, create_fleet, migrated_conn, placement, register,
        register_monitor,
    };
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn register_member_returns_the_registration_summary() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member_record(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            &[],
            Some(&placement(Some("%2"))),
            false,
        )
        .map(|record| crate::presentation::registered_member(&record))
        .unwrap();
        assert!(result["member_id"].as_i64().is_some());
        assert_eq!(result["name"], "analyst");
        assert!(crate::time::parse_lenient(result["registered_at"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn register_member_writes_no_monitor_config_row() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        register(&mut conn, fleet_id, "analyst", Some("%2"));
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='monitor_config'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "registration performs no monitoring enrollment");
    }

    #[test]
    fn get_member_shape_is_pinned() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member_record(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            &[],
            Some(&placement(Some("%2"))),
            false,
        )
        .map(|record| crate::presentation::registered_member(&record))
        .unwrap();
        let member_id = result["member_id"].as_i64().unwrap();
        let ts = result["registered_at"].as_str().unwrap().to_string();

        let member = broker::get_member_record(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        let expected = format!(
            r#"{{"member_id":{member_id},"name":"analyst","description":"test member","status":"active","registered_at":"{ts}","kind":"member","skills":[],"placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%2","coding_agent":"claude","created_at":"{ts}"}}}}"#
        );
        assert_eq!(format_json(&member), expected);
    }

    #[test]
    fn register_member_carries_skills_verbatim() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let skills = [json!("python"), json!("sql")];
        let member_id = broker::register_member_record(
            &mut conn,
            fleet_id,
            "analyst",
            "d",
            &skills,
            Some(&placement(Some("%2"))),
            false,
        )
        .map(|record| crate::presentation::registered_member(&record))
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        let member = broker::get_member_record(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(member["skills"], json!(["python", "sql"]));
    }

    #[test]
    fn placementless_member_has_null_placement() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id =
            broker::register_member_record(&mut conn, fleet_id, "ghost", "d", &[], None, false)
                .map(|record| crate::presentation::registered_member(&record))
                .unwrap()["member_id"]
                .as_i64()
                .unwrap();
        let member = broker::get_member_record(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"], Value::Null);
        let ts = member["registered_at"].as_str().unwrap();
        assert_eq!(
            format_json(&member),
            format!(
                r#"{{"member_id":{member_id},"name":"ghost","description":"d","status":"active","registered_at":"{ts}","kind":"member","skills":[],"placement":null}}"#
            )
        );
    }

    #[test]
    fn register_member_unknown_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = broker::register_member_record(&mut conn, 999, "x", "d", &[], None, false)
            .map(|record| crate::presentation::registered_member(&record))
            .expect_err("unknown fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), "Fleet '999' not found.");
    }

    #[test]
    fn register_member_deleted_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        broker::delete_fleet(&mut conn, fleet_id).unwrap();
        let err = broker::register_member_record(&mut conn, fleet_id, "x", "d", &[], None, false)
            .map(|record| crate::presentation::registered_member(&record))
            .expect_err("deleted fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), format!("fleet {fleet_id} is deleted"));
    }

    #[test]
    fn root_director_invariant_guard_fires_only_for_placed_registrations() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        conn.execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [director_id],
        )
        .unwrap();

        let err = broker::register_member_record(
            &mut conn,
            fleet_id,
            "worker",
            "d",
            &[],
            Some(&placement(Some("%2"))),
            false,
        )
        .map(|record| crate::presentation::registered_member(&record))
        .expect_err("a placed registration under an inactive Director must fail loudly");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            format!("fleet {fleet_id}'s root Director (member {director_id}) is not active.")
        );

        broker::register_member_record(&mut conn, fleet_id, "ghost", "d", &[], None, false)
            .map(|record| crate::presentation::registered_member(&record))
            .expect("a placementless registration skips the invariant guard");
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
            broker::get_member_record(&conn, member_id, fleet_b)
                .map(|record| record.as_ref().map(crate::presentation::member))
                .unwrap()
                .is_none()
        );
        broker::deregister_member(&mut conn, member_id).unwrap();
        assert!(
            broker::get_member_record(&conn, member_id, fleet_a)
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

        let updated = broker::update_placement_record(&mut conn, member_id, "%9")
            .map(|record| record.as_ref().map(crate::presentation::placement))
            .unwrap()
            .unwrap();
        assert_eq!(updated["mux_pane_id"], "%9");
        let member = broker::get_member_record(&conn, member_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"]["mux_pane_id"], "%9");

        let placementless =
            broker::register_member_record(&mut conn, fleet_id, "ghost", "d", &[], None, false)
                .map(|record| crate::presentation::registered_member(&record))
                .unwrap()["member_id"]
                .as_i64()
                .unwrap();
        assert!(
            broker::update_placement_record(&mut conn, placementless, "%1")
                .map(|record| record.as_ref().map(crate::presentation::placement))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn verify_member_fleet_is_status_agnostic() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, _) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let member_id = register(&mut conn, fleet_a, "worker", Some("%2"));

        assert!(broker::verify_member_fleet(&conn, member_id, fleet_a).unwrap());
        assert!(!broker::verify_member_fleet(&conn, member_id, fleet_b).unwrap());
        broker::deregister_member(&mut conn, member_id).unwrap();
        assert!(broker::verify_member_fleet(&conn, member_id, fleet_a).unwrap());
    }

    #[test]
    fn get_member_names_batches_and_includes_deregistered() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        broker::deregister_member(&mut conn, member_id).unwrap();

        assert!(broker::get_member_names(&conn, &[]).unwrap().is_empty());
        let names = broker::get_member_names(&conn, &[director_id, member_id]).unwrap();
        assert_eq!(
            names.get(&director_id).map(String::as_str),
            Some("Director")
        );
        assert_eq!(names.get(&member_id).map(String::as_str), Some("worker"));
    }

    #[test]
    fn list_members_rows_carry_kind_placement_and_activity_proxies() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let ghost_id =
            broker::register_member_record(&mut conn, fleet_id, "ghost", "d", &[], None, false)
                .map(|record| crate::presentation::registered_member(&record))
                .unwrap()["member_id"]
                .as_i64()
                .unwrap();

        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let rows = broker::list_member_records(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::member_activity)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            rows.len(),
            4,
            "Director + bootstrap monitor + worker + ghost"
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

        broker::ack_message_record(&mut conn, message_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
        let rows = broker::list_member_records(&conn, fleet_id)
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
        let rows = broker::list_member_records(&conn, fleet_id)
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
    fn monitor_registration_writes_the_member_card_kind_marker() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let dead_monitor = bootstrap_monitor(&conn, fleet_id);
        broker::deregister_member(&mut conn, dead_monitor).unwrap();
        let monitor_id = register_monitor(&mut conn, fleet_id, "monitor", Some("%2"));
        let member_id = register(&mut conn, fleet_id, "worker", Some("%3"));

        let cafleet_object = |id: i64| -> Option<String> {
            conn.query_row(
                "SELECT json_extract(member_card_json, '$.cafleet') \
                 FROM members WHERE member_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
        };
        let monitor_kind: Option<String> = conn
            .query_row(
                "SELECT json_extract(member_card_json, '$.cafleet.kind') \
                 FROM members WHERE member_id=?1",
                [monitor_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(monitor_kind.as_deref(), Some("monitor"));
        assert_eq!(
            cafleet_object(member_id),
            None,
            "an ordinary member writes no $.cafleet object"
        );
        assert_eq!(
            cafleet_object(director_id),
            None,
            "the Director writes no $.cafleet object"
        );

        let monitor = broker::get_member_record(&conn, monitor_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(
            monitor["skills"],
            json!([]),
            "the marker joins the card without displacing its keys"
        );
    }

    #[test]
    fn active_monitor_member_id_counts_only_active_monitor_members() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, _) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let monitor_a = bootstrap_monitor(&conn, fleet_a);
        let monitor_b = bootstrap_monitor(&conn, fleet_b);
        assert_ne!(monitor_a, monitor_b, "the lookup is fleet-scoped");

        broker::deregister_member(&mut conn, monitor_a).unwrap();
        assert_eq!(
            broker::active_monitor_member_id(&conn, fleet_a).unwrap(),
            None,
            "a deregistered monitor frees the slot"
        );
        assert_eq!(
            broker::active_monitor_member_id(&conn, fleet_b).unwrap(),
            Some(monitor_b),
            "a neighbour fleet's monitor never leaks into the lookup"
        );

        register(&mut conn, fleet_a, "worker", Some("%2"));
        assert_eq!(
            broker::active_monitor_member_id(&conn, fleet_a).unwrap(),
            None,
            "an ordinary member never fills the monitor slot"
        );

        let recovered_id = register_monitor(&mut conn, fleet_a, "monitor", Some("%3"));
        assert_eq!(
            broker::active_monitor_member_id(&conn, fleet_a).unwrap(),
            Some(recovered_id),
            "the recovery re-spawn refills the slot"
        );
    }

    #[test]
    fn member_kind_is_three_valued_across_member_views() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let monitor_id = bootstrap_monitor(&conn, fleet_id);
        let member_id = register(&mut conn, fleet_id, "worker", Some("%3"));
        let expected = [
            (director_id, "director"),
            (monitor_id, "monitor"),
            (member_id, "member"),
        ];

        for (id, kind) in expected {
            let member = broker::get_member_record(&conn, id, fleet_id)
                .map(|record| record.as_ref().map(crate::presentation::member))
                .unwrap()
                .unwrap();
            assert_eq!(member["kind"], kind, "get_member kind for member {id}");
        }
        let rows = broker::list_member_records(&conn, fleet_id)
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
        let roster = broker::list_roster_records(&conn, fleet_id, false)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::roster_member)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        for (id, kind) in expected {
            let row = roster.iter().find(|r| r["member_id"] == id).unwrap();
            assert_eq!(row["kind"], kind, "list_roster kind for member {id}");
        }
    }

    #[test]
    fn list_roster_surfaces_deregistered_message_holders_only_on_request() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let holder_id = register(&mut conn, fleet_id, "holder", Some("%2"));
        let silent_id = register(&mut conn, fleet_id, "silent", Some("%3"));
        let notifier = FakeNotifier::succeeding();
        common::send(&mut conn, &notifier, director_id, holder_id, "hi");
        broker::deregister_member(&mut conn, holder_id).unwrap();
        broker::deregister_member(&mut conn, silent_id).unwrap();

        let active_only = broker::list_roster_records(&conn, fleet_id, false)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::roster_member)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert!(active_only.iter().all(|r| r["member_id"] != holder_id));

        let with_holders = broker::list_roster_records(&conn, fleet_id, true)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::roster_member)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        let holder = with_holders
            .iter()
            .find(|r| r["member_id"] == holder_id)
            .expect("a deregistered member owning messages stays visible");
        assert_eq!(holder["status"], "deregistered");
        assert_eq!(holder["placement"], Value::Null);
        assert!(
            with_holders.iter().all(|r| r["member_id"] != silent_id),
            "a deregistered member without messages stays hidden"
        );
    }
}

#[cfg(test)]
mod compatibility_regressions {
    use super::*;
    use crate::broker::test_support as common;

    #[test]
    fn card_skills_keeps_legacy_empty_fallback_for_broken_missing_or_non_array_values() {
        for raw in [
            "not JSON",
            "{",
            "null",
            "[]",
            "{}",
            r#"{"skills":null}"#,
            r#"{"skills":"rust"}"#,
            r#"{"skills":{}}"#,
            r#"{"skills":42}"#,
        ] {
            assert_eq!(skills_from_card(raw), Vec::<Value>::new(), "{raw}");
        }
    }

    #[test]
    fn free_form_skills_preserve_nested_values_and_order_through_member_presentation() {
        let dir = tempfile::Builder::new()
            .prefix(".member-wire-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, _) = common::create_fleet(&mut conn, "compatibility");
        let skills = vec![
            json!(null),
            json!(true),
            json!(4),
            json!({"nested":["日本語",false]}),
            json!([3, 2, 1]),
        ];
        let registered =
            register_member_record(&mut conn, fleet, "worker", "desc", &skills, None, false)
                .map(|record| crate::presentation::registered_member(&record))
                .unwrap();
        let id = registered["member_id"].as_i64().unwrap();
        let member = get_member_record(&conn, id, fleet)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(member["skills"], json!(skills));
        for raw in ["{}", r#"{"skills":false}"#, r#"{"skills":"not-array"}"#] {
            conn.execute(
                "UPDATE members SET member_card_json=?1 WHERE member_id=?2",
                params![raw, id],
            )
            .unwrap();
            assert_eq!(
                get_member_record(&conn, id, fleet)
                    .map(|record| record.as_ref().map(crate::presentation::member))
                    .unwrap()
                    .unwrap()["skills"],
                json!([])
            );
        }
    }

    #[test]
    fn pending_placement_keeps_an_object_with_null_pane_and_exact_wire_order() {
        let dir = tempfile::Builder::new()
            .prefix(".placement-wire-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, _) = common::create_fleet(&mut conn, "compatibility");
        let id = common::register(&mut conn, fleet, "worker", None);
        let member = get_member_record(&conn, id, fleet)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        let ts = member["placement"]["created_at"].as_str().unwrap();
        assert_eq!(
            crate::output::format_json(&member["placement"]),
            format!(
                r#"{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":null,"coding_agent":"claude","created_at":"{ts}"}}"#
            )
        );
        let ghost = register_member_record(&mut conn, fleet, "ghost", "desc", &[], None, false)
            .map(|record| crate::presentation::registered_member(&record))
            .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        assert!(
            get_member_record(&conn, ghost, fleet)
                .map(|record| record.as_ref().map(crate::presentation::member))
                .unwrap()
                .unwrap()["placement"]
                .is_null()
        );
    }
}

#[cfg(test)]
mod step6_behavior_regressions {
    use super::*;
    use crate::broker::{self, test_support as common};

    fn memory_fleet() -> (Connection, i64, i64) {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let (fleet, director) = common::create_fleet(&mut conn, "query");
        (conn, fleet, director)
    }

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
                skills: vec![],
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

    #[test]
    fn step6_name_lookup_boundaries_return_sorted_unique_known_and_deregistered_ids() {
        let (conn, fleet, _) = memory_fleet();
        for id in 10000..11001_i64 {
            conn.execute("INSERT INTO members(member_id,fleet_id,name,description,status,registered_at,member_card_json) VALUES (?1,?2,?3,'','deregistered','2026-01-01T00:00:00Z','{}')", params![id,fleet,format!("member-{id}")]).unwrap();
        }
        for count in [0, 1, 500, 501, 1001] {
            let ids: Vec<i64> = (10000..10000 + count).rev().collect();
            let repeated: Vec<i64> = ids.iter().copied().cycle().take(ids.len() * 4).collect();
            let names = broker::get_member_names(&conn, &repeated).unwrap();
            assert_eq!(
                names.keys().copied().collect::<Vec<_>>(),
                (10000..10000 + count).collect::<Vec<_>>()
            );
            for (id, name) in names {
                assert_eq!(name, format!("member-{id}"));
            }
        }
        let names = broker::get_member_names(&conn, &[10000, i64::MAX, -1, 10000]).unwrap();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec![(10000, "member-10000".into())]
        );
    }

    #[test]
    fn step6_roster_includes_only_owner_history_and_preserves_role_placement_and_id_order() {
        let (mut conn, fleet, director) = memory_fleet();
        let owner = common::register(&mut conn, fleet, "owner", Some("%2"));
        let sender = common::register(&mut conn, fleet, "sender only", Some("%3"));
        let pending = common::register(&mut conn, fleet, "pending", None);
        common::send(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            sender,
            owner,
            "history",
        );
        broker::deregister_member(&mut conn, owner).unwrap();
        broker::deregister_member(&mut conn, sender).unwrap();
        let rows = broker::list_roster_records(&conn, fleet, true).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.member_id).collect::<Vec<_>>(),
            vec![
                director,
                common::bootstrap_monitor(&conn, fleet),
                owner,
                pending
            ]
        );
        assert_eq!(rows[0].kind, MemberKind::Director);
        assert_eq!(rows[1].kind, MemberKind::Monitor);
        assert_eq!(rows[2].status, MemberStatus::Deregistered);
        assert_eq!(rows[2].placement, None);
        assert_eq!(rows[3].kind, MemberKind::Member);
        assert_eq!(rows[3].placement.as_ref().unwrap().mux_pane_id, None);
        assert!(
            broker::list_roster_records(&conn, fleet, false)
                .unwrap()
                .iter()
                .all(|r| r.member_id != owner && r.member_id != sender)
        );
    }

    #[test]
    fn step6_activity_uses_owner_delivery_times_and_only_ack_status_time() {
        let (mut conn, fleet, director) = memory_fleet();
        let member = common::register(&mut conn, fleet, "worker", None);
        let sent = broker::send_message_record(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            common::MAX_TEXT_LEN,
            director,
            &member.to_string(),
            "work",
        )
        .unwrap()
        .message;
        let created = "2020-01-01T00:00:00Z";
        conn.execute(
            "UPDATE messages SET created_at=?1,status_timestamp=?1 WHERE message_id=?2",
            params![created, sent.message_id],
        )
        .unwrap();
        // Read fixture deliberately separates owner from the recipient column.
        conn.execute(
            "UPDATE messages SET to_member_id=?1 WHERE message_id=?2",
            params![director, sent.message_id],
        )
        .unwrap();
        let before = broker::list_member_records(&conn, fleet)
            .unwrap()
            .into_iter()
            .find(|r| r.member.member_id == member)
            .unwrap();
        assert_eq!(before.last_recv.as_deref(), Some(created));
        assert_eq!(before.last_ack, None);
        let ack = broker::ack_message_record(&mut conn, sent.message_id).unwrap();
        let after = broker::list_member_records(&conn, fleet)
            .unwrap()
            .into_iter()
            .find(|r| r.member.member_id == member)
            .unwrap();
        assert_eq!(ack.created_at, created);
        assert_eq!(after.last_recv, before.last_recv);
        assert_eq!(after.last_sent, None);
        assert_eq!(after.last_ack, Some(ack.status_timestamp));
        let director_row = broker::list_member_records(&conn, fleet)
            .unwrap()
            .into_iter()
            .find(|r| r.member.member_id == director)
            .unwrap();
        assert_eq!(director_row.last_sent.as_deref(), Some(created));
        assert_eq!(director_row.last_recv, None);
        assert_eq!(director_row.last_ack, None);
    }

    #[test]
    fn step6_broadcast_summary_counts_as_sent_but_never_received_or_acknowledged() {
        let (mut conn, fleet, director) = memory_fleet();
        let broadcast = broker::broadcast_message_record(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            common::MAX_TEXT_LEN,
            director,
            "broadcast",
        )
        .unwrap();
        conn.execute(
            "UPDATE messages SET created_at='2020-01-01T00:00:00Z' WHERE type='unicast'",
            [],
        )
        .unwrap();
        conn.execute("UPDATE messages SET created_at='2021-01-01T00:00:00Z',status_timestamp='2022-01-01T00:00:00Z' WHERE message_id=?1", [broadcast.message.message_id]).unwrap();
        let row = broker::list_member_records(&conn, fleet)
            .unwrap()
            .into_iter()
            .find(|r| r.member.member_id == director)
            .unwrap();
        assert_eq!(row.last_sent.as_deref(), Some("2021-01-01T00:00:00Z"));
        assert_eq!(row.last_recv, None);
        assert_eq!(row.last_ack, None);
    }
}
