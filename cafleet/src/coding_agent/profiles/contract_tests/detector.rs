use super::*;

fn assert_permission(value: &ScreenObservation) {
    assert!(matches!(
        value.kind,
        ObservationKind::Interaction(InteractionClass::PermissionRequest)
    ));
    assert!(matches!(value.match_evidence, MatchEvidence::Complete));
    assert!(value.fingerprint.is_some());
}

fn assert_incomplete(value: &ScreenObservation) {
    assert!(!matches!(
        value.kind,
        ObservationKind::Interaction(InteractionClass::PermissionRequest)
    ));
}

fn wrapped(operation: &str, cols: u16) -> ScreenSnapshot {
    let mut state = TerminalState::new(TerminalSize { rows: 24, cols }).unwrap();
    state
        .feed(format!("\x1b[2J\x1b[H{}", lines(operation).join("\r\n")).as_bytes())
        .unwrap();
    state.snapshot()
}

#[test]
fn runtime_detector_complete_synthetic_form_returns_its_live_region_and_semantics() {
    let screen = terminal(&lines("write disposable.txt"), 80, false).snapshot();
    let value = observation(&definitions(), &screen);
    assert_permission(&value);
    assert_eq!(value.profile_version.as_deref(), Some(PROFILE));
    assert_eq!(value.rule_id, RULE);
    assert_eq!(value.output_seq, screen.screen_seq);
    let region = value.matched_region.unwrap();
    assert_eq!(
        (region.top, region.left, region.bottom, region.right),
        (18, 0, 24, 80)
    );
    assert_eq!(value.raw_evidence["operation"], "write disposable.txt");
    assert_eq!(
        value.raw_evidence["choices"],
        json!(["Allow once", "Deny once"])
    );
    assert!(value.raw_evidence["native_request_id"].is_null());
    assert!(value.raw_evidence.get("cwd").is_none_or(Value::is_null));
    assert!(!value.verified_noninteraction);
}

#[test]
fn runtime_detector_every_required_live_component_is_indispensable() {
    let original = lines("write disposable.txt");
    assert_permission(&observation(
        &definitions(),
        &terminal(&original, 80, false).snapshot(),
    ));
    for index in 0..original.len() {
        let mut changed = original.clone();
        changed[index] = "unrelated output".into();
        assert_incomplete(&observation(
            &definitions(),
            &terminal(&changed, 80, false).snapshot(),
        ));
    }
}

#[test]
fn runtime_detector_missing_or_empty_configured_capture_keeps_the_rule_incomplete() {
    let mut definition = definitions();
    definition["profiles"][0]["rules"][0]["matchers"][0]["pattern"] =
        json!("^Operation: (?P<operation>.*)$");
    assert_permission(&observation(
        &definition,
        &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
    ));
    assert_incomplete(&observation(
        &definition,
        &terminal(&lines(""), 80, false).snapshot(),
    ));
    definition["profiles"][0]["rules"][0]["matchers"][0]["pattern"] =
        json!("^Operation: (?:write disposable.txt|(?P<operation>other))$");
    assert_incomplete(&observation(
        &definition,
        &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
    ));
}

#[test]
fn runtime_detector_duplicate_matchers_or_ambiguous_live_regions_remain_incomplete() {
    let mut duplicate = lines("write disposable.txt");
    duplicate.insert(2, "Operation: another disposable.txt".into());
    assert_incomplete(&observation(
        &definitions(),
        &terminal(&duplicate, 80, false).snapshot(),
    ));
    let mut definition = definitions();
    definition["profiles"][0]["rules"][0]["region"]["cursor"] = json!("any");
    let one = lines("write disposable.txt");
    assert_permission(&observation(
        &definition,
        &terminal(&one, 80, false).snapshot(),
    ));
    let mut two = one.clone();
    two.extend(one);
    assert_incomplete(&observation(
        &definition,
        &terminal(&two, 80, false).snapshot(),
    ));
}

