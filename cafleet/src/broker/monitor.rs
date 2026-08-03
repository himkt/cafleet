//! Monitor schedule + runtime DB layer (SPEC §6.2 *Monitor*). The colocated
//! tests pin the contract; see [`super::test_support`] for the API.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::members::{MONITORING_MEMBER_KIND, db_err};
use crate::error::CafleetError;
use crate::time::parse_lenient;

const PENDING_COUNT_SUBQUERY: &str = "(SELECT COUNT(*) FROM messages \
     WHERE owner_member_id=m.member_id AND status_state='input_required' AND type='unicast')";
const OLDEST_PENDING_SUBQUERY: &str = "(SELECT MIN(status_timestamp) FROM messages \
     WHERE owner_member_id=m.member_id AND status_state='input_required' AND type='unicast')";

/// The runtime-staleness tunables (SPEC §6.2) — the policy's single home,
/// re-exported by the monitor module.
pub const MONITOR_STALE_FACTOR: i64 = 3;
pub const MONITOR_STALE_FLOOR_SECONDS: i64 = 15;

fn stale_after_seconds(tick_seconds: i64) -> i64 {
    (MONITOR_STALE_FACTOR * tick_seconds).max(MONITOR_STALE_FLOOR_SECONDS)
}

/// Signal-0 process probe: `EPERM` corroborates alive, `ESRCH` dead.
fn process_alive(pid: i64) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
}

fn heartbeat_fresh(last_tick_at: Option<&str>, tick_seconds: i64, now: DateTime<Utc>) -> bool {
    let Some(last_tick_at) = last_tick_at else {
        return false;
    };
    let Ok(parsed) = parse_lenient(last_tick_at) else {
        return false;
    };
    (now - parsed).num_seconds() <= stale_after_seconds(tick_seconds)
}

struct RuntimeRow {
    pid: Option<i64>,
    started_at: Option<String>,
    last_tick_at: Option<String>,
    tick_seconds: i64,
}

