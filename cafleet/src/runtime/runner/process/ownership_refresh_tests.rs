use super::*;
use std::collections::VecDeque;

const GROUP: i32 = 41001;
const A: i64 = 41001;
const B: i64 = 41002;
const C: i64 = 41003;

#[derive(Debug)]
enum Step {
    Members(i32, Result<Vec<i64>>),
    Identity(i64, Result<Option<ProcessIdentity>>),
    Stopped(i64, Result<bool>),
}

struct Script(VecDeque<Step>);

impl Script {
    fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self(steps.into_iter().collect())
    }

    fn next(&mut self) -> Step {
        self.0.pop_front().expect("every observation is scripted")
    }

    fn finished(self) {
        assert!(self.0.is_empty(), "unused observations: {:?}", self.0);
    }
}

impl ProcessObservations for Script {
    fn members(&mut self, group: i32) -> Result<Vec<i64>> {
        let Step::Members(expected, result) = self.next() else {
            panic!("expected the next scripted operation to enumerate group {group}");
        };
        assert_eq!(group, expected);
        result
    }

    fn identity(&mut self, pid: i64) -> Result<Option<ProcessIdentity>> {
        let Step::Identity(expected, result) = self.next() else {
            panic!("expected the next scripted operation to identify PID {pid}");
        };
        assert_eq!(pid, expected);
        result
    }

    fn stopped(&mut self, pid: i64) -> Result<bool> {
        let Step::Stopped(expected, result) = self.next() else {
            panic!("expected the next scripted operation to inspect stopped PID {pid}");
        };
        assert_eq!(pid, expected);
        result
    }
}

fn identity(pid: i64) -> ProcessIdentity {
    ProcessIdentity {
        pid,
        start_identity: format!("synthetic-generation-{pid}"),
    }
}

fn replacement(pid: i64) -> ProcessIdentity {
    ProcessIdentity {
        pid,
        start_identity: format!("synthetic-replacement-{pid}"),
    }
}

fn members(pids: &[i64]) -> Step {
    Step::Members(GROUP, Ok(pids.to_vec()))
}

fn observed(process: ProcessIdentity) -> Step {
    Step::Identity(process.pid, Ok(Some(process)))
}

fn absent(pid: i64) -> Step {
    Step::Identity(pid, Ok(None))
}

fn active(process: ProcessIdentity) -> [Step; 2] {
    [
        observed(process.clone()),
        Step::Stopped(process.pid, Ok(false)),
    ]
}

fn inert(pinned: &[ProcessIdentity], traced: bool) -> PtySession {
    let origin = Instant::now();
    let mut session = PtySession {
        master: None,
        slave: None,
        io: None,
        child: None,
        native: None,
        exit: Some(ChildExit::Code(0)),
        owned: pinned.iter().cloned().map(|p| (p.pid, p)).collect(),
        group_closed: false,
        shutdown_clock: Some(
            ShutdownClock::new(origin).clipped(origin + Duration::from_millis(900)),
        ),
        ownership_diagnostics: None,
        pending_ownership_refresh: None,
    };
    if traced {
        session.enable_ownership_diagnostics().unwrap();
    }
    session.native = Some(identity(A));
    session
}

fn boundaries(session: &PtySession) -> [Instant; 3] {
    let clock = session
        .shutdown_clock
        .expect("fixture retains its original clock");
    [
        clock.child_deadline(),
        clock.display_deadline(),
        clock.storage_deadline(),
    ]
}

fn assert_state(
    session: &PtySession,
    pinned: &BTreeMap<i64, ProcessIdentity>,
    closed: bool,
    clock: [Instant; 3],
) {
    assert_eq!(&session.owned, pinned);
    assert_eq!(session.group_closed, closed);
    assert_eq!(boundaries(session), clock);
    assert!(session.master.is_none());
    assert!(session.slave.is_none());
    assert!(session.io.is_none());
    assert!(session.child.is_none());
    assert_eq!(session.native, Some(identity(A)));
    assert_eq!(session.exit, Some(ChildExit::Code(0)));
    assert!(session.pending_ownership_refresh.is_none());
}

fn assert_error(actual: Result<()>, expected: &CafleetError) {
    let actual = actual.expect_err("the exact scripted failure must propagate");
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
}

