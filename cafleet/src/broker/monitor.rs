//! Monitor runtime DB layer (SPEC §6.2 *Monitor*) — the fleet-level wake
//! roster (non-Director, non-monitor members) + ledger, the Director
//! descriptor for the wake's `Director:` segment, and the single-instance
//! runtime slot. The colocated tests pin the contract; see
//! [`super::test_support`] for the API.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::members::db_err;
use super::records::{MonitorMember, MonitorRuntime, MonitorRuntimeView, WakeTarget};
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

fn runtime_row(conn: &Connection, fleet_id: i64) -> Result<Option<MonitorRuntime>, CafleetError> {
    conn.query_row(
        "SELECT pid, started_at, last_tick_at, tick_seconds, wake_interval_seconds, last_wake_at, \
                wake_requested_at \
         FROM monitor_runtime WHERE fleet_id=?1",
        [fleet_id],
        |row| {
            Ok(MonitorRuntime {
                fleet_id,
                pid: row.get(0)?,
                started_at: row.get(1)?,
                last_tick_at: row.get(2)?,
                tick_seconds: row.get(3)?,
                wake_interval_seconds: row.get(4)?,
                last_wake_at: row.get(5)?,
                wake_requested_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

fn runtime_live(row: &MonitorRuntime, now: DateTime<Utc>) -> bool {
    match row.pid {
        None => false,
        Some(pid) => {
            heartbeat_fresh(row.last_tick_at.as_deref(), row.tick_seconds, now)
                && process_alive(pid)
        }
    }
}

/// Stamp the fleet's durable wake ledger; called only after a delivered
/// Director wake (the `woke`-gated write, SPEC §6.6). The same write clears
/// any pending forced-wake request — the wake the operator asked for has
/// happened, whether it fired forced or on schedule.
pub fn record_monitor_wake(
    conn: &mut Connection,
    fleet_id: i64,
    when: &str,
) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE monitor_runtime SET last_wake_at=?1, wake_requested_at=NULL WHERE fleet_id=?2",
        params![when, fleet_id],
    )
    .map_err(db_err)?;
    Ok(())
}

const NOT_MONITOR_PREDICATE: &str =
    "json_extract(m.member_card_json, '$.cafleet.kind') IS NOT 'monitor'";

/// The fleet-level wake roster: every active, non-Director, non-monitor
/// member with a placement (a pending pane still makes the roster), ordered
/// by `member_id` — the order the wake payload's entries render in. The
/// monitor member receives the wake and is never a roster entry.
pub fn list_fleet_wake_target_records(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<WakeTarget>, CafleetError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT m.member_id, m.name, p.coding_agent, {PENDING_COUNT_SUBQUERY} \
             FROM members m JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND m.status='active' \
               AND NOT EXISTS(SELECT 1 FROM fleets f \
                              WHERE f.fleet_id=m.fleet_id \
                                AND f.director_member_id=m.member_id) \
               AND {NOT_MONITOR_PREDICATE} \
             ORDER BY m.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], |row| {
            Ok(WakeTarget {
                member_id: row.get(0)?,
                name: row.get(1)?,
                coding_agent: row.get(2)?,
                pending_count: row.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// The fleet's Director descriptor for the wake's trailing `Director:`
/// segment — the same field grammar as a roster entry. A live fleet always
/// records its Director with a placement, so a missing row is a loud error,
/// not a skip.
pub fn fleet_wake_director_record(
    conn: &Connection,
    fleet_id: i64,
) -> Result<WakeTarget, CafleetError> {
    conn.query_row(
        &format!(
            "SELECT m.member_id, m.name, p.coding_agent, {PENDING_COUNT_SUBQUERY} \
             FROM fleets f \
             JOIN members m ON m.member_id=f.director_member_id \
             JOIN member_placements p ON p.member_id=m.member_id \
             WHERE f.fleet_id=?1"
        ),
        [fleet_id],
        |row| {
            Ok(WakeTarget {
                member_id: row.get(0)?,
                name: row.get(1)?,
                coding_agent: row.get(2)?,
                pending_count: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(db_err)?
    .ok_or_else(|| {
        CafleetError::App(format!(
            "fleet {fleet_id} has no Director with a placement recorded"
        ))
    })
}

/// Atomically claim the fleet's single-instance runtime slot. A live slot —
/// fresh heartbeat AND an alive owning process — is never stolen.
pub fn claim_monitor_runtime(
    conn: &mut Connection,
    fleet_id: i64,
    pid: i64,
    tick_seconds: i64,
    wake_interval: i64,
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
                "INSERT INTO monitor_runtime \
                 (fleet_id, pid, started_at, last_tick_at, tick_seconds, wake_interval_seconds) \
                 VALUES (?1, ?2, ?3, ?3, ?4, ?5)",
                params![fleet_id, pid, when, tick_seconds, wake_interval],
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
                 SET pid=?1, started_at=?2, last_tick_at=?2, tick_seconds=?3, \
                     wake_interval_seconds=?4, wake_requested_at=NULL \
                 WHERE fleet_id=?5",
                params![pid, when, tick_seconds, wake_interval, fleet_id],
            )
            .map_err(db_err)?;
        }
    }
    tx.commit().map_err(db_err)?;
    Ok(true)
}

/// Ownership-free forced-wake request (the WebUI `POST /api/monitor/wake`
/// write): `false` ⇔ no row. Repeat requests overwrite the timestamp, so
/// they coalesce into a single wake.
pub fn request_monitor_wake(
    conn: &mut Connection,
    fleet_id: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE monitor_runtime SET wake_requested_at=?1 WHERE fleet_id=?2",
            params![when, fleet_id],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

/// Ownership-free interval update (the WebUI `PATCH /api/monitor` write):
/// `false` ⇔ no row, i.e. the fleet's monitor has never run.
pub fn set_monitor_wake_interval(
    conn: &mut Connection,
    fleet_id: i64,
    wake_interval: i64,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE monitor_runtime SET wake_interval_seconds=?1 WHERE fleet_id=?2",
            params![wake_interval, fleet_id],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
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

/// Ownership-checked clear: nulls the process fields, preserves the durable
/// fields (`tick_seconds`, `wake_interval_seconds`, `last_wake_at`); a
/// non-owner's clear is a no-op.
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

pub fn read_monitor_runtime_record(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<MonitorRuntime>, CafleetError> {
    runtime_row(conn, fleet_id)
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
pub fn monitor_runtime_view(
    conn: &Connection,
    fleet_id: i64,
    now: DateTime<Utc>,
) -> Result<MonitorRuntimeView, CafleetError> {
    let row = runtime_row(conn, fleet_id)?;
    let mut view = MonitorRuntimeView {
        running: false,
        pid: None,
        tick_seconds: row.as_ref().map(|row| row.tick_seconds),
        wake_interval_seconds: row.as_ref().and_then(|row| row.wake_interval_seconds),
        last_tick_at: None,
        last_tick_age_seconds: None,
        started_at: None,
        last_wake_at: None,
        last_wake_age_seconds: None,
    };
    if let Some(row) = row.filter(|row| runtime_live(row, now)) {
        let age = |ts: Option<&str>| -> Option<i64> {
            let parsed = parse_lenient(ts?).ok()?;
            Some((now - parsed).num_seconds())
        };
        view.running = true;
        view.pid = row.pid;
        view.last_tick_age_seconds = age(row.last_tick_at.as_deref());
        view.last_wake_age_seconds = age(row.last_wake_at.as_deref());
        view.last_tick_at = row.last_tick_at;
        view.started_at = row.started_at;
        view.last_wake_at = row.last_wake_at;
    }
    Ok(view)
}

/// The per-member rows of `GET /api/monitor` (SPEC §6.8): one dict per
/// wake-roster member (non-Director, non-monitor), ages integer-truncated
/// against the supplied `now`.
pub fn monitor_member_records(
    conn: &Connection,
    fleet_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<MonitorMember>, CafleetError> {
    let age = |ts: Option<&str>| -> Option<i64> {
        let parsed = parse_lenient(ts?).ok()?;
        Some((now - parsed).num_seconds())
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT m.member_id, m.name, \
                    {PENDING_COUNT_SUBQUERY}, {OLDEST_PENDING_SUBQUERY} \
             FROM members m JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND m.status='active' \
               AND NOT EXISTS(SELECT 1 FROM fleets f \
                              WHERE f.fleet_id=m.fleet_id \
                                AND f.director_member_id=m.member_id) \
               AND {NOT_MONITOR_PREDICATE} \
             ORDER BY m.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(member_id, name, pending_count, oldest_pending_ts)| MonitorMember {
                member_id,
                name,
                pending_count,
                oldest_pending_age_seconds: age(oldest_pending_ts.as_deref()),
                oldest_pending_ts,
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
    use crate::broker::test_support::{
        FakeNotifier, bootstrap_monitor, create_fleet, migrated_conn, register,
    };
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
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();

        broker::record_monitor_wake(&mut conn, fleet_id, &when).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["last_wake_at"], when);

        let later = format_utc(base_time() + Duration::seconds(600));
        broker::record_monitor_wake(&mut conn, fleet_id, &later).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
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
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        broker::record_monitor_wake(&mut conn, fleet_id, &when).unwrap();

        // stale_after = max(3 * 5, 15) = 15; the 100-second-old heartbeat lets
        // a restarted loop reclaim the slot.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, 600, &later).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
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
        let ghost_id =
            broker::register_member_record(&mut conn, fleet_id, "ghost", "d", &[], None, false)
                .map(|record| crate::presentation::registered_member(&record))
                .unwrap()["member_id"]
                .as_i64()
                .unwrap();

        let targets = broker::list_fleet_wake_target_records(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::wake_target)
                    .collect::<Vec<_>>()
            })
            .unwrap();
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
        let first = common::send(&mut conn, &notifier, director_id, member_id, "one");
        common::send(&mut conn, &notifier, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();

        let targets = broker::list_fleet_wake_target_records(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::wake_target)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(targets[0]["pending_count"], 2);

        broker::ack_message_record(&mut conn, first_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
        let targets = broker::list_fleet_wake_target_records(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::wake_target)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(targets[0]["pending_count"], 1);
    }

    #[test]
    fn list_fleet_wake_targets_excludes_the_monitor_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let monitor_id = bootstrap_monitor(&conn, fleet_id);
        let worker_id = register(&mut conn, fleet_id, "worker", Some("%3"));

        let targets = broker::list_fleet_wake_target_records(&conn, fleet_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::wake_target)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(targets.len(), 1, "got: {targets:?}");
        assert_eq!(targets[0]["member_id"], worker_id);
        assert!(
            !targets.iter().any(|t| t["member_id"] == monitor_id),
            "the monitor member receives the wake, never a roster entry"
        );
    }

    #[test]
    fn monitor_members_payload_excludes_the_monitor_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let monitor_id = bootstrap_monitor(&conn, fleet_id);
        let worker_id = register(&mut conn, fleet_id, "worker", Some("%3"));

        let rows = broker::monitor_member_records(&conn, fleet_id, base_time())
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::monitor_member)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(rows.len(), 1, "got: {rows:?}");
        assert_eq!(rows[0]["member_id"], worker_id);
        assert!(
            !rows.iter().any(|r| r["member_id"] == monitor_id),
            "the members array re-sources to the wake roster, which excludes the monitor"
        );
    }

    #[test]
    fn fleet_wake_director_reports_the_director_descriptor() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let worker_id = register(&mut conn, fleet_id, "worker", Some("%2"));

        let director = broker::fleet_wake_director_record(&conn, fleet_id)
            .map(|record| crate::presentation::wake_target(&record))
            .unwrap();
        assert_eq!(director["member_id"], director_id);
        assert_eq!(director["name"], "Director");
        assert_eq!(director["coding_agent"], "claude");
        assert_eq!(director["pending_count"], 0);
        let keys: std::collections::BTreeSet<&str> = director
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["member_id", "name", "coding_agent", "pending_count"].into(),
            "the descriptor's key set matches the roster-entry grammar"
        );

        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, worker_id, director_id, "status");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();
        let director = broker::fleet_wake_director_record(&conn, fleet_id)
            .map(|record| crate::presentation::wake_target(&record))
            .unwrap();
        assert_eq!(
            director["pending_count"], 1,
            "pending_count counts the Director's input_required unicast deliveries"
        );

        broker::ack_message_record(&mut conn, message_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
        let director = broker::fleet_wake_director_record(&conn, fleet_id)
            .map(|record| crate::presentation::wake_target(&record))
            .unwrap();
        assert_eq!(director["pending_count"], 0);
    }

    #[test]
    fn fleet_wake_director_is_a_loud_error_when_the_row_is_missing() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        broker::fleet_wake_director_record(&conn, 999)
            .map(|record| crate::presentation::wake_target(&record))
            .expect_err("an unknown fleet has no Director row");

        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        conn.execute(
            "DELETE FROM member_placements WHERE member_id=?1",
            [director_id],
        )
        .unwrap();
        broker::fleet_wake_director_record(&conn, fleet_id)
            .map(|record| crate::presentation::wake_target(&record))
            .expect_err("a Director without a placement is a loud error, not a skip");
    }

    #[test]
    fn claim_inserts_a_fresh_slot_and_refuses_a_live_one() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());

        assert!(
            broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap()
        );
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["fleet_id"], fleet_id);
        assert_eq!(row["pid"], own_pid());
        assert_eq!(row["started_at"], when);
        assert_eq!(row["last_tick_at"], when);
        assert_eq!(row["tick_seconds"], 5);
        assert_eq!(
            row["last_wake_at"],
            Value::Null,
            "a fresh slot has no stamp"
        );

        let refused = broker::claim_monitor_runtime(
            &mut conn,
            fleet_id,
            own_pid() + 1,
            5,
            600,
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
                600,
                &format_utc(base_time())
            )
            .unwrap()
        );

        // stale_after = max(3 * 5, 15) = 15; a 100-second-old heartbeat is
        // stale even though the owning process (this test) is alive.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, 600, &later).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
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
                600,
                &format_utc(base_time())
            )
            .unwrap()
        );

        let reclaimed = broker::claim_monitor_runtime(
            &mut conn,
            fleet_id,
            own_pid(),
            5,
            600,
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
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();

        let tick = format_utc(base_time() + Duration::seconds(2));
        assert!(broker::heartbeat_monitor_runtime(&mut conn, fleet_id, own_pid(), &tick).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
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
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["last_tick_at"], tick, "the owner's heartbeat survives");
    }

    #[test]
    fn clear_is_ownership_checked_and_preserves_the_durable_fields() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 7, 600, &when).unwrap();
        broker::record_monitor_wake(&mut conn, fleet_id, &when).unwrap();

        broker::clear_monitor_runtime(&mut conn, fleet_id, 4242).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], own_pid(), "a loser's clear is a no-op");

        broker::clear_monitor_runtime(&mut conn, fleet_id, own_pid()).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], Value::Null);
        assert_eq!(row["started_at"], Value::Null);
        assert_eq!(row["last_tick_at"], Value::Null);
        assert_eq!(row["tick_seconds"], 7);
        assert_eq!(
            row["wake_interval_seconds"], 600,
            "the interval survives a stop, like tick_seconds"
        );
        assert_eq!(
            row["last_wake_at"], when,
            "the wake cadence survives a stop"
        );
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

        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &format_utc(now))
            .unwrap();
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

        let absent = broker::monitor_runtime_view(&conn, fleet_id, now)
            .map(|record| crate::presentation::monitor_runtime_view(&record))
            .unwrap();
        assert_eq!(absent["running"], false);
        assert_eq!(absent["pid"], Value::Null);
        assert_eq!(absent["tick_seconds"], Value::Null);

        let when = format_utc(now);
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        let live = broker::monitor_runtime_view(&conn, fleet_id, now + Duration::seconds(2))
            .map(|record| crate::presentation::monitor_runtime_view(&record))
            .unwrap();
        assert_eq!(live["running"], true);
        assert_eq!(live["pid"], own_pid());
        assert_eq!(live["tick_seconds"], 5);
        assert_eq!(live["last_tick_at"], when);
        assert_eq!(live["last_tick_age_seconds"], 2);
        assert_eq!(live["started_at"], when);

        let stale = broker::monitor_runtime_view(&conn, fleet_id, now + Duration::seconds(100))
            .map(|record| crate::presentation::monitor_runtime_view(&record))
            .unwrap();
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

    #[test]
    fn claim_stamps_the_wake_interval_on_insert_and_reclaim() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["wake_interval_seconds"], 600);

        // stale_after = max(3 * 5, 15) = 15; the 100-second-old heartbeat lets
        // a restarted loop reclaim the slot and re-stamp the interval.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, 300, &later).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            row["wake_interval_seconds"], 300,
            "a reclaim re-stamps the startup-resolved interval"
        );
    }

    #[test]
    fn set_monitor_wake_interval_updates_the_row_and_reports_a_never_run_fleet() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        assert!(
            !broker::set_monitor_wake_interval(&mut conn, fleet_id, 120).unwrap(),
            "no row ⇔ the fleet's monitor has never run"
        );

        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        assert!(broker::set_monitor_wake_interval(&mut conn, fleet_id, 120).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["wake_interval_seconds"], 120);

        broker::clear_monitor_runtime(&mut conn, fleet_id, own_pid()).unwrap();
        assert!(
            broker::set_monitor_wake_interval(&mut conn, fleet_id, 0).unwrap(),
            "the update is ownership-free; a stopped loop's row still updates"
        );
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["wake_interval_seconds"], 0);
    }

    #[test]
    fn runtime_payload_carries_the_wake_interval_in_every_shape() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let now = base_time();

        let absent = broker::monitor_runtime_view(&conn, fleet_id, now)
            .map(|record| crate::presentation::monitor_runtime_view(&record))
            .unwrap();
        assert_eq!(
            absent["wake_interval_seconds"],
            Value::Null,
            "no row has ever existed"
        );

        let when = format_utc(now);
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        let live = broker::monitor_runtime_view(&conn, fleet_id, now + Duration::seconds(2))
            .map(|record| crate::presentation::monitor_runtime_view(&record))
            .unwrap();
        assert_eq!(live["wake_interval_seconds"], 600);

        let stale = broker::monitor_runtime_view(&conn, fleet_id, now + Duration::seconds(100))
            .map(|record| crate::presentation::monitor_runtime_view(&record))
            .unwrap();
        assert_eq!(
            stale["wake_interval_seconds"], 600,
            "preserved from the stale row, like tick_seconds"
        );

        for payload in [&live, &stale] {
            let keys: Vec<&str> = payload
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            let tick_pos = keys.iter().position(|k| *k == "tick_seconds").unwrap();
            assert_eq!(
                keys.get(tick_pos + 1),
                Some(&"wake_interval_seconds"),
                "the key sits immediately after tick_seconds"
            );
        }

        // A row that predates the migration and was never re-claimed since.
        conn.execute(
            "UPDATE monitor_runtime SET wake_interval_seconds=NULL WHERE fleet_id=?1",
            [fleet_id],
        )
        .unwrap();
        let premigration =
            broker::monitor_runtime_view(&conn, fleet_id, now + Duration::seconds(100))
                .map(|record| crate::presentation::monitor_runtime_view(&record))
                .unwrap();
        assert_eq!(premigration["wake_interval_seconds"], Value::Null);
    }

    #[test]
    fn request_monitor_wake_stamps_the_row_and_reports_a_never_run_fleet() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        assert!(
            !broker::request_monitor_wake(&mut conn, fleet_id, &when).unwrap(),
            "no row ⇔ the fleet's monitor has never run"
        );

        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        assert!(broker::request_monitor_wake(&mut conn, fleet_id, &when).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["wake_requested_at"], when);

        let later = format_utc(base_time() + Duration::seconds(30));
        assert!(broker::request_monitor_wake(&mut conn, fleet_id, &later).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            row["wake_requested_at"], later,
            "repeat requests coalesce into the latest stamp"
        );
    }

    #[test]
    fn a_fresh_claim_reads_a_null_request_and_a_reclaim_resets_a_pending_one() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            row["wake_requested_at"],
            Value::Null,
            "a fresh slot has no pending request"
        );

        assert!(broker::request_monitor_wake(&mut conn, fleet_id, &when).unwrap());

        // stale_after = max(3 * 5, 15) = 15; the 100-second-old heartbeat lets
        // a restarted loop reclaim the slot.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, 600, &later).unwrap());
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            row["wake_requested_at"],
            Value::Null,
            "a pending request never survives into a later loop instance"
        );
    }

    #[test]
    fn record_monitor_wake_clears_a_pending_request() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, 600, &when).unwrap();
        assert!(broker::request_monitor_wake(&mut conn, fleet_id, &when).unwrap());

        let later = format_utc(base_time() + Duration::seconds(10));
        broker::record_monitor_wake(&mut conn, fleet_id, &later).unwrap();
        let row = broker::read_monitor_runtime_record(&conn, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(row["last_wake_at"], later);
        assert_eq!(
            row["wake_requested_at"],
            Value::Null,
            "a delivered wake consumes the request in the same write"
        );
    }
}

