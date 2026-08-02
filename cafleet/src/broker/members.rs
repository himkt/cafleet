//! Member registry, placement, roster, and activity proxies (SPEC §6.2
//! *Members*). The colocated tests pin the contract; see
//! [`super::test_support`] for the API.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::error::CafleetError;
use crate::time::{format_utc, now_utc, parse_lenient};

pub const MONITORING_MEMBER_KIND: &str = "monitoring-member";

/// The auto-enrollment ping cadence (SPEC §6.2) — the policy tunables'
/// single home, re-exported by the monitor module.
pub const MEMBER_PING_INTERVAL_SECONDS: i64 = 720;

#[derive(Debug, Clone)]
pub struct NewPlacement {
    pub backend: String,
    pub mux_session: String,
    pub mux_window_id: String,
    pub mux_pane_id: Option<String>,
    pub coding_agent: String,
}

pub(crate) fn db_err(e: rusqlite::Error) -> CafleetError {
    CafleetError::App(format!("database error: {e}"))
}

pub(crate) fn member_card(
    name: &str,
    description: &str,
    skills: &[Value],
    kind: Option<&str>,
) -> String {
    let mut card = json!({"name": name, "description": description, "skills": skills});
    if let Some(kind) = kind {
        card["cafleet"] = json!({"kind": kind});
    }
    card.to_string()
}

/// The single three-value collapse over the SQL-supplied `is_director` flag
/// and the raw card kind (SPEC §5.4); a malformed card kind deliberately
/// collapses to the ordinary kind.
pub(crate) fn derive_member_kind(is_director: bool, card_json: &str) -> &'static str {
    if is_director {
        return "director";
    }
    let card: Value = serde_json::from_str(card_json).unwrap_or(Value::Null);
    if card["cafleet"]["kind"] == MONITORING_MEMBER_KIND {
        "monitor"
    } else {
        "member"
    }
}

pub(crate) fn card_skills(card_json: &str) -> Value {
    let card: Value = serde_json::from_str(card_json).unwrap_or(Value::Null);
    match card.get("skills") {
        Some(Value::Array(skills)) => Value::Array(skills.clone()),
        _ => json!([]),
    }
}

pub(crate) fn placement_value(
    backend: &str,
    mux_session: &str,
    mux_window_id: &str,
    mux_pane_id: Option<&str>,
    coding_agent: &str,
    created_at: &str,
) -> Value {
    json!({
        "backend": backend,
        "mux_session": mux_session,
        "mux_window_id": mux_window_id,
        "mux_pane_id": mux_pane_id,
        "coding_agent": coding_agent,
        "created_at": created_at,
    })
}

pub(crate) fn enroll(conn: &Connection, member_id: i64) -> Result<(), CafleetError> {
    conn.execute(
        "INSERT INTO monitor_config (member_id, interval_seconds, enabled) VALUES (?1, ?2, 1)",
        params![member_id, MEMBER_PING_INTERVAL_SECONDS],
    )
    .map_err(db_err)?;
    Ok(())
}

fn active_monitoring_member_id(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<i64>, CafleetError> {
    conn.query_row(
        "SELECT member_id FROM members \
         WHERE fleet_id=?1 AND status='active' \
           AND json_extract(member_card_json, '$.cafleet.kind')=?2 \
         ORDER BY member_id LIMIT 1",
        params![fleet_id, MONITORING_MEMBER_KIND],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_err)
}

pub fn register_member(
    conn: &mut Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    skills: &[Value],
    placement: Option<&NewPlacement>,
    kind: Option<&str>,
) -> Result<Value, CafleetError> {
    let fleet = super::fleets::fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::Usage(format!("Fleet '{fleet_id}' not found.")))?;
    if fleet.deleted_at.is_some() {
        return Err(CafleetError::Usage(format!("fleet {fleet_id} is deleted")));
    }
    let is_monitor = kind == Some(MONITORING_MEMBER_KIND);
    if is_monitor {
        if placement.is_none() {
            return Err(CafleetError::App(
                "a monitoring member must be pane-bound; register it via \
                 'cafleet member create --role monitor' (placement required)."
                    .to_string(),
            ));
        }
        if let Some(existing) = active_monitoring_member_id(conn, fleet_id)? {
            return Err(CafleetError::App(format!(
                "fleet {fleet_id} already has an active monitoring member \
                 (member {existing}); only one is allowed."
            )));
        }
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
    let card = member_card(name, description, skills, kind);
    let tx = conn.transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at, member_card_json) \
         VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
        params![fleet_id, name, description, now, card],
    )
    .map_err(db_err)?;
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
        if !is_monitor {
            enroll(&tx, member_id)?;
        }
    }
    tx.commit().map_err(db_err)?;
    Ok(json!({"member_id": member_id, "name": name, "registered_at": now}))
}