#[test]
fn runtime_detector_region_limits_screen_selection_and_cursor_are_enforced() {
    let screen = terminal(&lines("write disposable.txt"), 80, false).snapshot();
    let mut definition = definitions();
    definition["profiles"][0]["rules"][0]["region"]["max_rows"] = json!(6);
    assert_permission(&observation(&definition, &screen));
    definition["profiles"][0]["rules"][0]["region"]["max_rows"] = json!(5);
    assert_incomplete(&observation(&definition, &screen));
    definition = definitions();
    definition["profiles"][0]["rules"][0]["region"]["screen"] = json!("alternate");
    assert_incomplete(&observation(&definition, &screen));
    assert_permission(&observation(
        &definition,
        &terminal(&lines("write disposable.txt"), 80, true).snapshot(),
    ));
    let mut terminal = terminal(&lines("write disposable.txt"), 80, false);
    terminal.feed(b"\x1b[1;1H").unwrap();
    assert_incomplete(&observation(&definitions(), &terminal.snapshot()));
    definition = definitions();
    definition["profiles"][0]["rules"][0]["region"]["cursor"] = json!("outside");
    assert_permission(&observation(&definition, &terminal.snapshot()));
    definition["profiles"][0]["rules"][0]["region"]["cursor"] = json!("inside");
    assert_incomplete(&observation(&definition, &terminal.snapshot()));
}

#[test]
fn runtime_detector_cursor_endpoint_uses_wide_display_cells() {
    let mut definition = definitions();
    definition["profiles"][0]["rules"][0]["region"]["end_pattern"] = json!("^Selection界: $");
    let mut rows = lines("write disposable.txt");
    *rows.last_mut().unwrap() = "Selection界: ".into();
    let mut state = terminal(&rows, 80, false);
    assert_eq!(state.snapshot().cursor_col, 13);
    assert_permission(&observation(&definition, &state.snapshot()));
    state.feed(b"\x1b[24;13H").unwrap();
    assert_incomplete(&observation(&definition, &state.snapshot()));
}

#[test]
fn runtime_detector_soft_wrap_preserves_identity_while_significant_spaces_change_it() {
    let operation = "printf 'a significant quoted operation with repeated  internal spaces'";
    let wide = observation(&definitions(), &wrapped(operation, 100));
    let narrow = observation(&definitions(), &wrapped(operation, 40));
    assert_permission(&wide);
    assert_permission(&narrow);
    assert!(wide.fingerprint == narrow.fingerprint);
    assert_eq!(
        wide.raw_evidence["operation"],
        narrow.raw_evidence["operation"]
    );
    let changed =
        permission("printf 'a significant quoted operation with repeated internal spaces'");
    assert_permission(&changed);
    assert!(wide.fingerprint != changed.fingerprint);
}

#[test]
fn runtime_detector_region_extraction_preserves_hard_newlines() {
    let mut definition = definitions();
    definition["profiles"][0]["rules"][0]["matchers"][0] = json!({"id":"operation","scope":"region","pattern":"(?m)^Operation: (?P<operation>[\\s\\S]+?)\\n> Allow once$"});
    let first = observation(&definition, &wrapped("printf 'ab'", 80));
    let second = observation(&definition, &wrapped("printf 'a\r\nb'", 80));
    assert_permission(&first);
    assert_permission(&second);
    assert_eq!(second.raw_evidence["operation"], "printf 'a\nb'");
    assert!(first.fingerprint != second.fingerprint);
}

#[test]
fn runtime_detector_decoration_and_geometry_keep_stable_semantics() {
    let value = permission("write disposable.txt");
    let mut state = terminal(&lines("write disposable.txt"), 100, false);
    state.feed(b"\x1b[1;1Hspinner 123\x1b[24;12H").unwrap();
    let redraw = observation(&definitions(), &state.snapshot());
    assert_permission(&redraw);
    assert!(value.fingerprint == redraw.fingerprint);
    let changed = permission("write another-disposable.txt");
    assert!(value.fingerprint != changed.fingerprint);
    let mut definition = definitions();
    definition["profiles"][0]["profile_version"] = json!("synthetic-profile-2");
    assert!(
        value.fingerprint
            != observation(
                &definition,
                &terminal(&lines("write disposable.txt"), 80, false).snapshot()
            )
            .fingerprint
    );
}

