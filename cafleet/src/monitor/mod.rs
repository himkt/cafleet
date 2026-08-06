//! Monitor heartbeat loop (SPEC §6.6) — the pure `wake_due` check, the
//! per-tick scan `monitor_tick` (ownership-checked heartbeat, fleet liveness,
//! the fleet-level Director wake, the `woke`-gated ledger write), and the
//! foreground driver. The colocated tests pin the contract.
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
//!         members: &[Value]) -> Result<bool, MultiplexerError>;
//! }
//!
//! pub enum TickResult { Continue, Stop }
//!
//! // Pure due-check for the fleet-level wake: a NULL or unparsable stamp is
//! // immediately due.
//! pub fn wake_due(last_wake_at: Option<&str>, wake_interval: u64,
//!     now: DateTime<Utc>) -> bool;
//!
//! pub fn monitor_tick(conn: &mut Connection, mux: &dyn MonitorMux,
//!     out: &mut dyn std::io::Write, fleet_id: i64, pid: i64,
//!     wake_interval: u64, now: DateTime<Utc>)
//!     -> Result<TickResult, CafleetError>;
//!
//! pub fn run_monitor_loop(conn: &mut Connection, mux: &dyn MonitorMux,
//!     out: &mut dyn std::io::Write, fleet_id: i64, tick_seconds: i64,
//!     wake_interval: u64) -> Result<(), CafleetError>;
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
/// single Director-wake keystroke.
pub trait MonitorMux {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[Value],
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
    ) -> Result<bool, MultiplexerError> {
        Multiplexer::send_wake_trigger(self, target_pane_id, fleet_id, members)
    }
}

pub enum TickResult {
    Continue,
    Stop,
}

fn mux_err(error: MultiplexerError) -> CafleetError {
    CafleetError::App(error.to_string())
}

/// Pure due-check for the fleet-level wake: a `NULL` or unparsable
/// `last_wake_at` is immediately due; otherwise due once the interval has
/// elapsed.
pub fn wake_due(last_wake_at: Option<&str>, wake_interval: u64, now: DateTime<Utc>) -> bool {
    let Some(last_wake_at) = last_wake_at else {
        return true;
    };
    let Ok(parsed) = parse_lenient(last_wake_at) else {
        return true;
    };
    (now - parsed).num_seconds() >= wake_interval as i64
}

/// One scan pass (SPEC §6.6): ownership-checked heartbeat → fleet liveness →
/// wake-interval gate → Director-pane resolution → one fleet-level wake →
/// the `woke`-gated ledger write and heartbeat echo.
pub fn monitor_tick(
    conn: &mut Connection,
    mux: &dyn MonitorMux,
    out: &mut dyn Write,
    fleet_id: i64,
    pid: i64,
    wake_interval: u64,
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

    if wake_interval == 0 {
        return Ok(TickResult::Continue);
    }
    let runtime = broker::read_monitor_runtime(conn, fleet_id)?
        .expect("the heartbeat just matched this fleet's runtime row");
    if !wake_due(runtime["last_wake_at"].as_str(), wake_interval, now) {
        return Ok(TickResult::Continue);
    }

    // A Director with no pane, or a pane absent from the live set, skips the
    // wake without stamping — the fleet stays due for the next tick.
    let director_id = fleet["director_member_id"]
        .as_i64()
        .expect("a live fleet records its Director");
    let director = broker::get_member(conn, director_id, fleet_id)?;
    let director_pane = director
        .as_ref()
        .and_then(|member| member["placement"]["mux_pane_id"].as_str());
    let Some(director_pane) = director_pane else {
        return Ok(TickResult::Continue);
    };
    let live_panes = mux.list_pane_ids().map_err(mux_err)?;
    if !live_panes.contains(director_pane) {
        return Ok(TickResult::Continue);
    }

    let roster = broker::list_fleet_wake_targets(conn, fleet_id)?;
    let woke = mux
        .send_wake_trigger(director_pane, fleet_id, &roster)
        .map_err(mux_err)?;
    if woke {
        broker::record_monitor_wake(conn, fleet_id, &iso)?;
        writeln!(
            out,
            "{iso} tick -> wake director {director_id} ({} members)",
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
    wake_interval: u64,
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

    let outcome = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        match monitor_tick(conn, mux, out, fleet_id, pid, wake_interval, now_utc()) {
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
            self.wakes
                .borrow_mut()
                .push((target_pane_id.to_string(), fleet_id, members.to_vec()));
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

            let due_at = now + Duration::seconds(600);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, due_at);
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

            let iso = format_utc(due_at);
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

            let (_, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, now);
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
                600,
                now + Duration::seconds(599),
            );
            assert_eq!(mux.wake_count(), 0, "599 < 600 since started_at → not due");
            assert!(echo.is_empty());

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
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
                600,
                now + Duration::seconds(1199),
            );
            assert_eq!(mux.wake_count(), 1, "599 since the first wake → not due");
            assert!(echo.is_empty());

            let (_, _) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
                now + Duration::seconds(1200),
            );
            assert_eq!(mux.wake_count(), 2, "600 since the first wake → second wake");
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

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
                now + Duration::seconds(600),
            );
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

            let (result, echo) = tick(
                &mut conn,
                &mux,
                fleet_id,
                own_pid(),
                600,
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
                600,
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
        fn a_fleet_with_no_members_still_wakes_the_director() {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
            let now = base_now();
            claim(&mut conn, fleet_id, own_pid(), now);
            let mux = FakeMux::with_live_panes(&["%0"]);

            let due_at = now + Duration::seconds(600);
            let (result, echo) = tick(&mut conn, &mux, fleet_id, own_pid(), 600, due_at);
            assert!(matches!(result, TickResult::Continue));

            let wakes = mux.wakes.borrow();
            assert_eq!(wakes.len(), 1, "the tick fires even with no other members");
            let (pane, _, members) = &wakes[0];
            assert_eq!(pane, "%0");
            assert!(members.is_empty(), "the N == 0 roster is empty");
            drop(wakes);

            let iso = format_utc(due_at);
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
