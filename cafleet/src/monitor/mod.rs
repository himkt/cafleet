//! Monitor heartbeat loop (SPEC §6.6) — the pure `wake_due` check, the
//! per-tick scan `monitor_tick` (ownership-checked heartbeat, fleet liveness,
//! the fleet-level wake into the monitor member's pane, the `woke`-gated
//! ledger write), and the foreground driver. The colocated tests pin the
//! contract.
//!
//! Expected public API:
//!
//! ```text
//! pub const DEFAULT_TICK_SECONDS: i64 = 5;
//! // Runtime-staleness tunables re-exported from their single broker home:
//! pub use crate::broker::{MONITOR_STALE_FACTOR /* 3 */,
//!     MONITOR_STALE_FLOOR_SECONDS /* 15 */};
//!
//! // What the tick consumes from the resolved backend.
//! pub trait MonitorMux {
//!     fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
//!     fn send_wake_trigger(&self, target_pane_id: &str, fleet_id: i64,
//!         members: &[Value], director: &Value)
//!         -> Result<bool, MultiplexerError>;
//! }
//!
//! pub enum TickResult { Continue, Stop }
//!
//! // Pure due-check for the fleet-level wake: a present last_wake_at always
//! // wins as the baseline (unparsable → immediately due); a NULL last_wake_at
//! // falls back to started_at (NULL or unparsable → immediately due).
//! pub fn wake_due(last_wake_at: Option<&str>, started_at: Option<&str>,
//!     wake_interval: i64, now: DateTime<Utc>) -> bool;
//!
//! // No interval parameter: each pass re-reads wake_interval_seconds from
//! // the fleet's runtime row, so an external update changes the cadence
//! // within one tick.
//! pub fn monitor_tick(conn: &mut Connection, mux: &dyn MonitorMux,
//!     out: &mut dyn std::io::Write, fleet_id: i64, pid: i64,
//!     now: DateTime<Utc>) -> Result<TickResult, CafleetError>;
//!
//! // wake_interval is used only to stamp the claim; the ticks read the
//! // stored value.
//! pub fn run_monitor_loop(conn: &mut Connection, mux: &dyn MonitorMux,
//!     out: &mut dyn std::io::Write, fleet_id: i64, tick_seconds: i64,
//!     wake_interval: i64) -> Result<(), CafleetError>;
//! ```

use std::collections::BTreeSet;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;

use crate::broker;
pub use crate::broker::{MONITOR_STALE_FACTOR, MONITOR_STALE_FLOOR_SECONDS};
use crate::error::CafleetError;
use crate::multiplexer::{Multiplexer, MultiplexerError};
use crate::time::{format_utc, now_utc, parse_lenient};

pub const DEFAULT_TICK_SECONDS: i64 = 5;

/// What the tick consumes from the resolved backend: pane liveness and the
/// single monitor-wake keystroke.
pub trait MonitorMux {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[Value],
        director: &Value,
    ) -> Result<bool, MultiplexerError>;
}

impl<M: Multiplexer> MonitorMux for M {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
        Multiplexer::list_pane_ids(self)
    }

    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[Value],
        director: &Value,
    ) -> Result<bool, MultiplexerError> {
        Multiplexer::send_wake_trigger(self, target_pane_id, fleet_id, members, director)
    }
}

pub enum TickResult {
    Continue,
    Stop,
}

fn mux_err(error: MultiplexerError) -> CafleetError {
    CafleetError::App(error.to_string())
}

/// Pure due-check for the fleet-level wake. A present `last_wake_at` always
/// wins as the baseline, even over a fresher post-reclaim `started_at`
/// (unparsable → immediately due); a `NULL` `last_wake_at` falls back to
/// `started_at` (`NULL` or unparsable → immediately due, corrupt state);
/// otherwise due once the interval has elapsed since the baseline.
pub fn wake_due(
    last_wake_at: Option<&str>,
    started_at: Option<&str>,
    wake_interval: i64,
    now: DateTime<Utc>,
) -> bool {
    let Some(baseline) = last_wake_at.or(started_at) else {
        return true;
    };
    let Ok(parsed) = parse_lenient(baseline) else {
        return true;
    };
    (now - parsed).num_seconds() >= wake_interval
}

