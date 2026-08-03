//! Monitor heartbeat loop (SPEC §6.6) — the pure `should_ping` due-check, the
//! per-tick scan `monitor_tick` (ownership-checked heartbeat, fleet liveness,
//! due-set + wake-reason computation, one synchronized wake, `woke`-gated
//! ledger writes), and the foreground driver. The colocated tests pin the
//! contract.
//!
//! Expected public API:
//!
//! ```text
//! pub const DEFAULT_TICK_SECONDS: i64 = 5;
//! // Policy tunables re-exported from their single broker home:
//! pub use crate::broker::{MEMBER_PING_INTERVAL_SECONDS /* 720 */,
//!     MONITOR_STALE_FACTOR /* 3 */, MONITOR_STALE_FLOOR_SECONDS /* 15 */};
//!
//! // What the tick consumes from the resolved backend.
//! pub trait MonitorMux {
//!     fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
//!     fn send_wake_trigger(&self, target_pane_id: &str, due_members: &[Value],
//!         director: &Value) -> Result<bool, MultiplexerError>;
//!     fn agent_status(&self, target_pane_id: &str)
//!         -> Result<Option<String>, MultiplexerError>;
//! }
//!
//! pub enum TickResult { Continue, Stop }
//! #[derive(Default)]
//! pub struct MonitorTickState { .. }  // the in-memory native-status map
//!
//! // Pure due-check over one scan row (a `list_monitor_targets` row with the
//! // loop-injected `pane_alive` bool): enabled → placed+alive → interval.
//! pub fn should_ping(target: &Value, now: DateTime<Utc>) -> bool;
//!
//! pub fn monitor_tick(conn: &mut Connection, mux: &dyn MonitorMux,
//!     state: &mut MonitorTickState, out: &mut dyn std::io::Write,
//!     fleet_id: i64, pid: i64, monitor_stall_interval: u64,
//!     now: DateTime<Utc>) -> Result<TickResult, CafleetError>;
//!
//! pub fn run_monitor_loop(conn: &mut Connection, mux: &dyn MonitorMux,
//!     out: &mut dyn std::io::Write, fleet_id: i64, tick_seconds: i64,
//!     monitor_stall_interval: u64) -> Result<(), CafleetError>;
//! ```

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::broker;
pub use crate::broker::{
    MEMBER_PING_INTERVAL_SECONDS, MONITOR_STALE_FACTOR, MONITOR_STALE_FLOOR_SECONDS,
};
use crate::error::CafleetError;
use crate::multiplexer::{Multiplexer, MultiplexerError};
use crate::time::{format_utc, now_utc, parse_lenient};

pub const DEFAULT_TICK_SECONDS: i64 = 5;

/// What the tick consumes from the resolved backend: pane liveness, the
/// single wake keystroke, and the optional native agent state.
pub trait MonitorMux {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        due_members: &[Value],
        director: &Value,
    ) -> Result<bool, MultiplexerError>;
    fn agent_status(&self, target_pane_id: &str) -> Result<Option<String>, MultiplexerError>;
}

impl<M: Multiplexer> MonitorMux for M {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
        Multiplexer::list_pane_ids(self)
    }

    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        due_members: &[Value],
        director: &Value,
    ) -> Result<bool, MultiplexerError> {
        Multiplexer::send_wake_trigger(self, target_pane_id, due_members, director)
    }

    fn agent_status(&self, target_pane_id: &str) -> Result<Option<String>, MultiplexerError> {
        Multiplexer::agent_status(self, target_pane_id)
    }
}

pub enum TickResult {
    Continue,
    Stop,
}

/// The in-memory native-status map: the last committed `agent_status` per
/// member, backing the edge-triggered `status:done` episode.
#[derive(Default)]
pub struct MonitorTickState {
    statuses: HashMap<i64, String>,
}

fn mux_err(error: MultiplexerError) -> CafleetError {
    CafleetError::App(error.to_string())
}

/// Pure due-check over one scan row (a `list_monitor_targets` row with the
/// loop-injected `pane_alive` bool): enabled → placed + alive → interval.
pub fn should_ping(target: &Value, now: DateTime<Utc>) -> bool {
    if target["enabled"] != true {
        return false;
    }
    if target["pane_id"].is_null() || target["pane_alive"] != true {
        return false;
    }
    let Some(last_ping_at) = target["last_ping_at"].as_str() else {
        return true;
    };
    let Ok(parsed) = parse_lenient(last_ping_at) else {
        return true;
    };
    let interval = target["interval_seconds"]
        .as_i64()
        .expect("scan rows carry interval_seconds");
    (now - parsed).num_seconds() >= interval
}

