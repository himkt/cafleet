use super::*;

fn sequenced(mut value: ScreenObservation, sequence: u64) -> ScreenObservation {
    value.output_seq = sequence;
    value.raw_evidence["screen_seq"] = json!(sequence);
    value
}

fn unknown(sequence: u64) -> ScreenObservation {
    sequenced(
        observation(&definitions(), &terminal(&[], 80, false).snapshot()),
        sequence,
    )
}

fn correlated(kind: &str, identity: Option<&str>, sequence: u64) -> ScreenObservation {
    let mut definition = definitions();
    definition["profiles"][0]["rules"][0]["observation"] = json!(kind);
    definition["profiles"][0]["rules"][0]["verified_noninteraction"] =
        json!(kind == "permission_denied");
    let mut rows = lines("write disposable.txt");
    if let Some(identity) = identity {
        rows.insert(5, format!("Native correlation: {identity}"));
        definition["profiles"][0]["rules"][0]["matchers"].as_array_mut().unwrap().push(json!({"id":"native","scope":"line","pattern":"^Native correlation: (?P<identity>[a-z0-9-]+)$"}));
        definition["profiles"][0]["rules"][0]["extract"]["native_request_id"] =
            json!({"matcher_id":"native","capture":"identity"});
    }
    sequenced(
        observation(&definition, &terminal(&rows, 80, false).snapshot()),
        sequence,
    )
}

fn secret_observation(
    secret: &str,
    sensitive: bool,
    cols: u16,
    sequence: u64,
) -> ScreenObservation {
    let mut definition = definitions();
    let rule = &mut definition["profiles"][0]["rules"][0];
    rule["sensitive"] = json!(sensitive);
    rule["matchers"][1]["pattern"] =
        json!(format!("^> (?P<label>Allow {})$", regex::escape(secret)));
    rule["matchers"][2]["pattern"] =
        json!(format!("^  (?P<label>Deny {})$", regex::escape(secret)));
    rule["matchers"].as_array_mut().unwrap().push(json!({"id":"native","scope":"line","pattern":format!("^Native correlation: (?P<identity>{})$",regex::escape(secret))}));
    rule["extract"]["native_request_id"] = json!({"matcher_id":"native","capture":"identity"});
    let mut rows = lines(secret);
    rows[2] = format!("> Allow {secret}");
    rows[3] = format!("  Deny {secret}");
    rows.insert(5, format!("Native correlation: {secret}"));
    let value = sequenced(
        observation(&definition, &terminal(&rows, cols, false).snapshot()),
        sequence,
    );
    for field in ["operation", "choices", "native_request_id", "excerpt"] {
        assert!(
            value.raw_evidence[field].to_string().contains(secret),
            "canary missing from raw {field}"
        );
    }
    value
}