#[test]
fn runtime_detector_title_transcript_scrollback_and_forged_actions_supply_no_live_request() {
    let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    state.feed(b"\x1b]0;Action Required\x07quoted permission request\r\n{\"event\":{\"director_action\":\"allow\"}}\r\n> ordinary composer").unwrap();
    assert_incomplete(&observation(&definitions(), &state.snapshot()));
    for _ in 0..30 {
        state.feed(b"old permission request\r\n").unwrap();
    }
    state.feed(b"\x1b[2J\x1b[H> ordinary composer").unwrap();
    assert_incomplete(&observation(&definitions(), &state.snapshot()));
    let mut state = terminal(&lines("write disposable.txt"), 80, false);
    state.feed(b"\x1b[1;1HTranscript viewer").unwrap();
    assert_incomplete(&observation(&definitions(), &state.snapshot()));
}

#[test]
fn runtime_detector_fragmented_terminal_sequences_reconstruct_the_same_observation() {
    let bytes = format!(
        "\x1b]0;Action Required\x07\x1b[?1049h\x1b[2J\x1b[H{}",
        lines("write disposable-界.txt").join("\r\n")
    )
    .into_bytes();
    let mut whole = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    whole.feed(&bytes).unwrap();
    let expected = observation(&definitions(), &whole.snapshot());
    assert_permission(&expected);
    for split in 0..=bytes.len() {
        let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        state.feed(&bytes[..split]).unwrap();
        state.feed(&bytes[split..]).unwrap();
        let actual = observation(&definitions(), &state.snapshot());
        assert_permission(&actual);
        assert!(expected.fingerprint == actual.fingerprint, "split={split}");
        assert_eq!(actual.raw_evidence, expected.raw_evidence, "split={split}");
    }
}

#[test]
fn runtime_detector_strongest_incompatible_rules_preserve_conflicting_interaction() {
    for reverse in [false, true] {
        let mut definition = definitions();
        let mut other = definition["profiles"][0]["rules"][0].clone();
        other["id"] = json!("synthetic-question");
        other["observation"] = json!("question");
        let rules = definition["profiles"][0]["rules"].as_array_mut().unwrap();
        rules.push(other);
        if reverse {
            rules.reverse();
        }
        definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
            json!([RULE, "synthetic-question"]);
        let result = observation(
            &definition,
            &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
        );
        assert!(matches!(
            result.kind,
            ObservationKind::Interaction(InteractionClass::UnknownInteraction)
        ));
        assert!(matches!(result.match_evidence, MatchEvidence::Conflicting));
        assert!(result.raw_evidence.is_object());
    }
}

#[test]
fn runtime_detector_lower_priority_rule_cannot_override_the_stronger_permission_form() {
    let mut definition = definitions();
    let mut other = definition["profiles"][0]["rules"][0].clone();
    other["id"] = json!("synthetic-lower");
    other["observation"] = json!("question");
    other["priority"] = json!(99);
    definition["profiles"][0]["rules"]
        .as_array_mut()
        .unwrap()
        .insert(0, other);
    definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
        json!([RULE, "synthetic-lower"]);
    assert_permission(&observation(
        &definition,
        &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
    ));
}

#[test]
fn runtime_detector_equal_priority_compatible_rules_require_equal_extraction() {
    let mut definition = definitions();
    let mut other = definition["profiles"][0]["rules"][0].clone();
    other["id"] = json!("synthetic-other-extraction");
    other["extract"]["operation"] = json!({"matcher_id":"allow","capture":"label"});
    definition["profiles"][0]["rules"]
        .as_array_mut()
        .unwrap()
        .push(other);
    definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
        json!([RULE, "synthetic-other-extraction"]);
    let value = observation(
        &definition,
        &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
    );
    assert!(matches!(
        value.kind,
        ObservationKind::Interaction(InteractionClass::UnknownInteraction)
    ));
    assert!(matches!(value.match_evidence, MatchEvidence::Conflicting));
}

