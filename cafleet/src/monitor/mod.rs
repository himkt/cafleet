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
    use std::collections::{BTreeSet, HashMap};

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support::{
        FakeNotifier, create_fleet, migrated_conn, placement, register,
    };
    use crate::monitor::{
        DEFAULT_TICK_SECONDS, MEMBER_PING_INTERVAL_SECONDS, MONITOR_STALE_FACTOR,
        MONITOR_STALE_FLOOR_SECONDS, MonitorMux, MonitorTickState, TickResult, monitor_tick,
        run_monitor_loop, should_ping,
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
        wakes: RefCell<Vec<(String, Vec<Value>, Value)>>,
        statuses: RefCell<HashMap<String, String>>,
    }

    impl FakeMux {
        fn with_live_panes(panes: &[&str]) -> Self {
            FakeMux {
                live_panes: panes.iter().map(|p| p.to_string()).collect(),
                wake_ok: Cell::new(true),
                wakes: RefCell::new(Vec::new()),
                statuses: RefCell::new(HashMap::new()),
            }
        }

        fn set_status(&self, pane: &str, status: &str) {
            self.statuses
                .borrow_mut()
                .insert(pane.to_string(), status.to_string());
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
            due_members: &[Value],
            director: &Value,
        ) -> Result<bool, MultiplexerError> {
            self.wakes.borrow_mut().push((
                target_pane_id.to_string(),
                due_members.to_vec(),
                director.clone(),
            ));
            Ok(self.wake_ok.get())
        }

        fn agent_status(&self, target_pane_id: &str) -> Result<Option<String>, MultiplexerError> {
            Ok(self.statuses.borrow().get(target_pane_id).cloned())
        }
    }

    /// Fleet with two pane-bound workers (enrolled @720) on `%2` and `%4` and a
    /// monitoring member on `%3`; the unenrolled Director sits on `%0`.
    fn monitored_fleet(conn: &mut rusqlite::Connection) -> (i64, i64, i64, i64) {
        let (fleet_id, director_id) = create_fleet(conn, "alpha");
        let member_id = register(conn, fleet_id, "worker", Some("%2"));
        let second_id = register(conn, fleet_id, "helper", Some("%4"));
        broker::register_member(
            conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap();
        (fleet_id, director_id, member_id, second_id)
    }

    fn claim(conn: &mut rusqlite::Connection, fleet_id: i64, pid: i64, now: DateTime<Utc>) {
        assert!(broker::claim_monitor_runtime(conn, fleet_id, pid, 5, &format_utc(now)).unwrap());
    }

    fn tick(
        conn: &mut rusqlite::Connection,
        mux: &FakeMux,
        state: &mut MonitorTickState,
        fleet_id: i64,
        pid: i64,
        stall_interval: u64,
        now: DateTime<Utc>,
    ) -> (TickResult, String) {
        let mut out = Vec::new();
        let result = monitor_tick(
            conn,
            mux,
            state,
            &mut out,
            fleet_id,
            pid,
            stall_interval,
            now,
        )
        .unwrap();
        (result, String::from_utf8(out).unwrap())
    }

    fn ping_at(conn: &mut rusqlite::Connection, member_ids: &[i64], when: DateTime<Utc>) {
        broker::record_pings(conn, member_ids, &format_utc(when)).unwrap();
    }

    fn last_ping(conn: &rusqlite::Connection, fleet_id: i64, member_id: i64) -> Value {
        broker::get_monitor_config(conn, fleet_id, member_id)
            .unwrap()
            .unwrap()["last_ping_at"]
            .clone()
    }

    mod constants {
        use super::*;

        #[test]
        fn the_policy_tunables_are_pinned() {
            assert_eq!(DEFAULT_TICK_SECONDS, 5);
            assert_eq!(MEMBER_PING_INTERVAL_SECONDS, 720);
            assert_eq!(MONITOR_STALE_FACTOR, 3);
            assert_eq!(MONITOR_STALE_FLOOR_SECONDS, 15);
        }
    }

    mod should_ping_tests {
        use super::*;

        fn target(
            enabled: bool,
            pane_id: Option<&str>,
            pane_alive: bool,
            interval: i64,
            last_ping_at: Option<String>,
        ) -> Value {
            json!({
                "member_id": 4,
                "name": "worker",
                "pane_id": pane_id,
                "coding_agent": "claude",
                "interval_seconds": interval,
                "last_ping_at": last_ping_at,
                "enabled": enabled,
                "last_stall_check_at": null,
                "pending_count": 0,
                "oldest_pending_ts": null,
                "pane_alive": pane_alive,
            })
        }

        #[test]
        fn disabled_members_are_never_due() {
            assert!(!should_ping(
                &target(false, Some("%2"), true, 1, None),
                base_now()
            ));
        }

        #[test]
        fn unplaced_or_dead_panes_are_always_skipped() {
            assert!(!should_ping(&target(true, None, true, 1, None), base_now()));
            assert!(!should_ping(
                &target(true, Some("%2"), false, 1, None),
                base_now()
            ));
        }

        #[test]
        fn a_never_pinged_live_member_is_immediately_due() {
            assert!(should_ping(
                &target(true, Some("%2"), true, 720, None),
                base_now()
            ));
        }

        #[test]
        fn the_interval_gates_a_previously_pinged_member() {
            let now = base_now();
            let recent = Some(format_utc(now - Duration::seconds(100)));
            assert!(
                !should_ping(&target(true, Some("%2"), true, 180, recent), now),
                "elapsed 100 < 180 → not yet due"
            );
            let old = Some(format_utc(now - Duration::seconds(200)));
            assert!(
                should_ping(&target(true, Some("%2"), true, 180, old), now),
                "elapsed 200 >= 180 → due"
            );
        }
    }

    mod monitor_tick_tests {
        use super::*;

        #[test]
        fn a_displaced_or_unclaimed_heartbeat_stops_the_loop() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = monitored_fleet(&mut conn);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let now = base_now();

            let (result, _) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);
            assert!(matches!(result, TickResult::Stop), "no claimed slot → Stop");

            claim(&mut conn, fleet_id, own_pid(), now);
            let (result, _) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid() + 1, 0, now);
            assert!(matches!(result, TickResult::Stop), "displaced pid → Stop");
            assert_eq!(mux.wake_count(), 0, "a stopping tick never wakes");
        }

        #[test]
        fn a_deleted_fleet_stops_the_loop_after_the_heartbeat() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            conn.execute(
                "UPDATE fleets SET deleted_at='2026-07-30T09:00:00.000000+00:00' \
                 WHERE fleet_id=?1",
                [fleet_id],
            )
            .unwrap();

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let (result, _) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);
            assert!(matches!(result, TickResult::Stop));
        }

        #[test]
        fn due_members_produce_one_wake_and_gated_ledger_writes() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();

            let (result, echo) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);
            assert!(matches!(result, TickResult::Continue));

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "at most one synchronized wake per tick");
            let (pane, due, director) = &wakes[0];
            assert_eq!(pane, "%3", "the watcher's own pane");
            assert_eq!(due.len(), 2, "both workers, never the watcher");
            assert_eq!(director["member_id"], director_id);
            assert_eq!(director["coding_agent"], "claude");
            assert!(
                due.iter().all(|d| d["member_id"] != director_id),
                "the Director is never a due entry, got: {due:?}"
            );
            let worker_entry = due.iter().find(|d| d["member_id"] == member_id).unwrap();
            assert_eq!(worker_entry["wake_reasons"], json!(["interval"]));
            let second_entry = due.iter().find(|d| d["member_id"] == second_id).unwrap();
            assert_eq!(second_entry["wake_reasons"], json!(["interval"]));
            drop(wakes);

            let iso = format_utc(now);
            assert!(
                echo.contains(&format!(
                    "{iso} due member {member_id} (worker) [interval] -> wake monitor"
                )),
                "got: {echo}"
            );
            assert!(
                echo.contains(&format!(
                    "{iso} due member {second_id} (helper) [interval] -> wake monitor"
                )),
                "got: {echo}"
            );
            assert_eq!(last_ping(&conn, fleet_id, member_id), json!(iso));
            assert_eq!(last_ping(&conn, fleet_id, second_id), json!(iso));

            let (_, echo) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(5),
            );
            assert_eq!(mux.wake_count(), 1, "just-pinged members are not due again");
            assert!(echo.is_empty());
        }

        #[test]
        fn a_failed_wake_records_nothing_and_retries_next_tick() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            mux.wake_ok.set(false);
            let mut state = MonitorTickState::default();

            let (result, echo) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 1);
            assert!(echo.is_empty(), "no echo on a failed wake");
            assert_eq!(last_ping(&conn, fleet_id, member_id), Value::Null);
            assert_eq!(last_ping(&conn, fleet_id, second_id), Value::Null);

            mux.wake_ok.set(true);
            let (_, echo) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(5),
            );
            assert_eq!(
                mux.wake_count(),
                2,
                "the due members stay flagged and retry"
            );
            assert!(!echo.is_empty());
        }

        #[test]
        fn dead_panes_are_reconciled_and_never_due() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            broker::record_monitor_dispatch(
                &mut conn,
                &[],
                &[member_id],
                &format_utc(now - Duration::seconds(30)),
            )
            .unwrap();

            let mux = FakeMux::with_live_panes(&["%0", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let (_, _) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 240, now);

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1);
            let (_, due, _) = &wakes[0];
            assert!(
                due.iter().all(|d| d["member_id"] != member_id),
                "the dead-pane worker is skipped"
            );
            assert!(due.iter().any(|d| d["member_id"] == second_id));
            drop(wakes);

            let config = broker::get_monitor_config(&conn, fleet_id, member_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                config["last_stall_check_at"],
                Value::Null,
                "reconciliation clears the dead row's stall stamp before due filtering"
            );
        }

        #[test]
        fn no_live_watcher_means_no_wake_and_no_ledger_writes() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _) = create_fleet(&mut conn, "alpha");
            let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2"]);
            let mut state = MonitorTickState::default();

            let (result, echo) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 0);
            assert!(echo.is_empty());
            assert_eq!(
                last_ping(&conn, fleet_id, member_id),
                Value::Null,
                "nothing is recorded without a wake"
            );
        }

        #[test]
        fn stall_checks_are_durable_and_never_advance_last_ping_at() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let recent = now - Duration::seconds(10);
            ping_at(&mut conn, &[member_id, second_id], recent);

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let (_, echo) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 240, now);

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "a null stall stamp is due immediately");
            let (_, due, _) = &wakes[0];
            for entry in due {
                assert_eq!(entry["wake_reasons"], json!(["stall-check"]));
            }
            drop(wakes);
            assert!(
                echo.contains("[stall-check] -> wake monitor"),
                "got: {echo}"
            );

            let config = broker::get_monitor_config(&conn, fleet_id, member_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                config["last_stall_check_at"],
                json!(format_utc(now)),
                "a successful wake persists the dispatch stamp"
            );
            assert_eq!(
                config["last_ping_at"],
                json!(format_utc(recent)),
                "a stall-check-only member never advances last_ping_at"
            );

            let (_, _) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                240,
                now + Duration::seconds(30),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "the full stall interval must elapse before the next dispatch"
            );
        }

        #[test]
        fn a_zero_stall_interval_disables_the_branch() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            ping_at(
                &mut conn,
                &[member_id, second_id],
                now - Duration::seconds(10),
            );

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let (_, _) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);
            assert_eq!(mux.wake_count(), 0, "no stall-check wakes when disabled");
        }

        #[test]
        fn a_done_transition_wakes_once_per_episode() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            ping_at(&mut conn, &[member_id, second_id], now);

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            mux.set_status("%2", "done");
            let mut state = MonitorTickState::default();

            let (_, echo) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(5),
            );
            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "the transition into done flags a wake");
            let (_, due, _) = &wakes[0];
            assert_eq!(due.len(), 1);
            assert_eq!(due[0]["member_id"], member_id);
            assert_eq!(due[0]["wake_reasons"], json!(["status:done"]));
            drop(wakes);
            assert!(
                echo.contains("[status:done] -> wake monitor"),
                "got: {echo}"
            );

            let (_, _) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(10),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "an unchanged done status is edge-triggered — one wake per episode"
            );
        }

        #[test]
        fn a_blocked_transition_is_recorded_but_never_wakes() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            ping_at(&mut conn, &[member_id, second_id], now);

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            mux.set_status("%2", "blocked");
            let mut state = MonitorTickState::default();
            let (_, _) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(5),
            );
            assert_eq!(mux.wake_count(), 0, "blocked never flags a wake");

            mux.set_status("%2", "done");
            let (_, _) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(10),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "the blocked read was committed, so blocked → done is a transition"
            );
        }

        #[test]
        fn a_failed_wake_leaves_the_done_episode_unconsumed() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            ping_at(&mut conn, &[member_id, second_id], now);

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            mux.set_status("%2", "done");
            mux.wake_ok.set(false);
            let mut state = MonitorTickState::default();
            let (_, _) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(5),
            );
            assert_eq!(mux.wake_count(), 1);

            mux.wake_ok.set(true);
            let (_, _) = tick(
                &mut conn,
                &mux,
                &mut state,
                fleet_id,
                own_pid(),
                0,
                now + Duration::seconds(10),
            );
            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 2, "the un-consumed episode re-flags next tick");
            let (_, due, _) = &wakes[1];
            assert_eq!(due[0]["wake_reasons"], json!(["status:done"]));
        }

        #[test]
        fn unacked_annotates_but_never_adds_a_row() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let notifier = FakeNotifier::succeeding();
            crate::broker::test_support::send(
                &mut conn,
                &notifier,
                fleet_id,
                director_id,
                member_id,
                "hi",
            );
            conn.execute(
                "UPDATE messages SET status_timestamp=?1 WHERE type='unicast'",
                [format_utc(now - Duration::seconds(800))],
            )
            .unwrap();
            ping_at(&mut conn, &[second_id], now);

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let (_, echo) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1);
            let (_, due, _) = &wakes[0];
            assert_eq!(
                due.len(),
                1,
                "only the interval-due worker; the annotation adds no row"
            );
            assert_eq!(due[0]["member_id"], member_id);
            assert_eq!(
                due[0]["wake_reasons"],
                json!(["interval", "unacked"]),
                "unacked is appended last to an already-due row"
            );
            drop(wakes);
            assert!(
                echo.contains("[interval,unacked] -> wake monitor"),
                "got: {echo}"
            );
        }

        #[test]
        fn a_young_pending_delivery_is_not_annotated() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id, member_id, second_id) = monitored_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let notifier = FakeNotifier::succeeding();
            crate::broker::test_support::send(
                &mut conn,
                &notifier,
                fleet_id,
                director_id,
                member_id,
                "hi",
            );
            conn.execute(
                "UPDATE messages SET status_timestamp=?1 WHERE type='unicast'",
                [format_utc(now - Duration::seconds(30))],
            )
            .unwrap();
            ping_at(&mut conn, &[second_id], now);

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut state = MonitorTickState::default();
            let (_, _) = tick(&mut conn, &mux, &mut state, fleet_id, own_pid(), 0, now);

            let wakes = mux.wakes.borrow();
            let (_, due, _) = &wakes[0];
            assert_eq!(
                due[0]["wake_reasons"],
                json!(["interval"]),
                "a delivery younger than the interval is omitted"
            );
        }
    }

    mod run_monitor_loop_tests {
        use super::*;

        #[test]
        fn a_live_slot_refuses_a_second_loop() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _) = monitored_fleet(&mut conn);
            claim(&mut conn, fleet_id, own_pid(), Utc::now());

            let mux = FakeMux::with_live_panes(&["%0", "%2", "%3", "%4"]);
            let mut out = Vec::new();
            let err = run_monitor_loop(&mut conn, &mux, &mut out, fleet_id, 5, 0)
                .expect_err("the atomic claim is authoritative");
            assert_eq!(
                err.message(),
                format!("monitor already running for fleet {fleet_id}")
            );
            assert!(matches!(err, crate::error::CafleetError::App(_)));
        }
    }
}