#[test]
fn runtime_episode_first_candidate_gates_before_the_100ms_opening_boundary() {
    let origin = Instant::now();
    let value = permission("write disposable.txt");
    let mut tracker = tracker();
    let first = tracker.observe(&value, origin, NOW).unwrap();
    assert!(first.transitions.is_empty());
    assert!(matches!(
        first.eligibility,
        InputEligibility::Hold(GateReason::Candidate)
    ));
    assert_eq!(
        first.next_deadline,
        Some(origin + Duration::from_millis(100))
    );
    assert!(
        tracker
            .observe(&value, origin + Duration::from_millis(99), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let stable = tracker
        .observe(&value, origin + Duration::from_millis(100), NOW)
        .unwrap();
    assert_eq!(stable.transitions.len(), 1);
    assert_eq!(stable.transitions[0].episode_seq, Some(1));
    assert_eq!(
        stable.transitions[0].kind,
        RuntimeEventKind::InteractionOpened
    );
    assert!(matches!(
        stable.eligibility,
        InputEligibility::Hold(GateReason::Interaction)
    ));
}

#[test]
fn runtime_episode_quiet_timer_recheck_uses_the_same_output_sequence() {
    let origin = Instant::now();
    let value = permission("write disposable.txt");
    let mut tracker = tracker();
    let opening = open(&mut tracker, &value, origin);
    assert_eq!(
        opening.transitions[0].evidence_json["screen_seq"],
        value.output_seq
    );
    assert_eq!(opening.transitions[0].observed_at, NOW);
    assert!(
        tracker
            .observe(&value, origin + Duration::from_secs(1), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
}

#[test]
fn runtime_episode_incompatible_initial_candidate_restarts_stability() {
    let origin = Instant::now();
    let a = sequenced(permission("write first.txt"), 1);
    let b = sequenced(permission("write other.txt"), 2);
    let mut tracker = tracker();
    tracker.observe(&a, origin, NOW).unwrap();
    assert!(
        tracker
            .observe(&b, origin + Duration::from_millis(99), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&b, origin + Duration::from_millis(100), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let result = tracker
        .observe(&b, origin + Duration::from_millis(199), NOW)
        .unwrap();
    assert_eq!(result.transitions.len(), 1);
    assert_eq!(result.transitions[0].episode_seq, Some(1));
}

#[test]
fn runtime_episode_stable_replacement_closes_old_episode_before_opening_the_next() {
    let origin = Instant::now();
    let a = sequenced(permission("write first.txt"), 1);
    let b = sequenced(permission("write other.txt"), 2);
    let mut tracker = tracker();
    open(&mut tracker, &a, origin);
    let pending = tracker
        .observe(&b, origin + Duration::from_millis(200), NOW)
        .unwrap();
    assert!(pending.transitions.is_empty());
    assert!(pending.latest_evidence.is_none());
    assert!(matches!(pending.eligibility, InputEligibility::Hold(_)));
    assert!(
        tracker
            .observe(&b, origin + Duration::from_millis(299), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let stable = tracker
        .observe(&b, origin + Duration::from_millis(300), NOW)
        .unwrap();
    assert_eq!(stable.transitions.len(), 2);
    assert_eq!(
        stable.transitions[0].kind,
        RuntimeEventKind::InteractionClosed
    );
    assert_eq!(
        stable.transitions[0].close_reason,
        Some(InteractionCloseReason::Superseded)
    );
    assert_eq!(stable.transitions[0].episode_seq, Some(1));
    assert_eq!(
        stable.transitions[1].kind,
        RuntimeEventKind::InteractionOpened
    );
    assert_eq!(stable.transitions[1].episode_seq, Some(2));
}

#[test]
fn runtime_episode_class_and_stable_choice_changes_supersede_after_stability() {
    for change in ["class", "choice"] {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        let mut definition = definitions();
        let mut rows = lines("write disposable.txt");
        if change == "class" {
            definition["profiles"][0]["rules"][0]["observation"] = json!("question");
        } else {
            definition["profiles"][0]["rules"][0]["matchers"][1]["pattern"] =
                json!("^> (?P<label>Allow this request)$");
            rows[2] = "> Allow this request".into();
        }
        let next = sequenced(
            observation(&definition, &terminal(&rows, 80, false).snapshot()),
            2,
        );
        assert!(
            tracker
                .observe(&next, origin + Duration::from_millis(200), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let stable = tracker
            .observe(&next, origin + Duration::from_millis(300), NOW)
            .unwrap();
        assert_eq!(stable.transitions.len(), 2);
        assert_eq!(
            stable.transitions[0].close_reason,
            Some(InteractionCloseReason::Superseded)
        );
        assert_eq!(stable.transitions[1].episode_seq, Some(2));
        assert_eq!(
            stable.transitions[1].classification,
            Some(if change == "class" {
                InteractionClass::Question
            } else {
                InteractionClass::PermissionRequest
            })
        );
    }
}

#[test]
fn runtime_episode_return_to_original_cancels_replacement_and_third_form_restarts_it() {
    let origin = Instant::now();
    let a = sequenced(permission("write first.txt"), 1);
    let b = sequenced(permission("write other.txt"), 2);
    let mut tracker = tracker();
    open(&mut tracker, &a, origin);
    tracker
        .observe(&b, origin + Duration::from_millis(200), NOW)
        .unwrap();
    assert!(
        tracker
            .observe(
                &sequenced(permission("write first.txt"), 3),
                origin + Duration::from_millis(250),
                NOW
            )
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(
                &sequenced(permission("write other.txt"), 4),
                origin + Duration::from_millis(300),
                NOW
            )
            .unwrap()
            .transitions
            .is_empty()
    );
    let c = sequenced(permission("write third.txt"), 5);
    assert!(
        tracker
            .observe(&c, origin + Duration::from_millis(350), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&c, origin + Duration::from_millis(400), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert_eq!(
        tracker
            .observe(&c, origin + Duration::from_millis(450), NOW)
            .unwrap()
            .transitions
            .len(),
        2
    );
}

#[test]
fn runtime_episode_verified_composer_remains_held_until_the_250ms_episode_closure() {
    let origin = Instant::now();
    let mut tracker = tracker();
    open(
        &mut tracker,
        &sequenced(permission("write disposable.txt"), 1),
        origin,
    );
    let empty = sequenced(state_observation("composer_empty", true), 2);
    let first = tracker
        .observe(&empty, origin + Duration::from_millis(200), NOW)
        .unwrap();
    assert!(first.transitions.is_empty());
    assert!(matches!(empty.kind, ObservationKind::ComposerEmpty));
    assert!(matches!(
        first.eligibility,
        InputEligibility::Hold(GateReason::Interaction)
    ));
    assert_eq!(
        first.next_deadline,
        Some(origin + Duration::from_millis(450))
    );
    let before = tracker
        .observe(&empty, origin + Duration::from_millis(449), NOW)
        .unwrap();
    assert!(before.transitions.is_empty());
    assert!(matches!(
        before.eligibility,
        InputEligibility::Hold(GateReason::Interaction)
    ));
    let closed = tracker
        .observe(&empty, origin + Duration::from_millis(450), NOW)
        .unwrap();
    assert_eq!(closed.transitions.len(), 1);
    assert_eq!(
        closed.transitions[0].kind,
        RuntimeEventKind::InteractionClosed
    );
    assert_eq!(
        closed.transitions[0].close_reason,
        Some(InteractionCloseReason::NotVisible)
    );
    assert!(matches!(
        closed.eligibility,
        InputEligibility::ComposerEmpty
    ));
    assert!(closed.latest_evidence.is_none());
}

#[test]
fn runtime_episode_uncertainty_interrupts_the_verified_boundary() {
    let origin = Instant::now();
    let mut tracker = tracker();
    open(
        &mut tracker,
        &sequenced(permission("write disposable.txt"), 1),
        origin,
    );
    tracker
        .observe(
            &sequenced(state_observation("composer_empty", true), 2),
            origin + Duration::from_millis(200),
            NOW,
        )
        .unwrap();
    tracker
        .observe(&unknown(3), origin + Duration::from_millis(449), NOW)
        .unwrap();
    let empty = sequenced(state_observation("composer_empty", true), 4);
    assert!(
        tracker
            .observe(&empty, origin + Duration::from_millis(450), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&empty, origin + Duration::from_millis(699), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert_eq!(
        tracker
            .observe(&empty, origin + Duration::from_millis(700), NOW)
            .unwrap()
            .transitions
            .len(),
        1
    );
}

#[test]
fn runtime_episode_continuous_uncertainty_emits_one_obscured_event_at_two_seconds() {
    let origin = Instant::now();
    let mut tracker = tracker();
    open(
        &mut tracker,
        &sequenced(permission("write disposable.txt"), 1),
        origin,
    );
    let obscured = unknown(2);
    let first = tracker
        .observe(&obscured, origin + Duration::from_millis(200), NOW)
        .unwrap();
    assert!(first.transitions.is_empty());
    assert!(matches!(first.visibility, Visibility::Obscured));
    assert!(matches!(first.eligibility, InputEligibility::Hold(_)));
    assert!(
        tracker
            .observe(&obscured, origin + Duration::from_millis(2199), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let stable = tracker
        .observe(&obscured, origin + Duration::from_millis(2200), NOW)
        .unwrap();
    assert_eq!(stable.transitions.len(), 1);
    assert_eq!(
        stable.transitions[0].kind,
        RuntimeEventKind::InteractionObscured
    );
    assert_eq!(stable.transitions[0].episode_seq, Some(1));
    assert!(
        tracker
            .observe(&obscured, origin + Duration::from_secs(5), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let returned = tracker
        .observe(
            &sequenced(permission("write disposable.txt"), 3),
            origin + Duration::from_secs(6),
            NOW,
        )
        .unwrap();
    assert!(returned.transitions.is_empty());
    assert!(matches!(returned.visibility, Visibility::Visible));
    assert_eq!(returned.latest_evidence.unwrap().episode_seq, 1);
}

#[test]
fn runtime_episode_equivalent_redraw_refreshes_evidence_without_another_opening() {
    let origin = Instant::now();
    let mut tracker = tracker();
    open(
        &mut tracker,
        &sequenced(permission("write disposable.txt"), 1),
        origin,
    );
    let redraw = sequenced(
        observation(
            &definitions(),
            &terminal(&lines("write disposable.txt"), 100, false).snapshot(),
        ),
        2,
    );
    let update = tracker
        .observe(&redraw, origin + Duration::from_millis(200), NOW)
        .unwrap();
    assert!(update.transitions.is_empty());
    let refresh = update.latest_evidence.unwrap();
    assert_eq!(
        (refresh.run_id, refresh.episode_seq, refresh.output_seq),
        (1, 1, 2)
    );
    assert_eq!(refresh.evidence_json["screen_seq"], 2);
    assert!(refresh.evidence_json["operation"].is_null());
    let identical = tracker
        .observe(&redraw, origin + Duration::from_millis(300), NOW)
        .unwrap();
    assert!(identical.transitions.is_empty());
    assert!(identical.latest_evidence.is_none());
}

#[test]
fn runtime_episode_identical_requests_are_distinct_only_after_a_verified_boundary() {
    let origin = Instant::now();
    let mut tracker = tracker();
    let first = sequenced(permission("write disposable.txt"), 1);
    open(&mut tracker, &first, origin);
    assert!(
        tracker
            .observe(
                &sequenced(permission("write disposable.txt"), 2),
                origin + Duration::from_millis(200),
                NOW
            )
            .unwrap()
            .transitions
            .is_empty()
    );
    let empty = sequenced(state_observation("composer_empty", true), 3);
    tracker
        .observe(&empty, origin + Duration::from_millis(300), NOW)
        .unwrap();
    tracker
        .observe(&empty, origin + Duration::from_millis(550), NOW)
        .unwrap();
    let second = sequenced(permission("write disposable.txt"), 4);
    assert!(
        tracker
            .observe(&second, origin + Duration::from_millis(600), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let opened = tracker
        .observe(&second, origin + Duration::from_millis(700), NOW)
        .unwrap();
    assert_eq!(opened.transitions.len(), 1);
    assert_eq!(opened.transitions[0].episode_seq, Some(2));
}

#[test]
fn runtime_episode_refusal_requires_stability_and_retains_deduplication_across_uncertainty() {
    let origin = Instant::now();
    let mut tracker = tracker();
    let refusal = correlated("permission_denied", None, 1);
    assert!(
        tracker
            .observe(&refusal, origin, NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&refusal, origin + Duration::from_millis(99), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let result = tracker
        .observe(&refusal, origin + Duration::from_millis(100), NOW)
        .unwrap();
    assert_eq!(result.transitions.len(), 1);
    assert_eq!(
        result.transitions[0].kind,
        RuntimeEventKind::PermissionDeniedObserved
    );
    assert_eq!(result.transitions[0].episode_seq, None);
    tracker
        .observe(&unknown(2), origin + Duration::from_millis(150), NOW)
        .unwrap();
    let repeated = correlated("permission_denied", None, 3);
    assert!(
        tracker
            .observe(&repeated, origin + Duration::from_millis(200), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&repeated, origin + Duration::from_millis(300), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
}

#[test]
fn runtime_episode_refusal_dedup_resets_after_a_verified_250ms_boundary() {
    let origin = Instant::now();
    let mut tracker = tracker();
    let first = correlated("permission_denied", None, 1);
    tracker.observe(&first, origin, NOW).unwrap();
    tracker
        .observe(&first, origin + Duration::from_millis(100), NOW)
        .unwrap();
    let empty = sequenced(state_observation("composer_empty", true), 2);
    tracker
        .observe(&empty, origin + Duration::from_millis(200), NOW)
        .unwrap();
    tracker
        .observe(&empty, origin + Duration::from_millis(450), NOW)
        .unwrap();
    let next = correlated("permission_denied", None, 3);
    tracker
        .observe(&next, origin + Duration::from_millis(500), NOW)
        .unwrap();
    let result = tracker
        .observe(&next, origin + Duration::from_millis(600), NOW)
        .unwrap();
    assert_eq!(result.transitions.len(), 1);
    assert_eq!(
        result.transitions[0].kind,
        RuntimeEventKind::PermissionDeniedObserved
    );
}

#[test]
fn runtime_episode_uncertainty_restarts_a_pending_refusal_before_its_first_emission() {
    let origin = Instant::now();
    let mut tracker = tracker();
    tracker
        .observe(&correlated("permission_denied", None, 1), origin, NOW)
        .unwrap();
    tracker
        .observe(&unknown(2), origin + Duration::from_millis(99), NOW)
        .unwrap();
    let refusal = correlated("permission_denied", None, 3);
    assert!(
        tracker
            .observe(&refusal, origin + Duration::from_millis(100), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&refusal, origin + Duration::from_millis(199), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let result = tracker
        .observe(&refusal, origin + Duration::from_millis(200), NOW)
        .unwrap();
    assert_eq!(result.transitions.len(), 1);
    assert_eq!(
        result.transitions[0].kind,
        RuntimeEventKind::PermissionDeniedObserved
    );
}

#[test]
fn runtime_episode_refusal_linkage_requires_a_matching_native_correlation() {
    for (identity, expected) in [
        (None, None),
        (Some("request-2"), None),
        (Some("request-1"), Some(1)),
    ] {
        let origin = Instant::now();
        let mut tracker = tracker();
        let request = correlated("permission_request", Some("request-1"), 1);
        open(&mut tracker, &request, origin);
        let result = correlated("permission_denied", identity, 2);
        tracker
            .observe(&result, origin + Duration::from_millis(200), NOW)
            .unwrap();
        let stable = tracker
            .observe(&result, origin + Duration::from_millis(300), NOW)
            .unwrap();
        let event = stable
            .transitions
            .iter()
            .find(|event| event.kind == RuntimeEventKind::PermissionDeniedObserved)
            .unwrap();
        assert_eq!(event.episode_seq, expected);
    }
}

#[test]
fn runtime_episode_gap_reset_preserves_open_identity_and_discards_pending_timers() {
    for reason in [GateReason::Gap, GateReason::ObserverUnavailable] {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write first.txt"), 1),
            origin,
        );
        tracker
            .observe(
                &sequenced(permission("write other.txt"), 2),
                origin + Duration::from_millis(200),
                NOW,
            )
            .unwrap();
        let reset = tracker.reset_observation(reason).unwrap();
        assert!(reset.transitions.is_empty());
        assert!(reset.next_deadline.is_none());
        assert!(matches!(reset.visibility, Visibility::Unknown));
        assert!(matches!(reset.eligibility, InputEligibility::Hold(_)));
        let returned = tracker
            .observe(
                &sequenced(permission("write first.txt"), 3),
                origin + Duration::from_secs(5),
                NOW,
            )
            .unwrap();
        assert!(returned.transitions.is_empty());
        assert!(matches!(returned.visibility, Visibility::Visible));
    }
}

#[test]
fn runtime_episode_gap_reset_restarts_an_uncommitted_candidate() {
    let origin = Instant::now();
    let mut tracker = tracker();
    let value = permission("write disposable.txt");
    tracker.observe(&value, origin, NOW).unwrap();
    tracker.reset_observation(GateReason::Gap).unwrap();
    assert!(
        tracker
            .observe(&value, origin + Duration::from_secs(5), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert_eq!(
        tracker
            .observe(&value, origin + Duration::from_millis(5100), NOW)
            .unwrap()
            .transitions
            .len(),
        1
    );
}

#[test]
fn runtime_episode_gap_reset_restarts_closure_debounce_and_keeps_the_active_episode_held() {
    let origin = Instant::now();
    let mut tracker = tracker();
    open(
        &mut tracker,
        &sequenced(permission("write disposable.txt"), 1),
        origin,
    );
    let empty = sequenced(state_observation("composer_empty", true), 2);
    tracker
        .observe(&empty, origin + Duration::from_millis(200), NOW)
        .unwrap();
    tracker.reset_observation(GateReason::Gap).unwrap();
    let fresh = tracker
        .observe(&empty, origin + Duration::from_secs(5), NOW)
        .unwrap();
    assert!(fresh.transitions.is_empty());
    assert!(matches!(
        fresh.eligibility,
        InputEligibility::Hold(GateReason::Interaction)
    ));
    let before = tracker
        .observe(&empty, origin + Duration::from_millis(5249), NOW)
        .unwrap();
    assert!(before.transitions.is_empty());
    assert!(matches!(
        before.eligibility,
        InputEligibility::Hold(GateReason::Interaction)
    ));
    let closed = tracker
        .observe(&empty, origin + Duration::from_millis(5250), NOW)
        .unwrap();
    assert_eq!(closed.transitions.len(), 1);
    assert!(matches!(
        closed.eligibility,
        InputEligibility::ComposerEmpty
    ));
}

#[test]
fn runtime_episode_gap_reset_discards_the_unverified_obscurity_interval() {
    let origin = Instant::now();
    let mut tracker = tracker();
    open(
        &mut tracker,
        &sequenced(permission("write disposable.txt"), 1),
        origin,
    );
    tracker
        .observe(&unknown(2), origin + Duration::from_millis(200), NOW)
        .unwrap();
    tracker.reset_observation(GateReason::Gap).unwrap();
    let unseen = unknown(3);
    assert!(
        tracker
            .observe(&unseen, origin + Duration::from_millis(2200), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    assert!(
        tracker
            .observe(&unseen, origin + Duration::from_millis(4199), NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
    let result = tracker
        .observe(&unseen, origin + Duration::from_millis(4200), NOW)
        .unwrap();
    assert_eq!(result.transitions.len(), 1);
    assert_eq!(
        result.transitions[0].kind,
        RuntimeEventKind::InteractionObscured
    );
}

#[test]
fn runtime_episode_reset_and_finish_validate_their_lifecycle_scope() {
    for reason in [
        GateReason::Candidate,
        GateReason::Interaction,
        GateReason::Working,
        GateReason::Draft,
        GateReason::Unknown,
    ] {
        assert!(tracker().reset_observation(reason).is_err());
    }
    for reason in [
        InteractionCloseReason::MonitorLost,
        InteractionCloseReason::NotVisible,
        InteractionCloseReason::Superseded,
    ] {
        assert!(tracker().finish(reason, NOW).is_err());
    }
    let origin = Instant::now();
    let mut tracker = tracker();
    open(&mut tracker, &permission("write disposable.txt"), origin);
    let closed = tracker
        .finish(InteractionCloseReason::ProcessExited, NOW)
        .unwrap();
    assert_eq!(closed.transitions.len(), 1);
    assert_eq!(
        closed.transitions[0].close_reason,
        Some(InteractionCloseReason::ProcessExited)
    );
    assert!(
        tracker
            .finish(InteractionCloseReason::ProcessExited, NOW)
            .unwrap()
            .transitions
            .is_empty()
    );
}

#[test]
fn runtime_episode_noninteraction_states_gate_independently_without_allocating_rows() {
    for (kind, reason) in [
        ("working", Some(GateReason::Working)),
        ("composer_draft", Some(GateReason::Draft)),
        ("composer_empty", None),
    ] {
        let mut tracker = tracker();
        let result = tracker
            .observe(&state_observation(kind, true), Instant::now(), NOW)
            .unwrap();
        assert!(result.transitions.is_empty());
        assert!(result.latest_evidence.is_none());
        match reason {
            Some(GateReason::Working) => assert!(matches!(
                result.eligibility,
                InputEligibility::Hold(GateReason::Working)
            )),
            Some(_) => assert!(matches!(
                result.eligibility,
                InputEligibility::Hold(GateReason::Draft)
            )),
            None => assert!(matches!(
                result.eligibility,
                InputEligibility::ComposerEmpty
            )),
        }
    }
    let result = tracker().observe(&unknown(1), Instant::now(), NOW).unwrap();
    assert!(matches!(
        result.eligibility,
        InputEligibility::Hold(GateReason::Unknown)
    ));
}

#[test]
fn runtime_episode_rejects_regressing_output_sequences_and_invalid_identity() {
    assert!(EpisodeTracker::new(0, 1, EvidencePolicy::metadata()).is_err());
    assert!(EpisodeTracker::new(1, 0, EvidencePolicy::metadata()).is_err());
    let origin = Instant::now();
    let mut tracker = tracker();
    tracker
        .observe(
            &sequenced(permission("write disposable.txt"), 2),
            origin,
            NOW,
        )
        .unwrap();
    assert!(
        tracker
            .observe(
                &sequenced(permission("write disposable.txt"), 1),
                origin + Duration::from_millis(100),
                NOW
            )
            .is_err()
    );
}

#[test]
fn runtime_episode_evidence_policy_precedes_every_transition_and_redraw_update() {
    let secret = "synthetic-secret-194";
    for (sensitive, named_sensitive) in [(false, false), (true, false), (false, true)] {
        let value = secret_observation(secret, sensitive, 80, 1);
        let policy = EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":[secret],"sensitive_rules":if named_sensitive {vec![RULE]}else{vec![]}}),
            &[RULE],
        )
        .unwrap();
        let mut tracker = EpisodeTracker::new(1, 1, policy).unwrap();
        let origin = Instant::now();
        let result = open(&mut tracker, &value, origin);
        let evidence = &result.transitions[0].evidence_json;
        assert!(!evidence.to_string().contains(secret));
        assert!(evidence.get("semantic_fingerprint").is_none());
        assert_eq!(
            evidence["operation"],
            if sensitive || named_sensitive {
                Value::Null
            } else {
                json!("[redacted]")
            }
        );
        let redraw = secret_observation(secret, sensitive, 100, 2);
        let refresh = tracker
            .observe(&redraw, origin + Duration::from_millis(200), NOW)
            .unwrap()
            .latest_evidence
            .unwrap();
        assert!(!refresh.evidence_json.to_string().contains(secret));
    }
}

#[test]
fn runtime_episode_metadata_removes_each_screen_field_before_materializing_events_or_refreshes() {
    let secret = "synthetic-secret-194";
    let origin = Instant::now();
    let mut tracker = tracker();
    let value = secret_observation(secret, false, 80, 1);
    let opened = open(&mut tracker, &value, origin);
    for field in ["operation", "choices", "native_request_id", "excerpt"] {
        assert!(opened.transitions[0].evidence_json[field].is_null());
    }
    assert!(
        !opened.transitions[0]
            .evidence_json
            .to_string()
            .contains(secret)
    );
    let mut pool = EventSequencePool::new();
    pool.supply(1..=256).unwrap();
    let event = pool.materialize(&opened.transitions[0]).unwrap();
    assert!(!serde_json::to_string(&event).unwrap().contains(secret));
    let redraw = tracker
        .observe(
            &secret_observation(secret, false, 100, 2),
            origin + Duration::from_millis(200),
            NOW,
        )
        .unwrap()
        .latest_evidence
        .unwrap();
    for field in ["operation", "choices", "native_request_id", "excerpt"] {
        assert!(redraw.evidence_json[field].is_null());
    }
    assert!(!redraw.evidence_json.to_string().contains(secret));
}

#[test]
fn runtime_episode_metadata_digest_contains_no_hash_of_hidden_semantics() {
    let origin = Instant::now();
    let first = sequenced(permission("write first-secret.txt"), 1);
    let second = sequenced(permission("write other-secret.txt"), 1);
    assert!(first.fingerprint != second.fingerprint);
    let a = open(&mut tracker(), &first, origin);
    let b = open(&mut tracker(), &second, origin);
    assert_eq!(
        a.transitions[0].evidence_json,
        b.transitions[0].evidence_json
    );
    assert_eq!(
        a.transitions[0].evidence_json["evidence_digest"],
        b.transitions[0].evidence_json["evidence_digest"]
    );
}

#[test]
fn runtime_episode_sequence_pool_uses_only_committed_grants_and_retains_borrowed_drafts() {
    let origin = Instant::now();
    let mut tracker = tracker();
    let update = open(&mut tracker, &permission("write disposable.txt"), origin);
    let draft = &update.transitions[0];
    let mut pool = EventSequencePool::new();
    assert!(pool.materialize(draft).is_err());
    assert_eq!(draft.observed_at, NOW);
    pool.supply(1..=256).unwrap();
    assert!(pool.supply(257..=512).is_err());
    for expected in 1..=256 {
        let event = pool.materialize(draft).unwrap();
        assert_eq!(event.event_seq, expected);
        assert_eq!(event.observed_at, draft.observed_at);
        assert_eq!(event.evidence_json, draft.evidence_json);
    }
    assert!(pool.materialize(draft).is_err());
    pool.supply(513..=768).unwrap();
    assert_eq!(pool.materialize(draft).unwrap().event_seq, 513);
}

#[test]
fn runtime_episode_sequence_pool_rejects_invalid_and_overlapping_reservations() {
    for range in [
        0..=255,
        1..=255,
        1..=257,
        (-255)..=0,
        (i64::MAX - 1)..=i64::MAX,
    ] {
        assert!(EventSequencePool::new().supply(range).is_err());
    }
    let origin = Instant::now();
    let mut tracker = tracker();
    let update = open(&mut tracker, &permission("write disposable.txt"), origin);
    let mut pool = EventSequencePool::new();
    pool.supply(257..=512).unwrap();
    for _ in 0..256 {
        pool.materialize(&update.transitions[0]).unwrap();
    }
    assert!(pool.supply(257..=512).is_err());
    assert!(pool.supply(1..=256).is_err());
    pool.supply(513..=768).unwrap();
    assert_eq!(
        pool.materialize(&update.transitions[0]).unwrap().event_seq,
        513
    );
}