pub fn get_member(
    conn: &Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<Option<Value>, CafleetError> {
    let row = conn
        .query_row(
            "SELECT m.name, m.description, m.status, m.registered_at, m.member_card_json, \
                    EXISTS(SELECT 1 FROM fleets f \
                           WHERE f.fleet_id=m.fleet_id AND f.director_member_id=m.member_id), \
                    p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, \
                    p.created_at \
             FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.member_id=?1 AND m.fleet_id=?2 AND m.status='active'",
            params![member_id, fleet_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            },
        )
        .optional()
        .map_err(db_err)?;
    Ok(row.map(
        |(
            name,
            description,
            status,
            registered_at,
            card,
            is_director,
            backend,
            session,
            window,
            pane,
            agent,
            created,
        )| {
            let placement = match backend {
                None => Value::Null,
                Some(backend) => placement_value(
                    &backend,
                    session
                        .as_deref()
                        .expect("placement row carries mux_session"),
                    window
                        .as_deref()
                        .expect("placement row carries mux_window_id"),
                    pane.as_deref(),
                    agent
                        .as_deref()
                        .expect("placement row carries coding_agent"),
                    created
                        .as_deref()
                        .expect("placement row carries created_at"),
                ),
            };
            json!({
                "member_id": member_id,
                "name": name,
                "description": description,
                "status": status,
                "registered_at": registered_at,
                "kind": derive_member_kind(is_director, &card),
                "skills": card_skills(&card),
                "placement": placement,
            })
        },
    ))
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
    tx.execute("DELETE FROM monitor_config WHERE member_id=?1", [member_id])
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(true)
}

pub fn update_placement_pane_id(
    conn: &mut Connection,
    member_id: i64,
    pane_id: &str,
) -> Result<Option<Value>, CafleetError> {
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
        |row| {
            Ok(placement_value(
                &row.get::<_, String>(0)?,
                &row.get::<_, String>(1)?,
                &row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?.as_deref(),
                &row.get::<_, String>(4)?,
                &row.get::<_, String>(5)?,
            ))
        },
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

struct RosterRow {
    member_id: i64,
    name: String,
    status: String,
    card: String,
    is_director: bool,
    placement: Option<Value>,
    last_sent: Option<String>,
    last_recv: Option<String>,
    last_ack: Option<String>,
    description: String,
    registered_at: String,
}

fn roster_rows(
    conn: &Connection,
    fleet_id: i64,
    include_message_holders: bool,
) -> Result<Vec<RosterRow>, CafleetError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.member_id, m.name, m.status, m.member_card_json, \
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
                    m.description, m.registered_at \
             FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND (m.status='active' OR (?2 AND EXISTS( \
                   SELECT 1 FROM messages WHERE owner_member_id=m.member_id))) \
             ORDER BY m.member_id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![fleet_id, include_message_holders], |row| {
            let backend: Option<String> = row.get(5)?;
            let placement = match backend {
                None => None,
                Some(backend) => Some(placement_value(
                    &backend,
                    &row.get::<_, String>(6)?,
                    &row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?.as_deref(),
                    &row.get::<_, String>(9)?,
                    &row.get::<_, String>(10)?,
                )),
            };
            Ok(RosterRow {
                member_id: row.get(0)?,
                name: row.get(1)?,
                status: row.get(2)?,
                card: row.get(3)?,
                is_director: row.get(4)?,
                placement,
                last_sent: row.get(11)?,
                last_recv: row.get(12)?,
                last_ack: row.get(13)?,
                description: row.get(14)?,
                registered_at: row.get(15)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn idle_seconds(row: &RosterRow, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let latest = [&row.last_sent, &row.last_recv, &row.last_ack]
        .into_iter()
        .flatten()
        .max()?;
    let parsed = parse_lenient(latest).ok()?;
    Some((now - parsed).num_seconds())
}

pub fn list_members(conn: &Connection, fleet_id: i64) -> Result<Vec<Value>, CafleetError> {
    let now = now_utc();
    Ok(roster_rows(conn, fleet_id, false)?
        .into_iter()
        .map(|row| {
            let idle = idle_seconds(&row, now);
            json!({
                "member_id": row.member_id,
                "name": row.name,
                "kind": derive_member_kind(row.is_director, &row.card),
                "placement": row.placement.clone().unwrap_or(Value::Null),
                "last_sent": row.last_sent,
                "last_recv": row.last_recv,
                "last_ack": row.last_ack,
                "idle": idle,
            })
        })
        .collect())
}

pub fn list_roster(
    conn: &Connection,
    fleet_id: i64,
    include_message_holders: bool,
) -> Result<Vec<Value>, CafleetError> {
    Ok(roster_rows(conn, fleet_id, include_message_holders)?
        .into_iter()
        .map(|row| {
            json!({
                "member_id": row.member_id,
                "name": row.name,
                "description": row.description,
                "status": row.status,
                "registered_at": row.registered_at,
                "kind": derive_member_kind(row.is_director, &row.card),
                "placement": row.placement.clone().unwrap_or(Value::Null),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, create_fleet, migrated_conn, placement, register,
    };
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn register_member_returns_the_registration_summary_and_enrolls_at_720() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            &[],
            Some(&placement(Some("%2"))),
            None,
        )
        .unwrap();
        let member_id = result["member_id"].as_i64().unwrap();
        assert_eq!(result["name"], "analyst");
        assert!(crate::time::parse_lenient(result["registered_at"].as_str().unwrap()).is_ok());

        let config = broker::get_monitor_config(&conn, fleet_id, member_id)
            .unwrap()
            .unwrap();
        assert_eq!(config["interval_seconds"], 720);
        assert_eq!(config["enabled"], true);
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
            &[],
            Some(&placement(Some("%2"))),
            None,
        )
        .unwrap();
        let member_id = result["member_id"].as_i64().unwrap();
        let ts = result["registered_at"].as_str().unwrap().to_string();

        let member = broker::get_member(&conn, member_id, fleet_id)
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
        let member_id = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "d",
            &skills,
            Some(&placement(Some("%2"))),
            None,
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["skills"], json!(["python", "sql"]));
    }

    #[test]
    fn placementless_member_is_not_enrolled_and_has_null_placement() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None)
            .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"], Value::Null);
        assert!(
            broker::get_monitor_config(&conn, fleet_id, member_id)
                .unwrap()
                .is_none(),
            "only pane-bound members are enrolled"
        );
    }

    #[test]
    fn register_member_unknown_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = broker::register_member(&mut conn, 999, "x", "d", &[], None, None)
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
        let err = broker::register_member(&mut conn, fleet_id, "x", "d", &[], None, None)
            .expect_err("deleted fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), format!("fleet {fleet_id} is deleted"));
    }

    #[test]
    fn monitoring_member_requires_a_placement() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let err = broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            None,
            Some("monitoring-member"),
        )
        .expect_err("a placementless monitoring member must be rejected");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            "a monitoring member must be pane-bound; register it via \
             'cafleet member create --role monitor' (placement required)."
        );
    }

    #[test]
    fn only_one_active_monitoring_member_per_fleet() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let first = broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        let err = broker::register_member(
            &mut conn,
            fleet_id,
            "watch2",
            "d",
            &[],
            Some(&placement(Some("%4"))),
            Some("monitoring-member"),
        )
        .expect_err("a second monitoring member must be rejected");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            format!(
                "fleet {fleet_id} already has an active monitoring member (member {first}); only one is allowed."
            )
        );
    }

    #[test]
    fn monitoring_member_is_unenrolled_with_kind_monitor() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let monitor_id = broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();

        let member = broker::get_member(&conn, monitor_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["kind"], "monitor");
        assert!(
            broker::get_monitor_config(&conn, fleet_id, monitor_id)
                .unwrap()
                .is_none(),
            "the monitoring member is the unenrolled watcher"
        );

        let found = broker::find_monitoring_member(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(found["member_id"], monitor_id);
        assert_eq!(found["name"], "watch");
        assert_eq!(found["pane_id"], "%3");
    }

    #[test]
    fn find_monitoring_member_treats_a_pending_pane_as_absent() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(None)),
            Some("monitoring-member"),
        )
        .unwrap();
        assert!(
            broker::find_monitoring_member(&conn, fleet_id)
                .unwrap()
                .is_none()
        );
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

        let err = broker::register_member(
            &mut conn,
            fleet_id,
            "worker",
            "d",
            &[],
            Some(&placement(Some("%2"))),
            None,
        )
        .expect_err("a placed registration under an inactive Director must fail loudly");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            format!("fleet {fleet_id}'s root Director (member {director_id}) is not active.")
        );

        broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None)
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
        assert!(
            broker::get_monitor_config(&conn, fleet_id, member_id)
                .unwrap()
                .is_none()
        );

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
                .unwrap()
                .is_none()
        );
        broker::deregister_member(&mut conn, member_id).unwrap();
        assert!(
            broker::get_member(&conn, member_id, fleet_a)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn non_object_card_kind_collapses_to_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        conn.execute(
            r#"UPDATE members SET member_card_json='{"name":"worker","description":"d","skills":[],"cafleet":"weird"}' WHERE member_id=?1"#,
            [member_id],
        )
        .unwrap();

        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["kind"], "member");
        let listed = broker::list_members(&conn, fleet_id).unwrap();
        let row = listed.iter().find(|r| r["member_id"] == member_id).unwrap();
        assert_eq!(row["kind"], "member");
    }

    #[test]
    fn update_placement_pane_id_patches_the_pending_pane() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", None);

        let updated = broker::update_placement_pane_id(&mut conn, member_id, "%9")
            .unwrap()
            .unwrap();
        assert_eq!(updated["mux_pane_id"], "%9");
        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"]["mux_pane_id"], "%9");

        let placementless = broker::register_member(
            &mut conn,
            fleet_id,
            "ghost",
            "d",
            &[],
            None,
            None,
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        assert!(
            broker::update_placement_pane_id(&mut conn, placementless, "%1")
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
        let ghost_id = broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None)
            .unwrap()["member_id"]
            .as_i64()
            .unwrap();

        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let rows = broker::list_members(&conn, fleet_id).unwrap();
        assert_eq!(rows.len(), 3);
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

        broker::ack_message(&mut conn, member_id, message_id).unwrap();
        let rows = broker::list_members(&conn, fleet_id).unwrap();
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
        let rows = broker::list_members(&conn, fleet_id).unwrap();
        assert!(rows.iter().all(|r| r["member_id"] != member_id));
    }

    #[test]
    fn list_roster_surfaces_deregistered_message_holders_only_on_request() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let holder_id = register(&mut conn, fleet_id, "holder", Some("%2"));
        let silent_id = register(&mut conn, fleet_id, "silent", Some("%3"));
        let notifier = FakeNotifier::succeeding();
        common::send(&mut conn, &notifier, fleet_id, director_id, holder_id, "hi");
        broker::deregister_member(&mut conn, holder_id).unwrap();
        broker::deregister_member(&mut conn, silent_id).unwrap();

        let active_only = broker::list_roster(&conn, fleet_id, false).unwrap();
        assert!(active_only.iter().all(|r| r["member_id"] != holder_id));

        let with_holders = broker::list_roster(&conn, fleet_id, true).unwrap();
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