fn runtime_row(conn: &Connection, fleet_id: i64) -> Result<Option<RuntimeRow>, CafleetError> {
    conn.query_row(
        "SELECT pid, started_at, last_tick_at, tick_seconds \
         FROM monitor_runtime WHERE fleet_id=?1",
        [fleet_id],
        |row| {
            Ok(RuntimeRow {
                pid: row.get(0)?,
                started_at: row.get(1)?,
                last_tick_at: row.get(2)?,
                tick_seconds: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

fn runtime_live(row: &RuntimeRow, now: DateTime<Utc>) -> bool {
    match row.pid {
        None => false,
        Some(pid) => {
            heartbeat_fresh(row.last_tick_at.as_deref(), row.tick_seconds, now)
                && process_alive(pid)
        }
    }
}

pub fn find_monitoring_member(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<Value>, CafleetError> {
    conn.query_row(
        "SELECT m.member_id, m.name, p.mux_pane_id \
         FROM members m JOIN member_placements p ON p.member_id=m.member_id \
         WHERE m.fleet_id=?1 AND m.status='active' \
           AND json_extract(m.member_card_json, '$.cafleet.kind')=?2 \
           AND p.mux_pane_id IS NOT NULL \
         ORDER BY m.member_id LIMIT 1",
        params![fleet_id, MONITORING_MEMBER_KIND],
        |row| {
            Ok(json!({
                "member_id": row.get::<_, i64>(0)?,
                "name": row.get::<_, String>(1)?,
                "pane_id": row.get::<_, String>(2)?,
            }))
        },
    )
    .optional()
    .map_err(db_err)
}

fn config_value(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "member_id": row.get::<_, i64>(0)?,
        "interval_seconds": row.get::<_, i64>(1)?,
        "last_ping_at": row.get::<_, Option<String>>(2)?,
        "enabled": row.get::<_, i64>(3)? != 0,
        "last_stall_check_at": row.get::<_, Option<String>>(4)?,
    }))
}

const CONFIG_SELECT: &str = "SELECT c.member_id, c.interval_seconds, c.last_ping_at, c.enabled, \
     c.last_stall_check_at \
     FROM monitor_config c JOIN members m ON m.member_id=c.member_id";

pub fn get_monitor_config(
    conn: &Connection,
    fleet_id: i64,
    member_id: i64,
) -> Result<Option<Value>, CafleetError> {
    conn.query_row(
        &format!("{CONFIG_SELECT} WHERE c.member_id=?1 AND m.fleet_id=?2"),
        params![member_id, fleet_id],
        config_value,
    )
    .optional()
    .map_err(db_err)
}

pub fn list_monitor_configs(conn: &Connection, fleet_id: i64) -> Result<Vec<Value>, CafleetError> {
    let mut stmt = conn
        .prepare(&format!(
            "{CONFIG_SELECT} WHERE m.fleet_id=?1 ORDER BY c.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], config_value)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn update_monitor_config(
    conn: &mut Connection,
    fleet_id: i64,
    member_id: i64,
    interval_seconds: Option<i64>,
    enabled: Option<bool>,
) -> Result<Value, CafleetError> {
    if get_monitor_config(conn, fleet_id, member_id)?.is_none() {
        return Err(CafleetError::App(format!(
            "member {member_id} is not enrolled in monitoring for fleet {fleet_id}."
        )));
    }
    let tx = conn.transaction().map_err(db_err)?;
    if let Some(interval_seconds) = interval_seconds {
        tx.execute(
            "UPDATE monitor_config SET interval_seconds=?1 WHERE member_id=?2",
            params![interval_seconds, member_id],
        )
        .map_err(db_err)?;
    }
    if let Some(enabled) = enabled {
        tx.execute(
            "UPDATE monitor_config SET enabled=?1 WHERE member_id=?2",
            params![i64::from(enabled), member_id],
        )
        .map_err(db_err)?;
        if !enabled {
            tx.execute(
                "UPDATE monitor_config SET last_stall_check_at=NULL WHERE member_id=?1",
                [member_id],
            )
            .map_err(db_err)?;
        }
    }
    tx.commit().map_err(db_err)?;
    Ok(get_monitor_config(conn, fleet_id, member_id)?.expect("the enrolled config row exists"))
}

pub fn record_pings(
    conn: &mut Connection,
    member_ids: &[i64],
    when: &str,
) -> Result<(), CafleetError> {
    record_monitor_dispatch(conn, member_ids, &[], when)
}

/// Commit both dispatch cadences atomically: `last_ping_at` for pinged
/// members, `last_stall_check_at` for stall-checked members.
pub fn record_monitor_dispatch(
    conn: &mut Connection,
    ping_member_ids: &[i64],
    stall_check_member_ids: &[i64],
    when: &str,
) -> Result<(), CafleetError> {
    let tx = conn.transaction().map_err(db_err)?;
    for member_id in ping_member_ids {
        tx.execute(
            "UPDATE monitor_config SET last_ping_at=?1 WHERE member_id=?2",
            params![when, member_id],
        )
        .map_err(db_err)?;
    }
    for member_id in stall_check_member_ids {
        tx.execute(
            "UPDATE monitor_config SET last_stall_check_at=?1 WHERE member_id=?2",
            params![when, member_id],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(())
}

/// Clear the stall-check stamp for the listed members of this fleet only
/// (members that lost their pane or availability).
pub fn reconcile_monitor_lifecycle(
    conn: &mut Connection,
    fleet_id: i64,
    unavailable_member_ids: &[i64],
) -> Result<(), CafleetError> {
    let tx = conn.transaction().map_err(db_err)?;
    for member_id in unavailable_member_ids {
        tx.execute(
            "UPDATE monitor_config SET last_stall_check_at=NULL \
             WHERE member_id=?1 AND member_id IN \
                   (SELECT member_id FROM members WHERE fleet_id=?2)",
            params![member_id, fleet_id],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(())
}

/// The per-tick scan rows: every enrolled active member of the fleet (the
/// monitoring member is never enrolled, so never a target).
pub fn list_monitor_targets(conn: &Connection, fleet_id: i64) -> Result<Vec<Value>, CafleetError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT m.member_id, \
                    m.name, p.mux_pane_id, p.coding_agent, \
                    c.interval_seconds, c.last_ping_at, c.enabled, c.last_stall_check_at, \
                    {PENDING_COUNT_SUBQUERY}, {OLDEST_PENDING_SUBQUERY} \
             FROM monitor_config c JOIN members m ON m.member_id=c.member_id \
             LEFT JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND m.status='active' \
             ORDER BY m.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], |row| {
            Ok(json!({
                "member_id": row.get::<_, i64>(0)?,
                "name": row.get::<_, String>(1)?,
                "pane_id": row.get::<_, Option<String>>(2)?,
                "coding_agent": row.get::<_, Option<String>>(3)?,
                "interval_seconds": row.get::<_, i64>(4)?,
                "last_ping_at": row.get::<_, Option<String>>(5)?,
                "enabled": row.get::<_, i64>(6)? != 0,
                "last_stall_check_at": row.get::<_, Option<String>>(7)?,
                "pending_count": row.get::<_, i64>(8)?,
                "oldest_pending_ts": row.get::<_, Option<String>>(9)?,
            }))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Atomically claim the fleet's single-instance runtime slot. A live slot —
/// fresh heartbeat AND an alive owning process — is never stolen.
pub fn claim_monitor_runtime(
    conn: &mut Connection,
    fleet_id: i64,
    pid: i64,
    tick_seconds: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    let now = parse_lenient(when)?;
    let tx = conn.transaction().map_err(db_err)?;
    let existing = tx
        .query_row(
            "SELECT pid, last_tick_at, tick_seconds FROM monitor_runtime WHERE fleet_id=?1",
            [fleet_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(db_err)?;
    match existing {
        None => {
            tx.execute(
                "INSERT INTO monitor_runtime (fleet_id, pid, started_at, last_tick_at, tick_seconds) \
                 VALUES (?1, ?2, ?3, ?3, ?4)",
                params![fleet_id, pid, when, tick_seconds],
            )
            .map_err(db_err)?;
        }
        Some((owner_pid, last_tick_at, row_tick)) => {
            let live = owner_pid.is_some_and(|owner| {
                heartbeat_fresh(last_tick_at.as_deref(), row_tick, now) && process_alive(owner)
            });
            if live {
                return Ok(false);
            }
            tx.execute(
                "UPDATE monitor_runtime \
                 SET pid=?1, started_at=?2, last_tick_at=?2, tick_seconds=?3 \
                 WHERE fleet_id=?4",
                params![pid, when, tick_seconds, fleet_id],
            )
            .map_err(db_err)?;
        }
    }
    tx.commit().map_err(db_err)?;
    Ok(true)
}

/// Ownership-checked heartbeat: a non-owner's tick matches zero rows and
/// returns `false` (the displaced loser self-terminates on it).
pub fn heartbeat_monitor_runtime(
    conn: &mut Connection,
    fleet_id: i64,
    pid: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE monitor_runtime SET last_tick_at=?1 WHERE fleet_id=?2 AND pid=?3",
            params![when, fleet_id, pid],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

/// Ownership-checked clear: nulls the process fields, preserves
/// `tick_seconds`; a non-owner's clear is a no-op.
pub fn clear_monitor_runtime(
    conn: &mut Connection,
    fleet_id: i64,
    pid: i64,
) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE monitor_runtime SET pid=NULL, started_at=NULL, last_tick_at=NULL \
         WHERE fleet_id=?1 AND pid=?2",
        params![fleet_id, pid],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn read_monitor_runtime(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<Value>, CafleetError> {
    Ok(runtime_row(conn, fleet_id)?.map(|row| {
        json!({
            "fleet_id": fleet_id,
            "pid": row.pid,
            "started_at": row.started_at,
            "last_tick_at": row.last_tick_at,
            "tick_seconds": row.tick_seconds,
        })
    }))
}

pub fn monitor_is_live(
    conn: &Connection,
    fleet_id: i64,
    now: DateTime<Utc>,
) -> Result<bool, CafleetError> {
    Ok(runtime_row(conn, fleet_id)?.is_some_and(|row| runtime_live(&row, now)))
}

/// The flat runtime keys of `GET /api/monitor` (SPEC §6.8): a stale or absent
/// slot never leaks a lingering pid, start time, or tick timestamp.
pub fn monitor_runtime_payload(
    conn: &Connection,
    fleet_id: i64,
    now: DateTime<Utc>,
) -> Result<Value, CafleetError> {
    let row = runtime_row(conn, fleet_id)?;
    let (running, tick_seconds) = match &row {
        None => (false, Value::Null),
        Some(row) => (runtime_live(row, now), json!(row.tick_seconds)),
    };
    if !running {
        return Ok(json!({
            "running": false,
            "pid": Value::Null,
            "tick_seconds": tick_seconds,
            "last_tick_at": Value::Null,
            "last_tick_age_seconds": Value::Null,
            "started_at": Value::Null,
        }));
    }
    let row = row.expect("a running slot has a row");
    let last_tick_age_seconds = row
        .last_tick_at
        .as_deref()
        .and_then(|ts| parse_lenient(ts).ok())
        .map(|parsed| (now - parsed).num_seconds());
    Ok(json!({
        "running": true,
        "pid": row.pid,
        "tick_seconds": row.tick_seconds,
        "last_tick_at": row.last_tick_at,
        "last_tick_age_seconds": last_tick_age_seconds,
        "started_at": row.started_at,
    }))
}

/// The shared per-member monitor rows (SPEC §6.2), ages integer-truncated
/// against the supplied `now`.
pub fn monitor_members_payload(
    conn: &Connection,
    fleet_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<Value>, CafleetError> {
    let age = |ts: Option<&str>| -> Option<i64> {
        let parsed = parse_lenient(ts?).ok()?;
        Some((now - parsed).num_seconds())
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT m.member_id, m.name, \
                    c.interval_seconds, c.enabled, c.last_ping_at, \
                    {PENDING_COUNT_SUBQUERY}, {OLDEST_PENDING_SUBQUERY} \
             FROM monitor_config c JOIN members m ON m.member_id=c.member_id \
             WHERE m.fleet_id=?1 AND m.status='active' \
             ORDER BY m.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                member_id,
                name,
                interval_seconds,
                enabled,
                last_ping_at,
                pending_count,
                oldest_pending_ts,
            )| {
                json!({
                    "member_id": member_id,
                    "name": name,
                    "interval_seconds": interval_seconds,
                    "enabled": enabled != 0,
                    "last_ping_at": last_ping_at,
                    "last_ping_age_seconds": age(last_ping_at.as_deref()),
                    "pending_count": pending_count,
                    "oldest_pending_ts": oldest_pending_ts,
                    "oldest_pending_age_seconds": age(oldest_pending_ts.as_deref()),
                })
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{FakeNotifier, create_fleet, migrated_conn, register};
    use crate::time::format_utc;

    fn own_pid() -> i64 {
        i64::from(std::process::id())
    }

    // A PID far above any real process id on macOS/Linux test hosts, so the
    // signal-0 probe reports no-such-process.
    const DEAD_PID: i64 = 999_999_999;

    fn base_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 30, 10, 0, 0).unwrap()
    }

    #[test]
    fn record_monitor_wake_stamps_a_durable_last_wake_at() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap();

        broker::record_monitor_wake(&mut conn, fleet_id, &when).unwrap();
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["last_wake_at"], when);

        let later = format_utc(base_time() + Duration::seconds(600));
        broker::record_monitor_wake(&mut conn, fleet_id, &later).unwrap();
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            row["last_wake_at"], later,
            "a later wake overwrites the stamp"
        );
    }

    #[test]
    fn last_wake_at_survives_a_reclaim() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap();
        broker::record_monitor_wake(&mut conn, fleet_id, &when).unwrap();

        // stale_after = max(3 * 5, 15) = 15; the 100-second-old heartbeat lets
        // a restarted loop reclaim the slot.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, &later).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            row["last_wake_at"], when,
            "a restart honors the remaining wake cadence instead of firing instantly"
        );
    }

    #[test]
    fn list_fleet_wake_targets_excludes_the_director_and_the_placementless() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let worker_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let pending_id = register(&mut conn, fleet_id, "pending", None);
        let ghost_id = broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None)
            .unwrap()["member_id"]
            .as_i64()
            .unwrap();

        let targets = broker::list_fleet_wake_targets(&conn, fleet_id).unwrap();
        assert_eq!(targets.len(), 2, "got: {targets:?}");
        assert!(
            !targets.iter().any(|t| t["member_id"] == director_id),
            "the Director is the wake recipient, never a roster entry"
        );
        assert!(
            !targets.iter().any(|t| t["member_id"] == ghost_id),
            "a member with no placement carries no coding_agent to report"
        );

        assert_eq!(targets[0]["member_id"], worker_id);
        assert_eq!(targets[0]["name"], "worker");
        assert_eq!(targets[0]["coding_agent"], "claude");
        assert_eq!(targets[0]["pending_count"], 0);
        assert_eq!(
            targets[1]["member_id"], pending_id,
            "a placement with a pending pane still makes the roster, ordered by member_id"
        );

        let keys: std::collections::BTreeSet<&str> = targets[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["member_id", "name", "coding_agent", "pending_count"].into(),
            "the roster-row key set is pinned"
        );
    }

    #[test]
    fn list_fleet_wake_targets_counts_pending_unicast_deliveries() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let first = common::send(
            &mut conn,
            &notifier,
            fleet_id,
            director_id,
            member_id,
            "one",
        );
        common::send(
            &mut conn,
            &notifier,
            fleet_id,
            director_id,
            member_id,
            "two",
        );
        let first_id = first["message"]["message_id"].as_i64().unwrap();

        let targets = broker::list_fleet_wake_targets(&conn, fleet_id).unwrap();
        assert_eq!(targets[0]["pending_count"], 2);

        broker::ack_message(&mut conn, member_id, first_id).unwrap();
        let targets = broker::list_fleet_wake_targets(&conn, fleet_id).unwrap();
        assert_eq!(targets[0]["pending_count"], 1);
    }

    #[test]
    fn claim_inserts_a_fresh_slot_and_refuses_a_live_one() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());

        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["fleet_id"], fleet_id);
        assert_eq!(row["pid"], own_pid());
        assert_eq!(row["started_at"], when);
        assert_eq!(row["last_tick_at"], when);
        assert_eq!(row["tick_seconds"], 5);
        assert_eq!(row["last_wake_at"], Value::Null, "a fresh slot has no stamp");

        let refused = broker::claim_monitor_runtime(
            &mut conn,
            fleet_id,
            own_pid() + 1,
            5,
            &format_utc(base_time() + Duration::seconds(1)),
        )
        .unwrap();
        assert!(!refused, "a live slot is never stolen");
    }

    #[test]
    fn claim_reclaims_a_stale_heartbeat() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        assert!(
            broker::claim_monitor_runtime(
                &mut conn,
                fleet_id,
                own_pid(),
                5,
                &format_utc(base_time())
            )
            .unwrap()
        );

        // stale_after = max(3 * 5, 15) = 15; a 100-second-old heartbeat is
        // stale even though the owning process (this test) is alive.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, &later).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], 4242);
        assert_eq!(row["started_at"], later);
    }

    #[test]
    fn claim_reclaims_a_dead_process_despite_a_fresh_heartbeat() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        assert!(
            broker::claim_monitor_runtime(
                &mut conn,
                fleet_id,
                DEAD_PID,
                5,
                &format_utc(base_time())
            )
            .unwrap()
        );

        let reclaimed = broker::claim_monitor_runtime(
            &mut conn,
            fleet_id,
            own_pid(),
            5,
            &format_utc(base_time() + Duration::seconds(1)),
        )
        .unwrap();
        assert!(reclaimed, "signal-0 no-such-process corroborates dead");
    }

    #[test]
    fn heartbeat_is_ownership_checked() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap();

        let tick = format_utc(base_time() + Duration::seconds(2));
        assert!(broker::heartbeat_monitor_runtime(&mut conn, fleet_id, own_pid(), &tick).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["last_tick_at"], tick);

        let displaced = broker::heartbeat_monitor_runtime(
            &mut conn,
            fleet_id,
            4242,
            &format_utc(base_time() + Duration::seconds(3)),
        )
        .unwrap();
        assert!(!displaced, "a non-owner heartbeat matches zero rows");
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["last_tick_at"], tick, "the owner's heartbeat survives");
    }

    #[test]
    fn clear_is_ownership_checked_and_preserves_tick_seconds_and_last_wake_at() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 7, &when).unwrap();
        broker::record_monitor_wake(&mut conn, fleet_id, &when).unwrap();

        broker::clear_monitor_runtime(&mut conn, fleet_id, 4242).unwrap();
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], own_pid(), "a loser's clear is a no-op");

        broker::clear_monitor_runtime(&mut conn, fleet_id, own_pid()).unwrap();
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], Value::Null);
        assert_eq!(row["started_at"], Value::Null);
        assert_eq!(row["last_tick_at"], Value::Null);
        assert_eq!(row["tick_seconds"], 7);
        assert_eq!(row["last_wake_at"], when, "the wake cadence survives a stop");
    }

    #[test]
    fn monitor_is_live_tracks_the_slot_lifecycle() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let now = base_time();
        assert!(
            !broker::monitor_is_live(&conn, fleet_id, now).unwrap(),
            "no row"
        );

        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &format_utc(now)).unwrap();
        assert!(broker::monitor_is_live(&conn, fleet_id, now + Duration::seconds(2)).unwrap());
        assert!(
            !broker::monitor_is_live(&conn, fleet_id, now + Duration::seconds(100)).unwrap(),
            "a stale heartbeat reads as dead"
        );

        broker::clear_monitor_runtime(&mut conn, fleet_id, own_pid()).unwrap();
        assert!(!broker::monitor_is_live(&conn, fleet_id, now + Duration::seconds(2)).unwrap());
    }

    #[test]
    fn runtime_payload_reports_live_fields_and_masks_stale_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let now = base_time();

        let absent = broker::monitor_runtime_payload(&conn, fleet_id, now).unwrap();
        assert_eq!(absent["running"], false);
        assert_eq!(absent["pid"], Value::Null);
        assert_eq!(absent["tick_seconds"], Value::Null);

        let when = format_utc(now);
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap();
        let live =
            broker::monitor_runtime_payload(&conn, fleet_id, now + Duration::seconds(2)).unwrap();
        assert_eq!(live["running"], true);
        assert_eq!(live["pid"], own_pid());
        assert_eq!(live["tick_seconds"], 5);
        assert_eq!(live["last_tick_at"], when);
        assert_eq!(live["last_tick_age_seconds"], 2);
        assert_eq!(live["started_at"], when);

        let stale =
            broker::monitor_runtime_payload(&conn, fleet_id, now + Duration::seconds(100)).unwrap();
        assert_eq!(stale["running"], false);
        assert_eq!(stale["pid"], Value::Null, "a stale row never leaks its pid");
        assert_eq!(
            stale["tick_seconds"], 5,
            "tick_seconds survives from the stale row"
        );
        assert_eq!(stale["last_tick_at"], Value::Null);
        assert_eq!(stale["last_tick_age_seconds"], Value::Null);
        assert_eq!(stale["started_at"], Value::Null);
    }
}
