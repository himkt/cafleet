use std::collections::BTreeSet;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveSnapshot {
    pub messages: Vec<MessageRecord>,
    pub pending: Vec<PendingPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotificationError {
    pub member_id: i64,
    pub message_id: i64,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptOutcome {
    pub sent: Vec<i64>,
    pub end: AttemptEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttemptEnd {
    NoPending,
    Drained,
    BudgetExhausted,
    Held,
    Failed(NotificationError),
}

impl AttemptOutcome {
    pub(crate) fn ended(end: AttemptEnd) -> Self {
        Self {
            sent: Vec::new(),
            end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitOutcome {
    pub messages: Vec<MessageRecord>,
}

pub(crate) trait ReceiveIo {
    fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError>;
    fn attempt(
        &mut self,
        member_id: i64,
        deadline: Deadline,
        attempted: &BTreeSet<i64>,
    ) -> Result<AttemptOutcome, CafleetError>;
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
    let mut attempted = BTreeSet::new();
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
            Err(error) => return Err(error),
        };
        messages = snapshot.messages;
        let now = clock.now();
        if !messages.is_empty() || now >= expires {
            break;
        }
        let interval = now.duration_since(started).as_secs();
        let selected = if last_interval == Some(interval) {
            None
        } else {
            let eligible = snapshot
                .pending
                .iter()
                .filter(|preview| !attempted.contains(&preview.message_id));
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
            let outcome =
                match io.attempt(
                    member,
                    Deadline::until(now + Duration::from_secs(5)),
                    &attempted,
                ) {
                    Ok(outcome) => outcome,
                    Err(
                        error @ (CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded),
                    ) => AttemptOutcome::ended(AttemptEnd::Failed(
                        crate::delivery::notification_error(member, preview.message_id, error),
                    )),
                    Err(error) => return Err(error),
                };
            match outcome.end {
                AttemptEnd::Held | AttemptEnd::Drained | AttemptEnd::BudgetExhausted => {}
                AttemptEnd::NoPending => {}
                AttemptEnd::Failed(error) => {
                    assert_eq!(
                        error.member_id, member,
                        "the owned attempt reports its selected member"
                    );
                    assert!(
                        attempted.insert(error.message_id),
                        "one invocation attempts each failed message at most once"
                    );
                    eprintln!("{}", error.error);
                }
            }
            next_check = clock.now();
        } else {
            next_check = started + Duration::from_secs(interval + 1);
        }
    }
    Ok(WaitOutcome { messages })
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
                .map(|(member_id, message_id)| PendingPreview {
                    member_id,
                    message_id,
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
        attempted: &BTreeSet<i64>,
    ) -> Result<AttemptOutcome, CafleetError> {
        let fleet = crate::input::with_database_budget(self.conn, deadline, |conn| {
            subject_fleet(conn, self.member_id, self.fleet_id)
        })?;
        crate::runtime::deliver_pending(
            self.conn,
            self.settings,
            member_id,
            Some(fleet),
            deadline,
            attempted,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::records::MessageStatus;
    use std::cell::Cell;

    struct TestClock {
        origin: Instant,
        elapsed: Cell<Duration>,
    }

    impl TestClock {
        fn new() -> Self {
            Self {
                origin: Instant::now(),
                elapsed: Cell::new(Duration::ZERO),
            }
        }

        fn advance(&self, duration: Duration) {
            self.elapsed.set(self.elapsed.get() + duration);
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.origin + self.elapsed.get()
        }

        fn sleep(&self, duration: Duration) {
            assert!(!duration.is_zero());
            self.advance(duration);
        }
    }

    struct RecordingIo<C, A> {
        check: C,
        attempt: A,
    }

    impl<C, A> ReceiveIo for RecordingIo<C, A>
    where
        C: FnMut(Deadline) -> Result<ReceiveSnapshot, CafleetError>,
        A: FnMut(i64, Deadline, &BTreeSet<i64>) -> Result<AttemptOutcome, CafleetError>,
    {
        fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError> {
            (self.check)(deadline)
        }

        fn attempt(
            &mut self,
            member: i64,
            deadline: Deadline,
            excluded: &BTreeSet<i64>,
        ) -> Result<AttemptOutcome, CafleetError> {
            (self.attempt)(member, deadline, excluded)
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
                })
                .collect(),
        }
    }

    fn message(id: i64) -> MessageRecord {
        MessageRecord {
            message_id: id,
            from_member_id: 2,
            to_member_id: 1,
            created_at: "2026-10-25T01:00:00.000000+00:00".into(),
            status: MessageStatus::InputRequired,
            status_timestamp: "2026-10-25T01:00:00.000000+00:00".into(),
            text: "雪\nfull reply".repeat(300),
        }
    }

    fn no_attempt(_: i64, _: Deadline, _: &BTreeSet<i64>) -> Result<AttemptOutcome, CafleetError> {
        panic!("the subject inbox or empty candidate set takes priority");
    }

    #[test]
    fn existing_inbox_returns_full_records_before_retry_or_sleep() {
        let clock = TestClock::new();
        let expected = vec![message(9), message(8)];
        let mut io = RecordingIo {
            check: |_| {
                Ok(ReceiveSnapshot {
                    messages: expected.clone(),
                    pending: snapshot(&[(2, 3)]).pending,
                })
            },
            attempt: no_attempt,
        };
        assert_eq!(
            wait(&clock, &mut io, Duration::from_secs(30))
                .unwrap()
                .messages,
            expected
        );
        assert_eq!(clock.elapsed.get(), Duration::ZERO);
    }

    #[test]
    fn idle_checks_cap_work_and_finish_at_the_original_expiry() {
        let clock = TestClock::new();
        let mut budgets = Vec::new();
        let mut io = RecordingIo {
            check: |deadline: Deadline| {
                budgets.push(deadline.remaining_at(clock.now()).unwrap());
                Ok(snapshot(&[]))
            },
            attempt: no_attempt,
        };
        assert!(
            wait(&clock, &mut io, Duration::from_millis(6500))
                .unwrap()
                .messages
                .is_empty()
        );
        assert_eq!(clock.elapsed.get(), Duration::from_millis(6500));
        assert_eq!(
            budgets,
            [5000, 5000, 4500, 3500, 2500, 1500, 500].map(Duration::from_millis)
        );
    }

    #[test]
    fn retries_rotate_members_and_exclude_each_failed_message_for_one_invocation() {
        let clock = TestClock::new();
        let mut attempts = Vec::new();
        let mut io = RecordingIo {
            check: |_| Ok(snapshot(&[(2, 20), (2, 21), (5, 50)])),
            attempt: |member, deadline: Deadline, excluded: &BTreeSet<i64>| {
                assert_eq!(
                    deadline.remaining_at(clock.now()).unwrap(),
                    Duration::from_secs(5)
                );
                let id = match member {
                    2 if excluded.contains(&20) => 21,
                    2 => 20,
                    5 => 50,
                    _ => panic!("unexpected retry member {member}"),
                };
                attempts.push((clock.elapsed.get(), member, excluded.clone()));
                Ok(AttemptOutcome::ended(AttemptEnd::Failed(
                    crate::delivery::notification_error(member, id, "unavailable"),
                )))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(4)).unwrap();
        assert_eq!(
            attempts,
            vec![
                (Duration::ZERO, 2, BTreeSet::new()),
                (Duration::from_secs(1), 5, BTreeSet::from([20])),
                (Duration::from_secs(2), 2, BTreeSet::from([20, 50])),
            ]
        );
        let mut retried = false;
        let mut io = RecordingIo {
            check: |_| Ok(snapshot(&[(2, 20)])),
            attempt: |_, _, excluded: &BTreeSet<i64>| {
                assert!(excluded.is_empty());
                retried = true;
                clock.advance(Duration::from_secs(1));
                Ok(AttemptOutcome::ended(AttemptEnd::Drained))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(1)).unwrap();
        assert!(retried);
    }

    #[test]
    fn a_reply_after_a_retry_precedes_the_next_retry() {
        let clock = TestClock::new();
        let attempted = Cell::new(false);
        let mut io = RecordingIo {
            check: |_| {
                Ok(if attempted.get() {
                    ReceiveSnapshot {
                        messages: vec![message(7)],
                        pending: snapshot(&[(5, 50)]).pending,
                    }
                } else {
                    snapshot(&[(2, 20), (5, 50)])
                })
            },
            attempt: |member, _, _: &BTreeSet<i64>| {
                assert_eq!(member, 2);
                assert!(!attempted.replace(true));
                Ok(AttemptOutcome::ended(AttemptEnd::Held))
            },
        };
        assert_eq!(
            wait(&clock, &mut io, Duration::from_secs(10))
                .unwrap()
                .messages,
            vec![message(7)]
        );
        assert_eq!(clock.elapsed.get(), Duration::ZERO);
    }

    #[test]
    fn live_membership_and_slow_attempts_preserve_rotation_without_catch_up() {
        let clock = TestClock::new();
        let mut members = Vec::new();
        let mut io = RecordingIo {
            check: |_| {
                Ok(if clock.elapsed.get().is_zero() {
                    snapshot(&[(2, 20), (5, 50)])
                } else {
                    snapshot(&[(1, 10), (8, 80)])
                })
            },
            attempt: |member, _, _: &BTreeSet<i64>| {
                members.push((clock.elapsed.get(), member));
                clock.advance(Duration::from_millis(2400));
                Ok(AttemptOutcome::ended(AttemptEnd::Held))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(5)).unwrap();
        assert_eq!(
            members,
            [
                (Duration::ZERO, 2),
                (Duration::from_millis(2400), 8),
                (Duration::from_millis(4800), 1)
            ]
        );
    }

    #[test]
    fn an_attempt_near_expiry_keeps_its_own_budget_and_performs_no_final_read() {
        let clock = TestClock::new();
        let mut checks = 0;
        let mut io = RecordingIo {
            check: |deadline: Deadline| {
                checks += 1;
                assert_eq!(checks, 1);
                assert_eq!(
                    deadline.remaining_at(clock.now()).unwrap(),
                    Duration::from_secs(1)
                );
                clock.advance(Duration::from_millis(900));
                Ok(snapshot(&[(2, 20)]))
            },
            attempt: |_, deadline: Deadline, _: &BTreeSet<i64>| {
                assert_eq!(
                    deadline.remaining_at(clock.now()).unwrap(),
                    Duration::from_secs(5)
                );
                clock.advance(Duration::from_secs(2));
                Ok(AttemptOutcome::ended(AttemptEnd::Drained))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(1)).unwrap();
        assert_eq!(checks, 1);
    }

    #[test]
    fn transient_snapshot_contention_recovers_and_fatal_errors_keep_their_cause() {
        let clock = TestClock::new();
        let mut calls = 0;
        let mut io = RecordingIo {
            check: |_| {
                calls += 1;
                if calls == 1 {
                    Err(CafleetError::DatabaseBusy("held writer".into()))
                } else {
                    Ok(ReceiveSnapshot {
                        messages: vec![message(3)],
                        pending: Vec::new(),
                    })
                }
            },
            attempt: no_attempt,
        };
        assert_eq!(
            wait(&clock, &mut io, Duration::from_secs(3))
                .unwrap()
                .messages,
            vec![message(3)]
        );
        assert_eq!(clock.elapsed.get(), Duration::from_secs(1));
        for during_attempt in [false, true] {
            let mut io = RecordingIo {
                check: |_| {
                    if during_attempt {
                        Ok(snapshot(&[(2, 20)]))
                    } else {
                        Err(CafleetError::Value("corrupt subject".into()))
                    }
                },
                attempt: |_, _, _: &BTreeSet<i64>| {
                    Err(CafleetError::Value("corrupt subject".into()))
                },
            };
            assert!(
                matches!(wait(&clock, &mut io, Duration::from_secs(1)), Err(CafleetError::Value(cause)) if cause == "corrupt subject")
            );
        }
    }

    #[test]
    fn attempt_contention_excludes_the_selected_message_and_late_checks_start_no_input() {
        for late in [false, true] {
            let clock = TestClock::new();
            let mut calls = 0;
            let mut io = RecordingIo {
                check: |_| {
                    if late {
                        clock.advance(Duration::from_secs(1));
                    }
                    Ok(snapshot(&[(2, 20)]))
                },
                attempt: |_, _, excluded: &BTreeSet<i64>| {
                    calls += 1;
                    assert!(excluded.is_empty());
                    Err(CafleetError::DeadlineExceeded)
                },
            };
            wait(
                &clock,
                &mut io,
                Duration::from_secs(if late { 1 } else { 3 }),
            )
            .unwrap();
            assert_eq!(calls, usize::from(!late));
        }
    }
}