#[cfg(test)]
mod compatibility_regressions {
    use super::*;
    use crate::broker::{self, test_support as common};

    #[test]
    fn raw_runtime_distinguishes_absence_legacy_null_zero_and_positive_intervals() {
        let dir = tempfile::Builder::new()
            .prefix(".runtime-wire-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, _) = common::create_fleet(&mut conn, "runtime");
        assert!(
            broker::read_monitor_runtime_record(&conn, fleet)
                .unwrap()
                .is_none()
        );
        conn.execute("INSERT INTO monitor_runtime(fleet_id) VALUES (?1)", [fleet])
            .unwrap();
        for interval in [None, Some(0), Some(90)] {
            conn.execute(
                "UPDATE monitor_runtime SET wake_interval_seconds=?1 WHERE fleet_id=?2",
                params![interval, fleet],
            )
            .unwrap();
            let record = broker::read_monitor_runtime_record(&conn, fleet)
                .unwrap()
                .unwrap();
            assert_eq!(record.fleet_id, fleet);
            assert_eq!(record.pid, None);
            assert_eq!(record.tick_seconds, 5);
            assert_eq!(record.wake_interval_seconds, interval);
            assert_eq!(record.started_at, None);
            assert_eq!(record.last_tick_at, None);
            assert_eq!(record.last_wake_at, None);
            assert_eq!(record.wake_requested_at, None);
            let row = crate::presentation::monitor_runtime(&record);
            let interval_json = serde_json::to_string(&interval).unwrap();
            assert_eq!(
                crate::output::format_json(&row),
                format!(
                    r#"{{"fleet_id":{fleet},"pid":null,"started_at":null,"last_tick_at":null,"tick_seconds":5,"wake_interval_seconds":{interval_json},"last_wake_at":null,"wake_requested_at":null}}"#
                )
            );
        }
        conn.execute(
            "UPDATE monitor_runtime SET pid=0 WHERE fleet_id=?1",
            [fleet],
        )
        .unwrap();
        assert_eq!(
            broker::read_monitor_runtime_record(&conn, fleet)
                .unwrap()
                .unwrap()
                .pid,
            Some(0),
            "zero is a stored PID, not a new null sentinel"
        );
    }

    #[test]
    fn clear_preserves_pending_request_and_wake_ledger_while_reclaim_resets_only_request() {
        let dir = tempfile::Builder::new()
            .prefix(".runtime-lifecycle-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, _) = common::create_fleet(&mut conn, "runtime");
        let pid = i64::from(std::process::id());
        let started = "2026-01-01T00:00:00+00:00";
        let wake = "2026-01-01T00:00:01+00:00";
        let request = "2026-01-01T00:00:02+00:00";
        broker::claim_monitor_runtime(&mut conn, fleet, pid, 7, 0, started).unwrap();
        broker::record_monitor_wake(&mut conn, fleet, wake).unwrap();
        broker::request_monitor_wake(&mut conn, fleet, request).unwrap();
        broker::clear_monitor_runtime(&mut conn, fleet, pid).unwrap();
        let stopped = broker::read_monitor_runtime_record(&conn, fleet)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::output::format_json(&stopped),
            format!(
                r#"{{"fleet_id":{fleet},"pid":null,"started_at":null,"last_tick_at":null,"tick_seconds":7,"wake_interval_seconds":0,"last_wake_at":"{wake}","wake_requested_at":"{request}"}}"#
            )
        );
        assert!(broker::claim_monitor_runtime(&mut conn, fleet, pid, 9, 45, request).unwrap());
        let reclaimed = broker::read_monitor_runtime_record(&conn, fleet)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed["last_wake_at"], wake);
        assert!(reclaimed["wake_requested_at"].is_null());
        assert_eq!(reclaimed["wake_interval_seconds"], 45);
        broker::request_monitor_wake(&mut conn, fleet, request).unwrap();
        broker::record_monitor_wake(&mut conn, fleet, request).unwrap();
        let delivered = broker::read_monitor_runtime_record(&conn, fleet)
            .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
            .unwrap()
            .unwrap();
        assert!(delivered["wake_requested_at"].is_null());
        assert_eq!(delivered["last_wake_at"], request);
    }