/// Whether the durable stall-check cadence has elapsed for an available
/// target (`0` disables the branch entirely).
fn stall_check_due(target: &Value, monitor_stall_interval: u64, now: DateTime<Utc>) -> bool {
    if monitor_stall_interval == 0 {
        return false;
    }
    if target["enabled"] != true || target["pane_id"].is_null() || target["pane_alive"] != true {
        return false;
    }
    let Some(last_stall_check_at) = target["last_stall_check_at"].as_str() else {
        return true;
    };
    let Ok(parsed) = parse_lenient(last_stall_check_at) else {
        return true;
    };
    (now - parsed).num_seconds() >= monitor_stall_interval as i64
}

/// Whether the target's oldest pending delivery has outlived its ping
/// interval — the `unacked` annotation on an already-due row.
fn unacked_overdue(target: &Value, now: DateTime<Utc>) -> bool {
    if target["pending_count"].as_i64().unwrap_or(0) == 0 {
        return false;
    }
    let Some(oldest) = target["oldest_pending_ts"].as_str() else {
        return false;
    };
    let Ok(parsed) = parse_lenient(oldest) else {
        return false;
    };
    let interval = target["interval_seconds"]
        .as_i64()
        .expect("scan rows carry interval_seconds");
    (now - parsed).num_seconds() >= interval
}

/// One scan pass (SPEC §6.6): ownership-checked heartbeat → fleet liveness →
/// pane reconciliation → due-set + wake-reason computation → one synchronized
/// wake → `woke`-gated ledger writes and heartbeat echoes.
#[allow(clippy::too_many_arguments)]
pub fn monitor_tick(
    conn: &mut Connection,
    mux: &dyn MonitorMux,
    state: &mut MonitorTickState,
    out: &mut dyn Write,
    fleet_id: i64,
    pid: i64,
    monitor_stall_interval: u64,
    now: DateTime<Utc>,
) -> Result<TickResult, CafleetError> {
    let iso = format_utc(now);
    if !broker::heartbeat_monitor_runtime(conn, fleet_id, pid, &iso)? {
        return Ok(TickResult::Stop);
    }
    let fleet = broker::get_fleet(conn, fleet_id)?;
    let live = match fleet {
        Some(ref fleet) => fleet["deleted_at"].is_null(),
        None => false,
    };
    if !live {
        return Ok(TickResult::Stop);
    }
    let fleet = fleet.expect("the live fleet row exists");

    let live_panes = mux.list_pane_ids().map_err(mux_err)?;
    let mut targets = broker::list_monitor_targets(conn, fleet_id)?;
    let mut unavailable: Vec<i64> = Vec::new();
    for target in &mut targets {
        let alive = target["pane_id"]
            .as_str()
            .is_some_and(|pane| live_panes.contains(pane));
        target["pane_alive"] = json!(alive);
        if !alive {
            unavailable.push(
                target["member_id"]
                    .as_i64()
                    .expect("scan rows carry member_id"),
            );
        }
    }
    if !unavailable.is_empty() {
        broker::reconcile_monitor_lifecycle(conn, fleet_id, &unavailable)?;
    }

    let watcher = broker::find_monitoring_member(conn, fleet_id)?.filter(|watcher| {
        watcher["pane_id"]
            .as_str()
            .is_some_and(|pane| live_panes.contains(pane))
    });
    let Some(watcher) = watcher else {
        return Ok(TickResult::Continue);
    };

    // Native agent-status scan over the available targets; a transition INTO
    // `done` flags a wake, `blocked` (and every other state) never does.
    let mut status_reads: Vec<(i64, Option<String>)> = Vec::new();
    let mut done_transitions: BTreeSet<i64> = BTreeSet::new();
    for target in &targets {
        if target["pane_alive"] != true {
            continue;
        }
        let member_id = target["member_id"]
            .as_i64()
            .expect("scan rows carry member_id");
        let pane = target["pane_id"]
            .as_str()
            .expect("an alive target has a pane");
        let status = mux.agent_status(pane).map_err(mux_err)?;
        if status.as_deref() == Some("done")
            && state.statuses.get(&member_id).map(String::as_str) != Some("done")
        {
            done_transitions.insert(member_id);
        }
        status_reads.push((member_id, status));
    }

    let mut due: Vec<Value> = Vec::new();
    let mut ping_ids: Vec<i64> = Vec::new();
    let mut stall_ids: Vec<i64> = Vec::new();
    for target in &targets {
        let member_id = target["member_id"]
            .as_i64()
            .expect("scan rows carry member_id");
        let mut reasons: Vec<&str> = Vec::new();
        if should_ping(target, now) {
            reasons.push("interval");
        }
        if stall_check_due(target, monitor_stall_interval, now) {
            reasons.push("stall-check");
        }
        if done_transitions.contains(&member_id) {
            reasons.push("status:done");
        }
        if reasons.is_empty() {
            continue;
        }
        if unacked_overdue(target, now) {
            reasons.push("unacked");
        }
        if reasons.contains(&"interval") {
            ping_ids.push(member_id);
        }
        if reasons.contains(&"stall-check") {
            stall_ids.push(member_id);
        }
        due.push(json!({
            "member_id": member_id,
            "name": target["name"],
            "coding_agent": target["coding_agent"],
            "wake_reasons": reasons,
        }));
    }

    let mut woke = true;
    if !due.is_empty() {
        let director_id = fleet["director_member_id"]
            .as_i64()
            .expect("a live fleet records its Director");
        let director_agent = broker::get_member(conn, director_id, fleet_id)?
            .expect("a live fleet's Director is registered")["placement"]["coding_agent"]
            .as_str()
            .expect("the root Director is pane-bound")
            .to_string();
        let director = json!({"member_id": director_id, "coding_agent": director_agent});
        let watcher_pane = watcher["pane_id"].as_str().expect("the watcher has a pane");
        woke = mux
            .send_wake_trigger(watcher_pane, &due, &director)
            .map_err(mux_err)?;
        if woke {
            broker::record_monitor_dispatch(conn, &ping_ids, &stall_ids, &iso)?;
            for entry in &due {
                let reasons = entry["wake_reasons"]
                    .as_array()
                    .expect("due entries carry wake_reasons")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",");
                writeln!(
                    out,
                    "{iso} due member {} ({}) [{reasons}] -> wake monitor",
                    entry["member_id"],
                    entry["name"]
                        .as_str()
                        .expect("due entries carry the raw name"),
                )
                .map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))?;
            }
        }
    }

    // Commit the status reads; an unconsumed done episode (a failed wake)
    // keeps its pre-transition value so it re-flags next tick.
    for (member_id, status) in status_reads {
        match status {
            Some(status) => {
                if status == "done" && done_transitions.contains(&member_id) && !woke {
                    continue;
                }
                state.statuses.insert(member_id, status);
            }
            None => {
                state.statuses.remove(&member_id);
            }
        }
    }

    Ok(TickResult::Continue)
}