/// One scan pass (SPEC §6.6): ownership-checked heartbeat → fleet liveness →
/// runtime-row read (the per-tick interval re-read) → the schedule gates
/// (wake-interval and due-check, both bypassed by a pending forced-wake
/// request) → monitor-pane resolution → one fleet-level wake into the monitor
/// member's pane → the `woke`-gated ledger write and heartbeat echo.
pub fn monitor_tick(
    conn: &mut Connection,
    mux: &dyn MonitorMux,
    out: &mut dyn Write,
    fleet_id: i64,
    pid: i64,
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

    let runtime = broker::read_monitor_runtime(conn, fleet_id)?
        .expect("the heartbeat just matched this fleet's runtime row");
    let wake_interval = runtime["wake_interval_seconds"]
        .as_i64()
        .expect("the owning loop stamped the interval at claim");
    // A pending operator request bypasses both schedule gates — a disabled
    // interval and a not-yet-due wake alike.
    let forced = !runtime["wake_requested_at"].is_null();
    if !forced {
        if wake_interval == 0 {
            return Ok(TickResult::Continue);
        }
        if !wake_due(
            runtime["last_wake_at"].as_str(),
            runtime["started_at"].as_str(),
            wake_interval,
            now,
        ) {
            return Ok(TickResult::Continue);
        }
    }

    // No active monitor member, a monitor with no pane, or a pane absent
    // from the live set skips the wake without stamping — the fleet stays
    // due for the next tick.
    let Some(monitor_id) = broker::active_monitor_member_id(conn, fleet_id)? else {
        return Ok(TickResult::Continue);
    };
    let monitor = broker::get_member(conn, monitor_id, fleet_id)?;
    let monitor_pane = monitor
        .as_ref()
        .and_then(|member| member["placement"]["mux_pane_id"].as_str());
    let Some(monitor_pane) = monitor_pane else {
        return Ok(TickResult::Continue);
    };
    let live_panes = mux.list_pane_ids().map_err(mux_err)?;
    if !live_panes.contains(monitor_pane) {
        return Ok(TickResult::Continue);
    }

    let roster = broker::list_fleet_wake_targets(conn, fleet_id)?;
    let director = broker::fleet_wake_director(conn, fleet_id)?;
    let woke = mux
        .send_wake_trigger(monitor_pane, fleet_id, &roster, &director)
        .map_err(mux_err)?;
    if woke {
        broker::record_monitor_wake(conn, fleet_id, &iso)?;
        let label = if forced { "forced wake" } else { "wake" };
        writeln!(
            out,
            "{iso} tick -> {label} monitor {monitor_id} ({} members)",
            roster.len()
        )
        .map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))?;
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
    wake_interval: i64,
) -> Result<(), CafleetError> {
    let pid = i64::from(std::process::id());
    let now = format_utc(now_utc());
    if !broker::claim_monitor_runtime(conn, fleet_id, pid, tick_seconds, wake_interval, &now)? {
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

    let outcome = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        match monitor_tick(conn, mux, out, fleet_id, pid, now_utc()) {
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
    use crate::broker::test_support::{
        bootstrap_monitor, create_fleet, migrated_conn, register, register_monitor,
    };
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

    type WakeCall = (String, i64, Vec<Value>, Value);

    struct FakeMux {
        live_panes: BTreeSet<String>,
        wake_ok: Cell<bool>,
        wakes: RefCell<Vec<WakeCall>>,
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
            director: &Value,
        ) -> Result<bool, MultiplexerError> {
            self.wakes.borrow_mut().push((
                target_pane_id.to_string(),
                fleet_id,
                members.to_vec(),
                director.clone(),
            ));
            Ok(self.wake_ok.get())
        }
    }

    /// Fleet with its bootstrap monitor member (the wake recipient) on `%1`
    /// and two pane-bound workers on `%2` and `%4`; the Director sits on `%0`.
    fn wake_fleet(conn: &mut rusqlite::Connection) -> (i64, i64, i64, i64, i64) {
        let (fleet_id, director_id) = create_fleet(conn, "alpha");
        let monitor_id = bootstrap_monitor(conn, fleet_id);
        let member_id = register(conn, fleet_id, "worker", Some("%2"));
        let second_id = register(conn, fleet_id, "helper", Some("%4"));
        (fleet_id, director_id, monitor_id, member_id, second_id)
    }

    fn claim(conn: &mut rusqlite::Connection, fleet_id: i64, pid: i64, now: DateTime<Utc>) {
        claim_with_interval(conn, fleet_id, pid, 600, now);
    }

    fn claim_with_interval(
        conn: &mut rusqlite::Connection,
        fleet_id: i64,
        pid: i64,
        wake_interval: i64,
        now: DateTime<Utc>,
    ) {
        assert!(
            broker::claim_monitor_runtime(conn, fleet_id, pid, 5, wake_interval, &format_utc(now))
                .unwrap()
        );
    }

    fn set_interval(conn: &mut rusqlite::Connection, fleet_id: i64, wake_interval: i64) {
        assert!(broker::set_monitor_wake_interval(conn, fleet_id, wake_interval).unwrap());
    }

    fn tick(
        conn: &mut rusqlite::Connection,
        mux: &FakeMux,
        fleet_id: i64,
        pid: i64,
        now: DateTime<Utc>,
    ) -> (TickResult, String) {
        let mut out = Vec::new();
        let result = monitor_tick(conn, mux, &mut out, fleet_id, pid, now).unwrap();
        (result, String::from_utf8(out).unwrap())
    }

    fn last_wake_at(conn: &rusqlite::Connection, fleet_id: i64) -> Value {
        broker::read_monitor_runtime(conn, fleet_id)
            .unwrap()
            .unwrap()["last_wake_at"]
            .clone()
    }

    fn wake_requested_at(conn: &rusqlite::Connection, fleet_id: i64) -> Value {
        broker::read_monitor_runtime(conn, fleet_id)
            .unwrap()
            .unwrap()["wake_requested_at"]
            .clone()
    }

    fn request_wake(conn: &mut rusqlite::Connection, fleet_id: i64, when: DateTime<Utc>) {
        assert!(broker::request_monitor_wake(conn, fleet_id, &format_utc(when)).unwrap());
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
            let now = base_now();
            let fresh = format_utc(now);
            assert!(
                wake_due(Some("not-a-timestamp"), Some(&fresh), 600, now),
                "an unparsable last_wake_at is due regardless of started_at"
            );
            assert!(wake_due(Some("not-a-timestamp"), None, 600, now));
            assert!(
                wake_due(None, None, 600, now),
                "no stamp at all → immediately due"
            );
            assert!(
                wake_due(None, Some("not-a-timestamp"), 600, now),
                "a NULL last_wake_at with a corrupt started_at → immediately due"
            );
        }

        #[test]
        fn a_null_last_wake_defers_to_the_started_at_baseline() {
            let now = base_now();
            let recent = format_utc(now - Duration::seconds(599));
            assert!(
                !wake_due(None, Some(&recent), 600, now),
                "elapsed 599 < 600 since started_at → not yet due"
            );
            let old = format_utc(now - Duration::seconds(600));
            assert!(
                wake_due(None, Some(&old), 600, now),
                "elapsed 600 >= 600 since started_at → due"
            );
        }

        #[test]
        fn a_present_last_wake_wins_over_a_fresher_started_at() {
            let now = base_now();
            let started = format_utc(now - Duration::seconds(1));
            let old_wake = format_utc(now - Duration::seconds(600));
            assert!(
                wake_due(Some(&old_wake), Some(&started), 600, now),
                "post-reclaim: the old last_wake_at drives due-ness, not the fresh started_at"
            );
            let recent_wake = format_utc(now - Duration::seconds(599));
            assert!(
                !wake_due(Some(&recent_wake), Some(&started), 600, now),
                "elapsed 599 < 600 since last_wake_at → not due"
            );
        }

        #[test]
        fn the_interval_gates_a_stamped_fleet() {
            let now = base_now();
            let started = format_utc(now - Duration::seconds(1_000));
            let recent = format_utc(now - Duration::seconds(599));
            assert!(
                !wake_due(Some(&recent), Some(&started), 600, now),
                "elapsed 599 < 600 → not yet due"
            );
            let old = format_utc(now - Duration::seconds(600));
            assert!(
                wake_due(Some(&old), Some(&started), 600, now),
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
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
            let now = base_now();

            let (result, _) = tick(&mut conn, &mux, fleet_id, own_pid(), now);
            assert!(matches!(result, TickResult::Stop), "no claimed slot → Stop");

            claim(&mut conn, fleet_id, own_pid(), now);
            let (result, _) = tick(&mut conn, &mux, fleet_id, own_pid() + 1, now);
            assert!(matches!(result, TickResult::Stop), "displaced pid → Stop");
            assert_eq!(mux.wake_count(), 0, "a stopping tick never wakes");
        }

        #[test]
        fn a_deleted_fleet_stops_the_loop_after_the_heartbeat() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            conn.execute(
                "UPDATE fleets SET deleted_at='2026-07-30T09:00:00.000000+00:00' \
                 WHERE fleet_id=?1",
                [fleet_id],
            )
            .unwrap();

            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
            let (result, _) = tick(&mut conn, &mux, fleet_id, own_pid(), now);
            assert!(matches!(result, TickResult::Stop));
        }

        #[test]
        fn a_due_tick_wakes_the_monitor_and_stamps_last_wake_at() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id, monitor_id, member_id, second_id) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let due_at = now + Duration::seconds(600);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), due_at);
            assert!(matches!(result, TickResult::Continue));

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "one keystroke at one pane per due tick");
            let (pane, wake_fleet_id, members, director) = &wakes[0];
            assert_eq!(pane, "%1", "the monitor member's own pane");
            assert_eq!(*wake_fleet_id, fleet_id);
            assert_eq!(
                members.len(),
                2,
                "both workers, never the Director or the monitor"
            );
            assert_eq!(members[0]["member_id"], member_id);
            assert_eq!(members[1]["member_id"], second_id);
            assert!(
                members
                    .iter()
                    .all(|m| m["member_id"] != director_id && m["member_id"] != monitor_id),
                "the monitor is the recipient and the Director rides its own segment"
            );
            assert_eq!(
                director["member_id"], director_id,
                "the Director descriptor passes through to the wake"
            );
            assert_eq!(director["name"], "Director");
            assert_eq!(director["coding_agent"], "claude");
            assert_eq!(director["pending_count"], 0);
            drop(wakes);

            let iso = format_utc(due_at);
            assert_eq!(
                echo,
                format!("{iso} tick -> wake monitor {monitor_id} (2 members)\n")
            );
            assert_eq!(last_wake_at(&conn, fleet_id), iso.as_str());
        }

        #[test]
        fn the_wake_interval_gates_the_next_wake() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let (_, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), now);
            assert_eq!(
                mux.wake_count(),
                0,
                "the first wake is deferred past the claim tick"
            );
            assert!(echo.is_empty());

            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(599),
            );
            assert_eq!(mux.wake_count(), 0, "599 < 600 since started_at → not due");
            assert!(echo.is_empty());

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "600 >= 600 since started_at → first wake"
            );

            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(1199),
            );
            assert_eq!(mux.wake_count(), 1, "599 since the first wake → not due");
            assert!(echo.is_empty());

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(1200),
            );
            assert_eq!(
                mux.wake_count(),
                2,
                "600 since the first wake → second wake"
            );
        }

        #[test]
        fn a_zero_interval_heartbeats_without_waking() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim_with_interval(&mut conn, fleet_id, own_pid(), 0, now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let later = now + Duration::seconds(2);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), later);
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
        fn a_mid_run_shrink_below_the_elapsed_time_is_due_on_the_next_tick() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert_eq!(mux.wake_count(), 1, "the first wake sets the baseline");

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(900),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "300 < 600 since the baseline → not due"
            );

            set_interval(&mut conn, fleet_id, 200);
            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(905),
            );
            assert_eq!(
                mux.wake_count(),
                2,
                "305 >= the shrunken 200 → due on the very next tick"
            );
        }

        #[test]
        fn a_mid_run_zero_disables_and_a_raise_re_enables_against_the_baseline() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert_eq!(mux.wake_count(), 1, "the first wake sets the baseline");

            set_interval(&mut conn, fleet_id, 0);
            let at = now + Duration::seconds(800);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), at);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(
                mux.wake_count(),
                1,
                "0 disables the wake from the next tick"
            );
            assert!(echo.is_empty());
            let row = broker::read_monitor_runtime(&conn, fleet_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                row["last_tick_at"],
                format_utc(at),
                "the loop keeps heartbeating while disabled"
            );

            set_interval(&mut conn, fleet_id, 300);
            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(899),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "299 < 300 since the existing baseline → not yet due"
            );

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(900),
            );
            assert_eq!(
                mux.wake_count(),
                2,
                "the raise re-enables, gated against the existing baseline"
            );
        }

        #[test]
        fn an_edit_before_the_first_wake_moves_the_first_wake_boundary() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(100),
            );
            assert_eq!(mux.wake_count(), 0, "not due under the startup 600");

            set_interval(&mut conn, fleet_id, 900);
            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert_eq!(
                mux.wake_count(),
                0,
                "the old started_at + 600 boundary no longer applies"
            );

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(899),
            );
            assert_eq!(mux.wake_count(), 0, "899 < 900 since started_at → not due");

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(900),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "the first wake fires at started_at + the new interval"
            );
        }

        #[test]
        fn a_dead_monitor_pane_skips_the_wake_without_stamping() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 0, "no live monitor pane → no wake");
            assert!(echo.is_empty());
            assert_eq!(
                last_wake_at(&conn, fleet_id),
                Value::Null,
                "a skipped wake stamps nothing — the fleet stays due"
            );
        }

        #[test]
        fn a_fleet_with_no_monitor_member_skips_the_wake_without_stamping() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _) = create_fleet(&mut conn, "alpha");
            let dead_monitor = bootstrap_monitor(&conn, fleet_id);
            broker::deregister_member(&mut conn, dead_monitor).unwrap();
            register(&mut conn, fleet_id, "worker", Some("%2"));
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2"]);

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 0, "no active monitor member → no wake");
            assert!(echo.is_empty());
            assert_eq!(last_wake_at(&conn, fleet_id), Value::Null);
        }

        #[test]
        fn a_monitor_with_a_pending_placement_skips_the_wake_without_stamping() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _) = create_fleet(&mut conn, "alpha");
            let dead_monitor = bootstrap_monitor(&conn, fleet_id);
            broker::deregister_member(&mut conn, dead_monitor).unwrap();
            register_monitor(&mut conn, fleet_id, "monitor", None);
            register(&mut conn, fleet_id, "worker", Some("%2"));
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%2"]);

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 0, "a paneless monitor → no wake");
            assert!(echo.is_empty());
            assert_eq!(last_wake_at(&conn, fleet_id), Value::Null);
        }

        #[test]
        fn a_failed_wake_commits_nothing_and_retries_next_tick() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
            mux.wake_ok.set(false);

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(600),
            );
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
                now + Duration::seconds(605),
            );
            assert_eq!(mux.wake_count(), 2, "the unstamped fleet stays due");
            assert!(!echo.is_empty());
            assert_eq!(
                last_wake_at(&conn, fleet_id),
                format_utc(now + Duration::seconds(605)).as_str()
            );
        }

        #[test]
        fn a_fleet_with_no_ordinary_members_still_wakes_the_monitor() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
            let monitor_id = bootstrap_monitor(&conn, fleet_id);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1"]);

            let due_at = now + Duration::seconds(600);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), due_at);
            assert!(matches!(result, TickResult::Continue));

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "the tick fires even with no other members");
            let (pane, _, members, director) = &wakes[0];
            assert_eq!(pane, "%1");
            assert!(members.is_empty(), "the N == 0 roster is empty");
            assert_eq!(
                director["member_id"], director_id,
                "the Director segment rides even the N == 0 wake"
            );
            drop(wakes);

            let iso = format_utc(due_at);
            assert_eq!(
                echo,
                format!("{iso} tick -> wake monitor {monitor_id} (0 members)\n")
            );
            assert_eq!(last_wake_at(&conn, fleet_id), iso.as_str());
        }

        #[test]
        fn a_forced_wake_fires_when_the_interval_is_zero() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim_with_interval(&mut conn, fleet_id, own_pid(), 0, now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
            request_wake(&mut conn, fleet_id, now + Duration::seconds(1));

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(4),
            );
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(
                mux.wake_count(),
                1,
                "an explicit operator request bypasses the disabled schedule"
            );
            assert!(!echo.is_empty());
        }

        #[test]
        fn a_forced_wake_fires_before_the_schedule_is_due_and_resets_the_baseline() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);

            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(95),
            );
            assert_eq!(
                mux.wake_count(),
                0,
                "no request → the not-yet-due gate holds"
            );
            assert!(echo.is_empty());

            request_wake(&mut conn, fleet_id, now + Duration::seconds(99));
            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(100),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "100 < 600 since started_at, but the request bypasses the wake_due gate"
            );

            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(699),
            );
            assert_eq!(
                mux.wake_count(),
                1,
                "599 < 600 since the forced wake → the baseline reset gates the schedule"
            );
            assert!(echo.is_empty());

            let (_, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(700),
            );
            assert_eq!(mux.wake_count(), 2, "600 since the forced wake → due again");
            assert!(
                !echo.contains("forced"),
                "a scheduled wake keeps the unchanged echo line: {echo}"
            );
        }

        #[test]
        fn a_skipped_forced_wake_leaves_the_request_pending_and_retries() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let requested_at = now + Duration::seconds(1);
            request_wake(&mut conn, fleet_id, requested_at);

            let dead_pane_mux = FakeMux::with_live_panes(&["%0", "%2", "%4"]);
            let (result, echo) = tick(
                &mut conn,
                &dead_pane_mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(5),
            );
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(
                dead_pane_mux.wake_count(),
                0,
                "no live monitor pane → no wake"
            );
            assert!(echo.is_empty());
            assert_eq!(
                wake_requested_at(&conn, fleet_id),
                format_utc(requested_at).as_str(),
                "a skipped wake never consumes the request"
            );
            assert_eq!(last_wake_at(&conn, fleet_id), Value::Null);

            let live_pane_mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
            let (_, _) = tick(
                &mut conn,
                &live_pane_mux,
                fleet_id,
                own_pid(),
                now + Duration::seconds(10),
            );
            assert_eq!(
                live_pane_mux.wake_count(),
                1,
                "the pending request retries on the next tick"
            );
        }

        #[test]
        fn a_delivered_forced_wake_clears_the_request_stamps_the_ledger_and_echoes() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, monitor_id, _, _) = wake_fleet(&mut conn);
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
            request_wake(&mut conn, fleet_id, now + Duration::seconds(1));

            let wake_at = now + Duration::seconds(5);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), wake_at);
            assert!(matches!(result, TickResult::Continue));
            assert_eq!(mux.wake_count(), 1);

            let iso = format_utc(wake_at);
            assert_eq!(
                echo,
                format!("{iso} tick -> forced wake monitor {monitor_id} (2 members)\n")
            );
            assert_eq!(last_wake_at(&conn, fleet_id), iso.as_str());
            assert_eq!(
                wake_requested_at(&conn, fleet_id),
                Value::Null,
                "the delivered wake consumes the request"
            );
        }
    }

    mod run_monitor_loop_tests {
        use super::*;

        #[test]
        fn a_live_slot_refuses_a_second_loop() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, _, _, _, _) = wake_fleet(&mut conn);
            claim(&mut conn, fleet_id, own_pid(), Utc::now());

            let mux = FakeMux::with_live_panes(&["%0", "%1", "%2", "%4"]);
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