fn baseline() {
    let mut session = inert(&[identity(A)], false);
    let clock = boundaries(&session);
    let mut script = Script::new(
        [members(&[A, B])]
            .into_iter()
            .chain(active(identity(A)))
            .chain(active(identity(B)))
            .chain([observed(identity(A))]),
    );
    session.refresh_owned_with(&mut script).unwrap();
    script.finished();
    let expected = [(A, identity(A)), (B, identity(B))].into_iter().collect();
    assert_state(&session, &expected, false, clock);
}

fn false_fold(recheck: Step) -> Script {
    Script::new(
        [members(&[A])]
            .into_iter()
            .chain(active(identity(A)))
            .chain([absent(A), recheck]),
    )
}

fn traced(session: &mut PtySession, script: &mut Script) -> Result<()> {
    session.refresh_owned_traced_with(OwnershipRefreshPhase::TryWait, script)
}

fn assert_false_record(record: &OwnershipRefreshRecord, outcome: OwnershipRefreshOutcome) {
    assert_eq!(record.phase, OwnershipRefreshPhase::TryWait);
    assert!(!record.child_present);
    assert_eq!(record.native_exit, Some(ChildExit::Code(0)));
    assert!(!record.seeded_anchor);
    assert!(!record.group_closed_before);
    assert_eq!(record.sampled_member_count, Some(1));
    assert_eq!(record.anchored, Some(false));
    assert_eq!(record.outcome, outcome);
    let active = record
        .active
        .as_ref()
        .expect("actual active scan is retained");
    assert_eq!(active.total, 1);
    assert_eq!(active.entries[0].pid, A);
    assert_eq!(record.pinned.total, 1);
    assert_eq!(record.pinned.entries[0].pid, A);
    assert_eq!(record.matches.attempted, 1);
    assert_eq!(record.matches.entries.len(), 1);
    assert_eq!(record.matches.entries[0].pid, A);
    assert_eq!(
        record.matches.entries[0].outcome,
        OwnershipMatchOutcome::DifferentOrAbsent
    );
}

#[test]
fn step5_refresh_matching_anchor_adopts_only_original_active_generations() {
    baseline();
}