/// The polled tick sleep (SPEC §6.6): a monotonic deadline drained in
/// `min(0.2s, remaining)` slices, so signal response stays ≤200 ms regardless
/// of `tick_seconds`.
fn interruptible_sleep(seconds: u64, stop: &AtomicBool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    while !stop.load(Ordering::Relaxed) {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(200)));
    }
}

/// The foreground driver: atomically claim the runtime slot, install the
/// SIGTERM/SIGINT stop flag, print the startup line, then tick until stopped
/// or displaced; the exit path is an ownership-checked clear.
pub fn run_monitor_loop(
    conn: &mut Connection,
    mux: &dyn MonitorMux,
    out: &mut dyn Write,
    fleet_id: i64,
    tick_seconds: i64,
    monitor_stall_interval: u64,
) -> Result<(), CafleetError> {
    let pid = i64::from(std::process::id());
    let now = format_utc(now_utc());
    if !broker::claim_monitor_runtime(conn, fleet_id, pid, tick_seconds, &now)? {
        return Err(CafleetError::App(format!(
            "monitor already running for fleet {fleet_id}"
        )));
    }

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, Arc::clone(&stop))
            .map_err(|e| CafleetError::App(format!("cannot install the signal handler: {e}")))?;
    }

    writeln!(
        out,
        "monitor loop started (fleet {fleet_id}, tick {tick_seconds}s, pid {pid})"
    )
    .map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))?;
    out.flush().ok();

    let mut state = MonitorTickState::default();
    let outcome = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        match monitor_tick(
            conn,
            mux,
            &mut state,
            out,
            fleet_id,
            pid,
            monitor_stall_interval,
            now_utc(),
        ) {
            Ok(TickResult::Continue) => {}
            Ok(TickResult::Stop) => break Ok(()),
            Err(error) => break Err(error),
        }
        out.flush().ok();
        interruptible_sleep(
            u64::try_from(tick_seconds).expect("tick_seconds is positive"),
            &stop,
        );
    };
    broker::clear_monitor_runtime(conn, fleet_id, pid)?;
    outcome
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support::{create_fleet, migrated_conn, register};
    use crate::monitor::{
        DEFAULT_TICK_SECONDS, MONITOR_STALE_FACTOR, MONITOR_STALE_FLOOR_SECONDS, MonitorMux,
        TickResult, monitor_tick, run_monitor_loop, wake_due,
    };
    use crate::multiplexer::MultiplexerError;
    use crate::time::format_utc;

    fn own_pid() -> i64 {
        i64::from(std::process::id())
    }

    fn base_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 30, 10, 0, 0).unwrap()
    }

    struct FakeMux {
        live_panes: BTreeSet<String>,
        wake_ok: Cell<bool>,
        wakes: RefCell<Vec<(String, i64, Vec<Value>)>>,
    }

    impl FakeMux {
        fn with_live_panes(panes: &[&str]) -> Self {
            FakeMux {
                live_panes: panes.iter().map(|p| p.to_string()).collect(),
                wake_ok: Cell::new(true),
                wakes: RefCell::new(Vec::new()),
            }
        }

        fn wake_count(&self) -> usize {
            self.wakes.borrow().len()
        }
    }

    impl MonitorMux for FakeMux {
        fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
            Ok(self.live_panes.clone())
        }

        fn send_wake_trigger(
            &self,
            target_pane_id: &str,
            fleet_id: i64,
            members: &[Value],
        ) -> Result<bool, MultiplexerError> {
            self.wakes.borrow_mut().push((
                target_pane_id.to_string(),
                fleet_id,
                members.to_vec(),
            ));
            Ok(self.wake_ok.get())
        }
    }

    /// Fleet with two pane-bound workers on `%2` and `%4`; the Director (the
    /// wake recipient) sits on `%0`.
    fn wake_fleet(conn: &mut rusqlite::Connection) -> (i64, i64, i64, i64) {
        let (fleet_id, director_id) = create_fleet(conn, "alpha");
        let member_id = register(conn, fleet_id, "worker", Some("%2"));
        let second_id = register(conn, fleet_id, "helper", Some("%4"));
        (fleet_id, director_id, member_id, second_id)
    }

    fn claim(conn: &mut rusqlite::Connection, fleet_id: i64, pid: i64, now: DateTime<Utc>) {
        assert!(broker::claim_monitor_runtime(conn, fleet_id, pid, 5, &format_utc(now)).unwrap());
    }

    fn tick(
        conn: &mut rusqlite::Connection,
        mux: &FakeMux,
        fleet_id: i64,
        pid: i64,
        wake_interval: u64,
        now: DateTime<Utc>,
    ) -> (TickResult, String) {
        let mut out = Vec::new();
        let result = monitor_tick(conn, mux, &mut out, fleet_id, pid, wake_interval, now).unwrap();
        (result, String::from_utf8(out).unwrap())
    }

    fn last_wake_at(conn: &rusqlite::Connection, fleet_id: i64) -> Value {
        broker::read_monitor_runtime(conn, fleet_id)
            .unwrap()
            .unwrap()["last_wake_at"]
            .clone()
    }

    mod constants {
        use super::*;

        #[test]
        fn the_policy_tunables_are_pinned() {
            assert_eq!(DEFAULT_TICK_SECONDS, 5);
            assert_eq!(MONITOR_STALE_FACTOR, 3);
            assert_eq!(MONITOR_STALE_FLOOR_SECONDS, 15);
        }
    }

    mod wake_due_tests {
        use super::*;

        #[test]
        fn a_missing_or_unparsable_stamp_is_immediately_due() {
            assert!(wake_due(None, 600, base_now()));
            assert!(wake_due(Some("not-a-timestamp"), 600, base_now()));
        }

        #[test]
        fn the_interval_gates_a_stamped_fleet() {
            let now = base_now();
            let recent = format_utc(now - Duration::seconds(599));
            assert!(
                !wake_due(Some(&recent), 600, now),
                "elapsed 599 < 600 → not yet due"
            );
            let old = format_utc(now - Duration::seconds(600));
            assert!(
                wake_due(Some(&old), 600, now),
                "elapsed 600 >= 600 → due"
            );
        }
    }

    mod monitor_tick_tests {
        use super::*;

        #[test]
        fn a_displaced_or_unclaimed_heartbeat_stops_the_loop() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);
            let now = base_now();

            let (result, _) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert!(matches!(result, TickResult::Stop), "no claimed slot → Stop");

            claim(&mut conn, fleet_id, own_pid(), now);
            let (result, _) = tick(&mut conn, &mux, fleet_id, own_pid() + 1, 600, now);
            assert!(matches!(result, TickResult::Stop), "displaced pid → Stop");
            assert_eq!(mux.wake_count(), 0, "a stopping tick never wakes");
        }

        #[test]
        fn a_deleted_fleet_stops_the_loop_after_the_heartbeat() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            conn.execute(
                "UPDATE fleets SET deleted_at='2026-07-30T09:00:00.000000+00:00' \
                 WHERE fleet_id=?1",
                [fleet_id],
            )
            .unwrap();

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);
            let (result, _) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert!(matches!(result, TickResult::Stop));
        }

        #[test]
        fn a_due_tick_wakes_the_director_and_stamps_last_wake_at() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id, member_id, second_id) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);

            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert!(matches!(result, TickResult::Continue));

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "one keystroke at one pane per due tick");
            let (pane, wake_fleet_id, members) = &wakes[0];
            assert_eq!(pane, "%0", "the Director's own pane");
            assert_eq!(*wake_fleet_id, fleet_id);
            assert_eq!(members.len(), 2, "both workers, never the Director");
            assert_eq!(members[0]["member_id"], member_id);
            assert_eq!(members[1]["member_id"], second_id);
            assert!(
                members.iter().all(|m| m["member_id"] != director_id),
                "the Director is the recipient, not a referent"
            );
            drop(wakes);

            let iso = format_utc(now);
            assert_eq!(
                echo,
                format!("{iso} tick -> wake director {director_id} (2 members)\n")
            );
            assert_eq!(last_wake_at(&conn, fleet_id), iso.as_str());
        }

        #[test]
        fn the_wake_interval_gates_the_next_wake() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);

            let (_, _) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert_eq!(mux.wake_count(), 1, "a NULL stamp is immediately due");

            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
                now + Duration::seconds(599),
            );
            assert_eq!(mux.wake_count(), 1, "599 < 600 → not due");
            assert!(echo.is_empty());

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
                now + Duration::seconds(600),
            );
            assert_eq!(mux.wake_count(), 2, "600 >= 600 → due again");
        }

        #[test]
        fn a_zero_interval_heartbeats_without_waking() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);

            let later = now + Duration::seconds(2);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 0, later);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 0, "interval 0 disables the wake");
            assert!(echo.is_empty());
            assert_eq!(last_wake_at(&conn, fleet_id), Value::Null);

            let row = broker::read_monitor_runtime(&conn, fleet_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                row["last_tick_at"],
                format_utc(later),
                "the loop keeps claiming the slot and heartbeating"
            );
        }

        #[test]
        fn a_dead_director_pane_skips_the_wake_without_stamping() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%2", "%4"]);

            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 0, "no live Director pane → no wake");
            assert!(echo.is_empty());
            assert_eq!(
                last_wake_at(&conn, fleet_id),
                Value::Null,
                "a skipped wake stamps nothing"
            );
        }

        #[test]
        fn a_failed_wake_commits_nothing_and_retries_next_tick() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);
            mux.wake_ok.set(false);

            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 1);
            assert!(echo.is_empty(), "no echo on a failed wake");
            assert_eq!(last_wake_at(&conn, fleet_id), Value::Null);

            mux.wake_ok.set(true);
            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
                now + Duration::seconds(5),
            );
            assert_eq!(mux.wake_count(), 2, "the unstamped fleet stays due");
            assert!(!echo.is_empty());
            assert_eq!(
                last_wake_at(&conn, fleet_id),
                format_utc(now + Duration::seconds(5)).as_str()
            );
        }

        #[test]
        fn a_fleet_with_no_members_still_wakes_the_director() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0"]);

            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
            assert!(matches!(result, TickResult::Continue));

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "the tick fires even with no other members");
            let (pane, _, members) = &wakes[0];
            assert_eq!(pane, "%0");
            assert!(members.is_empty(), "the N == 0 roster is empty");
            drop(wakes);

            let iso = format_utc(now);
            assert_eq!(
                echo,
                format!("{iso} tick -> wake director {director_id} (0 members)\n")
            );
            assert_eq!(last_wake_at(&conn, fleet_id), iso.as_str());
        }
    }

    mod run_monitor_loop_tests {
        use super::*;

        #[test]
        fn a_live_slot_refuses_a_second_loop() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = wake_fleet(&mut conn);
            claim(&mut conn, fleet_id, own_pid(), Utc::now());

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);
            let mut out = Vec::new();
            let err = run_monitor_loop(&mut conn, &mux, &mut out, fleet_id, 5, 600)
                .expect_err("the atomic claim is authoritative");
            assert_eq!(
                err.message(),
                format!("monitor already running for fleet {fleet_id}")
            );
            assert!(matches!(err, crate::error::CafleetError::App(_)));
        }
    }
}