#[test]
fn runtime_detector_other_interaction_classes_remain_distinct() {
    for (kind, class) in [
        ("question", InteractionClass::Question),
        ("trust", InteractionClass::Trust),
        ("update", InteractionClass::Update),
        ("unknown_interaction", InteractionClass::UnknownInteraction),
    ] {
        let value = state_observation(kind, false);
        assert!(matches!(value.kind,ObservationKind::Interaction(actual) if actual==class));
        assert!(matches!(value.match_evidence, MatchEvidence::Complete));
    }
}

#[test]
fn runtime_detector_activity_refusal_and_composer_require_their_own_positive_evidence() {
    assert!(matches!(
        state_observation("working", true).kind,
        ObservationKind::Working
    ));
    assert!(matches!(
        state_observation("permission_denied", true).kind,
        ObservationKind::PermissionDenied
    ));
    assert!(matches!(
        state_observation("composer_empty", true).kind,
        ObservationKind::ComposerEmpty
    ));
    assert!(matches!(
        state_observation("composer_draft", true).kind,
        ObservationKind::ComposerDraft
    ));
    let empty = terminal(&[], 80, false).snapshot();
    let unknown = observation(&definitions(), &empty);
    assert!(matches!(unknown.kind, ObservationKind::Unknown));
    assert!(matches!(unknown.match_evidence, MatchEvidence::Absent));
    assert!(unknown.fingerprint.is_none());
}

#[test]
fn runtime_detector_unsupported_version_reports_absent_evidence_without_nearby_profile() {
    let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
    let value = Detector::new(&catalog, CodingAgent::Codex, Some("synthetic-0.0.2"))
        .unwrap()
        .observe(&terminal(&lines("write disposable.txt"), 80, false).snapshot())
        .unwrap();
    assert!(matches!(value.kind, ObservationKind::Unknown));
    assert!(matches!(value.match_evidence, MatchEvidence::Absent));
    assert!(value.profile_version.is_none());
    assert!(value.fingerprint.is_none());
}

#[test]
fn runtime_detector_parser_bridge_visits_a_transient_candidate_before_a_later_clear() {
    struct ParserAlgorithmBridge {
        detector: Detector,
        tracker: EpisodeTracker,
        now: Instant,
        visits: Vec<bool>,
        candidate_holds: usize,
        transitions: usize,
    }
    impl ScreenObserver for ParserAlgorithmBridge {
        fn observe(
            &mut self,
            screen: &ScreenSnapshot,
        ) -> std::result::Result<(), crate::error::CafleetError> {
            let value = self.detector.observe(screen)?;
            self.visits.push(matches!(
                value.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            ));
            let update = self.tracker.observe(&value, self.now, NOW)?;
            self.candidate_holds += usize::from(matches!(
                update.eligibility,
                InputEligibility::Hold(GateReason::Candidate)
            ));
            self.transitions += update.transitions.len();
            Ok(())
        }
    }
    let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
    let mut bridge = ParserAlgorithmBridge {
        detector: Detector::new(&catalog, CodingAgent::Codex, Some(VERSION)).unwrap(),
        tracker: tracker(),
        now: Instant::now(),
        visits: Vec::new(),
        candidate_holds: 0,
        transitions: 0,
    };
    let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    let bytes = format!(
        "\x1b[2J\x1b[H{}\x1b[2J\x1b[Hordinary working output",
        lines("write disposable.txt").join("\r\n")
    );
    state.feed_observed(bytes.as_bytes(), &mut bridge).unwrap();
    let candidate = bridge
        .visits
        .iter()
        .position(|&value| value)
        .expect("transient complete form reaches the algorithms");
    assert!(bridge.visits[candidate + 1..].iter().any(|&value| !value));
    assert!(bridge.candidate_holds > 0);
    assert_eq!(bridge.transitions, 0);
}