#[test]
fn step5_refresh_initial_empty_closes_with_one_query_and_retains_map_and_clock() {
    let mut session = inert(&[identity(A)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = Script::new([members(&[])]);
    session.refresh_owned_with(&mut script).unwrap();
    script.finished();
    assert_state(&session, &pinned, true, clock);
}

#[test]
fn step5_refresh_stale_active_then_fresh_empty_closes_without_adoption_or_more_reads() {
    baseline();
    let mut session = inert(&[identity(A)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = Script::new(
        [members(&[A, B])]
            .into_iter()
            .chain(active(identity(A)))
            .chain(active(identity(B)))
            .chain([absent(A), members(&[])]),
    );
    session.refresh_owned_with(&mut script).unwrap();
    script.finished();
    assert_state(&session, &pinned, true, clock);
}

#[test]
fn step5_refresh_fresh_remaining_members_preserve_exact_refusal_without_adoption() {
    for fresh in [vec![B], vec![A], vec![B, C]] {
        baseline();
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = false_fold(members(&fresh));
        assert_error(
            session.refresh_owned_with(&mut script),
            &error("owned process group lost its pinned generation"),
        );
        script.finished();
        assert_state(&session, &pinned, false, clock);
    }
}

#[test]
fn step5_refresh_same_pid_replacement_cannot_supply_an_anchor_or_be_adopted() {
    baseline();
    let mut session = inert(&[identity(A)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = Script::new(
        [members(&[A])]
            .into_iter()
            .chain(active(identity(A)))
            .chain([observed(replacement(A)), members(&[A])]),
    );
    assert_error(
        session.refresh_owned_with(&mut script),
        &error("owned process group lost its pinned generation"),
    );
    script.finished();
    assert_state(&session, &pinned, false, clock);
}

#[test]
fn step5_refresh_original_observation_errors_stop_at_the_actual_first_failure() {
    let failure = CafleetError::DatabaseBusy("synthetic-original-observation".into());
    for prefix in [
        vec![Step::Members(GROUP, Err(failure.clone()))],
        vec![members(&[A, B]), Step::Identity(A, Err(failure.clone()))],
        vec![
            members(&[A, B]),
            observed(identity(A)),
            Step::Stopped(A, Err(failure.clone())),
        ],
        vec![
            members(&[A]),
            observed(identity(A)),
            Step::Stopped(A, Ok(false)),
            Step::Identity(A, Err(failure.clone())),
        ],
    ] {
        baseline();
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new(prefix);
        assert_error(session.refresh_owned_with(&mut script), &failure);
        script.finished();
        assert_state(&session, &pinned, false, clock);
    }
}

#[test]
fn step5_refresh_anchor_fold_still_reads_later_pins_after_a_match_and_propagates_error() {
    baseline();
    let failure = CafleetError::Value("synthetic-later-anchor-read".into());
    let mut session = inert(&[identity(A), identity(C)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = Script::new(
        [members(&[A])]
            .into_iter()
            .chain(active(identity(A)))
            .chain([
                observed(identity(A)),
                Step::Identity(C, Err(failure.clone())),
            ]),
    );
    assert_error(session.refresh_owned_with(&mut script), &failure);
    script.finished();
    assert_state(&session, &pinned, false, clock);
}

#[test]
fn step5_refresh_fresh_query_error_propagates_without_closure_or_followup_reads() {
    baseline();
    let failure = CafleetError::DatabaseBusy("synthetic-fresh-enumeration".into());
    let mut session = inert(&[identity(A)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = false_fold(Step::Members(GROUP, Err(failure.clone())));
    assert_error(session.refresh_owned_with(&mut script), &failure);
    script.finished();
    assert_state(&session, &pinned, false, clock);
}

#[test]
fn step5_refresh_anchored_pid_mismatch_retains_fence_without_fresh_query_fallback() {
    baseline();
    let mut session = inert(&[identity(A), identity(C)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = Script::new(
        [members(&[A, C])]
            .into_iter()
            .chain(active(replacement(A)))
            .chain(active(identity(C)))
            .chain([observed(replacement(A)), observed(identity(C))]),
    );
    assert_error(
        session.refresh_owned_with(&mut script),
        &error("owned PID was replaced by another generation"),
    );
    script.finished();
    assert_state(&session, &pinned, false, clock);
}

#[test]
fn step5_refresh_absent_and_stopped_members_close_before_any_anchor_fold() {
    let mut session = inert(&[identity(A)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = Script::new([
        members(&[A, B]),
        absent(A),
        observed(identity(B)),
        Step::Stopped(B, Ok(true)),
    ]);
    session.refresh_owned_with(&mut script).unwrap();
    script.finished();
    assert_state(&session, &pinned, true, clock);
}

#[test]
fn step5_refresh_traced_fresh_empty_records_second_observation_and_resolved_false_fold() {
    let mut session = inert(&[identity(A)], true);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = false_fold(members(&[]));
    traced(&mut session, &mut script).unwrap();
    script.finished();
    assert_state(&session, &pinned, true, clock);
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 1);
    assert_eq!(trace.records_discarded, 0);
    let record = &trace.records[0];
    assert_eq!(record.sequence, 1);
    assert_false_record(record, OwnershipRefreshOutcome::FreshEmpty);
    assert_eq!(record.group_recheck, Some(OwnershipGroupRecheck::Empty));
    let first = trace
        .first_unanchored
        .expect("completed false fold is retained");
    assert_eq!(first.sequence, 1);
    assert_false_record(&first, OwnershipRefreshOutcome::FreshEmpty);
    assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Empty));
}

#[test]
fn step5_refresh_traced_nonempty_records_exact_fresh_count_and_unresolved_refusal() {
    let mut session = inert(&[identity(A)], true);
    let mut script = false_fold(members(&[B, C]));
    assert_error(
        traced(&mut session, &mut script),
        &error("owned process group lost its pinned generation"),
    );
    script.finished();
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 1);
    let record = &trace.records[0];
    assert_false_record(record, OwnershipRefreshOutcome::Unanchored);
    assert_eq!(
        record.group_recheck,
        Some(OwnershipGroupRecheck::Nonempty { members: 2 })
    );
    let first = trace
        .first_unanchored
        .expect("first refusal retains its actual observation");
    assert_false_record(&first, OwnershipRefreshOutcome::Unanchored);
    assert_eq!(first.group_recheck, record.group_recheck);
}

#[test]
fn step5_refresh_traced_fresh_error_retains_false_fold_without_hiding_source_error() {
    let failure = CafleetError::Value("synthetic-fresh-query-trace".into());
    let mut session = inert(&[identity(A)], true);
    let mut script = false_fold(Step::Members(GROUP, Err(failure.clone())));
    assert_error(traced(&mut session, &mut script), &failure);
    script.finished();
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 1);
    let record = &trace.records[0];
    assert_false_record(record, OwnershipRefreshOutcome::Error);
    assert_eq!(record.group_recheck, Some(OwnershipGroupRecheck::Error));
    let first = trace
        .first_unanchored
        .expect("failed recheck retains its completed false fold");
    assert_false_record(&first, OwnershipRefreshOutcome::Error);
    assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Error));
}

#[test]
fn step5_refresh_trace_initial_empty_and_already_closed_remain_distinct_from_recheck() {
    let mut session = inert(&[identity(A)], true);
    let clock = boundaries(&session);
    let mut script = Script::new([members(&[])]);
    traced(&mut session, &mut script).unwrap();
    script.finished();
    let mut no_reads = Script::new([]);
    session
        .refresh_owned_traced_with(OwnershipRefreshPhase::Survivors, &mut no_reads)
        .unwrap();
    no_reads.finished();
    assert_eq!(boundaries(&session), clock);
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 2);
    assert!(trace.first_unanchored.is_none());
    let initial = &trace.records[0];
    assert_eq!(initial.outcome, OwnershipRefreshOutcome::NoMembers);
    assert_eq!(initial.sampled_member_count, Some(0));
    assert_eq!(initial.anchored, None);
    assert_eq!(initial.group_recheck, None);
    let closed = &trace.records[1];
    assert_eq!(closed.phase, OwnershipRefreshPhase::Survivors);
    assert_eq!(closed.outcome, OwnershipRefreshOutcome::AlreadyClosed);
    assert!(closed.group_closed_before);
    assert_eq!(closed.sampled_member_count, None);
    assert_eq!(closed.group_recheck, None);
}

#[test]
fn step5_refresh_trace_anchor_success_and_fold_error_have_no_recheck() {
    for fails in [false, true] {
        let mut session = inert(&[identity(A)], true);
        let failure = CafleetError::Value("synthetic-anchor-trace".into());
        let last = if fails {
            Step::Identity(A, Err(failure.clone()))
        } else {
            observed(identity(A))
        };
        let mut script = Script::new(
            [members(&[A])]
                .into_iter()
                .chain(active(identity(A)))
                .chain([last]),
        );
        let result = traced(&mut session, &mut script);
        if fails {
            assert_error(result, &failure);
        } else {
            result.unwrap();
        }
        script.finished();
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 1);
        assert!(trace.first_unanchored.is_none());
        let record = &trace.records[0];
        assert_eq!(record.group_recheck, None);
        assert_eq!(record.anchored, if fails { None } else { Some(true) });
        assert_eq!(
            record.outcome,
            if fails {
                OwnershipRefreshOutcome::Error
            } else {
                OwnershipRefreshOutcome::Anchored
            }
        );
        assert_eq!(record.matches.attempted, 1);
        assert_eq!(
            record.matches.entries[0].outcome,
            if fails {
                OwnershipMatchOutcome::Error
            } else {
                OwnershipMatchOutcome::Matches
            }
        );
    }
}

#[test]
fn step5_refresh_trace_first_resolved_false_fold_survives_ring_eviction() {
    let mut session = inert(&[identity(A)], true);
    let clock = boundaries(&session);
    let mut script = false_fold(members(&[]));
    traced(&mut session, &mut script).unwrap();
    script.finished();
    for _ in 0..70 {
        let mut no_reads = Script::new([]);
        traced(&mut session, &mut no_reads).unwrap();
        no_reads.finished();
    }
    assert_eq!(boundaries(&session), clock);
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 64);
    assert_eq!(trace.records_discarded, 7);
    assert_eq!(trace.records.first().unwrap().sequence, 8);
    assert_eq!(trace.records.last().unwrap().sequence, 71);
    assert!(
        trace
            .records
            .iter()
            .all(|r| r.outcome == OwnershipRefreshOutcome::AlreadyClosed)
    );
    let first = trace
        .first_unanchored
        .expect("resolved first false fold survives eviction");
    assert_eq!(first.sequence, 1);
    assert_false_record(&first, OwnershipRefreshOutcome::FreshEmpty);
    assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Empty));
}

#[test]
fn step5_refresh_trace_first_failed_recheck_survives_later_refusal_resolution_and_eviction() {
    let mut session = inert(&[identity(A)], true);
    let failure = CafleetError::Value("synthetic-first-recheck-error".into());
    let mut first_script = false_fold(Step::Members(GROUP, Err(failure.clone())));
    assert_error(traced(&mut session, &mut first_script), &failure);
    first_script.finished();
    let mut second = false_fold(members(&[B]));
    assert_error(
        traced(&mut session, &mut second),
        &error("owned process group lost its pinned generation"),
    );
    second.finished();
    let mut third = false_fold(members(&[]));
    traced(&mut session, &mut third).unwrap();
    third.finished();
    for _ in 0..64 {
        let mut no_reads = Script::new([]);
        traced(&mut session, &mut no_reads).unwrap();
        no_reads.finished();
    }
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 64);
    assert_eq!(trace.records_discarded, 3);
    assert_eq!(trace.records.first().unwrap().sequence, 4);
    assert_eq!(trace.records.last().unwrap().sequence, 67);
    let first = trace
        .first_unanchored
        .expect("the first failed recheck remains first");
    assert_eq!(first.sequence, 1);
    assert_false_record(&first, OwnershipRefreshOutcome::Error);
    assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Error));
}