    #[test]
    fn stopped_runtime_projection_preserves_intervals_but_masks_raw_timestamps() {
        let dir = tempfile::Builder::new()
            .prefix(".runtime-projection-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, _) = common::create_fleet(&mut conn, "runtime");
        let now = crate::time::parse_lenient("2026-01-01T00:00:00+00:00").unwrap();
        conn.execute("INSERT INTO monitor_runtime(fleet_id,pid,started_at,last_tick_at,last_wake_at,wake_requested_at) VALUES (?1,0,'unparsed-start','not-a-time','unparsed-wake','pending')", [fleet]).unwrap();
        for interval in [None, Some(0), Some(90)] {
            conn.execute(
                "UPDATE monitor_runtime SET wake_interval_seconds=?1 WHERE fleet_id=?2",
                params![interval, fleet],
            )
            .unwrap();
            let raw = broker::read_monitor_runtime_record(&conn, fleet)
                .unwrap()
                .unwrap();
            assert_eq!(raw.started_at.as_deref(), Some("unparsed-start"));
            assert_eq!(raw.last_tick_at.as_deref(), Some("not-a-time"));
            assert_eq!(raw.last_wake_at.as_deref(), Some("unparsed-wake"));
            assert_eq!(raw.wake_requested_at.as_deref(), Some("pending"));
            assert_eq!(raw.pid, Some(0));
            assert_eq!(raw.wake_interval_seconds, interval);
            let view = broker::monitor_runtime_view(&conn, fleet, now).unwrap();
            assert!(!view.running);
            assert_eq!(view.pid, None);
            assert_eq!(view.tick_seconds, Some(5));
            assert_eq!(view.wake_interval_seconds, interval);
            assert_eq!(view.started_at, None);
            assert_eq!(view.last_tick_at, None);
            assert_eq!(view.last_wake_at, None);
            assert_eq!(view.last_tick_age_seconds, None);
            assert_eq!(view.last_wake_age_seconds, None);
            let payload = crate::presentation::monitor_runtime_view(&view);
            let interval_json = serde_json::to_string(&interval).unwrap();
            assert_eq!(
                crate::output::format_json(&payload),
                format!(
                    r#"{{"running":false,"pid":null,"tick_seconds":5,"wake_interval_seconds":{interval_json},"last_tick_at":null,"last_tick_age_seconds":null,"started_at":null,"last_wake_at":null,"last_wake_age_seconds":null}}"#
                )
            );
        }
    }
}
