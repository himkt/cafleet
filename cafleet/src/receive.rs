use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::broker;
use crate::config::Settings;

use crate::broker::records::MessageRecord;
use crate::error::CafleetError;
use crate::input::Deadline;

pub(crate) trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingPreview {
    pub member_id: i64,
    pub message_id: i64,
    pub started: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveSnapshot {
    pub messages: Vec<MessageRecord>,
    pub pending: Vec<PendingPreview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    NotSent,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotificationError {
    pub member_id: i64,
    pub message_id: i64,
    pub delivery: Delivery,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttemptOutcome {
    Held,
    NoPending,
    Settled { message_id: i64 },
    Sent { message_id: i64 },
    Failed(NotificationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitOutcome {
    pub messages: Vec<MessageRecord>,
    pub notification_errors: Vec<NotificationError>,
}

pub(crate) trait ReceiveIo {
    fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError>;
    fn attempt(
        &mut self,
        member_id: i64,
        deadline: Deadline,
    ) -> Result<AttemptOutcome, CafleetError>;
}

fn preserve_uncertainty(
    cause: CafleetError,
    errors: &BTreeMap<i64, NotificationError>,
) -> CafleetError {
    let diagnostics: Vec<_> = errors
        .values()
        .filter(|error| error.delivery == Delivery::Uncertain)
        .map(|error| {
            format!(
                "preview {} delivery to member {} is uncertain: {}",
                error.message_id, error.member_id, error.error
            )
        })
        .collect();
    if diagnostics.is_empty() {
        cause
    } else {
        CafleetError::WithDiagnostics {
            cause: Box::new(cause),
            diagnostics,
        }
    }
}

pub(crate) fn wait<C: Clock, I: ReceiveIo>(
    clock: &C,
    io: &mut I,
    timeout: Duration,
) -> Result<WaitOutcome, CafleetError> {
    let started = clock.now();
    let expires = started + timeout;
    let mut next_check = started;
    let mut last_interval = None;
    let mut cursor = None;
    let mut messages = Vec::new();
    let mut errors: BTreeMap<i64, NotificationError> = BTreeMap::new();
    loop {
        let now = clock.now();
        if now >= expires {
            break;
        }
        if now < next_check {
            clock.sleep(next_check.min(expires).duration_since(now));
            continue;
        }
        let snapshot = match io.check(Deadline::until(expires.min(now + Duration::from_secs(5)))) {
            Ok(snapshot) => snapshot,
            Err(CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded) => {
                let now = clock.now();
                if now >= expires {
                    break;
                }
                next_check =
                    started + Duration::from_secs(now.duration_since(started).as_secs() + 1);
                continue;
            }
            Err(error) => return Err(preserve_uncertainty(error, &errors)),
        };
        errors.retain(|member, error| {
            snapshot.pending.iter().any(|preview| {
                preview.member_id == *member && preview.message_id == error.message_id
            })
        });
        for preview in snapshot.pending.iter().filter(|preview| preview.started) {
            errors.entry(preview.member_id).or_insert_with(|| NotificationError {
                member_id: preview.member_id,
                message_id: preview.message_id,
                delivery: Delivery::Uncertain,
                error: format!("preview {} delivery to member {} is uncertain: an earlier input attempt has no confirmed notification stamp. Inspect the pane before retrying.", preview.message_id, preview.member_id),
            });
        }
        messages = snapshot.messages;
        let now = clock.now();
        if !messages.is_empty() || now >= expires {
            break;
        }
        let interval = now.duration_since(started).as_secs();
        let selected = if last_interval == Some(interval) {
            None
        } else {
            let eligible = snapshot.pending.iter().filter(|preview| {
                !preview.started
                    && !errors.get(&preview.member_id).is_some_and(|error| {
                        error.delivery == Delivery::Uncertain
                            && error.message_id == preview.message_id
                    })
            });
            eligible
                .clone()
                .filter(|preview| cursor.is_none_or(|cursor| preview.member_id > cursor))
                .min_by_key(|preview| preview.member_id)
                .or_else(|| eligible.min_by_key(|preview| preview.member_id))
        };
        if let Some(preview) = selected {
            let member = preview.member_id;
            cursor = Some(member);
            last_interval = Some(interval);
            let outcome = match io.attempt(member, Deadline::until(now + Duration::from_secs(5))) {
                Ok(outcome) => outcome,
                Err(error @ (CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded)) => {
                    AttemptOutcome::Failed(NotificationError {
                        member_id: member,
                        message_id: preview.message_id,
                        delivery: Delivery::NotSent,
                        error: error.to_string(),
                    })
                }
                Err(error) => return Err(preserve_uncertainty(error, &errors)),
            };
            match outcome {
                AttemptOutcome::Held => {}
                AttemptOutcome::NoPending => {
                    if errors
                        .get(&member)
                        .is_some_and(|error| error.message_id == preview.message_id)
                    {
                        errors.remove(&member);
                    }
                }
                AttemptOutcome::Settled { message_id } | AttemptOutcome::Sent { message_id } => {
                    if errors
                        .get(&member)
                        .is_some_and(|error| error.message_id == message_id)
                    {
                        errors.remove(&member);
                    }
                }
                AttemptOutcome::Failed(error) => {
                    assert_eq!(
                        error.member_id, member,
                        "the owned attempt reports its selected member"
                    );
                    errors.insert(member, error);
                }
            }
            next_check = clock.now();
        } else {
            next_check = started + Duration::from_secs(interval + 1);
        }
    }
    Ok(WaitOutcome {
        messages,
        notification_errors: errors.into_values().collect(),
    })
}

pub(crate) struct BrokerReceiver<'a> {
    conn: &'a mut Connection,
    settings: &'a Settings,
    member_id: i64,
    fleet_id: Option<i64>,
}

impl<'a> BrokerReceiver<'a> {
    pub(crate) fn new(conn: &'a mut Connection, settings: &'a Settings, member_id: i64) -> Self {
        Self {
            conn,
            settings,
            member_id,
            fleet_id: None,
        }
    }
}

fn subject_fleet(
    conn: &Connection,
    member_id: i64,
    expected: Option<i64>,
) -> Result<i64, CafleetError> {
    let fleet = broker::active_member_fleet(conn, member_id)?
        .ok_or_else(|| CafleetError::Value(format!("Member {member_id} not found")))?;
    if expected.is_some_and(|expected| expected != fleet) {
        return Err(CafleetError::App(format!(
            "Member {member_id} changed fleets during receive"
        )));
    }
    broker::fleets::require_live_fleet(conn, fleet)?;
    Ok(fleet)
}

impl ReceiveIo for BrokerReceiver<'_> {
    fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError> {
        let (snapshot, fleet) = crate::input::with_database_budget(self.conn, deadline, |conn| {
            let tx = conn.transaction().map_err(broker::members::db_err)?;
            let fleet = subject_fleet(&tx, self.member_id, self.fleet_id)?;
            let messages = broker::poll_messages(&tx, self.member_id)?;
            let pending = broker::panes::pending_previews(&tx, fleet, self.member_id)?
                .into_iter()
                .map(|(member_id, message_id, started)| PendingPreview {
                    member_id,
                    message_id,
                    started,
                })
                .collect();
            tx.commit().map_err(broker::members::db_err)?;
            Ok((ReceiveSnapshot { messages, pending }, fleet))
        })?;
        self.fleet_id = Some(fleet);
        Ok(snapshot)
    }

    fn attempt(
        &mut self,
        member_id: i64,
        deadline: Deadline,
    ) -> Result<AttemptOutcome, CafleetError> {
        let fleet = crate::input::with_database_budget(self.conn, deadline, |conn| {
            subject_fleet(conn, self.member_id, self.fleet_id)
        })?;
        crate::runtime::attempt_preview(self.conn, self.settings, member_id, Some(fleet), deadline)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::broker::records::MessageStatus;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        Check {
            at: Duration,
            budget: Duration,
        },
        Attempt {
            at: Duration,
            member_id: i64,
            budget: Duration,
        },
        Sleep {
            at: Duration,
            duration: Duration,
        },
    }

    struct TestClock {
        origin: Instant,
        current: Cell<Instant>,
        expiry: Cell<Instant>,
        oversleep: Cell<Duration>,
        reads: Cell<usize>,
        events: RefCell<Vec<Event>>,
    }

    impl TestClock {
        fn new() -> Self {
            let origin = Instant::now() + Duration::from_secs(10_000);
            Self {
                origin,
                current: Cell::new(origin),
                expiry: Cell::new(origin),
                oversleep: Cell::new(Duration::ZERO),
                reads: Cell::new(0),
                events: RefCell::new(Vec::new()),
            }
        }

        fn elapsed(&self) -> Duration {
            self.current.get().duration_since(self.origin)
        }

        fn advance(&self, duration: Duration) {
            self.current.set(self.current.get() + duration);
        }

        fn record(&self, event: Event) {
            let mut events = self.events.borrow_mut();
            assert!(
                events.len() < 8192,
                "receive exceeded its bounded event script"
            );
            events.push(event);
        }

        fn run(
            &self,
            io: &mut impl ReceiveIo,
            timeout_ms: u64,
        ) -> Result<WaitOutcome, CafleetError> {
            let timeout = Duration::from_millis(timeout_ms);
            self.expiry.set(self.current.get() + timeout);
            wait(self, io, timeout)
        }

        fn operation_budget(&self, deadline: Deadline, attempt: bool) -> Duration {
            let now = self.current.get();
            assert!(
                now < self.expiry.get(),
                "new I/O started after receive expiry"
            );
            let budget = deadline
                .remaining_at(now)
                .expect("new I/O has a live absolute deadline in the injected clock domain");
            let expected = if attempt {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(5).min(self.expiry.get().duration_since(now))
            };
            assert_eq!(
                budget, expected,
                "I/O must retain its original absolute budget"
            );
            budget
        }

        fn attempts(&self) -> Vec<(Duration, i64)> {
            self.events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Event::Attempt { at, member_id, .. } => Some((*at, *member_id)),
                    _ => None,
                })
                .collect()
        }

        fn checks(&self) -> Vec<(Duration, Duration)> {
            self.events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Event::Check { at, budget } => Some((*at, *budget)),
                    _ => None,
                })
                .collect()
        }

        fn sleeps(&self) -> Vec<(Duration, Duration)> {
            self.events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Event::Sleep { at, duration } => Some((*at, *duration)),
                    _ => None,
                })
                .collect()
        }

        fn assert_retry_cadence(&self) {
            let mut previous_interval = None;
            let mut checked = false;
            for event in self.events.borrow().iter() {
                match event {
                    Event::Check { .. } => checked = true,
                    Event::Attempt { at, .. } => {
                        assert!(
                            checked,
                            "check the inbox before another notification attempt"
                        );
                        if let Some(previous) = previous_interval {
                            assert!(
                                at.as_secs() > previous,
                                "retry catch-up burst: {:?}",
                                self.events.borrow()
                            );
                        }
                        previous_interval = Some(at.as_secs());
                        checked = false;
                    }
                    Event::Sleep { .. } => {}
                }
            }
        }

        fn assert_last_io_was_attempt(&self) {
            assert!(
                matches!(self.events.borrow().last(), Some(Event::Attempt { .. })),
                "{:?}",
                self.events.borrow()
            );
            assert!(self.current.get() >= self.expiry.get());
            assert!(self.current.get() <= self.expiry.get() + Duration::from_secs(5));
            self.assert_retry_cadence();
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> Instant {
            let reads = self.reads.get() + 1;
            assert!(
                reads < 65_536,
                "receive spun without advancing the injected clock"
            );
            self.reads.set(reads);
            self.current.get()
        }

        fn sleep(&self, duration: Duration) {
            let now = self.current.get();
            assert!(
                now < self.expiry.get(),
                "sleep started after receive expiry"
            );
            assert!(
                !duration.is_zero(),
                "sleep advances the monotonic receive clock"
            );
            assert!(
                duration <= self.expiry.get().duration_since(now),
                "sleep exceeds remaining receive time"
            );
            self.record(Event::Sleep {
                at: self.elapsed(),
                duration,
            });
            self.advance(duration + self.oversleep.get());
        }
    }

    struct Step<T> {
        duration: Duration,
        result: Result<T, CafleetError>,
    }

    impl<T> Step<T> {
        fn done(value: T) -> Self {
            Self::after(0, value)
        }

        fn after(milliseconds: u64, value: T) -> Self {
            Self {
                duration: Duration::from_millis(milliseconds),
                result: Ok(value),
            }
        }

        fn failed(error: CafleetError) -> Self {
            Self {
                duration: Duration::ZERO,
                result: Err(error),
            }
        }

        fn finish(self, clock: &TestClock, budget: Duration) -> Result<T, CafleetError> {
            assert!(
                self.duration <= budget,
                "scripted I/O fits its owned operation budget"
            );
            clock.advance(self.duration);
            self.result
        }
    }

    struct ScriptedIo<'a, C, A> {
        clock: &'a TestClock,
        check: C,
        attempt: A,
        checks: usize,
        attempts: usize,
    }

    impl<'a, C, A> ScriptedIo<'a, C, A> {
        fn new(clock: &'a TestClock, check: C, attempt: A) -> Self {
            Self {
                clock,
                check,
                attempt,
                checks: 0,
                attempts: 0,
            }
        }
    }

    impl<C, A> ReceiveIo for ScriptedIo<'_, C, A>
    where
        C: FnMut(usize, usize, Duration) -> Step<ReceiveSnapshot>,
        A: FnMut(usize, i64, Duration) -> Step<AttemptOutcome>,
    {
        fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError> {
            let budget = self.clock.operation_budget(deadline, false);
            let at = self.clock.elapsed();
            self.clock.record(Event::Check { at, budget });
            self.checks += 1;
            (self.check)(self.checks, self.attempts, at).finish(self.clock, budget)
        }

        fn attempt(
            &mut self,
            member_id: i64,
            deadline: Deadline,
        ) -> Result<AttemptOutcome, CafleetError> {
            let budget = self.clock.operation_budget(deadline, true);
            let at = self.clock.elapsed();
            self.clock.record(Event::Attempt {
                at,
                member_id,
                budget,
            });
            self.attempts += 1;
            (self.attempt)(self.attempts, member_id, at).finish(self.clock, budget)
        }
    }

    fn snapshot(pending: &[(i64, i64)]) -> ReceiveSnapshot {
        ReceiveSnapshot {
            messages: Vec::new(),
            pending: pending
                .iter()
                .map(|&(member_id, message_id)| PendingPreview {
                    member_id,
                    message_id,
                    started: false,
                })
                .collect(),
        }
    }

    fn messages() -> Vec<MessageRecord> {
        vec![
            MessageRecord {
                message_id: 91,
                from_member_id: 22,
                to_member_id: 1,
                created_at: "2026-01-01T00:00:00+00:00".into(),
                status: MessageStatus::Completed,
                status_timestamp: "2026-02-01T00:00:00+00:00".into(),
                text: "second reply\nwith full text 界".into(),
            },
            MessageRecord {
                message_id: 90,
                from_member_id: 11,
                to_member_id: 1,
                created_at: "2026-01-02T00:00:00+00:00".into(),
                status: MessageStatus::InputRequired,
                status_timestamp: "2026-01-31T00:00:00+00:00".into(),
                text: " question requiring a deferred decision ".into(),
            },
        ]
    }

    fn incoming(pending: &[(i64, i64)]) -> ReceiveSnapshot {
        let mut snapshot = snapshot(pending);
        snapshot.messages = messages();
        snapshot
    }

    fn diagnostic(
        member_id: i64,
        message_id: i64,
        delivery: Delivery,
        error: &str,
    ) -> NotificationError {
        NotificationError {
            member_id,
            message_id,
            delivery,
            error: error.into(),
        }
    }

    fn held(_: usize, _: i64, _: Duration) -> Step<AttemptOutcome> {
        Step::done(AttemptOutcome::Held)
    }

    fn unexpected_attempt(_: usize, member_id: i64, _: Duration) -> Step<AttemptOutcome> {
        panic!("unexpected notification attempt for member {member_id}");
    }

    #[test]
    fn a_durable_claim_suspends_only_its_exact_preview() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                let mut state = if attempts == 0 {
                    snapshot(&[(11, 101), (11, 102)])
                } else {
                    snapshot(&[(11, 101)])
                };
                state.pending[0].started = true;
                Step::done(state)
            },
            |attempt, member, _| {
                assert_eq!((attempt, member), (1, 11));
                Step::done(AttemptOutcome::Sent { message_id: 102 })
            },
        );
        let outcome = clock.run(&mut io, 2500).unwrap();
        assert_eq!(clock.attempts().len(), 1);
        assert_eq!(outcome.notification_errors.len(), 1);
        assert_eq!(outcome.notification_errors[0].message_id, 101);
        assert_eq!(outcome.notification_errors[0].delivery, Delivery::Uncertain);
    }

    #[test]
    fn an_obsolete_later_candidate_preserves_the_earlier_claim_diagnostic() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| {
                let mut state = snapshot(&[(11, 101), (11, 102)]);
                state.pending[0].started = true;
                Step::done(state)
            },
            |_, _, _| Step::after(2000, AttemptOutcome::NoPending),
        );
        let outcome = clock.run(&mut io, 1500).unwrap();
        assert_eq!(outcome.notification_errors.len(), 1);
        assert_eq!(outcome.notification_errors[0].message_id, 101);
        assert_eq!(outcome.notification_errors[0].delivery, Delivery::Uncertain);
    }

    #[test]
    fn a_prewrite_database_error_names_the_unclaimed_candidate() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| {
                let mut state = snapshot(&[(11, 101), (11, 102)]);
                state.pending[0].started = true;
                Step::done(state)
            },
            |_, member, _| {
                assert_eq!(member, 11);
                Step::failed(CafleetError::DatabaseBusy("busy".into()))
            },
        );
        let outcome = clock.run(&mut io, 1500).unwrap();
        assert_eq!(outcome.notification_errors.len(), 1);
        assert_eq!(outcome.notification_errors[0].message_id, 102);
        assert_eq!(outcome.notification_errors[0].delivery, Delivery::NotSent);
    }

    #[test]
    fn uncertain_delivery_is_retained_without_retrying_that_member_during_the_same_wait() {
        let clock = TestClock::new();
        let error = diagnostic(
            11,
            101,
            Delivery::Uncertain,
            "Enter result is unknown; inspect the pane",
        );
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101), (22, 202)])),
            |attempt, member, _| match member {
                11 => {
                    assert_eq!(attempt, 1, "the uncertain member is attempted once");
                    Step::done(AttemptOutcome::Failed(error.clone()))
                }
                22 => Step::done(AttemptOutcome::Held),
                _ => panic!("unexpected member"),
            },
        );
        let outcome = clock.run(&mut io, 4500).unwrap();
        assert_eq!(outcome.notification_errors, vec![error]);
        assert_eq!(
            clock
                .attempts()
                .iter()
                .filter(|(_, member)| *member == 11)
                .count(),
            1
        );
        assert!(clock.attempts().iter().any(|(_, member)| *member == 22));
    }

    #[test]
    fn a_fatal_snapshot_keeps_uncertainty_and_the_original_typed_error() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |check, _, _| {
                if check == 1 {
                    Step::done(snapshot(&[(11, 101)]))
                } else {
                    Step::failed(CafleetError::InvalidStoredValue {
                        field: "members.status".into(),
                        value: "broken".into(),
                    })
                }
            },
            |attempt, _, _| {
                assert_eq!(attempt, 1);
                Step::done(AttemptOutcome::Failed(diagnostic(
                    11,
                    101,
                    Delivery::Uncertain,
                    "Enter result is unknown",
                )))
            },
        );
        let error = clock.run(&mut io, 5000).unwrap_err();
        assert_eq!(error.exit_code(), 1);
        let message = error.message();
        assert!(message.contains("members.status") && message.contains("broken"));
        assert!(message.contains("preview 101 delivery to member 11 is uncertain"));
        assert!(matches!(error, CafleetError::WithDiagnostics { cause, .. }
            if matches!(*cause, CafleetError::InvalidStoredValue { .. })));
    }

    #[test]
    fn snapshot_contention_at_receive_expiry_returns_timeout_with_prior_diagnostics() {
        for failure in [
            CafleetError::DatabaseBusy("database is locked".into()),
            CafleetError::DeadlineExceeded,
        ] {
            let clock = TestClock::new();
            let error = diagnostic(
                11,
                101,
                Delivery::Uncertain,
                "stamp failed; inspect the pane",
            );
            let mut io = ScriptedIo::new(
                &clock,
                |check, _, _| {
                    if check == 1 {
                        Step::done(snapshot(&[(11, 101)]))
                    } else {
                        Step {
                            duration: Duration::from_millis(2500),
                            result: Err(failure.clone()),
                        }
                    }
                },
                |attempt, _, _| {
                    assert_eq!(attempt, 1);
                    Step::done(AttemptOutcome::Failed(error.clone()))
                },
            );
            let outcome = clock.run(&mut io, 2500).unwrap();
            assert!(outcome.messages.is_empty());
            assert_eq!(outcome.notification_errors, vec![error.clone()]);
            assert_eq!(clock.elapsed(), Duration::from_millis(2500));
            assert_eq!(io.checks, 2);
        }
    }

    #[test]
    fn transient_snapshot_contention_can_recover_before_receive_expiry() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |check, _, _| {
                if check == 1 {
                    Step {
                        duration: Duration::from_secs(5),
                        result: Err(CafleetError::DatabaseBusy("database is locked".into())),
                    }
                } else {
                    Step::done(incoming(&[]))
                }
            },
            unexpected_attempt,
        );
        let outcome = clock.run(&mut io, 6500).unwrap();
        assert_eq!(outcome.messages, messages());
        assert_eq!(clock.elapsed(), Duration::from_secs(6));
    }

    #[test]
    fn pre_write_database_contention_retains_other_diagnostics_at_receive_expiry() {
        let clock = TestClock::new();
        let first = diagnostic(11, 101, Delivery::Uncertain, "Enter result is unknown");
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101), (22, 202)])),
            |attempt, _, _| {
                if attempt == 1 {
                    Step::done(AttemptOutcome::Failed(first.clone()))
                } else {
                    Step {
                        duration: Duration::from_secs(5),
                        result: Err(CafleetError::DatabaseBusy("database is locked".into())),
                    }
                }
            },
        );
        let outcome = clock.run(&mut io, 2500).unwrap();
        assert_eq!(
            outcome.notification_errors,
            vec![
                first,
                diagnostic(22, 202, Delivery::NotSent, "database is locked")
            ]
        );
        clock.assert_last_io_was_attempt();
    }

    #[test]
    fn existing_messages_return_full_ordered_records_without_retry_or_sleep() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(incoming(&[(11, 101)])),
            unexpected_attempt,
        );
        let outcome = clock.run(&mut io, 300_000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert!(outcome.notification_errors.is_empty());
        assert_eq!(io.checks, 1);
        assert_eq!(clock.elapsed(), Duration::ZERO);
        assert!(clock.sleeps().is_empty());
    }

    #[test]
    fn empty_receive_checks_each_second_and_caps_the_final_sleep() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[])),
            unexpected_attempt,
        );
        let outcome = clock.run(&mut io, 3500).unwrap();
        assert_eq!(
            outcome,
            WaitOutcome {
                messages: vec![],
                notification_errors: vec![]
            }
        );
        assert_eq!(clock.elapsed(), Duration::from_millis(3500));
        assert_eq!(
            clock.checks(),
            vec![
                (Duration::ZERO, Duration::from_millis(3500)),
                (Duration::from_secs(1), Duration::from_millis(2500)),
                (Duration::from_secs(2), Duration::from_millis(1500)),
                (Duration::from_secs(3), Duration::from_millis(500)),
            ]
        );
        assert_eq!(
            clock.sleeps(),
            vec![
                (Duration::ZERO, Duration::from_secs(1)),
                (Duration::from_secs(1), Duration::from_secs(1)),
                (Duration::from_secs(2), Duration::from_secs(1)),
                (Duration::from_secs(3), Duration::from_millis(500)),
            ]
        );
    }

    #[test]
    fn default_and_maximum_receives_advance_only_the_injected_clock() {
        for seconds in [300, 1800] {
            let clock = TestClock::new();
            let mut io = ScriptedIo::new(
                &clock,
                |_, _, _| Step::done(snapshot(&[])),
                unexpected_attempt,
            );
            let outcome = clock.run(&mut io, seconds * 1000).unwrap();
            assert!(outcome.messages.is_empty());
            assert!(outcome.notification_errors.is_empty());
            assert_eq!(clock.elapsed(), Duration::from_secs(seconds));
            assert_eq!(u64::try_from(io.checks).unwrap(), seconds);
            assert_eq!(clock.checks().first().unwrap().1, Duration::from_secs(5));
            assert_eq!(clock.checks().last().unwrap().1, Duration::from_secs(1));
        }
    }

    #[test]
    fn retries_rotate_through_members_once_per_interval_with_inbox_checks() {
        let clock = TestClock::new();
        let pending = [(11, 101), (22, 202), (33, 303)];
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                Step::done(if attempts == 6 {
                    incoming(&pending)
                } else {
                    snapshot(&pending)
                })
            },
            held,
        );
        let outcome = clock.run(&mut io, 12_000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert!(outcome.notification_errors.is_empty());
        let attempts = clock.attempts();
        assert_eq!(attempts.len(), 6);
        let members: Vec<_> = attempts.iter().map(|&(_, member)| member).collect();
        for pair in members.windows(2) {
            assert_eq!(
                pair[1],
                match pair[0] {
                    11 => 22,
                    22 => 33,
                    33 => 11,
                    member => panic!("ineligible member {member}"),
                }
            );
        }
        clock.assert_retry_cadence();
    }

    #[test]
    fn a_reply_observed_after_one_attempt_returns_before_another_attempt() {
        let clock = TestClock::new();
        let pending = [(11, 101), (22, 202)];
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                Step::done(if attempts == 0 {
                    snapshot(&pending)
                } else {
                    incoming(&pending)
                })
            },
            held,
        );
        let outcome = clock.run(&mut io, 5000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert_eq!(io.attempts, 1);
        clock.assert_retry_cadence();
    }

    #[test]
    fn changed_membership_uses_the_live_set_and_preserves_cursor_progress() {
        let clock = TestClock::new();
        let first_member = Cell::new(None);
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                let pending: Vec<_> = if attempts == 0 {
                    vec![(11, 101), (22, 202), (33, 303)]
                } else {
                    [5, 11, 22, 33]
                        .into_iter()
                        .filter(|member| Some(*member) != first_member.get())
                        .map(|member| (member, member * 10))
                        .collect()
                };
                Step::done(if attempts == 4 {
                    incoming(&pending)
                } else {
                    snapshot(&pending)
                })
            },
            |attempt, member, _| {
                if attempt == 1 {
                    first_member.set(Some(member));
                }
                Step::done(AttemptOutcome::Held)
            },
        );
        let outcome = clock.run(&mut io, 10_000).unwrap();
        assert_eq!(outcome.messages, messages());
        let attempts = clock.attempts();
        assert_eq!(attempts.len(), 4);
        let first = first_member.get().unwrap();
        let live: Vec<_> = [5, 11, 22, 33]
            .into_iter()
            .filter(|&member| member != first)
            .collect();
        let mut cursor = first;
        for &(_, member) in &attempts[1..] {
            let expected = live
                .iter()
                .copied()
                .find(|&member| member > cursor)
                .unwrap_or(live[0]);
            assert_eq!(member, expected);
            cursor = member;
        }
        clock.assert_retry_cadence();
    }

    #[test]
    fn a_slow_attempt_skips_elapsed_intervals_without_a_catch_up_burst() {
        let clock = TestClock::new();
        let pending = [(11, 101), (22, 202), (33, 303)];
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                Step::done(if attempts == 3 {
                    incoming(&pending)
                } else {
                    snapshot(&pending)
                })
            },
            |attempt, _, _| Step::after(if attempt == 1 { 3250 } else { 0 }, AttemptOutcome::Held),
        );
        let outcome = clock.run(&mut io, 12_000).unwrap();
        assert_eq!(outcome.messages, messages());
        let attempts = clock.attempts();
        assert_eq!(attempts.len(), 3);
        assert!(attempts[1].0 >= attempts[0].0 + Duration::from_millis(3250));
        clock.assert_retry_cadence();
    }

    #[test]
    fn a_slow_receive_check_skips_elapsed_intervals_without_a_catch_up_burst() {
        let clock = TestClock::new();
        let pending = [(11, 101), (22, 202)];
        let delayed = Cell::new(false);
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                if attempts == 1 && !delayed.replace(true) {
                    Step::after(3250, snapshot(&pending))
                } else {
                    Step::done(if attempts == 3 {
                        incoming(&pending)
                    } else {
                        snapshot(&pending)
                    })
                }
            },
            held,
        );
        let outcome = clock.run(&mut io, 12_000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert!(delayed.get());
        let attempts = clock.attempts();
        assert_eq!(attempts.len(), 3);
        assert!(attempts[1].0 >= attempts[0].0 + Duration::from_millis(3250));
        clock.assert_retry_cadence();
    }

    #[test]
    fn receive_checks_get_the_smaller_of_five_seconds_and_remaining_receive_time() {
        for timeout in [2500, 12_000] {
            let clock = TestClock::new();
            let mut io = ScriptedIo::new(
                &clock,
                |check, _, _| Step::after(if check == 1 { 2000 } else { 0 }, snapshot(&[])),
                unexpected_attempt,
            );
            let outcome = clock.run(&mut io, timeout).unwrap();
            assert!(outcome.messages.is_empty());
            assert_eq!(clock.elapsed(), Duration::from_millis(timeout));
            let budgets: Vec<_> = clock
                .checks()
                .into_iter()
                .map(|(_, budget)| budget)
                .collect();
            assert_eq!(budgets[0], Duration::from_millis(timeout.min(5000)));
            assert!(
                budgets
                    .iter()
                    .any(|&budget| budget < Duration::from_secs(5))
            );
        }
    }

    #[test]
    fn an_attempt_near_receive_expiry_keeps_its_full_five_second_budget() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101)])),
            |attempt, member, _| {
                assert_eq!(member, 11);
                match attempt {
                    1 => Step::done(AttemptOutcome::Held),
                    2 => Step::after(5000, AttemptOutcome::Sent { message_id: 101 }),
                    _ => panic!("receive started another attempt after its deadline"),
                }
            },
        );
        let outcome = clock.run(&mut io, 2100).unwrap();
        assert!(outcome.messages.is_empty());
        assert!(outcome.notification_errors.is_empty());
        let attempts = clock.attempts();
        assert_eq!(attempts.len(), 2);
        assert!(attempts[1].0 >= Duration::from_secs(1));
        assert!(attempts[1].0 < Duration::from_millis(2100));
        assert_eq!(clock.elapsed(), attempts[1].0 + Duration::from_secs(5));
        clock.assert_last_io_was_attempt();
    }

    #[test]
    fn a_late_wake_starts_no_receive_check_or_notification_attempt() {
        let clock = TestClock::new();
        clock.oversleep.set(Duration::from_millis(1500));
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[])),
            unexpected_attempt,
        );
        let outcome = clock.run(&mut io, 1500).unwrap();
        assert!(outcome.messages.is_empty());
        assert!(outcome.notification_errors.is_empty());
        assert_eq!(io.checks, 1);
        assert_eq!(clock.sleeps().len(), 1);
        assert!(clock.elapsed() >= Duration::from_millis(1500));
    }

    #[test]
    fn a_check_completing_at_receive_expiry_returns_its_snapshot_without_more_io() {
        for has_messages in [false, true] {
            let clock = TestClock::new();
            let mut io = ScriptedIo::new(
                &clock,
                |_, _, _| {
                    Step::after(
                        1000,
                        if has_messages {
                            incoming(&[(11, 101)])
                        } else {
                            snapshot(&[(11, 101)])
                        },
                    )
                },
                unexpected_attempt,
            );
            let outcome = clock.run(&mut io, 1000).unwrap();
            assert_eq!(
                outcome.messages,
                if has_messages { messages() } else { vec![] }
            );
            assert!(outcome.notification_errors.is_empty());
            assert_eq!(io.checks, 1);
            assert!(clock.sleeps().is_empty());
            assert_eq!(clock.elapsed(), Duration::from_secs(1));
        }
    }

    #[test]
    fn a_late_held_attempt_preserves_the_last_snapshot_diagnostic_without_a_final_read() {
        let clock = TestClock::new();
        let error = diagnostic(
            11,
            101,
            Delivery::NotSent,
            "member 11 is draft; preview 101 was not sent.",
        );
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101)])),
            |attempt, _, _| match attempt {
                1 => Step::done(AttemptOutcome::Failed(error.clone())),
                2 => Step::after(5000, AttemptOutcome::Held),
                _ => panic!("late hold started another attempt"),
            },
        );
        let outcome = clock.run(&mut io, 2500).unwrap();
        assert!(outcome.messages.is_empty());
        assert_eq!(outcome.notification_errors, vec![error.clone()]);
        assert_eq!(io.attempts, 2);
        clock.assert_last_io_was_attempt();
    }

    #[test]
    fn a_late_success_clears_its_matching_error_and_preserves_other_members_errors() {
        let clock = TestClock::new();
        let errors = RefCell::new(Vec::new());
        let sent_member = Cell::new(None);
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101), (22, 202)])),
            |attempt, member, _| {
                let message = if member == 11 {
                    101
                } else {
                    assert_eq!(member, 22);
                    202
                };
                match attempt {
                    1 | 2 => {
                        let error = diagnostic(
                            member,
                            message,
                            Delivery::NotSent,
                            "capture transport failed before writing: owned attempt",
                        );
                        errors.borrow_mut().push(error.clone());
                        Step::done(AttemptOutcome::Failed(error))
                    }
                    3 => {
                        sent_member.set(Some(member));
                        Step::after(
                            5000,
                            AttemptOutcome::Sent {
                                message_id: message,
                            },
                        )
                    }
                    _ => panic!("late success started another attempt"),
                }
            },
        );
        let outcome = clock.run(&mut io, 3500).unwrap();
        let sent_member = sent_member.get().unwrap();
        let expected: Vec<_> = errors
            .borrow()
            .iter()
            .filter(|error| error.member_id != sent_member)
            .cloned()
            .collect();
        assert_eq!(expected.len(), 1);
        assert_eq!(outcome.notification_errors, expected);
        assert!(outcome.messages.is_empty());
        clock.assert_last_io_was_attempt();
    }

    #[test]
    fn a_late_failure_retains_its_typed_delivery_and_revalidated_message_without_a_final_read() {
        for delivery in [Delivery::NotSent, Delivery::Uncertain] {
            let clock = TestClock::new();
            let error = diagnostic(
                11,
                102,
                delivery,
                "actual transport cause: quotes ' and newline\nretained",
            );
            let mut io = ScriptedIo::new(
                &clock,
                |_, _, _| Step::done(snapshot(&[(11, 101)])),
                |attempt, member, _| {
                    assert_eq!(attempt, 1);
                    assert_eq!(member, 11);
                    Step::after(5000, AttemptOutcome::Failed(error.clone()))
                },
            );
            let outcome = clock.run(&mut io, 2000).unwrap();
            assert!(outcome.messages.is_empty());
            assert_eq!(outcome.notification_errors, vec![error.clone()]);
            clock.assert_last_io_was_attempt();
        }
    }

    #[test]
    fn a_later_error_replaces_the_same_members_previous_error_and_delivery() {
        let clock = TestClock::new();
        let first = diagnostic(
            11,
            101,
            Delivery::NotSent,
            "member 11 is unknown; preview 101 was not sent.",
        );
        let latest = diagnostic(
            11,
            101,
            Delivery::Uncertain,
            "literal transport failed after typing may have begun",
        );
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                Step::done(if attempts == 2 {
                    incoming(&[(11, 101)])
                } else {
                    snapshot(&[(11, 101)])
                })
            },
            |attempt, _, _| {
                Step::done(AttemptOutcome::Failed(match attempt {
                    1 => first.clone(),
                    2 => latest.clone(),
                    _ => panic!("inbox was not checked before another retry"),
                }))
            },
        );
        let outcome = clock.run(&mut io, 5000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert_eq!(outcome.notification_errors, vec![latest.clone()]);
        assert_eq!(io.attempts, 2);
        clock.assert_retry_cadence();
    }

    #[test]
    fn a_different_pending_preview_replaces_the_members_previous_diagnostic() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                Step::done(match attempts {
                    0 => snapshot(&[(11, 101)]),
                    1 => snapshot(&[(11, 102)]),
                    2 => incoming(&[(11, 102)]),
                    _ => panic!("return available incoming messages"),
                })
            },
            |attempt, member, _| {
                assert_eq!(member, 11);
                let message = match attempt {
                    1 => 101,
                    2 => 102,
                    _ => panic!("unexpected retry"),
                };
                Step::done(AttemptOutcome::Failed(diagnostic(
                    member,
                    message,
                    Delivery::NotSent,
                    "preview remains unknown",
                )))
            },
        );
        let outcome = clock.run(&mut io, 5000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert_eq!(
            outcome.notification_errors,
            vec![diagnostic(
                11,
                102,
                Delivery::NotSent,
                "preview remains unknown"
            )]
        );
        clock.assert_retry_cadence();
    }

    #[test]
    fn successful_snapshots_discard_diagnostics_for_ineligible_exact_messages_before_returning() {
        for pending in [vec![], vec![(11, 102)], vec![(22, 202)]] {
            let clock = TestClock::new();
            let mut io = ScriptedIo::new(
                &clock,
                |_, attempts, _| {
                    Step::done(if attempts == 0 {
                        snapshot(&[(11, 101)])
                    } else {
                        incoming(&pending)
                    })
                },
                |attempt, member, _| {
                    assert_eq!(attempt, 1);
                    assert_eq!(member, 11);
                    Step::done(AttemptOutcome::Failed(diagnostic(
                        member,
                        101,
                        Delivery::Uncertain,
                        "stamp write failed",
                    )))
                },
            );
            let outcome = clock.run(&mut io, 5000).unwrap();
            assert_eq!(outcome.messages, messages());
            assert!(outcome.notification_errors.is_empty());
            assert_eq!(io.attempts, 1);
        }
    }

    #[test]
    fn a_normal_hold_adds_no_error_and_preserves_another_members_diagnostic() {
        let clock = TestClock::new();
        let first_error = RefCell::new(None);
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                Step::done(if attempts == 2 {
                    incoming(&[(11, 101), (22, 202)])
                } else {
                    snapshot(&[(11, 101), (22, 202)])
                })
            },
            |attempt, member, _| match attempt {
                1 => {
                    let error = diagnostic(
                        member,
                        if member == 11 {
                            101
                        } else {
                            assert_eq!(member, 22);
                            202
                        },
                        Delivery::NotSent,
                        "unsupported composer",
                    );
                    first_error.replace(Some(error.clone()));
                    Step::done(AttemptOutcome::Failed(error))
                }
                2 => Step::done(AttemptOutcome::Held),
                _ => panic!("held member delayed the next inbox check"),
            },
        );
        let outcome = clock.run(&mut io, 5000).unwrap();
        assert_eq!(outcome.messages, messages());
        assert_eq!(
            outcome.notification_errors,
            vec![first_error.borrow().clone().unwrap()]
        );
        let attempts = clock.attempts();
        assert_ne!(attempts[0].1, attempts[1].1);
        clock.assert_retry_cadence();
    }

    #[test]
    fn an_initial_fatal_snapshot_error_preserves_its_domain_variant_and_cause() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| {
                Step::failed(CafleetError::InvalidStoredValue {
                    field: "members.status".into(),
                    value: "corrupt-status".into(),
                })
            },
            unexpected_attempt,
        );
        let error = clock.run(&mut io, 5000).unwrap_err();
        assert!(
            matches!(error, CafleetError::InvalidStoredValue { field, value } if field == "members.status" && value == "corrupt-status")
        );
        assert_eq!(io.checks, 1);
        assert!(clock.sleeps().is_empty());
    }

    #[test]
    fn a_fatal_snapshot_error_after_a_retry_returns_the_storage_cause() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, attempts, _| {
                if attempts == 0 {
                    Step::done(snapshot(&[(11, 101)]))
                } else {
                    Step::failed(CafleetError::App("database disk image is malformed".into()))
                }
            },
            |_, _, _| {
                Step::done(AttemptOutcome::Failed(diagnostic(
                    11,
                    101,
                    Delivery::NotSent,
                    "draft remains",
                )))
            },
        );
        let error = clock.run(&mut io, 5000).unwrap_err();
        assert!(
            matches!(error, CafleetError::App(cause) if cause == "database disk image is malformed")
        );
        assert_eq!(io.attempts, 1);
        clock.assert_retry_cadence();
    }

    #[test]
    fn a_fatal_attempt_error_returns_the_domain_error_instead_of_stale_success() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101)])),
            |attempt, _, _| match attempt {
                1 => Step::done(AttemptOutcome::Failed(diagnostic(
                    11,
                    101,
                    Delivery::NotSent,
                    "capture failed before input",
                ))),
                2 => Step::failed(CafleetError::InvalidStoredValue {
                    field: "messages.status_state".into(),
                    value: "corrupt-message".into(),
                }),
                _ => panic!("fatal storage error was swallowed"),
            },
        );
        let error = clock.run(&mut io, 5000).unwrap_err();
        assert!(
            matches!(error, CafleetError::InvalidStoredValue { field, value } if field == "messages.status_state" && value == "corrupt-message")
        );
        assert_eq!(io.attempts, 2);
        clock.assert_retry_cadence();
    }

    #[test]
    fn a_late_fatal_attempt_error_preserves_the_actual_cause_without_a_final_read() {
        let clock = TestClock::new();
        let mut io = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101)])),
            |attempt, _, _| {
                assert_eq!(attempt, 1);
                Step {
                    duration: Duration::from_secs(5),
                    result: Err(CafleetError::App(
                        "fleet disappeared during revalidation".into(),
                    )),
                }
            },
        );
        let error = clock.run(&mut io, 2000).unwrap_err();
        assert!(
            matches!(error, CafleetError::App(cause) if cause == "fleet disappeared during revalidation")
        );
        clock.assert_last_io_was_attempt();
    }

    #[test]
    fn a_new_invocation_has_fresh_diagnostics_and_a_new_receive_deadline() {
        let clock = TestClock::new();
        let error = diagnostic(
            11,
            101,
            Delivery::Uncertain,
            "prior invocation stamp failed",
        );
        let mut first = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[(11, 101)])),
            |attempt, _, _| {
                assert_eq!(attempt, 1);
                Step::after(5000, AttemptOutcome::Failed(error.clone()))
            },
        );
        let first_outcome = clock.run(&mut first, 2000).unwrap();
        assert_eq!(first_outcome.notification_errors, vec![error.clone()]);
        clock.assert_last_io_was_attempt();
        let second_start = clock.elapsed();
        let mut second = ScriptedIo::new(
            &clock,
            |_, _, _| Step::done(snapshot(&[])),
            unexpected_attempt,
        );
        let second_outcome = clock.run(&mut second, 1500).unwrap();
        assert!(second_outcome.messages.is_empty());
        assert!(second_outcome.notification_errors.is_empty());
        assert_eq!(clock.elapsed(), second_start + Duration::from_millis(1500));
        assert_eq!(second.checks, 2);
        let checks = clock.checks();
        assert_eq!(
            checks[checks.len() - 2],
            (second_start, Duration::from_millis(1500))
        );
        assert_eq!(
            checks[checks.len() - 1],
            (
                second_start + Duration::from_secs(1),
                Duration::from_millis(500)
            )
        );
    }
}
