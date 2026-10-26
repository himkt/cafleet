use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::*;
use crate::error::CafleetError;

struct Host {
    now: Instant,
    calls: Vec<(&'static str, Instant)>,
    deadlines: Vec<(&'static str, Instant)>,
    fail: BTreeSet<&'static str>,
    native: Option<ChildExit>,
    dropped: u64,
    pending: u64,
    gap: u64,
}

impl Host {
    fn new(now: Instant) -> Self {
        Self {
            now,
            calls: Vec::new(),
            deadlines: Vec::new(),
            fail: BTreeSet::new(),
            native: Some(ChildExit::Code(0)),
            dropped: 0,
            pending: 0,
            gap: 0,
        }
    }
    fn call(&mut self, name: &'static str) -> Result<(), CafleetError> {
        self.calls.push((name, self.now));
        if self.fail.contains(name) {
            Err(CafleetError::App(format!("fixture {name} failure")))
        } else {
            Ok(())
        }
    }
}

impl ShutdownHost for Host {
    fn now(&self) -> Instant {
        self.now
    }
    fn gate_programmatic_input(&mut self) {
        self.calls.push(("gate", self.now));
    }
    fn term_verified_descendants(&mut self) -> Result<(), CafleetError> {
        self.call("term")
    }
    fn drain_and_reap_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<ChildExit>, CafleetError> {
        self.deadlines.push(("drain", deadline));
        self.now = self.now.max(deadline);
        self.call("drain")?;
        Ok(self.native)
    }
    fn kill_verified_survivors(&mut self) -> Result<(), CafleetError> {
        self.call("kill")
    }
    fn close_pty(&mut self) -> Result<(), CafleetError> {
        self.call("close")
    }
    fn flush_display_until(&mut self, deadline: Instant) -> Result<u64, CafleetError> {
        self.deadlines.push(("display", deadline));
        self.now = self.now.max(deadline);
        self.call("display")?;
        Ok(self.dropped)
    }
    fn finish_events_until(&mut self, deadline: Instant) -> Result<(u64, u64), CafleetError> {
        self.deadlines.push(("events", deadline));
        self.now = self.now.max(deadline);
        self.call("events")?;
        Ok((self.pending, self.gap))
    }
    fn restore_terminal(&mut self) -> Result<(), CafleetError> {
        self.call("restore")
    }
}

#[test]
fn runner_shutdown_uses_one_clock_for_descendants_display_events_and_restoration() {
    let start = Instant::now();
    let mut host = Host::new(start);
    host.native = None;
    let outcome = shutdown(
        &mut host,
        ShutdownClock::new(start),
        Some(ChildExit::Code(7)),
    );
    assert_eq!(
        host.calls.iter().map(|(call, _)| *call).collect::<Vec<_>>(),
        [
            "gate", "term", "drain", "kill", "close", "display", "events", "restore"
        ]
    );
    assert_eq!(
        host.deadlines,
        [
            ("drain", start + Duration::from_secs(2)),
            ("display", start + Duration::from_secs(3)),
            ("events", start + Duration::from_secs(4))
        ]
    );
    for (name, time) in &host.calls {
        if ["kill", "close"].contains(name) {
            assert_eq!(*time, start + Duration::from_secs(2));
        }
    }
    assert_eq!(outcome.native_exit, Some(ChildExit::Code(7)));
    assert_eq!(outcome.cleanup_state, CleanupState::Complete);
}

#[test]
fn runner_shutdown_collects_each_failure_and_still_restores_terminal_without_an_extra_budget() {
    let start = Instant::now();
    let mut host = Host::new(start);
    host.native = None;
    host.fail = [
        "term", "drain", "kill", "close", "display", "events", "restore",
    ]
    .into();
    let outcome = shutdown(
        &mut host,
        ShutdownClock::new(start),
        Some(ChildExit::Code(9)),
    );
    assert_eq!(host.calls.last().unwrap().0, "restore");
    assert_eq!(host.now, start + Duration::from_secs(4));
    assert_eq!(outcome.native_exit, Some(ChildExit::Code(9)));
    assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
    for failure in &host.fail {
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|line| line.contains(failure)),
            "{failure}"
        );
    }
}

#[test]
fn runner_shutdown_retains_display_truncation_pending_frames_and_unsaved_gaps_separately() {
    let start = Instant::now();
    let mut host = Host::new(start);
    host.dropped = 17;
    host.pending = 2;
    host.gap = 3;
    let outcome = shutdown(&mut host, ShutdownClock::new(start), None);
    assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
    assert_eq!(outcome.dropped_display_bytes, 17);
    assert_eq!(outcome.pending_event_frames, 2);
    assert_eq!(outcome.unsaved_observation_gap, 3);
    assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
    assert!(!outcome.diagnostics.is_empty());
}

#[test]
fn runner_shutdown_never_rebases_a_clock_that_was_started_before_the_current_phase() {
    let start = Instant::now();
    let mut host = Host::new(start + Duration::from_secs(3));
    host.native = None;
    let outcome = shutdown(
        &mut host,
        ShutdownClock::new(start),
        Some(ChildExit::Signal(15)),
    );
    assert_eq!(host.deadlines[0].1, start + Duration::from_secs(2));
    assert_eq!(host.deadlines[2].1, start + Duration::from_secs(4));
    assert_eq!(host.now, start + Duration::from_secs(4));
    assert_eq!(outcome.native_exit, Some(ChildExit::Signal(15)));
}