#[test]
fn step5_refresh_trace_identity_match_and_utf8_limits_also_bound_retained_false_fold() {
    let pins: Vec<_> = (A..A + 18)
        .map(|pid| ProcessIdentity {
            pid,
            start_identity: "界".repeat(50),
        })
        .collect();
    let mut session = inert(&pins, true);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut steps = vec![members(&pins.iter().map(|p| p.pid).collect::<Vec<_>>())];
    for pin in &pins {
        steps.extend(active(pin.clone()));
    }
    steps.extend(pins.iter().map(|p| absent(p.pid)));
    steps.push(members(&[]));
    let mut script = Script::new(steps);
    traced(&mut session, &mut script).unwrap();
    script.finished();
    assert_state(&session, &pinned, true, clock);
    let trace = session.take_ownership_diagnostics().unwrap();
    assert_eq!(trace.records.len(), 1);
    let first = trace
        .first_unanchored
        .as_ref()
        .expect("bounded false fold is retained");
    for record in [&trace.records[0], first.as_ref()] {
        assert_eq!(record.outcome, OwnershipRefreshOutcome::FreshEmpty);
        assert_eq!(record.anchored, Some(false));
        assert_eq!(record.group_recheck, Some(OwnershipGroupRecheck::Empty));
        assert_eq!(record.sampled_member_count, Some(18));
        for snapshot in [record.active.as_ref().unwrap(), &record.pinned] {
            assert_eq!(snapshot.total, 18);
            assert_eq!(snapshot.entries.len(), 16);
            assert!(snapshot.truncated);
            for (offset, entry) in snapshot.entries.iter().enumerate() {
                assert_eq!(entry.pid, A + i64::try_from(offset).unwrap());
                assert_eq!(entry.original_start_bytes, 150);
                assert_eq!(entry.start_identity.len(), 126);
                assert_eq!(entry.start_identity, "界".repeat(42));
                assert!(entry.start_truncated);
            }
        }
        assert_eq!(record.matches.attempted, 18);
        assert_eq!(record.matches.entries.len(), 16);
        assert!(record.matches.truncated);
        for (offset, entry) in record.matches.entries.iter().enumerate() {
            assert_eq!(entry.pid, A + i64::try_from(offset).unwrap());
            assert_eq!(entry.outcome, OwnershipMatchOutcome::DifferentOrAbsent);
        }
    }
}

#[test]
fn step5_refresh_traced_wrapper_without_diagnostics_uses_only_the_scripted_core() {
    let mut session = inert(&[identity(A)], false);
    let pinned = session.owned.clone();
    let clock = boundaries(&session);
    let mut script = false_fold(members(&[]));
    traced(&mut session, &mut script).unwrap();
    script.finished();
    assert_state(&session, &pinned, true, clock);
    assert!(session.ownership_diagnostics.is_none());
}
