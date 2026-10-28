mod catalog;
mod composer;
mod detector;
use super::CodingAgent;
use crate::broker::runtime::PermissionDecision;
#[cfg(test)]
use crate::error::CafleetError;
#[cfg(test)]
pub(crate) use catalog::inspection_tests::{
    FOCUS_SGR, GATED_REASON_RULE, GATED_RULE, gated_fixture_definition, gated_fixture_form,
    gated_fixture_form_styled, gated_reason_dialog, inspection_fixture_definition,
    inspection_fixture_form,
};
pub(crate) use catalog::{AnswerPlan, AnswerStage, StageGate};
pub use catalog::{NativeCapabilities, ProfileCatalog};
pub(crate) use composer::{ComposerOperation, ComposerPlan};
pub(crate) use detector::*;

#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::broker::runtime::{
        Coverage, InteractionClass, InteractionCloseReason, RuntimeEventKind,
        evidence::EvidencePolicy,
    };
    use crate::runtime::runner::*;
    use serde_json::{Value, json};
    use std::time::{Duration, Instant};

    mod catalog {
        use super::*;

        fn invalid(change: impl FnOnce(&mut Value)) {
            assert!(ProfileCatalog::from_definitions(&definitions()).is_ok());
            let mut value = definitions();
            change(&mut value);
            assert!(
                ProfileCatalog::from_definitions(&value).is_err(),
                "invalid definition accepted"
            );
        }

        fn with_answer(decision: &str) -> Value {
            let mut value = definitions();
            let choice = if decision == "allow" {
                "allow-once"
            } else {
                "deny-once"
            };
            let focus = if decision == "allow" { "allow" } else { "deny" };
            value["profiles"][0]["capabilities"]["answers"] = json!([{
                "decision":decision,"request_rule_id":RULE,"answer_rule_id":format!("synthetic-{decision}-plan"),
                "choice_id":choice,"focus_matcher_id":focus,"fixture_ids":[FIXTURE],
                "stages":[{"bytes_hex":"78","required_rule_id":RULE,"required_choice_id":choice}]
            }]);
            value
        }

        #[test]
        fn runtime_profile_synthetic_example_has_matching_geometry_and_source_digest() {
            use sha2::{Digest, Sha256};
            let bytes = include_bytes!("profiles/contract_tests/synthetic-permission-screen.json");
            let source: Value = serde_json::from_slice(bytes).unwrap();
            let value = definitions();
            assert_eq!(
                Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
                value["fixtures"][0]["sha256"]
            );
            assert_eq!(source["provenance"], "synthetic");
            assert_eq!(source["native_version"], VERSION);
            assert_eq!(source["lines"].as_array().unwrap().len(), 24);
            let rendered = terminal(&lines("write disposable.txt"), 80, false).snapshot();
            assert_eq!(
                (
                    rendered.rows,
                    rendered.cols,
                    rendered.cursor_row,
                    rendered.cursor_col
                ),
                (24, 80, 23, 11)
            );
            assert!(rendered.contents.contains("write disposable.txt"));
            assert_eq!(source["lines"][19], "Operation: write disposable.txt");
            assert!(ProfileCatalog::from_definitions(&value).is_ok());
        }

        #[test]
        fn runtime_profile_capabilities_are_independent_and_exact_version_bound() {
            let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
            let supported = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
            assert_eq!(supported.backend, CodingAgent::Codex);
            assert_eq!(supported.native_version.as_deref(), Some(VERSION));
            assert_eq!(supported.profile_version.as_deref(), Some(PROFILE));
            assert_eq!(supported.detection, Coverage::Tested);
            assert_eq!(supported.composer, Coverage::Unknown);
            assert!(supported.answer_decisions.is_empty());
            for version in [None, Some("synthetic-0.0.2")] {
                let unknown = catalog.capabilities(CodingAgent::Codex, version);
                assert_eq!(unknown.detection, Coverage::Unknown);
                assert_eq!(unknown.composer, Coverage::Unknown);
                assert!(unknown.answer_decisions.is_empty());
                assert!(unknown.capability_reason.is_some());
            }
            for &backend in CodingAgent::all()
                .iter()
                .filter(|&&backend| backend != CodingAgent::Codex)
            {
                assert_eq!(
                    catalog.capabilities(backend, Some(VERSION)).detection,
                    Coverage::Unknown
                );
            }
        }

        #[test]
        fn runtime_profile_composer_provenance_supplies_only_its_declared_component() {
            let value = simple_definition("composer_empty", true);
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            let capabilities = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
            assert_eq!(capabilities.detection, Coverage::Unknown);
            assert_eq!(capabilities.composer, Coverage::Tested);
            assert!(capabilities.answer_decisions.is_empty());
        }

        #[test]
        fn runtime_profile_answer_decisions_have_separate_request_and_version_capabilities() {
            for (name, decision, other) in [
                ("allow", PermissionDecision::Allow, PermissionDecision::Deny),
                ("deny", PermissionDecision::Deny, PermissionDecision::Allow),
            ] {
                let value = with_answer(name);
                let catalog = ProfileCatalog::from_definitions(&value).unwrap();
                assert_eq!(
                    catalog
                        .capabilities(CodingAgent::Codex, Some(VERSION))
                        .answer_decisions,
                    vec![decision]
                );
                let capability = catalog
                    .answer_capability(CodingAgent::Codex, VERSION, RULE, decision)
                    .unwrap()
                    .unwrap();
                assert_eq!(capability.backend(), CodingAgent::Codex);
                assert_eq!(capability.native_version(), VERSION);
                assert_eq!(capability.profile_version(), PROFILE);
                assert_eq!(capability.request_rule_id(), RULE);
                assert_eq!(
                    capability.answer_rule_id(),
                    format!("synthetic-{name}-plan")
                );
                assert_eq!(capability.decision(), decision);
                assert_eq!(capability.fixture_ids(), &[FIXTURE.to_string()]);
                assert!(
                    catalog
                        .answer_capability(CodingAgent::Codex, VERSION, RULE, other)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    catalog
                        .answer_capability(CodingAgent::Codex, "synthetic-0.0.2", RULE, decision)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    catalog
                        .answer_capability(CodingAgent::Codex, VERSION, "another-form", decision)
                        .unwrap()
                        .is_none()
                );
            }
        }

        #[test]
        fn runtime_profile_both_answers_can_be_declared_without_composer_coverage() {
            let mut value = with_answer("allow");
            let deny = with_answer("deny")["profiles"][0]["capabilities"]["answers"][0].clone();
            value["profiles"][0]["capabilities"]["answers"]
                .as_array_mut()
                .unwrap()
                .push(deny);
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            let capabilities = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
            assert_eq!(capabilities.composer, Coverage::Unknown);
            assert!(
                capabilities
                    .answer_decisions
                    .contains(&PermissionDecision::Allow)
            );
            assert!(
                capabilities
                    .answer_decisions
                    .contains(&PermissionDecision::Deny)
            );
        }

        #[test]
        fn runtime_profile_root_schema_and_unknown_fields_fail_initialization() {
            for value in [
                Value::Null,
                json!([]),
                json!({}),
                json!({"schema_version":2,"fixtures":[],"profiles":[]}),
            ] {
                assert!(ProfileCatalog::from_definitions(&value).is_err());
            }
            for path in [
                "",
                "/fixtures/0",
                "/profiles/0",
                "/profiles/0/rules/0",
                "/profiles/0/rules/0/region",
                "/profiles/0/rules/0/matchers/0",
                "/profiles/0/rules/0/extract",
                "/profiles/0/rules/0/extract/choices/0",
                "/profiles/0/rules/0/extract/operation",
                "/profiles/0/capabilities",
                "/profiles/0/capabilities/detection",
            ] {
                invalid(|value| {
                    value
                        .pointer_mut(path)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .insert("unexpected".into(), json!(true));
                });
            }
            invalid(|value| {
                value.as_object_mut().unwrap().remove("fixtures");
            });
            invalid(|value| {
                value["profiles"][0]["rules"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("sensitive");
            });
        }

        #[test]
        fn runtime_profile_enums_geometry_and_real_boolean_types_are_validated() {
            for (path, replacement) in [
                ("/fixtures/0/kind", json!("unverified")),
                ("/fixtures/0/backend", json!("other")),
                ("/fixtures/0/rows", json!(0)),
                ("/fixtures/0/cols", json!(-1)),
                ("/fixtures/0/sha256", json!("A".repeat(64))),
                ("/fixtures/0/source_ref", json!(" ")),
                ("/profiles/0/native_versions", json!([])),
                ("/profiles/0/rules/0/priority", json!(1.5)),
                ("/profiles/0/rules/0/observation", json!("approved")),
                (
                    "/profiles/0/rules/0/verified_noninteraction",
                    json!("false"),
                ),
                ("/profiles/0/rules/0/sensitive", json!(0)),
                ("/profiles/0/rules/0/region/screen", json!("scrollback")),
                ("/profiles/0/rules/0/region/max_rows", json!(0)),
                ("/profiles/0/rules/0/region/cursor", json!("nearby")),
                ("/profiles/0/rules/0/matchers/0/scope", json!("transcript")),
                (
                    "/profiles/0/rules/0/extract/choices/0/scope",
                    json!("forever"),
                ),
            ] {
                invalid(|value| {
                    *value.pointer_mut(path).unwrap() = replacement;
                });
            }
        }

        #[test]
        fn runtime_profile_duplicate_identity_and_ambiguous_backend_version_fail() {
            invalid(|value| {
                let copy = value["fixtures"][0].clone();
                value["fixtures"].as_array_mut().unwrap().push(copy);
            });
            invalid(|value| {
                let copy = value["profiles"][0].clone();
                value["profiles"].as_array_mut().unwrap().push(copy);
            });
            invalid(|value| {
                value["profiles"][0]["native_versions"] = json!([VERSION, VERSION]);
            });
            for array in [
                "/profiles/0/rules",
                "/profiles/0/rules/0/matchers",
                "/profiles/0/rules/0/extract/choices",
            ] {
                invalid(|value| {
                    let items = value.pointer_mut(array).unwrap().as_array_mut().unwrap();
                    items.push(items[0].clone());
                });
            }
        }

        #[test]
        fn runtime_profile_regex_and_capture_relationships_fail_at_initialization() {
            for (path, replacement) in [
                ("/profiles/0/rules/0/region/start_pattern", json!("[")),
                ("/profiles/0/rules/0/region/end_pattern", json!("[")),
                ("/profiles/0/rules/0/matchers/0/pattern", json!("[")),
                (
                    "/profiles/0/rules/0/extract/operation/matcher_id",
                    json!("missing"),
                ),
                (
                    "/profiles/0/rules/0/extract/operation/capture",
                    json!("missing"),
                ),
            ] {
                invalid(|value| {
                    *value.pointer_mut(path).unwrap() = replacement;
                });
            }
            invalid(|value| {
                value["profiles"][0]["rules"][0]["verified_noninteraction"] = json!(true);
            });
        }

        #[test]
        fn runtime_profile_operation_row_margin_must_be_a_nonempty_single_row_string() {
            let mut margined = definitions();
            margined["profiles"][0]["rules"][0]["extract"]["operation"]["row_margin"] = json!(" ");
            assert!(ProfileCatalog::from_definitions(&margined).is_ok());
            for margin in [
                json!(1),
                json!(null),
                json!([" "]),
                json!(""),
                json!("\n"),
                json!(" \n "),
            ] {
                invalid(|value| {
                    value["profiles"][0]["rules"][0]["extract"]["operation"]["row_margin"] = margin;
                });
            }
            invalid(|value| {
                let operation = &mut value["profiles"][0]["rules"][0]["extract"]["operation"];
                operation["row_margin"] = json!(" ");
                operation["reassembly"] = json!("rows");
            });
        }

        #[test]
        fn runtime_profile_capability_provenance_references_are_checked() {
            for (path, replacement) in [
                ("/fixtures/0/backend", json!("claude")),
                ("/fixtures/0/native_version", json!("different-version")),
                (
                    "/profiles/0/capabilities/detection/rule_ids",
                    json!(["missing-rule"]),
                ),
                (
                    "/profiles/0/capabilities/detection/fixture_ids",
                    json!(["missing-fixture"]),
                ),
                ("/profiles/0/capabilities/detection/fixture_ids", json!([])),
            ] {
                invalid(|value| {
                    *value.pointer_mut(path).unwrap() = replacement;
                });
            }
            invalid(|value| {
                value["profiles"][0]["capabilities"]["composer"] =
                    value["profiles"][0]["capabilities"]["detection"].clone();
            });
        }

        #[test]
        fn runtime_profile_each_declared_exact_version_needs_compatible_provenance() {
            invalid(|value| {
                value["profiles"][0]["native_versions"] = json!([VERSION, "synthetic-0.0.2"]);
            });
        }

        #[test]
        fn runtime_profile_answer_scope_focus_stages_and_fixture_relationships_are_checked() {
            let valid = with_answer("allow");
            assert!(ProfileCatalog::from_definitions(&valid).is_ok());
            for (path, replacement) in [
                (
                    "/profiles/0/capabilities/answers/0/decision",
                    json!("approve-session"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/request_rule_id",
                    json!("missing"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/focus_matcher_id",
                    json!("missing"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/choice_id",
                    json!("missing"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/fixture_ids",
                    json!(["missing"]),
                ),
                ("/profiles/0/capabilities/answers/0/stages", json!([])),
                (
                    "/profiles/0/capabilities/answers/0/stages/0/bytes_hex",
                    json!("7"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/stages/0/bytes_hex",
                    json!("AF"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/stages/0/bytes_hex",
                    json!(""),
                ),
                (
                    "/profiles/0/capabilities/answers/0/stages/0/required_rule_id",
                    json!("missing"),
                ),
                (
                    "/profiles/0/capabilities/answers/0/stages/0/required_choice_id",
                    json!("deny-once"),
                ),
                (
                    "/profiles/0/rules/0/extract/choices/0/scope",
                    json!("session"),
                ),
                (
                    "/profiles/0/rules/0/extract/choices/0/scope",
                    json!("persistent"),
                ),
                ("/profiles/0/rules/0/observation", json!("trust")),
            ] {
                let mut value = valid.clone();
                *value.pointer_mut(path).unwrap() = replacement;
                assert!(
                    ProfileCatalog::from_definitions(&value).is_err(),
                    "invalid answer relationship {path}"
                );
            }
        }

        #[test]
        fn runtime_profile_answer_objects_have_exact_fields_and_unique_plan_identities() {
            let valid = with_answer("allow");
            assert!(ProfileCatalog::from_definitions(&valid).is_ok());
            for path in [
                "/profiles/0/capabilities/answers/0",
                "/profiles/0/capabilities/answers/0/stages/0",
            ] {
                let mut value = valid.clone();
                value
                    .pointer_mut(path)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), json!(true));
                assert!(ProfileCatalog::from_definitions(&value).is_err());
            }
            let mut duplicate = valid.clone();
            duplicate["profiles"][0]["capabilities"]["answers"]
                .as_array_mut()
                .unwrap()
                .push(valid["profiles"][0]["capabilities"]["answers"][0].clone());
            assert!(ProfileCatalog::from_definitions(&duplicate).is_err());
        }

        #[test]
        fn runtime_profile_bundled_loader_validates_its_packaged_declarations() {
            let catalog = ProfileCatalog::bundled().unwrap();
            for &backend in CodingAgent::all() {
                let unsupported =
                    catalog.capabilities(backend, Some("synthetic-uninstalled-version"));
                assert_eq!(unsupported.detection, Coverage::Unknown);
                assert_eq!(unsupported.composer, Coverage::Unknown);
                assert!(unsupported.answer_decisions.is_empty());
            }
        }

        #[test]
        fn runtime_profile_private_fingerprint_supports_comparison_without_debug_or_serialization()
        {
            fn comparable<T: Clone + Eq>() {}
            comparable::<SemanticFingerprint>();
            trait AmbiguousIfImplemented<Marker> {
                fn check() {}
            }
            struct DebugImplemented;
            struct SerializeImplemented;
            impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
            impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
            impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
            // A forbidden trait implementation makes the inferred marker ambiguous.
            let _ = <SemanticFingerprint as AmbiguousIfImplemented<_>>::check;
        }
    }
    mod focus_resolution {
        use super::*;

        fn gated_catalog() -> ProfileCatalog {
            ProfileCatalog::from_definitions(&gated_fixture_definition()).unwrap()
        }

        fn screen(bytes: &[u8]) -> ScreenSnapshot {
            let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            state.feed(bytes).unwrap();
            state.snapshot()
        }

        fn detector() -> Detector {
            Detector::new(&gated_catalog(), CodingAgent::Codex, Some(VERSION)).unwrap()
        }

        fn gated_plan(decision: PermissionDecision) -> AnswerPlan {
            gated_catalog()
                .answer_plan(CodingAgent::Codex, VERSION, GATED_RULE, decision)
                .unwrap()
                .unwrap()
        }

        fn matched(bytes: &[u8], decision: PermissionDecision) -> AnswerMatch {
            detector()
                .match_answer(&screen(bytes), &gated_plan(decision))
                .unwrap()
                .expect("the gated form matches regardless of its focus")
        }

        fn focused(bytes: &[u8]) -> Option<String> {
            let allow = matched(bytes, PermissionDecision::Allow);
            let deny = matched(bytes, PermissionDecision::Deny);
            assert_eq!(
                allow.focused_choice_id(),
                deny.focused_choice_id(),
                "both plans read the same focus matcher"
            );
            allow.focused_choice_id().map(str::to_owned)
        }

        #[test]
        fn focus_style_background_resolves_the_one_focused_choice() {
            for (focus, expected) in [("allow", "allow-once"), ("deny", "deny-once")] {
                let bytes = gated_fixture_form("write disposable.txt", focus);
                assert_eq!(focused(&bytes).as_deref(), Some(expected), "{focus}");
                let observation = detector().observe(&screen(&bytes)).unwrap();
                assert_eq!(
                    observation.kind,
                    ObservationKind::Interaction(InteractionClass::PermissionRequest)
                );
                assert_eq!(observation.rule_id, GATED_RULE);
                assert_eq!(observation.match_evidence, MatchEvidence::Complete);
                assert_eq!(
                    observation.raw_evidence["choices"],
                    json!(["Allow once", "Deny once"])
                );
            }
        }

        #[test]
        fn focus_style_background_resolves_no_choice_for_none_several_or_an_unlisted_option() {
            for (case, bytes) in [
                ("no styled label", gated_fixture_form("write disposable.txt", "none")),
                (
                    "an option without a choice entry",
                    gated_fixture_form("write disposable.txt", "always"),
                ),
                (
                    "several styled choices",
                    gated_fixture_form_styled(
                        "write disposable.txt",
                        true,
                        &["Allow once", "Deny once"],
                        "",
                        FOCUS_SGR,
                    ),
                ),
                (
                    "a label only partly styled",
                    format!(
                        "\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: write disposable.txt\r\n  {FOCUS_SGR}Allow\x1b[0m once   Always allow   Deny once\r\nSelection: "
                    )
                    .into_bytes(),
                ),
            ] {
                assert_eq!(focused(&bytes), None, "{case}");
            }
        }

        #[test]
        fn focus_style_background_compares_backgrounds_not_theme_colours() {
            for (case, always, row, focus) in [
                ("indexed theme", true, "\x1b[48;5;236m", "\x1b[48;5;214m"),
                (
                    "rgb theme",
                    true,
                    "\x1b[48;2;30;30;30m",
                    "\x1b[48;2;10;200;10m",
                ),
                ("two-option layout", false, "\x1b[48;2;30;30;30m", FOCUS_SGR),
                ("default row background", false, "", "\x1b[48;5;4m"),
            ] {
                for (label, expected) in [("Allow once", "allow-once"), ("Deny once", "deny-once")]
                {
                    let bytes = gated_fixture_form_styled(
                        "write disposable.txt",
                        always,
                        &[label],
                        row,
                        focus,
                    );
                    assert_eq!(focused(&bytes).as_deref(), Some(expected), "{case} {label}");
                }
                let unfocused =
                    gated_fixture_form_styled("write disposable.txt", always, &[], row, focus);
                assert_eq!(focused(&unfocused), None, "{case} on the row background");
            }
        }

        #[test]
        fn styling_only_focus_change_changes_the_request_focus_but_not_its_semantics() {
            let allow = gated_fixture_form("write disposable.txt", "allow");
            let deny = gated_fixture_form("write disposable.txt", "deny");
            let always = gated_fixture_form("write disposable.txt", "always");
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let first = matched(&allow, decision);
                let moved = matched(&deny, decision);
                let unlisted = matched(&always, decision);
                assert!(first.semantic() == moved.semantic());
                assert_eq!(first.choices(), moved.choices());
                assert_ne!(first.focus(), moved.focus(), "{decision:?}");
                assert_ne!(first.focus(), unlisted.focus(), "{decision:?}");
                assert!(!first.same_request(&moved));
                assert!(first.same_request(&matched(&allow, decision)));
            }
            let detector = detector();
            let a = detector.observe(&screen(&allow)).unwrap();
            let b = detector.observe(&screen(&deny)).unwrap();
            assert!(
                a.fingerprint == b.fingerprint,
                "a focus move keeps the episode identity"
            );
        }

        #[test]
        fn continuation_rule_observes_only_the_empty_reason_dialog() {
            let detector = detector();
            let empty = detector
                .observe(&screen(&gated_reason_dialog(None)))
                .unwrap();
            assert_eq!(empty.kind, ObservationKind::PermissionContinuation);
            assert_eq!(empty.rule_id, GATED_REASON_RULE);
            assert_eq!(empty.match_evidence, MatchEvidence::Complete);
            assert!(!empty.verified_noninteraction);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(
                    detector
                        .match_answer(&screen(&gated_reason_dialog(None)), &gated_plan(decision))
                        .unwrap()
                        .is_none(),
                    "a continuation is not an answerable request form"
                );
            }
            for reason in ["typed reason", "(empty) and more"] {
                let typed = detector
                    .observe(&screen(&gated_reason_dialog(Some(reason))))
                    .unwrap();
                assert_ne!(
                    typed.kind,
                    ObservationKind::PermissionContinuation,
                    "{reason}"
                );
                assert_ne!(typed.rule_id, GATED_REASON_RULE, "{reason}");
            }
        }
    }
    mod continuation_episodes {
        use super::*;

        fn gated(bytes: &[u8], sequence: u64) -> ScreenObservation {
            let catalog = ProfileCatalog::from_definitions(&gated_fixture_definition()).unwrap();
            let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            state.feed(bytes).unwrap();
            let mut value = Detector::new(&catalog, CodingAgent::Codex, Some(VERSION))
                .unwrap()
                .observe(&state.snapshot())
                .unwrap();
            value.output_seq = sequence;
            value.raw_evidence["screen_seq"] = json!(sequence);
            value
        }

        fn request(sequence: u64) -> ScreenObservation {
            gated(
                &gated_fixture_form("write disposable.txt", "deny"),
                sequence,
            )
        }

        fn dialog(sequence: u64) -> ScreenObservation {
            let value = gated(&gated_reason_dialog(None), sequence);
            assert_eq!(value.kind, ObservationKind::PermissionContinuation);
            value
        }

        #[test]
        fn continuation_keeps_the_active_episode_open_and_visible_without_events() {
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &request(1), origin);
            let episode = tracker.active_episode();
            assert!(episode.is_some());
            for (index, millis) in [200u64, 400, 2300, 4500].into_iter().enumerate() {
                let update = tracker
                    .observe(
                        &dialog(2 + index as u64),
                        origin + Duration::from_millis(millis),
                        NOW,
                    )
                    .unwrap();
                assert!(
                    update.transitions.is_empty(),
                    "a continuation emits no event at {millis} ms: {:?}",
                    update.transitions
                );
                assert_eq!(update.visibility, Visibility::Visible, "{millis} ms");
                assert_eq!(
                    update.eligibility,
                    InputEligibility::Hold(GateReason::Interaction),
                    "{millis} ms"
                );
                assert!(
                    update.latest_evidence.is_none(),
                    "the stored request evidence is unchanged"
                );
                assert_eq!(tracker.active_episode(), episode);
            }
            let returned = tracker
                .observe(&request(9), origin + Duration::from_secs(5), NOW)
                .unwrap();
            assert!(returned.transitions.is_empty());
            assert_eq!(returned.visibility, Visibility::Visible);
            assert_eq!(tracker.active_episode(), episode);
        }

        #[test]
        fn continuation_starts_no_obscured_timer_that_a_later_uncertainty_inherits() {
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &request(1), origin);
            tracker
                .observe(&dialog(2), origin + Duration::from_millis(200), NOW)
                .unwrap();
            tracker
                .observe(&dialog(3), origin + Duration::from_millis(3000), NOW)
                .unwrap();
            let blank = gated(b"\x1b[0m\x1b[2J", 4);
            let uncertain = tracker
                .observe(&blank, origin + Duration::from_millis(3100), NOW)
                .unwrap();
            assert!(
                uncertain.transitions.is_empty(),
                "uncertainty after a continuation starts its own two-second interval"
            );
            let obscured = tracker
                .observe(&blank, origin + Duration::from_millis(5100), NOW)
                .unwrap();
            assert_eq!(obscured.transitions.len(), 1);
            assert_eq!(
                obscured.transitions[0].kind,
                RuntimeEventKind::InteractionObscured
            );
        }

        #[test]
        fn continuation_without_an_active_episode_is_an_unknown_screen() {
            let origin = Instant::now();
            let mut continuation = tracker();
            let mut unknown = tracker();
            let blank = gated(b"\x1b[0m\x1b[2J", 1);
            for millis in [0u64, 150, 400, 2500] {
                let at = origin + Duration::from_millis(millis);
                let left = continuation.observe(&dialog(1 + millis), at, NOW).unwrap();
                let mut blank = blank.clone();
                blank.output_seq = 1 + millis;
                blank.raw_evidence["screen_seq"] = json!(1 + millis);
                let right = unknown.observe(&blank, at, NOW).unwrap();
                assert!(left.transitions.is_empty(), "{millis} ms");
                assert_eq!(left.visibility, right.visibility, "{millis} ms");
                assert_eq!(left.eligibility, right.eligibility, "{millis} ms");
                assert_eq!(continuation.active_episode(), None);
            }
        }
    }
    mod gated_catalog {
        use super::*;

        const STAGES: &str = "/profiles/0/capabilities/answers/1/stages";
        const ALLOW_STAGES: &str = "/profiles/0/capabilities/answers/0/stages";
        const REASON: &str = "/profiles/0/rules/1";

        fn rejected(case: &str, change: impl FnOnce(&mut Value)) {
            assert!(ProfileCatalog::from_definitions(&gated_fixture_definition()).is_ok());
            let mut value = gated_fixture_definition();
            change(&mut value);
            assert!(
                ProfileCatalog::from_definitions(&value).is_err(),
                "{case} must fail catalog loading"
            );
        }

        fn stage(bytes: &str, choice: &str, gate: (&str, &str)) -> Value {
            let mut stage = json!({"bytes_hex":bytes,"required_rule_id":GATED_RULE,"required_choice_id":choice});
            stage[gate.0] = json!(gate.1);
            stage
        }

        fn plan(decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::from_definitions(&gated_fixture_definition())
                .unwrap()
                .answer_plan(CodingAgent::Codex, VERSION, GATED_RULE, decision)
                .unwrap()
                .expect("gated synthetic plan")
        }

        #[test]
        fn gated_catalog_retains_each_stage_gate_in_declared_order() {
            let allow = plan(PermissionDecision::Allow);
            assert_eq!(allow.stages().len(), 1);
            assert_eq!(allow.stages()[0].bytes(), b"\r");
            assert_eq!(
                allow.stages()[0].required_focus_choice_id(),
                Some("allow-once")
            );
            assert_eq!(allow.stages()[0].required_screen_rule_id(), None);
            let deny = plan(PermissionDecision::Deny);
            let gates = deny
                .stages()
                .iter()
                .map(|stage| {
                    (
                        stage.bytes().to_vec(),
                        stage.required_choice_id().to_owned(),
                        stage.required_focus_choice_id().map(str::to_owned),
                        stage.required_screen_rule_id().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                gates,
                [
                    (
                        b"h".to_vec(),
                        "deny-once".into(),
                        Some("allow-once".into()),
                        None
                    ),
                    (
                        b"\r".to_vec(),
                        "deny-once".into(),
                        Some("deny-once".into()),
                        None
                    ),
                    (
                        b"\r".to_vec(),
                        "deny-once".into(),
                        None,
                        Some(GATED_REASON_RULE.into())
                    ),
                ]
            );
        }

        #[test]
        fn gated_catalog_keeps_existing_ungated_plans_without_gates() {
            let synthetic = ProfileCatalog::from_definitions(&inspection_fixture_definition())
                .unwrap()
                .answer_plan(
                    CodingAgent::Codex,
                    VERSION,
                    "synthetic-permission",
                    PermissionDecision::Allow,
                )
                .unwrap()
                .unwrap();
            let bundled = ProfileCatalog::bundled().unwrap();
            let mut plans = vec![synthetic];
            for (backend, version, rule) in [
                (CodingAgent::Claude, "2.1.302", "claude-bash-permission"),
                (CodingAgent::Codex, "0.163.0", "codex-exec-approval"),
            ] {
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    plans.push(
                        bundled
                            .answer_plan(backend, version, rule, decision)
                            .unwrap()
                            .expect("bundled ungated plan"),
                    );
                }
            }
            for plan in plans {
                for stage in plan.stages() {
                    assert_eq!(stage.required_focus_choice_id(), None);
                    assert_eq!(stage.required_screen_rule_id(), None);
                }
            }
        }

        #[test]
        fn gated_catalog_rejects_a_focus_gate_on_an_unknown_choice_or_a_malformed_gate() {
            for (case, value) in [
                ("unknown choice", json!("always-allow")),
                ("non-string gate", json!(1)),
                ("null gate", Value::Null),
                ("blank gate", json!(" ")),
            ] {
                rejected(case, |definition| {
                    *definition
                        .pointer_mut(&format!("{STAGES}/0/required_focus_choice_id"))
                        .unwrap() = value;
                });
            }
            rejected("unknown continuation rule", |definition| {
                *definition
                    .pointer_mut(&format!("{STAGES}/2/required_screen_rule_id"))
                    .unwrap() = json!("missing-rule");
            });
            rejected("both gates on one stage", |definition| {
                definition.pointer_mut(&format!("{STAGES}/1")).unwrap()["required_screen_rule_id"] =
                    json!(GATED_REASON_RULE);
            });
            rejected("unknown stage field", |definition| {
                definition.pointer_mut(&format!("{STAGES}/0")).unwrap()["required_focus"] =
                    json!("allow-once");
            });
        }

        #[test]
        fn gated_catalog_rejects_a_plan_that_gates_only_some_stages() {
            rejected("ungated first stage", |definition| {
                definition
                    .pointer_mut(&format!("{STAGES}/0"))
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("required_focus_choice_id");
            });
            rejected("ungated middle stage", |definition| {
                definition
                    .pointer_mut(&format!("{STAGES}/1"))
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("required_focus_choice_id");
            });
            rejected("ungated final stage", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([
                    stage("68", "deny-once", ("required_focus_choice_id", "allow-once")),
                    {"bytes_hex":"0d","required_rule_id":GATED_RULE,"required_choice_id":"deny-once"}
                ]);
            });
        }

        #[test]
        fn gated_catalog_rejects_a_final_gate_on_another_choice() {
            rejected("allow ends on the deny focus", |definition| {
                *definition.pointer_mut(ALLOW_STAGES).unwrap() = json!([stage(
                    "0d",
                    "allow-once",
                    ("required_focus_choice_id", "deny-once")
                )]);
            });
            rejected("deny ends on the allow focus", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([stage(
                    "0d",
                    "deny-once",
                    ("required_focus_choice_id", "allow-once")
                )]);
            });
            let mut ends_on_deny_focus = gated_fixture_definition();
            *ends_on_deny_focus.pointer_mut(STAGES).unwrap() = json!([
                stage(
                    "68",
                    "deny-once",
                    ("required_focus_choice_id", "allow-once")
                ),
                stage("0d", "deny-once", ("required_focus_choice_id", "deny-once"))
            ]);
            assert!(
                ProfileCatalog::from_definitions(&ends_on_deny_focus).is_ok(),
                "a final focus gate on the answer's own choice is valid"
            );
        }

        #[test]
        fn gated_catalog_rejects_screen_gates_without_their_continuation_succession() {
            rejected("screen gate on the request rule itself", |definition| {
                *definition
                    .pointer_mut(&format!("{STAGES}/2/required_screen_rule_id"))
                    .unwrap() = json!(GATED_RULE);
            });
            rejected("screen gate as the first stage", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([stage(
                    "0d",
                    "deny-once",
                    ("required_screen_rule_id", GATED_REASON_RULE)
                )]);
            });
            rejected("screen gate after a stage on another focus", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([
                    stage(
                        "68",
                        "deny-once",
                        ("required_focus_choice_id", "allow-once")
                    ),
                    stage(
                        "0d",
                        "deny-once",
                        ("required_screen_rule_id", GATED_REASON_RULE)
                    )
                ]);
            });
            rejected("continuation of an unrelated request rule", |definition| {
                let mut other = definition["profiles"][0]["rules"][0].clone();
                other["id"] = json!("synthetic-other-request");
                other["region"]["start_pattern"] = json!("^Synthetic other request$");
                other["matchers"][0]["pattern"] = json!("^Synthetic other request$");
                definition["profiles"][0]["rules"]
                    .as_array_mut()
                    .unwrap()
                    .push(other);
                definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
                    json!([GATED_RULE, "synthetic-other-request"]);
                definition.pointer_mut(REASON).unwrap()["continues_rule_id"] =
                    json!("synthetic-other-request");
            });
            rejected("final continuation of another choice", |definition| {
                definition.pointer_mut(REASON).unwrap()["continues_choice_id"] =
                    json!("allow-once");
                *definition.pointer_mut(STAGES).unwrap() = json!([
                    stage(
                        "0d",
                        "deny-once",
                        ("required_focus_choice_id", "allow-once")
                    ),
                    stage(
                        "0d",
                        "deny-once",
                        ("required_screen_rule_id", GATED_REASON_RULE)
                    )
                ]);
            });
        }

        #[test]
        fn gated_catalog_validates_the_continuation_rule_declaration() {
            rejected("continuation with an operation", |definition| {
                let rule = definition.pointer_mut(REASON).unwrap();
                rule["matchers"].as_array_mut().unwrap().push(
                    json!({"id":"operation","scope":"line","pattern":"^Reason: (?P<operation>.+)$"}),
                );
                rule["extract"]["operation"] =
                    json!({"matcher_id":"operation","capture":"operation"});
            });
            rejected("continuation with choices", |definition| {
                let rule = definition.pointer_mut(REASON).unwrap();
                rule["matchers"].as_array_mut().unwrap().push(
                    json!({"id":"confirm","scope":"line","pattern":"^Confirm: (?P<label>enter)$"}),
                );
                rule["extract"]["choices"] = json!([{"id":"confirm","label":{"matcher_id":"confirm","capture":"label"},"scope":"single_request"}]);
            });
            rejected("continuation as an answer request rule", |definition| {
                definition["profiles"][0]["capabilities"]["answers"][0]["request_rule_id"] =
                    json!(GATED_REASON_RULE);
            });
            rejected("continuation verifying noninteraction", |definition| {
                definition.pointer_mut(REASON).unwrap()["verified_noninteraction"] = json!(true);
            });
            for key in ["continues_rule_id", "continues_choice_id", "fixture_ids"] {
                rejected(&format!("continuation without {key}"), |definition| {
                    definition
                        .pointer_mut(REASON)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(key);
                });
            }
            for (case, key, value) in [
                (
                    "unknown continued rule",
                    "continues_rule_id",
                    json!("missing-rule"),
                ),
                (
                    "unknown continued choice",
                    "continues_choice_id",
                    json!("missing-choice"),
                ),
                ("no cited fixture", "fixture_ids", json!([])),
                (
                    "unknown cited fixture",
                    "fixture_ids",
                    json!(["missing-fixture"]),
                ),
                (
                    "synthetic cited fixture",
                    "fixture_ids",
                    json!(["synthetic-permission-1"]),
                ),
            ] {
                rejected(case, |definition| {
                    definition.pointer_mut(REASON).unwrap()[key] = value;
                });
            }
            rejected("continuation of a non-permission rule", |definition| {
                definition["profiles"][0]["rules"][0]["observation"] = json!("question");
                definition["profiles"][0]["capabilities"]["answers"] = json!([]);
            });
            for key in ["continues_rule_id", "continues_choice_id", "fixture_ids"] {
                rejected(
                    &format!("{key} on a permission request rule"),
                    |definition| {
                        let value = definition.pointer(REASON).unwrap()[key].clone();
                        definition["profiles"][0]["rules"][0][key] = value;
                    },
                );
            }
        }

        #[test]
        fn gated_catalog_requires_a_background_focus_matcher_over_line_scoped_labels() {
            rejected("focus matcher without focus_style", |definition| {
                definition["profiles"][0]["rules"][0]["matchers"][2]
                    .as_object_mut()
                    .unwrap()
                    .remove("focus_style");
            });
            for style in [json!("bold"), json!(""), Value::Null, json!(true)] {
                rejected(&format!("focus_style {style}"), |definition| {
                    definition["profiles"][0]["rules"][0]["matchers"][2]["focus_style"] = style;
                });
            }
            rejected("region-scoped choice label", |definition| {
                definition["profiles"][0]["rules"][0]["matchers"][3]["scope"] = json!("region");
            });
            let mut textual = definitions();
            textual["profiles"][0]["rules"][0]["matchers"][1]["focus_style"] = json!("background");
            assert!(
                ProfileCatalog::from_definitions(&textual).is_ok(),
                "an ungated profile may declare a background focus matcher"
            );
        }

        #[test]
        fn every_backend_rejects_an_escape_byte_in_any_answer_stage() {
            for bytes in ["1b", "0d1b", "1b5b41"] {
                rejected(&format!("gated stage {bytes}"), |definition| {
                    *definition
                        .pointer_mut(&format!("{STAGES}/0/bytes_hex"))
                        .unwrap() = json!(bytes);
                });
                for backend in ["codex", "claude", "opencode"] {
                    let mut value = definitions();
                    value["fixtures"][0]["backend"] = json!(backend);
                    value["profiles"][0]["backend"] = json!(backend);
                    value["profiles"][0]["capabilities"]["answers"] = json!([{
                        "decision":"deny","request_rule_id":RULE,"answer_rule_id":"synthetic-escape",
                        "choice_id":"deny-once","focus_matcher_id":"deny","fixture_ids":[FIXTURE],
                        "stages":[{"bytes_hex":"6e","required_rule_id":RULE,"required_choice_id":"deny-once"}]
                    }]);
                    assert!(
                        ProfileCatalog::from_definitions(&value).is_ok(),
                        "{backend} baseline"
                    );
                    value["profiles"][0]["capabilities"]["answers"][0]["stages"][0]["bytes_hex"] =
                        json!(bytes);
                    assert!(
                        ProfileCatalog::from_definitions(&value).is_err(),
                        "{backend} ungated stage {bytes}"
                    );
                }
            }
        }
    }
    mod detector {
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
            definition["profiles"][0]["rules"][0]["region"]["end_pattern"] =
                json!("^Selection界: $");
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
            let operation =
                "printf 'a significant quoted operation with repeated  internal spaces'";
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
        fn runtime_detector_title_transcript_scrollback_and_forged_actions_supply_no_live_request()
        {
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
    }
    mod episodes {
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
            rule["extract"]["native_request_id"] =
                json!({"matcher_id":"native","capture":"identity"});
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
            assert!(matches!(
                returned.eligibility,
                InputEligibility::Hold(GateReason::Interaction)
            ));
            assert!(returned.latest_evidence.is_none());
        }

        #[test]
        fn runtime_episode_equivalent_redraw_schedules_no_refresh_and_no_opening() {
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
            for (sequence, millis) in [(2, 200), (3, 300)] {
                let update = tracker
                    .observe(
                        &sequenced(redraw.clone(), sequence),
                        origin + Duration::from_millis(millis),
                        NOW,
                    )
                    .unwrap();
                assert!(update.transitions.is_empty());
                assert!(matches!(update.visibility, Visibility::Visible));
                assert!(update.latest_evidence.is_none());
            }
        }

        fn detailed(detail: &str, sequence: u64) -> ScreenObservation {
            let mut rows = lines("write disposable.txt");
            rows.insert(5, format!("Detail: {detail}"));
            sequenced(
                observation(&definitions(), &terminal(&rows, 80, false).snapshot()),
                sequence,
            )
        }

        #[test]
        fn runtime_episode_excerpt_only_change_refreshes_policy_evidence_without_another_opening() {
            let secret = "synthetic-secret-194";
            let policy = EvidencePolicy::redacted(
                &json!({"schema_version":1,"patterns":[secret],"sensitive_rules":[]}),
                &[RULE],
            )
            .unwrap();
            let mut tracker = EpisodeTracker::new(1, 1, policy).unwrap();
            let origin = Instant::now();
            let first = detailed("first", 1);
            assert!(first.fingerprint == detailed("second", 1).fingerprint);
            open(&mut tracker, &first, origin);
            let unchanged = tracker
                .observe(
                    &detailed("first", 2),
                    origin + Duration::from_millis(200),
                    NOW,
                )
                .unwrap();
            assert!(unchanged.transitions.is_empty());
            assert!(unchanged.latest_evidence.is_none());
            let changed = tracker
                .observe(
                    &detailed("second", 3),
                    origin + Duration::from_millis(300),
                    NOW,
                )
                .unwrap();
            assert!(changed.transitions.is_empty());
            let refresh = changed.latest_evidence.unwrap();
            assert_eq!(
                (refresh.run_id, refresh.episode_seq, refresh.output_seq),
                (1, 1, 3)
            );
            assert_eq!(refresh.evidence_json["screen_seq"], 3);
            assert!(
                refresh.evidence_json["excerpt"]
                    .as_str()
                    .unwrap()
                    .contains("Detail: second")
            );
            let secret_detail = tracker
                .observe(
                    &detailed(secret, 4),
                    origin + Duration::from_millis(400),
                    NOW,
                )
                .unwrap();
            assert!(secret_detail.transitions.is_empty());
            let refresh = secret_detail.latest_evidence.unwrap();
            assert_eq!(refresh.evidence_json["excerpt"], json!("[redacted]"));
            assert!(!refresh.evidence_json.to_string().contains(secret));
            let settled = tracker
                .observe(
                    &detailed(secret, 5),
                    origin + Duration::from_millis(500),
                    NOW,
                )
                .unwrap();
            assert!(settled.latest_evidence.is_none());
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
        fn runtime_episode_refusal_requires_stability_and_retains_deduplication_across_uncertainty()
        {
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
                let update = tracker
                    .observe(&redraw, origin + Duration::from_millis(200), NOW)
                    .unwrap();
                assert!(update.transitions.is_empty());
                assert!(update.latest_evidence.is_none());
            }
        }

        #[test]
        fn runtime_episode_metadata_removes_each_screen_field_and_leaves_no_redraw_refresh() {
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
            for (sequence, detail) in [(2, "first"), (3, secret)] {
                let mut redraw = secret_observation(secret, false, 100, sequence);
                redraw.raw_evidence["excerpt"] = json!(format!("Detail: {detail}"));
                let update = tracker
                    .observe(
                        &redraw,
                        origin + Duration::from_millis(100 + sequence * 100),
                        NOW,
                    )
                    .unwrap();
                assert!(update.transitions.is_empty());
                assert!(update.latest_evidence.is_none());
            }
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
    }
    mod executable_composer {
        use super::*;
        use crate::coding_agent::{ComposerOperation, ComposerPlan, ProfileCatalog};

        const EMPTY: &str = "synthetic-composer_empty";
        const DRAFT: &str = "synthetic-composer_draft";
        const PROMPT_FIXTURE: &str = "synthetic-prompt-plan";
        const EXEC_FIXTURE: &str = "synthetic-exec-plan";
        const VERSION_TWO: &str = "synthetic-0.0.2";

        fn definition(prompt: Option<&str>, exec: Option<&str>) -> Value {
            let mut value = simple_definition("composer_empty", true);
            let draft =
                simple_definition("composer_draft", true)["profiles"][0]["rules"][0].clone();
            value["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .push(draft);
            for id in [PROMPT_FIXTURE, EXEC_FIXTURE] {
                let mut fixture = value["fixtures"][0].clone();
                fixture["id"] = json!(id);
                value["fixtures"].as_array_mut().unwrap().push(fixture);
            }
            let prompt = prompt
                .map(|submit| json!({"prefix_hex":"","submit_hex":submit,"fixture_ids":[PROMPT_FIXTURE]}));
            let exec = exec.map(
                |submit| json!({"prefix_hex":"2120","submit_hex":submit,"fixture_ids":[EXEC_FIXTURE]}),
            );
            value["profiles"][0]["capabilities"]["composer"] = json!({
                "rule_ids":[DRAFT,EMPTY],"fixture_ids":[FIXTURE],
                "submission":{"prompt":prompt,"exec":exec}
            });
            value
        }

        fn operation_name(operation: ComposerOperation) -> &'static str {
            match operation {
                ComposerOperation::Prompt => "prompt",
                ComposerOperation::Exec => "exec",
            }
        }

        fn operation_prefix(operation: ComposerOperation) -> &'static [u8] {
            match operation {
                ComposerOperation::Prompt => b"",
                ComposerOperation::Exec => b"! ",
            }
        }

        fn plan(catalog: &ProfileCatalog, operation: ComposerOperation) -> ComposerPlan {
            catalog
                .composer_plan(CodingAgent::Codex, VERSION, operation)
                .unwrap()
                .expect("valid selected operation has an executable plan")
        }

        fn capabilities(
            catalog: &ProfileCatalog,
            backend: CodingAgent,
            version: Option<&str>,
            observation: Coverage,
            operations: (Coverage, Coverage),
        ) {
            let actual = catalog.capabilities(backend, version);
            assert_eq!(actual.composer, observation);
            assert_eq!(actual.prompt_submission, operations.0);
            assert_eq!(actual.native_exec_submission, operations.1);
        }

        fn metadata(plan: &ComposerPlan, fixtures: &[String]) {
            assert_eq!(plan.backend(), CodingAgent::Codex);
            assert_eq!(plan.native_version(), VERSION);
            assert_eq!(plan.profile_version(), PROFILE);
            assert_eq!(plan.rule_ids(), &[DRAFT.to_owned(), EMPTY.to_owned()]);
            assert_eq!(plan.fixture_ids(), fixtures);
        }

        fn reject(
            good: &Value,
            bad: &Value,
            operation: ComposerOperation,
            label: &str,
        ) -> CafleetError {
            let catalog = ProfileCatalog::from_definitions(good).unwrap();
            let _ = plan(&catalog, operation)
                .encode("same-operation positive control")
                .unwrap();
            match ProfileCatalog::from_definitions(bad) {
                Err(error) => error,
                Ok(_) => panic!("invalid executable definition rejected: {label}"),
            }
        }

        fn expected(operation: ComposerOperation, literal: &str, submit: &[u8]) -> Vec<u8> {
            let mut bytes = operation_prefix(operation).to_vec();
            bytes.extend_from_slice(literal.as_bytes());
            bytes.extend_from_slice(submit);
            bytes
        }

        #[test]
        fn step5_composer_e1_observation_only_definition_preserves_observation_without_executable_plans()
         {
            let value = simple_definition("composer_empty", true);
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            capabilities(
                &catalog,
                CodingAgent::Codex,
                Some(VERSION),
                Coverage::Tested,
                (Coverage::Unknown, Coverage::Unknown),
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(
                    catalog
                        .composer_plan(CodingAgent::Codex, VERSION, operation)
                        .unwrap()
                        .is_none()
                );
            }
            assert_eq!(
                catalog
                    .capabilities(CodingAgent::Codex, Some(VERSION))
                    .detection,
                Coverage::Unknown
            );
            assert!(
                catalog
                    .capabilities(CodingAgent::Codex, Some(VERSION))
                    .answer_decisions
                    .is_empty()
            );
        }

        #[test]
        fn step5_composer_e2_nullable_operations_and_absent_composer_keep_executable_coverage_unknown()
         {
            for value in [definition(None, None), definitions()] {
                let catalog = ProfileCatalog::from_definitions(&value).unwrap();
                let observation = if value["profiles"][0]["capabilities"]["composer"].is_null() {
                    Coverage::Unknown
                } else {
                    Coverage::Tested
                };
                capabilities(
                    &catalog,
                    CodingAgent::Codex,
                    Some(VERSION),
                    observation,
                    (Coverage::Unknown, Coverage::Unknown),
                );
                for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                    assert!(
                        catalog
                            .composer_plan(CodingAgent::Codex, VERSION, operation)
                            .unwrap()
                            .is_none()
                    );
                }
            }
        }

        #[test]
        fn step5_composer_e3_prompt_plan_retains_exact_selected_metadata_and_independent_coverage()
        {
            let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
            metadata(
                &plan(&catalog, ComposerOperation::Prompt),
                &[PROMPT_FIXTURE.to_owned()],
            );
            capabilities(
                &catalog,
                CodingAgent::Codex,
                Some(VERSION),
                Coverage::Tested,
                (Coverage::Tested, Coverage::Unknown),
            );
            assert!(
                catalog
                    .composer_plan(CodingAgent::Codex, VERSION, ComposerOperation::Exec)
                    .unwrap()
                    .is_none()
            );
            let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
            assert_eq!(actual.detection, Coverage::Unknown);
            assert!(actual.answer_decisions.is_empty());
        }

        #[test]
        fn step5_composer_e4_exec_plan_retains_exact_selected_metadata_and_independent_coverage() {
            let catalog =
                ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
            metadata(
                &plan(&catalog, ComposerOperation::Exec),
                &[EXEC_FIXTURE.to_owned()],
            );
            capabilities(
                &catalog,
                CodingAgent::Codex,
                Some(VERSION),
                Coverage::Tested,
                (Coverage::Unknown, Coverage::Tested),
            );
            assert!(
                catalog
                    .composer_plan(CodingAgent::Codex, VERSION, ComposerOperation::Prompt)
                    .unwrap()
                    .is_none()
            );
            let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
            assert_eq!(actual.detection, Coverage::Unknown);
            assert!(actual.answer_decisions.is_empty());
        }

        #[test]
        fn step5_composer_e5_operation_provenance_and_exact_selection_do_not_authorize_other_profiles()
         {
            let mut value = definition(Some("7e"), Some("6b6c"));
            value["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .push(definitions()["profiles"][0]["rules"][0].clone());
            value["profiles"][0]["capabilities"]["detection"] =
                json!({"rule_ids":[RULE],"fixture_ids":[FIXTURE]});
            let answers = [("allow", "allow-once", "allow"), ("deny", "deny-once", "deny")].map(|(decision, choice, focus)| {
                json!({"decision":decision,"request_rule_id":RULE,"answer_rule_id":format!("synthetic-{decision}-plan"),
                    "choice_id":choice,"focus_matcher_id":focus,"fixture_ids":[FIXTURE],
                    "stages":[{"bytes_hex":"78","required_rule_id":RULE,"required_choice_id":choice}]})
            });
            value["profiles"][0]["capabilities"]["answers"] = json!(answers);
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            let prompt = plan(&catalog, ComposerOperation::Prompt);
            let exec = plan(&catalog, ComposerOperation::Exec);
            metadata(&prompt, &[PROMPT_FIXTURE.to_owned()]);
            metadata(&exec, &[EXEC_FIXTURE.to_owned()]);
            assert_eq!(prompt.encode("literal").unwrap(), b"literal~");
            assert_eq!(exec.encode("literal").unwrap(), b"! literalkl");
            capabilities(
                &catalog,
                CodingAgent::Codex,
                Some(VERSION),
                Coverage::Tested,
                (Coverage::Tested, Coverage::Tested),
            );
            let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
            assert_eq!(actual.detection, Coverage::Tested);
            assert_eq!(
                actual.answer_decisions,
                [PermissionDecision::Allow, PermissionDecision::Deny]
            );
            for version in [None, Some(VERSION_TWO), Some("synthetic-0.0.1 ")] {
                capabilities(
                    &catalog,
                    CodingAgent::Codex,
                    version,
                    Coverage::Unknown,
                    (Coverage::Unknown, Coverage::Unknown),
                );
                if let Some(version) = version {
                    for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                        assert!(
                            catalog
                                .composer_plan(CodingAgent::Codex, version, operation)
                                .unwrap()
                                .is_none()
                        );
                    }
                }
            }
            let bundled = ProfileCatalog::bundled().unwrap();
            for &backend in CodingAgent::all() {
                capabilities(
                    &bundled,
                    backend,
                    Some(VERSION),
                    Coverage::Unknown,
                    (Coverage::Unknown, Coverage::Unknown),
                );
                for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                    assert!(
                        bundled
                            .composer_plan(backend, VERSION, operation)
                            .unwrap()
                            .is_none()
                    );
                    if backend != CodingAgent::Codex {
                        assert!(
                            catalog
                                .composer_plan(backend, VERSION, operation)
                                .unwrap()
                                .is_none()
                        );
                        capabilities(
                            &catalog,
                            backend,
                            Some(VERSION),
                            Coverage::Unknown,
                            (Coverage::Unknown, Coverage::Unknown),
                        );
                    }
                }
            }
        }

        #[test]
        fn step5_composer_e6_prompt_preserves_literal_utf8_between_empty_prefix_and_tested_submit_bytes()
         {
            let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
            let literal = "  literal 世界 '$x' `backticks` \\\"quotes\\\"  ";
            let bytes = plan(&catalog, ComposerOperation::Prompt)
                .encode(literal)
                .unwrap();
            assert_eq!(bytes, expected(ComposerOperation::Prompt, literal, b"~"));
            assert_eq!(&bytes[..literal.len()], literal.as_bytes());
        }

        #[test]
        fn step5_composer_e7_exec_owns_exact_bang_space_prefix_without_changing_the_caller_literal()
        {
            let catalog =
                ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
            let literal = "printf '%s' 世界 '$x' `literal`";
            let bytes = plan(&catalog, ComposerOperation::Exec)
                .encode(literal)
                .unwrap();
            assert_eq!(bytes, expected(ComposerOperation::Exec, literal, b"kl"));
            assert_eq!(&bytes[..2], b"! ");
            assert_eq!(&bytes[2..2 + literal.len()], literal.as_bytes());
        }

        #[test]
        fn step5_composer_e8_submit_byte_limits_are_inclusive_before_plan_construction() {
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                for length in [1, 32] {
                    let hex = "7a".repeat(length);
                    let good = match operation {
                        ComposerOperation::Prompt => definition(Some(&hex), None),
                        ComposerOperation::Exec => definition(None, Some(&hex)),
                    };
                    let catalog = ProfileCatalog::from_definitions(&good).unwrap();
                    assert_eq!(
                        plan(&catalog, operation).encode("literal").unwrap(),
                        expected(operation, "literal", &vec![b'z'; length])
                    );
                    for invalid in [0, 33] {
                        let mut bad = good.clone();
                        bad["profiles"][0]["capabilities"]["composer"]["submission"]
                            [operation_name(operation)]["submit_hex"] = json!("7a".repeat(invalid));
                        reject(&good, &bad, operation, "decoded submit length");
                    }
                }
            }
        }

        #[test]
        fn step5_composer_e9_prefix_shape_hex_and_operation_identity_are_validated() {
            let good = definition(Some("7e"), Some("6b6c"));
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let values = match operation {
                    ComposerOperation::Prompt => vec![
                        json!("00"),
                        json!("2120"),
                        json!("A0"),
                        json!("0"),
                        json!("gg"),
                    ],
                    ComposerOperation::Exec => vec![
                        json!(""),
                        json!("21"),
                        json!("2121"),
                        json!("212000"),
                        json!("212"),
                        json!("212A"),
                        json!("212g"),
                    ],
                };
                for invalid in values.into_iter().chain([
                    Value::Null,
                    json!(17),
                    json!(true),
                    json!([]),
                    json!({}),
                ]) {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["prefix_hex"] = invalid;
                    reject(&good, &bad, operation, "operation prefix");
                }
            }
        }

        #[test]
        fn step5_composer_e10_submit_hex_requires_nonempty_even_lowercase_hex_and_string_type() {
            let good = definition(Some("7e"), Some("6b6c"));
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                for invalid in [
                    json!(""),
                    json!("0"),
                    json!("ABCDEF"),
                    json!("zz"),
                    json!("7e "),
                    json!("é"),
                    Value::Null,
                    json!(17),
                    json!(false),
                    json!([]),
                    json!({}),
                ] {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["submit_hex"] = invalid;
                    reject(&good, &bad, operation, "submit hex");
                }
            }
        }

        #[test]
        fn step5_composer_e11_optional_submission_and_operation_objects_have_exact_complete_fields()
        {
            let good = definition(Some("7e"), Some("6b6c"));
            for field in ["rule_ids", "fixture_ids"] {
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field)
                    .unwrap();
                reject(
                    &good,
                    &bad,
                    ComposerOperation::Prompt,
                    "missing composer field",
                );
            }
            let mut extra = good.clone();
            extra["profiles"][0]["capabilities"]["composer"]["unknown"] = json!(true);
            reject(
                &good,
                &extra,
                ComposerOperation::Prompt,
                "unknown composer field",
            );
            let submission = "/profiles/0/capabilities/composer/submission";
            for invalid in [
                Value::Null,
                json!(true),
                json!(17),
                json!("object required"),
                json!([]),
            ] {
                let mut bad = good.clone();
                *bad.pointer_mut(submission).unwrap() = invalid;
                reject(&good, &bad, ComposerOperation::Prompt, "submission object");
            }
            for field in ["prompt", "exec"] {
                let mut bad = good.clone();
                bad.pointer_mut(submission)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(field)
                    .unwrap();
                reject(&good, &bad, ComposerOperation::Prompt, "missing operation");
            }
            let mut extra = good.clone();
            extra.pointer_mut(submission).unwrap()["unknown"] = json!(true);
            reject(
                &good,
                &extra,
                ComposerOperation::Prompt,
                "unknown submission field",
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let pointer = format!("{submission}/{}", operation_name(operation));
                for field in ["prefix_hex", "submit_hex", "fixture_ids"] {
                    let mut bad = good.clone();
                    bad.pointer_mut(&pointer)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(field)
                        .unwrap();
                    reject(&good, &bad, operation, "missing plan field");
                }
                let mut extra = good.clone();
                extra.pointer_mut(&pointer).unwrap()["unknown"] = json!(true);
                reject(&good, &extra, operation, "unknown plan field");
                for invalid in [json!(true), json!(17), json!("plan required"), json!([])] {
                    let mut bad = good.clone();
                    *bad.pointer_mut(&pointer).unwrap() = invalid;
                    reject(&good, &bad, operation, "plan object");
                }
            }
        }

        #[test]
        fn step5_composer_e12_each_operation_requires_its_own_valid_fixture_reference_set() {
            let good = definition(Some("7e"), Some("6b6c"));
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let own = match operation {
                    ComposerOperation::Prompt => PROMPT_FIXTURE,
                    ComposerOperation::Exec => EXEC_FIXTURE,
                };
                for invalid in [
                    json!([]),
                    json!([""]),
                    json!([" "]),
                    json!(["unknown-fixture"]),
                    json!([own, own]),
                    Value::Null,
                    json!(own),
                    json!(17),
                    json!({}),
                ] {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["fixture_ids"] = invalid;
                    reject(&good, &bad, operation, "operation fixture set");
                }
                for mismatch in ["backend", "native_version"] {
                    let mut bad = good.clone();
                    let mut foreign = bad["fixtures"][0].clone();
                    foreign["id"] = json!("foreign-operation-fixture");
                    foreign[mismatch] = if mismatch == "backend" {
                        json!(
                            CodingAgent::all()
                                .iter()
                                .find(|&&backend| backend != CodingAgent::Codex)
                                .unwrap()
                                .name()
                        )
                    } else {
                        json!(VERSION_TWO)
                    };
                    bad["fixtures"].as_array_mut().unwrap().push(foreign);
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["fixture_ids"] =
                        json!(["foreign-operation-fixture"]);
                    reject(&good, &bad, operation, "foreign operation provenance");
                }
            }
        }

        #[test]
        fn step5_composer_e13_operation_provenance_covers_every_declared_exact_native_version() {
            let mut good = definition(Some("7e"), Some("6b6c"));
            good["profiles"][0]["native_versions"] = json!([VERSION, VERSION_TWO]);
            let originals = good["fixtures"].as_array().unwrap().clone();
            for mut fixture in originals {
                fixture["id"] = json!(format!("{}-v2", fixture["id"].as_str().unwrap()));
                fixture["native_version"] = json!(VERSION_TWO);
                good["fixtures"].as_array_mut().unwrap().push(fixture);
            }
            good["profiles"][0]["capabilities"]["composer"]["fixture_ids"] =
                json!([FIXTURE, format!("{FIXTURE}-v2")]);
            for (operation, fixture) in [
                (ComposerOperation::Prompt, PROMPT_FIXTURE),
                (ComposerOperation::Exec, EXEC_FIXTURE),
            ] {
                good["profiles"][0]["capabilities"]["composer"]["submission"]
                    [operation_name(operation)]["fixture_ids"] =
                    json!([fixture, format!("{fixture}-v2")]);
            }
            let catalog = ProfileCatalog::from_definitions(&good).unwrap();
            for (operation, fixture) in [
                (ComposerOperation::Prompt, PROMPT_FIXTURE),
                (ComposerOperation::Exec, EXEC_FIXTURE),
            ] {
                let selected = catalog
                    .composer_plan(CodingAgent::Codex, VERSION_TWO, operation)
                    .unwrap()
                    .unwrap();
                assert_eq!(selected.native_version(), VERSION_TWO);
                assert_eq!(
                    selected.fixture_ids(),
                    &[fixture.to_owned(), format!("{fixture}-v2")]
                );
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]["submission"]
                    [operation_name(operation)]["fixture_ids"] = json!([fixture]);
                reject(&good, &bad, operation, "uncovered operation version");
            }
        }

        #[test]
        fn step5_composer_e14_composer_rule_references_keep_component_identity_and_declared_order()
        {
            let mut good = definition(Some("7e"), Some("6b6c"));
            good["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .push(definitions()["profiles"][0]["rules"][0].clone());
            good["profiles"][0]["capabilities"]["detection"] =
                json!({"rule_ids":[RULE],"fixture_ids":[FIXTURE]});
            let catalog = ProfileCatalog::from_definitions(&good).unwrap();
            assert_eq!(
                catalog
                    .capabilities(CodingAgent::Codex, Some(VERSION))
                    .detection,
                Coverage::Tested
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let own = match operation {
                    ComposerOperation::Prompt => PROMPT_FIXTURE,
                    ComposerOperation::Exec => EXEC_FIXTURE,
                };
                metadata(&plan(&catalog, operation), &[own.to_owned()]);
                for invalid in [
                    json!([]),
                    json!(["unknown-rule"]),
                    json!([RULE]),
                    json!([EMPTY, EMPTY]),
                ] {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["rule_ids"] = invalid;
                    reject(&good, &bad, operation, "composer rule component");
                }
            }
        }

        #[test]
        fn step5_composer_e15_native_payload_limit_counts_prefix_utf8_literal_and_submit_bytes() {
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                for submit_length in [1, 32] {
                    let hex = "6b".repeat(submit_length);
                    let value = match operation {
                        ComposerOperation::Prompt => definition(Some(&hex), None),
                        ComposerOperation::Exec => definition(None, Some(&hex)),
                    };
                    let catalog = ProfileCatalog::from_definitions(&value).unwrap();
                    let selected = plan(&catalog, operation);
                    let literal_size = 65536 - operation_prefix(operation).len() - submit_length;
                    let literal = format!(
                        "{}{}",
                        "界".repeat(literal_size / 3),
                        "x".repeat(literal_size % 3)
                    );
                    assert_eq!(literal.len(), literal_size);
                    let actual = selected.encode(&literal).unwrap();
                    assert_eq!(actual.len(), 65536);
                    assert_eq!(
                        actual,
                        expected(operation, &literal, &vec![b'k'; submit_length])
                    );
                    let oversized = format!("{literal}x");
                    assert!(
                        selected.encode(&oversized).is_err(),
                        "65537 native payload bytes reject"
                    );
                }
            }
        }

        #[test]
        fn step5_composer_e16_plans_withhold_debug_serialization_and_private_bytes_from_errors() {
            trait AmbiguousIfImplemented<Marker> {
                fn check() {}
            }
            struct DebugImplemented;
            struct SerializeImplemented;
            impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
            impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
            impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
            let _ = <ComposerPlan as AmbiguousIfImplemented<_>>::check;
            const SUBMIT: &str = "E16_PRIVATE_SUBMIT";
            const LITERAL: &str = "E16_PRIVATE_LITERAL";
            let hex = SUBMIT
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let good = match operation {
                    ComposerOperation::Prompt => definition(Some(&hex), None),
                    ComposerOperation::Exec => definition(None, Some(&hex)),
                };
                let catalog = ProfileCatalog::from_definitions(&good).unwrap();
                let selected = plan(&catalog, operation);
                assert_eq!(
                    selected.encode("positive").unwrap(),
                    expected(operation, "positive", SUBMIT.as_bytes())
                );
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]["submission"]
                    [operation_name(operation)]["prefix_hex"] = match operation {
                    ComposerOperation::Prompt => json!("00"),
                    ComposerOperation::Exec => json!(""),
                };
                let definition_error =
                    reject(&good, &bad, operation, "private submit with invalid prefix");
                let literal = format!("{LITERAL}{}", "x".repeat(65537));
                let encode_error = match selected.encode(&literal) {
                    Err(error) => error,
                    Ok(_) => panic!("oversized private literal rejects"),
                };
                for error in [definition_error, encode_error] {
                    let diagnostic = format!("{error:?}\n{error}\n{}", error.message());
                    for private in [SUBMIT, LITERAL, &hex] {
                        assert!(
                            !diagnostic.contains(private),
                            "plan errors withhold selected bytes and literal payload"
                        );
                    }
                }
            }
        }
    }

    mod bundled_claude {
        use super::*;
        use sha2::{Digest, Sha256};
        use std::collections::BTreeSet;

        const CLAUDE_VERSION: &str = "2.1.302";
        const NATIVE_FIXTURE: &str = "claude-2.1.302-default-permission-1";
        const NATIVE_SOURCE: &str = "native/claude-2.1.302-default-permission-1.json";
        const NATIVE_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-default-permission-1.json");
        const DEFINITIONS: &str = include_str!("profiles/bundled.json");
        const PERMISSION_RULE: &str = "claude-bash-permission";
        const IDLE_RULE: &str = "claude-idle-composer";
        const PROFILE_VERSION: &str = "claude-2.1.302-default-3";
        const ALLOW_FIXTURE: &str = "claude-2.1.302-allow-pre-stage-1";
        const DENY_FIXTURE: &str = "claude-2.1.302-deny-pre-stage-1";
        const IDLE_FIXTURE: &str = "claude-2.1.302-idle-composer-1";
        const WRAPPED_FIXTURE: &str = "claude-2.1.302-wrapped-permission-1";
        const WRAPPED_ALLOW_FIXTURE: &str = "claude-2.1.302-wrapped-allow-pre-stage-1";
        const WRAPPED_DENY_FIXTURE: &str = "claude-2.1.302-wrapped-deny-pre-stage-1";
        const ALLOW_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-allow-pre-stage-1.json");
        const DENY_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-deny-pre-stage-1.json");
        const IDLE_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-idle-composer-1.json");
        const WRAPPED_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-wrapped-permission-1.json");
        const WRAPPED_ALLOW_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-wrapped-allow-pre-stage-1.json");
        const WRAPPED_DENY_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-wrapped-deny-pre-stage-1.json");
        const WRAPPED_FIXTURES: [&str; 3] =
            [WRAPPED_FIXTURE, WRAPPED_ALLOW_FIXTURE, WRAPPED_DENY_FIXTURE];
        const BOXED_FIXTURE: &str = "claude-2.1.302-boxed-permission-1";
        const BOXED_SINGLE_ROW_FIXTURE: &str = "claude-2.1.302-boxed-single-row-permission-1";
        const BOXED_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-boxed-permission-1.json");
        const BOXED_SINGLE_ROW_BYTES: &[u8] =
            include_bytes!("profiles/native/claude-2.1.302-boxed-single-row-permission-1.json");
        const BOXED_FIXTURES: [&str; 2] = [BOXED_FIXTURE, BOXED_SINGLE_ROW_FIXTURE];
        pub(super) const FIXTURES: [(&str, &str, &[u8]); 9] = [
            (NATIVE_FIXTURE, NATIVE_SOURCE, NATIVE_BYTES),
            (
                ALLOW_FIXTURE,
                "native/claude-2.1.302-allow-pre-stage-1.json",
                ALLOW_BYTES,
            ),
            (
                DENY_FIXTURE,
                "native/claude-2.1.302-deny-pre-stage-1.json",
                DENY_BYTES,
            ),
            (
                IDLE_FIXTURE,
                "native/claude-2.1.302-idle-composer-1.json",
                IDLE_BYTES,
            ),
            (
                WRAPPED_FIXTURE,
                "native/claude-2.1.302-wrapped-permission-1.json",
                WRAPPED_BYTES,
            ),
            (
                WRAPPED_ALLOW_FIXTURE,
                "native/claude-2.1.302-wrapped-allow-pre-stage-1.json",
                WRAPPED_ALLOW_BYTES,
            ),
            (
                WRAPPED_DENY_FIXTURE,
                "native/claude-2.1.302-wrapped-deny-pre-stage-1.json",
                WRAPPED_DENY_BYTES,
            ),
            (
                BOXED_FIXTURE,
                "native/claude-2.1.302-boxed-permission-1.json",
                BOXED_BYTES,
            ),
            (
                BOXED_SINGLE_ROW_FIXTURE,
                "native/claude-2.1.302-boxed-single-row-permission-1.json",
                BOXED_SINGLE_ROW_BYTES,
            ),
        ];

        pub(super) fn embedded() -> Vec<(&'static str, &'static [u8])> {
            FIXTURES
                .iter()
                .chain(super::bundled_codex::FIXTURES.iter())
                .chain(super::bundled_opencode::FIXTURES.iter())
                .map(|&(_, source, bytes)| (source, bytes))
                .collect()
        }

        fn fixture(id: &str) -> (&'static str, &'static [u8]) {
            FIXTURES
                .iter()
                .find(|(fixture, _, _)| *fixture == id)
                .map(|&(_, source, bytes)| (source, bytes))
                .expect("known bundled fixture")
        }

        fn lines(bytes: &[u8]) -> Vec<String> {
            serde_json::from_slice::<Value>(bytes).unwrap()["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap().to_owned())
                .collect()
        }

        fn bundled_observation(bytes: &[u8]) -> ScreenObservation {
            let catalog = ProfileCatalog::bundled().unwrap();
            let screen = super::super::detector::inspection_tests::replay_native_fixture(
                std::str::from_utf8(bytes).unwrap(),
                |_| {},
                true,
                None,
            );
            Detector::new(&catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
                .unwrap()
                .observe(&screen)
                .unwrap()
        }

        fn digest(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        fn definitions() -> Value {
            serde_json::from_str(DEFINITIONS).unwrap()
        }

        fn native_screen() -> Value {
            serde_json::from_slice(NATIVE_BYTES).unwrap()
        }

        fn entry<'a>(definitions: &'a mut Value, id: &str) -> &'a mut Value {
            definitions["fixtures"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["id"] == id)
                .expect("bundled catalog cites the native fixture")
        }

        fn native_entry(definitions: &mut Value) -> &mut Value {
            entry(definitions, NATIVE_FIXTURE)
        }

        type ScreenChange = (&'static str, fn(&mut Value));

        fn guarded_fixture(
            id: &str,
            change_definitions: impl FnOnce(&mut Value),
            change_screen: impl FnOnce(&mut Value),
        ) -> Result<ProfileCatalog, CafleetError> {
            let (source, original) = fixture(id);
            let mut screen: Value = serde_json::from_slice(original).unwrap();
            change_screen(&mut screen);
            let bytes = serde_json::to_vec_pretty(&screen).unwrap();
            let mut value = definitions();
            entry(&mut value, id)["sha256"] = json!(digest(&bytes));
            change_definitions(&mut value);
            let mut embedded = embedded();
            embedded.retain(|&(cited, _)| cited != source);
            embedded.push((source, &bytes));
            ProfileCatalog::from_bundled_definitions(&value, &embedded)
        }

        fn guarded(
            change_definitions: impl FnOnce(&mut Value),
            change_screen: impl FnOnce(&mut Value),
        ) -> Result<ProfileCatalog, CafleetError> {
            guarded_fixture(NATIVE_FIXTURE, change_definitions, change_screen)
        }

        #[test]
        fn bundled_claude_native_fixture_matches_its_catalog_entry_and_declares_masked_header_rows()
        {
            let mut value = definitions();
            assert_eq!(
                native_entry(&mut value),
                &json!({"id":NATIVE_FIXTURE,"kind":"native","backend":"claude","native_version":CLAUDE_VERSION,
                    "rows":36,"cols":120,"source_ref":NATIVE_SOURCE,"sha256":digest(NATIVE_BYTES)})
            );
            let screen = native_screen();
            assert_eq!(
                screen
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    "schema_version",
                    "fixture_id",
                    "provenance",
                    "backend",
                    "native_version",
                    "rows",
                    "cols",
                    "alternate_screen",
                    "screen_seq",
                    "cursor_row",
                    "cursor_col",
                    "lines",
                    "masked_rows",
                ])
            );
            assert_eq!(screen["schema_version"], 1);
            assert_eq!(screen["fixture_id"], NATIVE_FIXTURE);
            assert_eq!(screen["provenance"], "native");
            assert_eq!(screen["backend"], "claude");
            assert_eq!(screen["native_version"], CLAUDE_VERSION);
            assert_eq!(
                (screen["rows"].clone(), screen["cols"].clone()),
                (json!(36), json!(120))
            );
            assert_eq!(screen["alternate_screen"], true);
            assert_eq!(screen["screen_seq"], 11424);
            assert_eq!(
                (screen["cursor_row"].clone(), screen["cursor_col"].clone()),
                (json!(23), json!(1))
            );
            let lines = screen["lines"].as_array().unwrap();
            assert_eq!(lines.len(), 36);
            assert_eq!(
                screen["masked_rows"],
                json!(
                    (0..12)
                        .map(|row| json!({"row":row,"reason":"reviewed_privacy"}))
                        .collect::<Vec<_>>()
                )
            );
            for (row, line) in lines.iter().enumerate() {
                let line = line.as_str().unwrap();
                if row < 12 {
                    assert_eq!(line, "", "masked row {row} is empty");
                }
                assert!(
                    !line.contains("/Users/") && !line.contains("himkt"),
                    "row {row} carries no personal path"
                );
            }
            assert_eq!(lines[14], " Bash command");
            assert_eq!(
                lines[18],
                " /private/tmp/cafleet-194/cells-002/permission-probe allow"
            );
            assert_eq!(lines[20], " This command requires approval");
            assert_eq!(lines[22], " Do you want to proceed?");
            assert_eq!(lines[23], " ❯ 1. Yes");
            assert_eq!(lines[26], "   4. No");
            assert_eq!(lines[28], " Esc to cancel · Tab to amend");
            assert_eq!(lines[29], " ".repeat(17));
        }

        #[test]
        fn bundled_claude_2_1_302_declares_detection_idle_composer_and_both_answers_only() {
            let bundled = ProfileCatalog::bundled().unwrap();
            let claude = bundled.capabilities(CodingAgent::Claude, Some(CLAUDE_VERSION));
            assert_eq!(claude.backend, CodingAgent::Claude);
            assert_eq!(claude.native_version.as_deref(), Some(CLAUDE_VERSION));
            assert_eq!(claude.profile_version.as_deref(), Some(PROFILE_VERSION));
            assert_eq!(claude.detection, Coverage::Tested);
            assert_eq!(claude.composer, Coverage::Tested);
            assert_eq!(claude.prompt_submission, Coverage::Unknown);
            assert_eq!(claude.native_exec_submission, Coverage::Unknown);
            assert_eq!(claude.answer_decisions.len(), 2);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(claude.answer_decisions.contains(&decision), "{decision:?}");
            }
            for version in [None, Some("2.1.301"), Some("2.1.303")] {
                let unknown = bundled.capabilities(CodingAgent::Claude, version);
                assert_eq!(unknown.detection, Coverage::Unknown, "{version:?}");
                assert!(unknown.answer_decisions.is_empty());
                assert!(unknown.capability_reason.is_some());
            }
            for &backend in CodingAgent::all()
                .iter()
                .filter(|&&backend| backend != CodingAgent::Claude)
            {
                let other = bundled.capabilities(backend, Some(CLAUDE_VERSION));
                assert_eq!(other.detection, Coverage::Unknown, "{backend:?}");
                assert!(other.answer_decisions.is_empty());
            }
        }

        #[test]
        fn bundled_guard_accepts_declared_masks_and_rejects_synthetic_altered_or_mismatched_fixtures()
         {
            assert!(ProfileCatalog::from_bundled_definitions(&definitions(), &embedded()).is_ok());
            assert!(guarded(|_| {}, |_| {}).is_ok());
            let mut tampered = NATIVE_BYTES.to_vec();
            tampered.push(b'\n');
            let mut with_tampered = embedded();
            with_tampered.retain(|&(source, _)| source != NATIVE_SOURCE);
            with_tampered.push((NATIVE_SOURCE, &tampered));
            assert!(
                ProfileCatalog::from_bundled_definitions(&definitions(), &with_tampered).is_err(),
                "digest mismatch"
            );
            let mut missing = embedded();
            missing.retain(|&(source, _)| source != NATIVE_SOURCE);
            assert!(
                ProfileCatalog::from_bundled_definitions(&definitions(), &missing).is_err(),
                "cited fixture is embedded"
            );
            let screen_changes: [ScreenChange; 13] = [
                ("synthetic provenance", |s| {
                    s["provenance"] = json!("synthetic")
                }),
                ("extra field", |s| s["note"] = json!("unreviewed")),
                ("missing masked rows", |s| {
                    s.as_object_mut().unwrap().remove("masked_rows");
                }),
                ("nonempty masked row", |s| {
                    s["lines"][3] = json!("restored header")
                }),
                ("masked row out of range", |s| {
                    s["masked_rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"row":36,"reason":"reviewed_privacy"}))
                }),
                ("duplicate masked row", |s| {
                    s["masked_rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"row":0,"reason":"reviewed_privacy"}))
                }),
                ("unreviewed mask reason", |s| {
                    s["masked_rows"][0]["reason"] = json!("convenience")
                }),
                ("fixture id", |s| s["fixture_id"] = json!("other-fixture")),
                ("backend", |s| s["backend"] = json!("codex")),
                ("native version", |s| s["native_version"] = json!("2.1.301")),
                ("rows", |s| s["rows"] = json!(35)),
                ("line count", |s| {
                    s["lines"].as_array_mut().unwrap().pop();
                }),
                ("cursor outside geometry", |s| s["cursor_row"] = json!(36)),
            ];
            for (case, change) in screen_changes {
                assert!(guarded(|_| {}, change).is_err(), "{case} must be rejected");
            }
            assert!(
                guarded(|d| native_entry(d)["kind"] = json!("synthetic"), |_| {}).is_err(),
                "synthetic catalog fixture kind must be rejected"
            );
        }

        #[test]
        fn bundled_claude_answer_and_idle_fixtures_match_their_entries_and_declare_privacy_masks() {
            let mut value = definitions();
            let header: Vec<usize> = (0..12).collect();
            let idle_masks: Vec<usize> = (0..12).chain([33, 34]).collect();
            for (id, sequence, cursor, masked) in [
                (ALLOW_FIXTURE, 10819, (23, 1), &header),
                (DENY_FIXTURE, 11866, (23, 1), &header),
                (IDLE_FIXTURE, 7035, (31, 2), &idle_masks),
            ] {
                let (source, bytes) = fixture(id);
                assert_eq!(
                    entry(&mut value, id),
                    &json!({"id":id,"kind":"native","backend":"claude","native_version":CLAUDE_VERSION,
                        "rows":36,"cols":120,"source_ref":source,"sha256":digest(bytes)})
                );
                let screen: Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(
                    screen.as_object().unwrap().len(),
                    native_screen().as_object().unwrap().len(),
                    "{id} has exactly the documented fields"
                );
                assert_eq!(screen["fixture_id"], id);
                assert_eq!(screen["provenance"], "native");
                assert_eq!(screen["backend"], "claude");
                assert_eq!(screen["native_version"], CLAUDE_VERSION);
                assert_eq!(
                    (screen["rows"].clone(), screen["cols"].clone()),
                    (json!(36), json!(120))
                );
                assert_eq!(screen["alternate_screen"], true);
                assert_eq!(screen["screen_seq"], sequence);
                assert_eq!(
                    (screen["cursor_row"].clone(), screen["cursor_col"].clone()),
                    (json!(cursor.0), json!(cursor.1))
                );
                assert_eq!(
                    screen["masked_rows"],
                    json!(
                        masked
                            .iter()
                            .map(|row| json!({"row":row,"reason":"reviewed_privacy"}))
                            .collect::<Vec<_>>()
                    )
                );
                let rows = lines(bytes);
                assert_eq!(rows.len(), 36);
                for (row, line) in rows.iter().enumerate() {
                    if masked.contains(&row) {
                        assert_eq!(line, "", "{id} masked row {row} is empty");
                    }
                    for private in ["/Users/", "himkt", "Opus", "$ ", "--resume"] {
                        assert!(
                            !line.contains(private),
                            "{id} row {row} carries {private:?}"
                        );
                    }
                }
            }
            for (bytes, operation) in [
                (
                    ALLOW_BYTES,
                    " /private/tmp/cafleet-194/ans-allow-001/permission-probe allow",
                ),
                (
                    DENY_BYTES,
                    " /private/tmp/cafleet-194/ans-deny-001/permission-probe deny",
                ),
            ] {
                let rows = lines(bytes);
                assert_eq!(rows[14], " Bash command");
                assert_eq!(rows[18], operation);
                assert_eq!(rows[23], " ❯ 1. Yes");
                assert_eq!(rows[26], "   4. No");
            }
            let idle = lines(IDLE_BYTES);
            assert_eq!(idle[31], "❯\u{a0}");
            assert!(idle[30].starts_with('─') && idle[32].starts_with('─'));
            assert_eq!(idle[35], "  ⏸ manual mode on");
        }

        #[test]
        fn bundled_claude_answer_plans_are_the_measured_single_digit_stages() {
            let bundled = ProfileCatalog::bundled().unwrap();
            for (decision, byte, choice, answer_rule, provenance) in [
                (
                    PermissionDecision::Allow,
                    b'1',
                    "allow-once",
                    "claude-bash-allow-digit",
                    [ALLOW_FIXTURE, WRAPPED_ALLOW_FIXTURE],
                ),
                (
                    PermissionDecision::Deny,
                    b'4',
                    "deny-once",
                    "claude-bash-deny-digit",
                    [DENY_FIXTURE, WRAPPED_DENY_FIXTURE],
                ),
            ] {
                let plan = bundled
                    .answer_plan(
                        CodingAgent::Claude,
                        CLAUDE_VERSION,
                        PERMISSION_RULE,
                        decision,
                    )
                    .unwrap()
                    .expect("measured bundled answer plan");
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.focus_matcher_id(), "focus");
                assert_eq!(plan.stages().len(), 1, "{decision:?} is one measured stage");
                let stage = &plan.stages()[0];
                assert_eq!(stage.bytes(), &[byte]);
                assert_eq!(stage.required_rule_id(), PERMISSION_RULE);
                assert_eq!(stage.required_choice_id(), choice);
                let capability = plan.capability();
                assert_eq!(capability.backend(), CodingAgent::Claude);
                assert_eq!(capability.native_version(), CLAUDE_VERSION);
                assert_eq!(capability.profile_version(), PROFILE_VERSION);
                assert_eq!(capability.request_rule_id(), PERMISSION_RULE);
                assert_eq!(capability.answer_rule_id(), answer_rule);
                assert_eq!(capability.decision(), decision);
                assert_eq!(
                    capability.fixture_ids(),
                    provenance.map(str::to_owned).as_slice(),
                    "{decision:?} rests on the unwrapped and wrapped pre-stage captures"
                );
                for (backend, version) in [
                    (CodingAgent::Claude, "2.1.301"),
                    (CodingAgent::Codex, CLAUDE_VERSION),
                    (CodingAgent::Opencode, CLAUDE_VERSION),
                ] {
                    assert!(
                        bundled
                            .answer_plan(backend, version, PERMISSION_RULE, decision)
                            .unwrap()
                            .is_none(),
                        "{backend:?} {version}"
                    );
                }
            }
        }

        #[test]
        fn bundled_guard_rejects_synthetic_or_altered_answer_and_idle_fixtures() {
            for id in [ALLOW_FIXTURE, DENY_FIXTURE, IDLE_FIXTURE] {
                assert!(guarded_fixture(id, |_| {}, |_| {}).is_ok(), "{id}");
                assert!(
                    guarded_fixture(id, |d| entry(d, id)["kind"] = json!("synthetic"), |_| {})
                        .is_err(),
                    "{id} synthetic catalog kind"
                );
                assert!(
                    guarded_fixture(id, |_| {}, |s| s["provenance"] = json!("synthetic")).is_err(),
                    "{id} synthetic provenance"
                );
                assert!(
                    guarded_fixture(id, |_| {}, |s| s["lines"][0] = json!("restored header"))
                        .is_err(),
                    "{id} nonempty masked row"
                );
                let (source, bytes) = fixture(id);
                let mut tampered = bytes.to_vec();
                tampered.push(b'\n');
                let mut with_tampered = embedded();
                with_tampered.retain(|&(cited, _)| cited != source);
                with_tampered.push((source, &tampered));
                assert!(
                    ProfileCatalog::from_bundled_definitions(&definitions(), &with_tampered)
                        .is_err(),
                    "{id} digest mismatch"
                );
            }
            assert!(
                guarded_fixture(
                    IDLE_FIXTURE,
                    |_| {},
                    |s| s["lines"][33] = json!("  Opus 5.5 (1M context)")
                )
                .is_err(),
                "a restored masked status row is rejected"
            );
        }

        #[test]
        fn bundled_claude_episode_closes_not_visible_once_the_idle_composer_holds_250_ms() {
            let permission = bundled_observation(NATIVE_BYTES);
            let idle = bundled_observation(IDLE_BYTES);
            assert_eq!(idle.rule_id, IDLE_RULE);
            assert!(idle.verified_noninteraction);
            let at = |observation: &ScreenObservation, output_seq: u64| {
                let mut replayed = observation.clone();
                replayed.output_seq = output_seq;
                replayed.raw_evidence["screen_seq"] = json!(output_seq);
                replayed
            };
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &at(&permission, 1), origin);
            let closed = |update: &EpisodeUpdate| {
                update
                    .transitions
                    .iter()
                    .filter(|t| t.kind == RuntimeEventKind::InteractionClosed)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let flash = tracker
                .observe(&at(&idle, 2), origin + Duration::from_millis(200), NOW)
                .unwrap();
            assert!(closed(&flash).is_empty(), "idle shown for 0 ms stays open");
            let back = tracker
                .observe(
                    &at(&permission, 3),
                    origin + Duration::from_millis(300),
                    NOW,
                )
                .unwrap();
            assert!(
                closed(&back).is_empty(),
                "an idle flash under 250 ms keeps the episode"
            );
            let first = tracker
                .observe(&at(&idle, 4), origin + Duration::from_millis(400), NOW)
                .unwrap();
            assert!(closed(&first).is_empty());
            let held = tracker
                .observe(&at(&idle, 5), origin + Duration::from_millis(650), NOW)
                .unwrap();
            let closes = closed(&held);
            assert_eq!(closes.len(), 1, "{:?}", held.transitions);
            assert_eq!(
                closes[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
        }

        #[test]
        fn bundled_claude_wrapped_fixtures_match_their_catalog_entries_and_declare_masked_header_rows()
         {
            let mut value = definitions();
            for (id, sequence, cursor, heading, operation) in [
                (
                    WRAPPED_FIXTURE,
                    6916,
                    (28, 1),
                    17,
                    [
                        " /tmp/cafleet-194/claude-wrap-cells-001/permission-",
                        " probe allow",
                    ],
                ),
                (
                    WRAPPED_ALLOW_FIXTURE,
                    7053,
                    (27, 1),
                    16,
                    [
                        " /tmp/cafleet-194/claude-wrap-ans-allow-001/permiss",
                        " ion-probe allow",
                    ],
                ),
                (
                    WRAPPED_DENY_FIXTURE,
                    7146,
                    (27, 1),
                    16,
                    [
                        " /tmp/cafleet-194/claude-wrap-ans-deny-001/permissi",
                        " on-probe deny",
                    ],
                ),
            ] {
                let (source, bytes) = fixture(id);
                assert_eq!(source, format!("native/{id}.json"));
                assert_eq!(
                    entry(&mut value, id),
                    &json!({"id":id,"kind":"native","backend":"claude","native_version":CLAUDE_VERSION,
                        "rows":36,"cols":52,"source_ref":source,"sha256":digest(bytes)})
                );
                let screen: Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(
                    screen.as_object().unwrap().keys().collect::<BTreeSet<_>>(),
                    native_screen()
                        .as_object()
                        .unwrap()
                        .keys()
                        .collect::<BTreeSet<_>>(),
                    "{id} has exactly the documented fields"
                );
                assert_eq!(screen["schema_version"], 1);
                assert_eq!(screen["fixture_id"], id);
                assert_eq!(screen["provenance"], "native");
                assert_eq!(screen["backend"], "claude");
                assert_eq!(screen["native_version"], CLAUDE_VERSION);
                assert_eq!(
                    (screen["rows"].clone(), screen["cols"].clone()),
                    (json!(36), json!(52))
                );
                assert_eq!(screen["alternate_screen"], true, "{id}");
                assert_eq!(screen["screen_seq"], sequence, "{id}");
                assert_eq!(
                    (screen["cursor_row"].clone(), screen["cursor_col"].clone()),
                    (json!(cursor.0), json!(cursor.1)),
                    "{id} cursor sits on the focused Yes row"
                );
                assert_eq!(
                    screen["masked_rows"],
                    json!(
                        (0..12)
                            .map(|row| json!({"row":row,"reason":"reviewed_privacy"}))
                            .collect::<Vec<_>>()
                    )
                );
                let rows = lines(bytes);
                assert_eq!(rows.len(), 36);
                for (row, line) in rows.iter().enumerate() {
                    if row < 12 {
                        assert_eq!(line, "", "{id} masked row {row} is empty");
                    }
                    assert!(line.chars().count() <= 52, "{id} row {row} fits 52 columns");
                    for private in ["/Users/", "/root", "/development-pvc", "himkt", "Opus"] {
                        assert!(
                            !line.contains(private),
                            "{id} row {row} carries {private:?}"
                        );
                    }
                }
                assert_eq!(rows[heading], " Bash command", "{id}");
                for rule in [heading + 4, heading + 7] {
                    assert!(rows[rule].starts_with('╌'), "{id} row {rule}");
                    assert_eq!(rows[rule].chars().count(), 52, "{id} row {rule}");
                }
                assert_eq!(
                    [rows[heading + 5].as_str(), rows[heading + 6].as_str()],
                    operation,
                    "{id} operation spans two rows"
                );
                assert_eq!(
                    rows[heading + 5].chars().count(),
                    51,
                    "{id} first operation row reaches the last usable column"
                );
                assert_eq!(rows[heading + 8], " This command requires approval");
                assert_eq!(rows[heading + 10], " Do you want to proceed?");
                assert_eq!(rows[heading + 11], " ❯ 1. Yes");
                assert_eq!(heading + 11, cursor.0);
                assert_eq!(rows[33], "   4. No", "{id}");
                assert_eq!(rows[35], " Esc to cancel · Tab to amend", "{id}");
                assert!(
                    35 - heading + 1 > 16 && 35 - heading < 24,
                    "{id} form exceeds the unwrapped bound and fits 24 rows"
                );
            }
        }

        #[test]
        fn bundled_claude_boxed_fixtures_match_their_catalog_entries_and_declare_masked_header_rows()
         {
            let mut value = definitions();
            let path_089 =
                "/tmp/cafleet-194/claude-boxed-cells-089-wide-operation-probe-01/permission-probe";
            let path_120 =
                "/tmp/cafleet-194/claude-boxed-cells-120-wide-operation-probe-01/permission-probe";
            for (id, cols, sequence, cursor, heading, operation, closing) in [
                (
                    BOXED_FIXTURE,
                    89,
                    8800,
                    (27, 1),
                    17,
                    vec![format!(" │ {path_089}"), " │ allow".to_owned()],
                    34,
                ),
                (
                    BOXED_SINGLE_ROW_FIXTURE,
                    120,
                    9055,
                    (23, 1),
                    14,
                    vec![format!(" │ {path_120} allow")],
                    29,
                ),
            ] {
                let (source, bytes) = fixture(id);
                assert_eq!(source, format!("native/{id}.json"));
                assert_eq!(
                    entry(&mut value, id),
                    &json!({"id":id,"kind":"native","backend":"claude","native_version":CLAUDE_VERSION,
                        "rows":36,"cols":cols,"source_ref":source,"sha256":digest(bytes)})
                );
                let screen: Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(
                    screen.as_object().unwrap().keys().collect::<BTreeSet<_>>(),
                    native_screen()
                        .as_object()
                        .unwrap()
                        .keys()
                        .collect::<BTreeSet<_>>(),
                    "{id} has exactly the documented fields"
                );
                assert_eq!(screen["schema_version"], 1);
                assert_eq!(screen["fixture_id"], id);
                assert_eq!(screen["provenance"], "native");
                assert_eq!(screen["backend"], "claude");
                assert_eq!(screen["native_version"], CLAUDE_VERSION);
                assert_eq!(
                    (screen["rows"].clone(), screen["cols"].clone()),
                    (json!(36), json!(cols))
                );
                assert_eq!(screen["alternate_screen"], true, "{id}");
                assert_eq!(screen["screen_seq"], sequence, "{id}");
                assert_eq!(
                    (screen["cursor_row"].clone(), screen["cursor_col"].clone()),
                    (json!(cursor.0), json!(cursor.1)),
                    "{id} cursor sits on the focused Yes row"
                );
                assert_eq!(
                    screen["masked_rows"],
                    json!(
                        (0..12)
                            .map(|row| json!({"row":row,"reason":"reviewed_privacy"}))
                            .collect::<Vec<_>>()
                    )
                );
                let rows = lines(bytes);
                assert_eq!(rows.len(), 36);
                for (row, line) in rows.iter().enumerate() {
                    if row < 12 {
                        assert_eq!(line, "", "{id} masked row {row} is empty");
                    }
                    assert!(
                        line.chars().count() <= cols,
                        "{id} row {row} fits {cols} columns"
                    );
                    for private in ["/Users/", "/root", "/development-pvc", "himkt", "Opus"] {
                        assert!(
                            !line.contains(private),
                            "{id} row {row} carries {private:?}"
                        );
                    }
                }
                assert_eq!(rows[heading], " Bash command", "{id}");
                let first_rule = heading + 3;
                let last_rule = first_rule + operation.len() + 1;
                for rule in [first_rule, last_rule] {
                    assert!(rows[rule].starts_with('╌'), "{id} row {rule}");
                    assert_eq!(rows[rule].chars().count(), cols, "{id} row {rule}");
                }
                assert_eq!(
                    rows[first_rule + 1..last_rule],
                    operation[..],
                    "{id} boxed operation rows"
                );
                assert_eq!(rows[last_rule + 1], " This command requires approval");
                assert_eq!(rows[last_rule + 3], " Do you want to proceed?");
                assert_eq!(rows[cursor.0], " ❯ 1. Yes", "{id}");
                assert_eq!(cursor.0, last_rule + 4);
                assert_eq!(rows[closing], " Esc to cancel · Tab to amend", "{id}");
                assert!(closing - heading < 24, "{id} form fits the 24-row bound");
            }
            let boxed = lines(BOXED_BYTES);
            assert_eq!(boxed[21].chars().count(), 83, "short of the 88-cell edge");
            assert_eq!(boxed[32], "   4. No");
            assert_eq!(
                boxed[33], "  ",
                "two written spaces before the closing line"
            );
            assert_eq!(
                lines(BOXED_SINGLE_ROW_BYTES)[18].chars().count(),
                89,
                "the single-row box follows the operation length"
            );
        }

        #[test]
        fn bundled_claude_detection_and_catalog_cite_the_unwrapped_wrapped_and_boxed_native_captures()
         {
            let value = definitions();
            let profile = value["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|profile| profile["backend"] == "claude")
                .expect("bundled Claude profile");
            assert_eq!(profile["profile_version"], PROFILE_VERSION);
            assert_eq!(profile["native_versions"], json!([CLAUDE_VERSION]));
            assert_eq!(
                profile["capabilities"]["detection"]["fixture_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap())
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    NATIVE_FIXTURE,
                    WRAPPED_FIXTURE,
                    BOXED_FIXTURE,
                    BOXED_SINGLE_ROW_FIXTURE
                ])
            );
            assert_eq!(
                profile["capabilities"]["composer"]["fixture_ids"],
                json!([IDLE_FIXTURE])
            );
            let cited = value["fixtures"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["id"].as_str().unwrap())
                .collect::<BTreeSet<_>>();
            for (id, _, _) in FIXTURES {
                assert!(cited.contains(id), "{id} is a bundled catalog fixture");
            }
        }

        #[test]
        fn bundled_guard_accepts_the_wrapped_and_boxed_fixtures_and_rejects_altered_ones() {
            assert!(ProfileCatalog::from_bundled_definitions(&definitions(), &embedded()).is_ok());
            for id in WRAPPED_FIXTURES.into_iter().chain(BOXED_FIXTURES) {
                assert!(guarded_fixture(id, |_| {}, |_| {}).is_ok(), "{id}");
                assert!(
                    guarded_fixture(id, |d| entry(d, id)["kind"] = json!("synthetic"), |_| {})
                        .is_err(),
                    "{id} synthetic catalog kind"
                );
                let screen_changes: [ScreenChange; 9] = [
                    ("synthetic provenance", |s| {
                        s["provenance"] = json!("synthetic")
                    }),
                    ("nonempty masked row", |s| {
                        s["lines"][0] = json!("restored header")
                    }),
                    ("backend", |s| s["backend"] = json!("codex")),
                    ("native version", |s| s["native_version"] = json!("2.1.301")),
                    ("cols differ from the entry", |s| {
                        s["cols"] = json!(s["cols"].as_u64().unwrap() + 1)
                    }),
                    ("rows", |s| s["rows"] = json!(35)),
                    ("line count", |s| {
                        s["lines"].as_array_mut().unwrap().pop();
                    }),
                    ("extra field", |s| s["note"] = json!("unreviewed")),
                    ("cursor past the geometry", |s| {
                        s["cursor_col"] = json!(s["cols"].as_u64().unwrap() + 1)
                    }),
                ];
                for (case, change) in screen_changes {
                    assert!(
                        guarded_fixture(id, |_| {}, change).is_err(),
                        "{id} {case} must be rejected"
                    );
                }
                let (source, bytes) = fixture(id);
                let mut tampered = bytes.to_vec();
                tampered.push(b'\n');
                let mut with_tampered = embedded();
                with_tampered.retain(|&(cited, _)| cited != source);
                with_tampered.push((source, &tampered));
                assert!(
                    ProfileCatalog::from_bundled_definitions(&definitions(), &with_tampered)
                        .is_err(),
                    "{id} digest mismatch"
                );
                let mut missing = embedded();
                missing.retain(|&(cited, _)| cited != source);
                assert!(
                    ProfileCatalog::from_bundled_definitions(&definitions(), &missing).is_err(),
                    "{id} cited fixture is embedded"
                );
            }
        }

        fn sequenced_replay(
            bytes: &[u8],
            change: impl FnOnce(&mut Vec<String>),
            output_seq: u64,
        ) -> ScreenObservation {
            let catalog = ProfileCatalog::bundled().unwrap();
            let screen = super::super::detector::inspection_tests::replay_native_fixture(
                std::str::from_utf8(bytes).unwrap(),
                change,
                true,
                None,
            );
            let mut observation =
                Detector::new(&catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
                    .unwrap()
                    .observe(&screen)
                    .unwrap();
            observation.output_seq = output_seq;
            observation.raw_evidence["screen_seq"] = json!(output_seq);
            observation
        }

        #[test]
        fn bundled_claude_resize_between_wrapped_and_unwrapped_renderings_replaces_the_episode() {
            let wrapped = |output_seq| sequenced_replay(WRAPPED_BYTES, |_| {}, output_seq);
            let unwrapped = |output_seq| {
                sequenced_replay(
                    NATIVE_BYTES,
                    |lines| {
                        lines[18] =
                            " /tmp/cafleet-194/claude-wrap-cells-001/permission-probe allow".into()
                    },
                    output_seq,
                )
            };
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &wrapped(1), origin);
            let mut episode = 1;
            for (start, rendering) in [(200, unwrapped(2)), (800, wrapped(3))] {
                let pending = tracker
                    .observe(&rendering, origin + Duration::from_millis(start), NOW)
                    .unwrap();
                assert!(
                    pending.transitions.is_empty(),
                    "the rewrap at {start} ms first awaits stability: {:?}",
                    pending.transitions
                );
                let replaced = tracker
                    .observe(&rendering, origin + Duration::from_millis(start + 100), NOW)
                    .unwrap();
                assert_eq!(replaced.transitions.len(), 2, "{:?}", replaced.transitions);
                assert_eq!(
                    replaced.transitions[0].kind,
                    RuntimeEventKind::InteractionClosed
                );
                assert_eq!(
                    replaced.transitions[0].close_reason,
                    Some(InteractionCloseReason::Superseded),
                    "the rewrap at {start} ms displays different operation rows"
                );
                assert_eq!(replaced.transitions[0].episode_seq, Some(episode));
                assert_eq!(
                    replaced.transitions[1].kind,
                    RuntimeEventKind::InteractionOpened
                );
                episode += 1;
                assert_eq!(replaced.transitions[1].episode_seq, Some(episode));
            }
            let redraw = tracker
                .observe(&wrapped(4), origin + Duration::from_millis(1200), NOW)
                .unwrap();
            assert!(
                redraw.transitions.is_empty(),
                "a redraw of the same displayed rows keeps the episode: {:?}",
                redraw.transitions
            );
            assert!(matches!(redraw.visibility, Visibility::Visible));
        }
    }

    mod bundled_codex {
        use super::*;
        use sha2::{Digest, Sha256};
        use std::collections::BTreeSet;

        const CODEX_VERSION: &str = "0.163.0";
        const PROFILE_VERSION: &str = "codex-0.163.0-on-request-1";
        const PERMISSION_RULE: &str = "codex-exec-approval";
        const IDLE_RULE: &str = "codex-idle-composer";
        const PERMISSION_FIXTURE: &str = "codex-0.163.0-on-request-permission-1";
        const ALLOW_FIXTURE: &str = "codex-0.163.0-allow-pre-stage-1";
        const DENY_FIXTURE: &str = "codex-0.163.0-deny-pre-stage-1";
        const IDLE_FIXTURE: &str = "codex-0.163.0-idle-composer-1";
        const PERMISSION_BYTES: &[u8] =
            include_bytes!("profiles/native/codex-0.163.0-on-request-permission-1.json");
        const ALLOW_BYTES: &[u8] =
            include_bytes!("profiles/native/codex-0.163.0-allow-pre-stage-1.json");
        const DENY_BYTES: &[u8] =
            include_bytes!("profiles/native/codex-0.163.0-deny-pre-stage-1.json");
        const IDLE_BYTES: &[u8] =
            include_bytes!("profiles/native/codex-0.163.0-idle-composer-1.json");
        pub(super) const FIXTURES: [(&str, &str, &[u8]); 4] = [
            (
                PERMISSION_FIXTURE,
                "native/codex-0.163.0-on-request-permission-1.json",
                PERMISSION_BYTES,
            ),
            (
                ALLOW_FIXTURE,
                "native/codex-0.163.0-allow-pre-stage-1.json",
                ALLOW_BYTES,
            ),
            (
                DENY_FIXTURE,
                "native/codex-0.163.0-deny-pre-stage-1.json",
                DENY_BYTES,
            ),
            (
                IDLE_FIXTURE,
                "native/codex-0.163.0-idle-composer-1.json",
                IDLE_BYTES,
            ),
        ];
        const APPROVAL_HEADING: &str = "  Would you like to run the following command?";
        const ALLOW_CHOICE: &str = "› 1. Yes, proceed (y)";
        const DENY_CHOICE: &str = "  3. No, and tell Codex what to do differently (esc)";
        const CLOSING_LINE: &str = "  Press enter to confirm or esc to cancel";

        type ScreenChange = (&'static str, fn(&mut Value));

        fn fixture(id: &str) -> (&'static str, &'static [u8]) {
            FIXTURES
                .iter()
                .find(|(fixture, _, _)| *fixture == id)
                .map(|&(_, source, bytes)| (source, bytes))
                .expect("known bundled Codex fixture")
        }

        fn lines(bytes: &[u8]) -> Vec<String> {
            serde_json::from_slice::<Value>(bytes).unwrap()["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap().to_owned())
                .collect()
        }

        fn digest(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        fn definitions() -> Value {
            serde_json::from_str(include_str!("profiles/bundled.json")).unwrap()
        }

        fn entry<'a>(definitions: &'a mut Value, id: &str) -> &'a mut Value {
            definitions["fixtures"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["id"] == id)
                .expect("bundled catalog cites the Codex fixture")
        }

        fn guarded_fixture(
            id: &str,
            change_definitions: impl FnOnce(&mut Value),
            change_screen: impl FnOnce(&mut Value),
        ) -> Result<ProfileCatalog, CafleetError> {
            let (source, original) = fixture(id);
            let mut screen: Value = serde_json::from_slice(original).unwrap();
            change_screen(&mut screen);
            let bytes = serde_json::to_vec_pretty(&screen).unwrap();
            let mut value = definitions();
            entry(&mut value, id)["sha256"] = json!(digest(&bytes));
            change_definitions(&mut value);
            let mut embedded = super::bundled_claude::embedded();
            embedded.retain(|&(cited, _)| cited != source);
            embedded.push((source, &bytes));
            ProfileCatalog::from_bundled_definitions(&value, &embedded)
        }

        fn bundled_observation(bytes: &[u8]) -> ScreenObservation {
            let catalog = ProfileCatalog::bundled().unwrap();
            let screen = super::super::detector::inspection_tests::replay_native_fixture(
                std::str::from_utf8(bytes).unwrap(),
                |_| {},
                true,
                None,
            );
            Detector::new(&catalog, CodingAgent::Codex, Some(CODEX_VERSION))
                .unwrap()
                .observe(&screen)
                .unwrap()
        }

        #[test]
        fn bundled_codex_fixtures_match_their_catalog_entries_and_declare_masked_header_rows() {
            let mut value = definitions();
            let header: Vec<usize> = (0..12).collect();
            for (id, sequence, cursor) in [
                (PERMISSION_FIXTURE, 25293, (35, 120)),
                (ALLOW_FIXTURE, 24241, (35, 120)),
                (DENY_FIXTURE, 25695, (35, 120)),
                (IDLE_FIXTURE, 18164, (32, 2)),
            ] {
                let (source, bytes) = fixture(id);
                assert_eq!(
                    entry(&mut value, id),
                    &json!({"id":id,"kind":"native","backend":"codex","native_version":CODEX_VERSION,
                        "rows":36,"cols":120,"source_ref":source,"sha256":digest(bytes)})
                );
                let screen: Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(
                    screen
                        .as_object()
                        .unwrap()
                        .keys()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>(),
                    BTreeSet::from([
                        "schema_version",
                        "fixture_id",
                        "provenance",
                        "backend",
                        "native_version",
                        "rows",
                        "cols",
                        "alternate_screen",
                        "screen_seq",
                        "cursor_row",
                        "cursor_col",
                        "lines",
                        "masked_rows",
                    ]),
                    "{id} has exactly the documented fields"
                );
                assert_eq!(screen["schema_version"], 1);
                assert_eq!(screen["fixture_id"], id);
                assert_eq!(screen["provenance"], "native");
                assert_eq!(screen["backend"], "codex");
                assert_eq!(screen["native_version"], CODEX_VERSION);
                assert_eq!(
                    (screen["rows"].clone(), screen["cols"].clone()),
                    (json!(36), json!(120))
                );
                assert_eq!(screen["alternate_screen"], true, "{id}");
                assert_eq!(screen["screen_seq"], sequence);
                assert_eq!(
                    (screen["cursor_row"].clone(), screen["cursor_col"].clone()),
                    (json!(cursor.0), json!(cursor.1)),
                    "{id}"
                );
                assert_eq!(
                    screen["masked_rows"],
                    json!(
                        header
                            .iter()
                            .map(|row| json!({"row":row,"reason":"reviewed_privacy"}))
                            .collect::<Vec<_>>()
                    )
                );
                let rows = lines(bytes);
                assert_eq!(rows.len(), 36);
                for (row, line) in rows.iter().enumerate() {
                    if header.contains(&row) {
                        assert_eq!(line, "", "{id} masked row {row} is empty");
                    }
                    for private in ["/Users/", "/root", "himkt"] {
                        assert!(
                            !line.contains(private),
                            "{id} row {row} carries {private:?}"
                        );
                    }
                }
            }
            let permission = lines(PERMISSION_BYTES);
            assert_eq!(permission[22], APPROVAL_HEADING);
            assert_eq!(
                permission[28],
                "  $ /tmp/cafleet-194/codex-cells-001/permission-probe allow"
            );
            assert_eq!(permission[31].trim_end(), ALLOW_CHOICE);
            assert!(
                permission[32].starts_with("  2. Yes, and don't ask again")
                    && permission[32].ends_with("` (p)"),
                "choice 2 fits one row"
            );
            assert_eq!(permission[33], DENY_CHOICE);
            assert_eq!(permission[35].trim_end(), CLOSING_LINE);
            assert_eq!(
                permission[35].chars().count(),
                120,
                "the closing row fills the width"
            );
            for (bytes, operation) in [
                (
                    ALLOW_BYTES,
                    "  $ /tmp/cafleet-194/codex-ans-allow-001/permission-probe allow",
                ),
                (
                    DENY_BYTES,
                    "  $ /tmp/cafleet-194/codex-ans-deny-001/permission-probe deny",
                ),
            ] {
                let rows = lines(bytes);
                assert_eq!(rows[21], APPROVAL_HEADING);
                assert_eq!(rows[27], operation);
                assert_eq!(rows[30].trim_end(), ALLOW_CHOICE);
                assert!(
                    rows[31].ends_with('`'),
                    "choice 2 continues on the next row"
                );
                assert_eq!(rows[32], "     (p)");
                assert_eq!(rows[33], DENY_CHOICE);
                assert_eq!(rows[35].trim_end(), CLOSING_LINE);
                assert_eq!(rows[35].chars().count(), 120);
            }
            assert_eq!(lines(IDLE_BYTES)[32], "› Ask Codex to do anything");
        }

        #[test]
        fn bundled_codex_0_163_0_declares_detection_idle_composer_and_both_answers_only() {
            let bundled = ProfileCatalog::bundled().unwrap();
            let codex = bundled.capabilities(CodingAgent::Codex, Some(CODEX_VERSION));
            assert_eq!(codex.backend, CodingAgent::Codex);
            assert_eq!(codex.native_version.as_deref(), Some(CODEX_VERSION));
            assert_eq!(codex.profile_version.as_deref(), Some(PROFILE_VERSION));
            assert_eq!(codex.detection, Coverage::Tested);
            assert_eq!(codex.composer, Coverage::Tested);
            assert_eq!(codex.prompt_submission, Coverage::Unknown);
            assert_eq!(codex.native_exec_submission, Coverage::Unknown);
            assert_eq!(codex.answer_decisions.len(), 2);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(codex.answer_decisions.contains(&decision), "{decision:?}");
            }
            for version in [None, Some("0.162.0"), Some("0.164.0")] {
                let unknown = bundled.capabilities(CodingAgent::Codex, version);
                assert_eq!(unknown.detection, Coverage::Unknown, "{version:?}");
                assert_eq!(unknown.composer, Coverage::Unknown, "{version:?}");
                assert!(unknown.answer_decisions.is_empty());
                assert!(unknown.capability_reason.is_some());
            }
            for &backend in CodingAgent::all()
                .iter()
                .filter(|&&backend| backend != CodingAgent::Codex)
            {
                let other = bundled.capabilities(backend, Some(CODEX_VERSION));
                assert_eq!(other.detection, Coverage::Unknown, "{backend:?}");
                assert!(other.answer_decisions.is_empty());
            }
        }

        #[test]
        fn bundled_codex_answer_plans_are_the_measured_single_byte_stages() {
            let bundled = ProfileCatalog::bundled().unwrap();
            for (decision, byte, choice, answer_rule, provenance) in [
                (
                    PermissionDecision::Allow,
                    0x79,
                    "allow-once",
                    "codex-exec-allow-y",
                    ALLOW_FIXTURE,
                ),
                (
                    PermissionDecision::Deny,
                    0x33,
                    "deny-once",
                    "codex-exec-deny-digit",
                    DENY_FIXTURE,
                ),
            ] {
                let plan = bundled
                    .answer_plan(CodingAgent::Codex, CODEX_VERSION, PERMISSION_RULE, decision)
                    .unwrap()
                    .expect("measured bundled answer plan");
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.focus_matcher_id(), "focus");
                assert_eq!(plan.stages().len(), 1, "{decision:?} is one measured stage");
                let stage = &plan.stages()[0];
                assert_eq!(stage.bytes(), &[byte]);
                assert_eq!(stage.required_rule_id(), PERMISSION_RULE);
                assert_eq!(stage.required_choice_id(), choice);
                let capability = plan.capability();
                assert_eq!(capability.backend(), CodingAgent::Codex);
                assert_eq!(capability.native_version(), CODEX_VERSION);
                assert_eq!(capability.profile_version(), PROFILE_VERSION);
                assert_eq!(capability.request_rule_id(), PERMISSION_RULE);
                assert_eq!(capability.answer_rule_id(), answer_rule);
                assert_eq!(capability.decision(), decision);
                assert_eq!(capability.fixture_ids(), &[provenance.to_owned()]);
                for (backend, version) in [
                    (CodingAgent::Codex, "0.162.0"),
                    (CodingAgent::Claude, CODEX_VERSION),
                    (CodingAgent::Opencode, CODEX_VERSION),
                ] {
                    assert!(
                        bundled
                            .answer_plan(backend, version, PERMISSION_RULE, decision)
                            .unwrap()
                            .is_none(),
                        "{backend:?} {version}"
                    );
                }
            }
        }

        #[test]
        fn bundled_guard_rejects_synthetic_or_altered_codex_fixtures() {
            for (id, _, _) in FIXTURES {
                assert!(guarded_fixture(id, |_| {}, |_| {}).is_ok(), "{id}");
                assert!(
                    guarded_fixture(id, |d| entry(d, id)["kind"] = json!("synthetic"), |_| {})
                        .is_err(),
                    "{id} synthetic catalog kind"
                );
                let screen_changes: [ScreenChange; 6] = [
                    ("synthetic provenance", |s| {
                        s["provenance"] = json!("synthetic")
                    }),
                    ("nonempty masked row", |s| {
                        s["lines"][0] = json!("restored header")
                    }),
                    ("backend", |s| s["backend"] = json!("claude")),
                    ("native version", |s| s["native_version"] = json!("0.162.0")),
                    ("extra field", |s| s["note"] = json!("unreviewed")),
                    ("line count", |s| {
                        s["lines"].as_array_mut().unwrap().pop();
                    }),
                ];
                for (case, change) in screen_changes {
                    assert!(
                        guarded_fixture(id, |_| {}, change).is_err(),
                        "{id} {case} must be rejected"
                    );
                }
                let (source, bytes) = fixture(id);
                let mut tampered = bytes.to_vec();
                tampered.push(b'\n');
                let mut with_tampered = super::bundled_claude::embedded();
                with_tampered.retain(|&(cited, _)| cited != source);
                with_tampered.push((source, &tampered));
                assert!(
                    ProfileCatalog::from_bundled_definitions(&definitions(), &with_tampered)
                        .is_err(),
                    "{id} digest mismatch"
                );
            }
        }

        #[test]
        fn bundled_guard_accepts_a_pending_wrap_cursor_at_cols_and_rejects_one_past_it() {
            assert!(
                guarded_fixture(PERMISSION_FIXTURE, |_| {}, |s| s["cursor_col"] = json!(120))
                    .is_ok(),
                "cursor_col equal to cols records the pending-wrap position"
            );
            assert!(
                guarded_fixture(IDLE_FIXTURE, |_| {}, |s| s["cursor_col"] = json!(120)).is_ok(),
                "the pending-wrap position is valid on any row"
            );
            for (case, change) in [
                ("one past the pending-wrap column", (35, 121)),
                ("a row below the geometry", (36, 0)),
            ] {
                assert!(
                    guarded_fixture(
                        PERMISSION_FIXTURE,
                        |_| {},
                        |s| {
                            s["cursor_row"] = json!(change.0);
                            s["cursor_col"] = json!(change.1);
                        }
                    )
                    .is_err(),
                    "{case} must be rejected"
                );
            }
        }

        #[test]
        fn bundled_codex_episode_closes_not_visible_once_the_idle_composer_holds_250_ms() {
            let permission = bundled_observation(PERMISSION_BYTES);
            let idle = bundled_observation(IDLE_BYTES);
            assert_eq!(idle.rule_id, IDLE_RULE);
            assert!(idle.verified_noninteraction);
            let at = |observation: &ScreenObservation, output_seq: u64| {
                let mut replayed = observation.clone();
                replayed.output_seq = output_seq;
                replayed.raw_evidence["screen_seq"] = json!(output_seq);
                replayed
            };
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &at(&permission, 1), origin);
            let closed = |update: &EpisodeUpdate| {
                update
                    .transitions
                    .iter()
                    .filter(|t| t.kind == RuntimeEventKind::InteractionClosed)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let flash = tracker
                .observe(&at(&idle, 2), origin + Duration::from_millis(200), NOW)
                .unwrap();
            assert!(closed(&flash).is_empty(), "idle shown for 0 ms stays open");
            let back = tracker
                .observe(
                    &at(&permission, 3),
                    origin + Duration::from_millis(300),
                    NOW,
                )
                .unwrap();
            assert!(
                closed(&back).is_empty(),
                "an idle flash under 250 ms keeps the episode"
            );
            let first = tracker
                .observe(&at(&idle, 4), origin + Duration::from_millis(400), NOW)
                .unwrap();
            assert!(closed(&first).is_empty());
            let held = tracker
                .observe(&at(&idle, 5), origin + Duration::from_millis(650), NOW)
                .unwrap();
            let closes = closed(&held);
            assert_eq!(closes.len(), 1, "{:?}", held.transitions);
            assert_eq!(
                closes[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
        }
    }

    mod bundled_opencode {
        use super::*;
        use sha2::{Digest, Sha256};
        use std::collections::BTreeSet;

        const OPENCODE_VERSION: &str = "2.1.4";
        const PROFILE_VERSION: &str = "opencode-2.1.4-cafleet-ask-1";
        const PERMISSION_RULE: &str = "opencode-shell-permission";
        const REASON_RULE: &str = "opencode-reject-reason";
        const IDLE_RULE: &str = "opencode-idle-composer";
        const PERMISSION_FIXTURE: &str = "opencode-2.1.4-ask-permission-1";
        const ALLOW_FIXTURE: &str = "opencode-2.1.4-allow-pre-stage-1";
        const DENY_1_FIXTURE: &str = "opencode-2.1.4-deny-pre-stage-1";
        const DENY_2_FIXTURE: &str = "opencode-2.1.4-deny-pre-stage-2";
        const DENY_3_FIXTURE: &str = "opencode-2.1.4-deny-pre-stage-3";
        const ALWAYS_FIXTURE: &str = "opencode-2.1.4-focus-always-1";
        const IDLE_FIXTURE: &str = "opencode-2.1.4-idle-composer-1";
        const PERMISSION_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-ask-permission-1.json");
        const ALLOW_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-allow-pre-stage-1.json");
        const DENY_1_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-deny-pre-stage-1.json");
        const DENY_2_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-deny-pre-stage-2.json");
        const DENY_3_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-deny-pre-stage-3.json");
        const ALWAYS_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-focus-always-1.json");
        const IDLE_BYTES: &[u8] =
            include_bytes!("profiles/native/opencode-2.1.4-idle-composer-1.json");
        pub(super) const FIXTURES: [(&str, &str, &[u8]); 7] = [
            (
                PERMISSION_FIXTURE,
                "native/opencode-2.1.4-ask-permission-1.json",
                PERMISSION_BYTES,
            ),
            (
                ALLOW_FIXTURE,
                "native/opencode-2.1.4-allow-pre-stage-1.json",
                ALLOW_BYTES,
            ),
            (
                DENY_1_FIXTURE,
                "native/opencode-2.1.4-deny-pre-stage-1.json",
                DENY_1_BYTES,
            ),
            (
                DENY_2_FIXTURE,
                "native/opencode-2.1.4-deny-pre-stage-2.json",
                DENY_2_BYTES,
            ),
            (
                DENY_3_FIXTURE,
                "native/opencode-2.1.4-deny-pre-stage-3.json",
                DENY_3_BYTES,
            ),
            (
                ALWAYS_FIXTURE,
                "native/opencode-2.1.4-focus-always-1.json",
                ALWAYS_BYTES,
            ),
            (
                IDLE_FIXTURE,
                "native/opencode-2.1.4-idle-composer-1.json",
                IDLE_BYTES,
            ),
        ];
        const FORM_FIXTURES: [(&[u8], &str, &str); 5] = [
            (PERMISSION_BYTES, "opencode-cells-001", "allow"),
            (ALLOW_BYTES, "opencode-ans-allow-001", "allow"),
            (DENY_1_BYTES, "opencode-ans-deny-001", "deny"),
            (DENY_2_BYTES, "opencode-ans-deny-002", "deny"),
            (ALWAYS_BYTES, "opencode-focus-always-002", "deny"),
        ];
        const HEADING: &str = "  △ Permission required";
        const CHOICE_ROW: &str = "   Allow once   Always allow   Reject                                             ⇆ select  enter confirm  esc reject";
        const REASON_HEADING: &str = "  △ Reject permission";
        const REASON_CONTROL: &str = "  Tell OpenCode what to do differently                                                      enter confirm  esc cancel";

        type ScreenChange = (&'static str, fn(&mut Value));
        type LineChange = Box<dyn Fn(&mut Vec<String>)>;

        fn source(id: &str) -> &'static str {
            FIXTURES
                .iter()
                .find(|(fixture, _, _)| *fixture == id)
                .map(|&(_, source, _)| source)
                .expect("known bundled OpenCode fixture")
        }

        fn fixture(id: &str) -> &'static [u8] {
            FIXTURES
                .iter()
                .find(|(fixture, _, _)| *fixture == id)
                .map(|&(_, _, bytes)| bytes)
                .expect("known bundled OpenCode fixture")
        }

        fn parsed(bytes: &[u8]) -> Value {
            serde_json::from_slice(bytes).unwrap()
        }

        fn lines(bytes: &[u8]) -> Vec<String> {
            parsed(bytes)["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap().to_owned())
                .collect()
        }

        fn digest(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        fn definitions() -> Value {
            serde_json::from_str(include_str!("profiles/bundled.json")).unwrap()
        }

        fn entry<'a>(definitions: &'a mut Value, id: &str) -> &'a mut Value {
            definitions["fixtures"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["id"] == id)
                .expect("bundled catalog cites the OpenCode fixture")
        }

        fn profile(definitions: &Value) -> &Value {
            definitions["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|profile| profile["backend"] == "opencode")
                .expect("bundled OpenCode profile")
        }

        fn rule<'a>(definitions: &'a Value, id: &str) -> &'a Value {
            profile(definitions)["rules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rule| rule["id"] == id)
                .expect("bundled OpenCode rule")
        }

        fn guarded_fixture(
            id: &str,
            change_screen: impl FnOnce(&mut Value),
        ) -> Result<ProfileCatalog, CafleetError> {
            let mut screen = parsed(fixture(id));
            change_screen(&mut screen);
            let bytes = serde_json::to_vec_pretty(&screen).unwrap();
            let mut value = definitions();
            entry(&mut value, id)["sha256"] = json!(digest(&bytes));
            let cited = source(id);
            let mut embedded = super::bundled_claude::embedded();
            embedded.retain(|&(reference, _)| reference != cited);
            embedded.push((cited, &bytes));
            ProfileCatalog::from_bundled_definitions(&value, &embedded)
        }

        fn replay(bytes: &[u8]) -> ScreenSnapshot {
            replay_changed(bytes, |_| {}, None)
        }

        fn replay_changed(
            bytes: &[u8],
            change: impl FnOnce(&mut Vec<String>),
            cursor: Option<(u16, u16)>,
        ) -> ScreenSnapshot {
            super::super::detector::inspection_tests::replay_native_fixture(
                std::str::from_utf8(bytes).unwrap(),
                change,
                false,
                cursor,
            )
        }

        fn detector() -> Detector {
            Detector::new(
                &ProfileCatalog::bundled().unwrap(),
                CodingAgent::Opencode,
                Some(OPENCODE_VERSION),
            )
            .unwrap()
        }

        fn observe(screen: &ScreenSnapshot) -> ScreenObservation {
            detector().observe(screen).unwrap()
        }

        fn plan(decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::bundled()
                .unwrap()
                .answer_plan(
                    CodingAgent::Opencode,
                    OPENCODE_VERSION,
                    PERMISSION_RULE,
                    decision,
                )
                .unwrap()
                .expect("measured bundled OpenCode answer plan")
        }

        fn focused(screen: &ScreenSnapshot) -> Option<String> {
            let detector = detector();
            let mut resolved = BTreeSet::new();
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let matched = detector
                    .match_answer(screen, &plan(decision))
                    .unwrap()
                    .expect("the form matches both plans whatever its focus");
                resolved.insert(matched.focused_choice_id().map(str::to_owned));
            }
            assert_eq!(resolved.len(), 1, "both plans share the focus matcher");
            resolved.pop_first().unwrap()
        }

        fn assert_form(observation: &ScreenObservation, case: &str, probe: &str) {
            assert_eq!(
                observation.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest),
                "{case}"
            );
            assert_eq!(observation.rule_id, PERMISSION_RULE, "{case}");
            assert_eq!(
                observation.match_evidence,
                MatchEvidence::Complete,
                "{case}"
            );
            assert_eq!(
                observation.profile_version.as_deref(),
                Some(PROFILE_VERSION)
            );
            assert!(!observation.verified_noninteraction);
            assert_eq!(
                observation.raw_evidence["operation"],
                json!(format!("/tmp/cafleet-194/{case}/permission-probe {probe}")),
                "{case}"
            );
            assert_eq!(observation.raw_evidence["operation_rows"], json!(1));
            assert_eq!(
                observation.raw_evidence["choices"],
                json!(["Allow once", "Reject"])
            );
            assert_eq!(observation.raw_evidence["native_request_id"], Value::Null);
            assert_eq!(observation.native_correlation, None);
            let region = observation.matched_region.expect("live form region");
            assert_eq!((region.top, region.bottom), (10, 22), "{case}");
        }

        #[test]
        fn bundled_opencode_fixtures_match_their_catalog_entries_and_declare_the_masked_row() {
            let mut value = definitions();
            for (id, _, bytes) in FIXTURES {
                assert_eq!(
                    entry(&mut value, id),
                    &json!({"id":id,"kind":"native","backend":"opencode","native_version":OPENCODE_VERSION,
                        "rows":36,"cols":120,"source_ref":source(id),"sha256":digest(bytes)})
                );
                let screen = parsed(bytes);
                let mut expected = BTreeSet::from([
                    "schema_version",
                    "fixture_id",
                    "provenance",
                    "backend",
                    "native_version",
                    "rows",
                    "cols",
                    "alternate_screen",
                    "screen_seq",
                    "cursor_row",
                    "cursor_col",
                    "lines",
                    "masked_rows",
                ]);
                if id != IDLE_FIXTURE {
                    expected.insert("cell_backgrounds");
                }
                assert_eq!(
                    screen
                        .as_object()
                        .unwrap()
                        .keys()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>(),
                    expected,
                    "{id} has exactly the documented fields"
                );
                assert_eq!(screen["schema_version"], 1);
                assert_eq!(screen["fixture_id"], id);
                assert_eq!(screen["provenance"], "native");
                assert_eq!(screen["backend"], "opencode");
                assert_eq!(screen["native_version"], OPENCODE_VERSION);
                assert_eq!(
                    (screen["rows"].clone(), screen["cols"].clone()),
                    (json!(36), json!(120))
                );
                assert_eq!(
                    screen["alternate_screen"], false,
                    "{id} uses the primary screen"
                );
                assert_eq!(
                    screen["masked_rows"],
                    json!([{"row":35,"reason":"reviewed_privacy"}])
                );
                let rows = lines(bytes);
                assert_eq!(rows.len(), 36);
                assert_eq!(rows[35], "");
                for (row, line) in rows.iter().enumerate() {
                    for private in ["/Users/", "/root", "/home/", "himkt"] {
                        assert!(
                            !line.contains(private),
                            "{id} row {row} carries {private:?}"
                        );
                    }
                }
            }
            for (bytes, case, probe) in FORM_FIXTURES {
                let rows = lines(bytes);
                assert_eq!(rows[10].trim_end(), HEADING, "{case}");
                assert!(
                    rows[14].starts_with(&format!(
                        "  $ /tmp/cafleet-194/{case}/permission-probe {probe} "
                    )),
                    "{case}"
                );
                assert_eq!(rows[21].trim_end(), CHOICE_ROW, "{case}");
                assert_eq!(rows[21].chars().count(), 120);
            }
            let dialog = lines(DENY_3_BYTES);
            assert_eq!(dialog[10].trim_end(), REASON_HEADING);
            assert_eq!(
                dialog[12].trim_end(),
                "  Tell OpenCode what to do differently"
            );
            assert_eq!(dialog[21].trim_end(), REASON_CONTROL);
            let idle = lines(IDLE_BYTES);
            assert_eq!(idle[11].trim_end(), "┃");
            assert_eq!(
                (
                    parsed(IDLE_BYTES)["cursor_row"].clone(),
                    parsed(IDLE_BYTES)["cursor_col"].clone()
                ),
                (json!(11), json!(2))
            );
        }

        #[test]
        fn bundled_opencode_2_1_4_declares_detection_idle_composer_and_both_answers_only() {
            let bundled = ProfileCatalog::bundled().unwrap();
            let opencode = bundled.capabilities(CodingAgent::Opencode, Some(OPENCODE_VERSION));
            assert_eq!(opencode.backend, CodingAgent::Opencode);
            assert_eq!(opencode.native_version.as_deref(), Some(OPENCODE_VERSION));
            assert_eq!(opencode.profile_version.as_deref(), Some(PROFILE_VERSION));
            assert_eq!(opencode.detection, Coverage::Tested);
            assert_eq!(opencode.composer, Coverage::Tested);
            assert_eq!(opencode.prompt_submission, Coverage::Unknown);
            assert_eq!(opencode.native_exec_submission, Coverage::Unknown);
            assert_eq!(opencode.answer_decisions.len(), 2);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(
                    opencode.answer_decisions.contains(&decision),
                    "{decision:?}"
                );
            }
            assert_eq!(opencode.capability_reason, None);
            for version in [None, Some("2.1.3"), Some("2.1.5"), Some("v2.1.4")] {
                let unknown = bundled.capabilities(CodingAgent::Opencode, version);
                assert_eq!(unknown.detection, Coverage::Unknown, "{version:?}");
                assert_eq!(unknown.composer, Coverage::Unknown, "{version:?}");
                assert_eq!(unknown.profile_version, None, "{version:?}");
                assert!(unknown.answer_decisions.is_empty());
                assert!(unknown.capability_reason.is_some());
            }
            for backend in [CodingAgent::Claude, CodingAgent::Codex] {
                let other = bundled.capabilities(backend, Some(OPENCODE_VERSION));
                assert_eq!(other.detection, Coverage::Unknown, "{backend:?}");
                assert!(other.answer_decisions.is_empty());
            }
            let definitions = definitions();
            let capabilities = &profile(&definitions)["capabilities"];
            assert_eq!(
                profile(&definitions)["native_versions"],
                json!([OPENCODE_VERSION])
            );
            assert_eq!(profile(&definitions)["profile_version"], PROFILE_VERSION);
            assert_eq!(
                capabilities["detection"]["rule_ids"],
                json!([PERMISSION_RULE])
            );
            assert_eq!(
                capabilities["detection"]["fixture_ids"],
                json!([PERMISSION_FIXTURE])
            );
            assert_eq!(capabilities["composer"]["rule_ids"], json!([IDLE_RULE]));
            assert_eq!(
                capabilities["composer"]["fixture_ids"],
                json!([IDLE_FIXTURE])
            );
            assert!(capabilities["composer"].get("submission").is_none());
            let reason = rule(&definitions, REASON_RULE);
            assert_eq!(reason["observation"], "permission_continuation");
            assert_eq!(reason["continues_rule_id"], PERMISSION_RULE);
            assert_eq!(reason["continues_choice_id"], "deny-once");
            assert_eq!(reason["fixture_ids"], json!([DENY_3_FIXTURE]));
            assert_eq!(
                reason["extract"],
                json!({"operation":null,"choices":[],"native_request_id":null})
            );
            assert_eq!(reason["region"]["cursor"], "any");
            let form = rule(&definitions, PERMISSION_RULE);
            assert_eq!(form["observation"], "permission_request");
            assert_eq!(form["region"]["screen"], "primary");
            assert_eq!(form["region"]["cursor"], "any");
            assert_eq!(form["region"]["max_rows"], 24);
            assert_eq!(form["sensitive"], false);
            assert_eq!(form["extract"]["native_request_id"], Value::Null);
            assert_eq!(
                form["extract"]["choices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|choice| (choice["id"].clone(), choice["scope"].clone()))
                    .collect::<Vec<_>>(),
                [
                    (json!("allow-once"), json!("single_request")),
                    (json!("deny-once"), json!("single_request"))
                ],
                "the always choice has no choice entry"
            );
            let ids = form["matchers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|matcher| matcher["id"].as_str().unwrap())
                .collect::<BTreeSet<_>>();
            for id in ["heading", "operation", "focus", "allow", "deny"] {
                assert!(ids.contains(id), "required matcher {id}");
            }
            let focus = form["matchers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|matcher| matcher["id"] == "focus")
                .unwrap();
            assert_eq!(focus["focus_style"], "background");
            let idle = rule(&definitions, IDLE_RULE);
            assert_eq!(idle["observation"], "composer_empty");
            assert_eq!(idle["region"]["cursor"], "at_end");
            assert_eq!(idle["verified_noninteraction"], true);
        }

        #[test]
        fn bundled_opencode_answer_plans_are_the_measured_gated_stages() {
            let allow = plan(PermissionDecision::Allow);
            let deny = plan(PermissionDecision::Deny);
            for (plan, decision, choice, answer_rule, fixtures, stages) in [
                (
                    &allow,
                    PermissionDecision::Allow,
                    "allow-once",
                    "opencode-shell-allow-once",
                    vec![ALLOW_FIXTURE.to_owned()],
                    vec![(vec![0x0d], Some("allow-once"), None)],
                ),
                (
                    &deny,
                    PermissionDecision::Deny,
                    "deny-once",
                    "opencode-shell-deny-once",
                    vec![
                        DENY_1_FIXTURE.to_owned(),
                        DENY_2_FIXTURE.to_owned(),
                        DENY_3_FIXTURE.to_owned(),
                    ],
                    vec![
                        (vec![0x68], Some("allow-once"), None),
                        (vec![0x0d], Some("deny-once"), None),
                        (vec![0x0d], None, Some(REASON_RULE)),
                    ],
                ),
            ] {
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.focus_matcher_id(), "focus");
                let actual = plan
                    .stages()
                    .iter()
                    .map(|stage| {
                        assert_eq!(stage.required_rule_id(), PERMISSION_RULE);
                        assert_eq!(stage.required_choice_id(), choice);
                        assert!(!stage.bytes().contains(&0x1b), "no stage sends Escape");
                        (
                            stage.bytes().to_vec(),
                            stage.required_focus_choice_id(),
                            stage.required_screen_rule_id(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(actual, stages, "{decision:?}");
                let capability = plan.capability();
                assert_eq!(capability.backend(), CodingAgent::Opencode);
                assert_eq!(capability.native_version(), OPENCODE_VERSION);
                assert_eq!(capability.profile_version(), PROFILE_VERSION);
                assert_eq!(capability.request_rule_id(), PERMISSION_RULE);
                assert_eq!(capability.answer_rule_id(), answer_rule);
                assert_eq!(capability.decision(), decision);
                assert_eq!(capability.fixture_ids(), fixtures.as_slice());
                let bundled = ProfileCatalog::bundled().unwrap();
                for (backend, version, request) in [
                    (CodingAgent::Opencode, "2.1.3", PERMISSION_RULE),
                    (CodingAgent::Opencode, OPENCODE_VERSION, REASON_RULE),
                    (CodingAgent::Opencode, OPENCODE_VERSION, IDLE_RULE),
                    (CodingAgent::Codex, OPENCODE_VERSION, PERMISSION_RULE),
                    (CodingAgent::Claude, OPENCODE_VERSION, PERMISSION_RULE),
                ] {
                    assert!(
                        bundled
                            .answer_plan(backend, version, request, decision)
                            .unwrap()
                            .is_none(),
                        "{backend:?} {version} {request}"
                    );
                }
            }
        }

        #[test]
        fn bundled_opencode_detects_each_native_form_from_its_live_region() {
            for (bytes, case, probe) in FORM_FIXTURES {
                let observation = observe(&replay(bytes));
                assert_form(&observation, case, probe);
                assert!(observation.fingerprint.is_some());
            }
            let detector = detector();
            let a = detector.observe(&replay(DENY_1_BYTES)).unwrap();
            let b = detector.observe(&replay(DENY_2_BYTES)).unwrap();
            assert!(
                a.fingerprint != b.fingerprint,
                "different cases carry different operations"
            );
            let cursor_elsewhere = observe(&replay_changed(DENY_1_BYTES, |_| {}, Some((5, 0))));
            assert_form(&cursor_elsewhere, "opencode-ans-deny-001", "deny");
        }

        #[test]
        fn bundled_opencode_focus_resolves_from_the_captured_cell_backgrounds() {
            for (bytes, expected) in [
                (PERMISSION_BYTES, Some("allow-once")),
                (ALLOW_BYTES, Some("allow-once")),
                (DENY_1_BYTES, Some("allow-once")),
                (DENY_2_BYTES, Some("deny-once")),
                (ALWAYS_BYTES, None),
            ] {
                assert_eq!(
                    focused(&replay(bytes)).as_deref(),
                    expected,
                    "{}",
                    parsed(bytes)["fixture_id"]
                );
            }
            let mut text_only = parsed(PERMISSION_BYTES);
            text_only
                .as_object_mut()
                .unwrap()
                .remove("cell_backgrounds");
            let text_only = serde_json::to_vec(&text_only).unwrap();
            let screen = replay(&text_only);
            assert_form(&observe(&screen), "opencode-cells-001", "allow");
            assert_eq!(
                focused(&screen),
                None,
                "the form marks focus by cell background only"
            );
        }

        #[test]
        fn bundled_opencode_focus_resolution_is_independent_of_the_theme_colours() {
            for (bytes, expected) in [
                (PERMISSION_BYTES, Some("allow-once")),
                (DENY_2_BYTES, Some("deny-once")),
                (ALWAYS_BYTES, None),
            ] {
                let mut screen = parsed(bytes);
                for run in screen["cell_backgrounds"].as_array_mut().unwrap() {
                    let rgb = (
                        run["bg"]["r"].clone(),
                        run["bg"]["g"].clone(),
                        run["bg"]["b"].clone(),
                    );
                    run["bg"] = if rgb == (json!(250), json!(178), json!(131)) {
                        json!({"kind":"indexed","index":214})
                    } else if rgb == (json!(30), json!(30), json!(30)) {
                        json!({"kind":"rgb","r":238,"g":238,"b":238})
                    } else {
                        json!({"kind":"indexed","index":250})
                    };
                }
                let rethemed = serde_json::to_vec(&screen).unwrap();
                assert_eq!(focused(&replay(&rethemed)).as_deref(), expected);
            }
        }

        #[test]
        fn bundled_opencode_stage_gates_hold_on_their_pre_stage_fixtures() {
            let allow = plan(PermissionDecision::Allow);
            assert_eq!(
                focused(&replay(ALLOW_BYTES)).as_deref(),
                allow.stages()[0].required_focus_choice_id()
            );
            let deny = plan(PermissionDecision::Deny);
            assert_eq!(
                focused(&replay(DENY_1_BYTES)).as_deref(),
                deny.stages()[0].required_focus_choice_id()
            );
            assert_eq!(
                focused(&replay(DENY_2_BYTES)).as_deref(),
                deny.stages()[1].required_focus_choice_id()
            );
            let dialog = observe(&replay(DENY_3_BYTES));
            assert_eq!(dialog.kind, ObservationKind::PermissionContinuation);
            assert_eq!(dialog.match_evidence, MatchEvidence::Complete);
            assert_eq!(
                Some(dialog.rule_id.as_str()),
                deny.stages()[2].required_screen_rule_id()
            );
            assert!(!dialog.verified_noninteraction);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(
                    detector()
                        .match_answer(&replay(DENY_3_BYTES), &plan(decision))
                        .unwrap()
                        .is_none(),
                    "the rejection-reason dialog is not an answerable form"
                );
            }
            let always = focused(&replay(ALWAYS_BYTES));
            for stage in allow.stages().iter().chain(deny.stages()) {
                if let Some(gate) = stage.required_focus_choice_id() {
                    assert_ne!(
                        always.as_deref(),
                        Some(gate),
                        "no focus gate holds on Always allow"
                    );
                }
            }
        }

        #[test]
        fn bundled_opencode_reject_reason_requires_the_empty_placeholder_textarea() {
            let typed = observe(&replay_changed(
                DENY_3_BYTES,
                |lines| {
                    lines[21] = lines[21].replacen(
                        "Tell OpenCode what to do differently",
                        "run it in the sandbox instead      ",
                        1,
                    )
                },
                None,
            ));
            assert_ne!(typed.kind, ObservationKind::PermissionContinuation);
            assert_ne!(typed.rule_id, REASON_RULE);
            let appended = observe(&replay_changed(
                DENY_3_BYTES,
                |lines| {
                    lines[21] = lines[21].replacen(
                        "Tell OpenCode what to do differently ",
                        "Tell OpenCode what to do differently!",
                        1,
                    )
                },
                None,
            ));
            assert_ne!(
                appended.rule_id, REASON_RULE,
                "the span equals the placeholder exactly"
            );
            let without_prompt = observe(&replay_changed(
                DENY_3_BYTES,
                |lines| lines[12] = String::new(),
                None,
            ));
            assert_ne!(without_prompt.rule_id, REASON_RULE);
        }

        #[test]
        fn bundled_opencode_idle_composer_is_a_verified_non_interaction_boundary() {
            let idle = observe(&replay(IDLE_BYTES));
            assert_eq!(idle.kind, ObservationKind::ComposerEmpty);
            assert_eq!(idle.match_evidence, MatchEvidence::Complete);
            assert!(idle.verified_noninteraction);
            assert_eq!(idle.rule_id, IDLE_RULE);
            assert_eq!(idle.raw_evidence["operation"], Value::Null);
            assert!(idle.fingerprint.is_none());
            let draft = observe(&replay_changed(
                IDLE_BYTES,
                |lines| lines[11] = "┃ hello".into(),
                Some((11, 7)),
            ));
            assert_ne!(draft.kind, ObservationKind::ComposerEmpty, "typed draft");
            assert!(!draft.verified_noninteraction);
            let away = observe(&replay_changed(IDLE_BYTES, |_| {}, Some((11, 0))));
            assert_ne!(
                away.kind,
                ObservationKind::ComposerEmpty,
                "cursor before the prefix end"
            );
            assert!(!away.verified_noninteraction);
            let working = observe(&replay(PERMISSION_BYTES));
            assert!(!working.verified_noninteraction);
            assert_ne!(working.kind, ObservationKind::ComposerEmpty);
        }

        #[test]
        fn bundled_opencode_detects_nothing_from_form_or_dialog_text_outside_a_live_region() {
            let form = lines(PERMISSION_BYTES);
            let dialog = lines(DENY_3_BYTES);
            let cases: [(&str, LineChange); 6] = [
                ("idle screen", Box::new(|_| {})),
                (
                    "quoted heading and command in the transcript",
                    Box::new(|lines: &mut Vec<String>| {
                        lines[5] = HEADING.into();
                        lines[6] =
                            "  $ /tmp/cafleet-194/opencode-cells-001/permission-probe allow".into();
                    }),
                ),
                (
                    "quoted form without its control line",
                    Box::new(move |lines: &mut Vec<String>| {
                        lines[..11].clone_from_slice(&form[10..21]);
                    }),
                ),
                (
                    "quoted control line without its heading",
                    Box::new(|lines: &mut Vec<String>| lines[5] = CHOICE_ROW.into()),
                ),
                (
                    "heading and control line too far apart",
                    Box::new(|lines: &mut Vec<String>| {
                        lines[0] = HEADING.into();
                        lines[34] = CHOICE_ROW.into();
                    }),
                ),
                (
                    "quoted reject dialog without its control line",
                    Box::new(move |lines: &mut Vec<String>| {
                        lines[5] = dialog[10].clone();
                        lines[7] = dialog[12].clone();
                    }),
                ),
            ];
            for (case, change) in cases {
                let observation = observe(&replay_changed(IDLE_BYTES, |lines| change(lines), None));
                assert!(
                    !matches!(
                        observation.kind,
                        ObservationKind::Interaction(_) | ObservationKind::PermissionContinuation
                    ),
                    "{case} must not open a request"
                );
                assert_ne!(observation.rule_id, PERMISSION_RULE, "{case}");
                assert_ne!(observation.rule_id, REASON_RULE, "{case}");
            }
        }

        #[test]
        fn bundled_opencode_replay_paints_each_captured_background_run() {
            let screen = replay(PERMISSION_BYTES);
            let bg = |row: u16, col: u16| screen.screen.cell(row, col).unwrap().bgcolor();
            assert_eq!(bg(21, 0), vt100::Color::Rgb(30, 30, 30));
            for col in 2..14 {
                assert_eq!(bg(21, col), vt100::Color::Rgb(250, 178, 131), "col {col}");
            }
            assert_eq!(bg(21, 14), vt100::Color::Rgb(30, 30, 30));
            assert_eq!(bg(10, 0), vt100::Color::Rgb(20, 20, 20));
            assert_eq!(bg(3, 0), vt100::Color::Default);
            assert_eq!(bg(23, 0), vt100::Color::Default);
            assert_eq!(
                screen.screen.cell(21, 3).unwrap().contents(),
                "A",
                "the repaint keeps the captured text"
            );
            let idle = replay(IDLE_BYTES);
            for row in 0..36 {
                for col in 0..120 {
                    assert_eq!(
                        idle.screen.cell(row, col).unwrap().bgcolor(),
                        vt100::Color::Default,
                        "an absent key means every cell has the default background"
                    );
                }
            }
        }

        #[test]
        fn bundled_guard_accepts_the_opencode_fixtures_and_rejects_altered_backgrounds() {
            for (id, _, _) in FIXTURES {
                assert!(guarded_fixture(id, |_| {}).is_ok(), "{id}");
                let screen_changes: [ScreenChange; 4] = [
                    ("synthetic provenance", |s| {
                        s["provenance"] = json!("synthetic")
                    }),
                    ("nonempty masked row", |s| {
                        s["lines"][35] = json!("restored")
                    }),
                    ("native version", |s| s["native_version"] = json!("2.1.3")),
                    ("extra field", |s| s["note"] = json!("unreviewed")),
                ];
                for (case, change) in screen_changes {
                    assert!(guarded_fixture(id, change).is_err(), "{id} {case}");
                }
            }
            let run = |row: u64, start: u64, end: u64, bg: Value| json!({"row":row,"start_col":start,"end_col":end,"bg":bg});
            let rgb = json!({"kind":"rgb","r":1,"g":2,"b":3});
            for (case, runs) in [
                (
                    "indexed colour",
                    json!([run(30, 0, 4, json!({"kind":"indexed","index":255}))]),
                ),
                (
                    "adjacent runs",
                    json!([
                        run(30, 0, 4, rgb.clone()),
                        run(30, 4, 8, json!({"kind":"indexed","index":3}))
                    ]),
                ),
                ("full-width run", json!([run(30, 0, 120, rgb.clone())])),
            ] {
                assert!(
                    guarded_fixture(IDLE_FIXTURE, |s| s["cell_backgrounds"] = runs).is_ok(),
                    "{case} is a valid run set"
                );
            }
            for (case, runs) in [
                (
                    "row below the geometry",
                    json!([run(36, 0, 4, rgb.clone())]),
                ),
                (
                    "end past the width",
                    json!([run(30, 100, 121, rgb.clone())]),
                ),
                ("masked row", json!([run(35, 0, 4, rgb.clone())])),
                ("empty run", json!([run(30, 4, 4, rgb.clone())])),
                ("reversed run", json!([run(30, 8, 4, rgb.clone())])),
                (
                    "overlapping runs",
                    json!([run(30, 0, 6, rgb.clone()), run(30, 5, 9, rgb.clone())]),
                ),
                (
                    "default colour",
                    json!([run(30, 0, 4, json!({"kind":"default"}))]),
                ),
                (
                    "unknown colour kind",
                    json!([run(30, 0, 4, json!({"kind":"named","name":"red"}))]),
                ),
                (
                    "rgb component out of range",
                    json!([run(30, 0, 4, json!({"kind":"rgb","r":256,"g":0,"b":0}))]),
                ),
                (
                    "rgb component missing",
                    json!([run(30, 0, 4, json!({"kind":"rgb","r":1,"g":2}))]),
                ),
                (
                    "rgb extra field",
                    json!([run(30, 0, 4, json!({"kind":"rgb","r":1,"g":2,"b":3,"a":4}))]),
                ),
                (
                    "indexed out of range",
                    json!([run(30, 0, 4, json!({"kind":"indexed","index":256}))]),
                ),
                (
                    "negative column",
                    json!([{"row":30,"start_col":-1,"end_col":4,"bg":rgb.clone()}]),
                ),
                (
                    "run extra field",
                    json!([{"row":30,"start_col":0,"end_col":4,"bg":rgb.clone(),"fg":rgb.clone()}]),
                ),
                (
                    "run missing field",
                    json!([{"row":30,"start_col":0,"bg":rgb.clone()}]),
                ),
                ("not an array", json!({"row":30})),
            ] {
                assert!(
                    guarded_fixture(IDLE_FIXTURE, |s| s["cell_backgrounds"] = runs).is_err(),
                    "{case} must be rejected"
                );
            }
            assert!(
                guarded_fixture(PERMISSION_FIXTURE, |s| {
                    s["cell_backgrounds"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"row":21,"start_col":5,"end_col":7,"bg":{"kind":"indexed","index":1}}));
                })
                .is_err(),
                "a run overlapping a captured run is rejected"
            );
            let mut tampered = PERMISSION_BYTES.to_vec();
            tampered.push(b'\n');
            let cited = source(PERMISSION_FIXTURE);
            let mut with_tampered = super::bundled_claude::embedded();
            with_tampered.retain(|&(reference, _)| reference != cited);
            with_tampered.push((cited, &tampered));
            assert!(
                ProfileCatalog::from_bundled_definitions(&definitions(), &with_tampered).is_err()
            );
        }

        #[test]
        fn bundled_opencode_episode_survives_the_dialog_and_closes_not_visible_on_the_idle_composer()
         {
            let at = |bytes: &[u8], output_seq: u64| {
                let mut replayed = observe(&replay(bytes));
                replayed.output_seq = output_seq;
                replayed.raw_evidence["screen_seq"] = json!(output_seq);
                replayed
            };
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &at(PERMISSION_BYTES, 1), origin);
            let episode = tracker.active_episode();
            for (index, millis) in [200u64, 400, 3000].into_iter().enumerate() {
                let update = tracker
                    .observe(
                        &at(DENY_3_BYTES, 2 + index as u64),
                        origin + Duration::from_millis(millis),
                        NOW,
                    )
                    .unwrap();
                assert!(
                    update.transitions.is_empty(),
                    "{millis} ms: {:?}",
                    update.transitions
                );
                assert_eq!(update.visibility, Visibility::Visible, "{millis} ms");
                assert_eq!(tracker.active_episode(), episode);
            }
            let first = tracker
                .observe(
                    &at(IDLE_BYTES, 10),
                    origin + Duration::from_millis(3100),
                    NOW,
                )
                .unwrap();
            assert!(first.transitions.is_empty());
            let held = tracker
                .observe(
                    &at(IDLE_BYTES, 11),
                    origin + Duration::from_millis(3350),
                    NOW,
                )
                .unwrap();
            let closes = held
                .transitions
                .iter()
                .filter(|t| t.kind == RuntimeEventKind::InteractionClosed)
                .collect::<Vec<_>>();
            assert_eq!(closes.len(), 1, "{:?}", held.transitions);
            assert_eq!(
                closes[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
        }
    }

    const VERSION: &str = "synthetic-0.0.1";
    const PROFILE: &str = "synthetic-profile-1";
    const RULE: &str = "synthetic-permission";
    const FIXTURE: &str = "synthetic-permission-1";
    const NOW: &str = "2026-10-26T15:35:00.000000+00:00";

    fn definitions() -> Value {
        json!({
            "schema_version":1,
            "fixtures":[{"id":FIXTURE,"kind":"synthetic","backend":"codex","native_version":VERSION,"rows":24,"cols":80,"source_ref":"synthetic-permission-screen.json","sha256":"689868668a9fa97d9efca75cda3f2cc0a87fde25884a5cb36aab3163b8abd773"}],
            "profiles":[{"backend":"codex","native_versions":[VERSION],"profile_version":PROFILE,
                "rules":[{"id":RULE,"priority":100,"observation":"permission_request",
                    "region":{"screen":"either","start_pattern":"^Synthetic permission request$","end_pattern":"^Selection: $","max_rows":8,"cursor":"at_end"},
                    "matchers":[
                        {"id":"operation","scope":"line","pattern":"^Operation: (?P<operation>.+)$"},
                        {"id":"allow","scope":"line","pattern":"^> (?P<label>Allow once)$"},
                        {"id":"deny","scope":"line","pattern":"^  (?P<label>Deny once)$"},
                        {"id":"form","scope":"line","pattern":"^Request form: synthetic-only$"}
                    ],
                    "extract":{"operation":{"matcher_id":"operation","capture":"operation"},"choices":[
                        {"id":"allow-once","label":{"matcher_id":"allow","capture":"label"},"scope":"single_request"},
                        {"id":"deny-once","label":{"matcher_id":"deny","capture":"label"},"scope":"single_request"}
                    ],"native_request_id":null},"verified_noninteraction":false,"sensitive":false}],
                "capabilities":{"detection":{"rule_ids":[RULE],"fixture_ids":[FIXTURE]},"composer":null,"answers":[]}}]
        })
    }

    fn lines(operation: &str) -> Vec<String> {
        vec![
            "Synthetic permission request".into(),
            format!("Operation: {operation}"),
            "> Allow once".into(),
            "  Deny once".into(),
            "Request form: synthetic-only".into(),
            "Selection: ".into(),
        ]
    }

    fn terminal(lines: &[String], cols: u16, alternate: bool) -> TerminalState {
        assert!(lines.len() <= 24);
        let mut state = TerminalState::new(TerminalSize { rows: 24, cols }).unwrap();
        if alternate {
            state.feed(b"\x1b[?1049h").unwrap();
        }
        state.feed(b"\x1b[2J").unwrap();
        for (index, line) in lines.iter().enumerate() {
            state
                .feed(format!("\x1b[{};1H{line}", 25 - lines.len() + index).as_bytes())
                .unwrap();
        }
        state
    }

    fn observation(definition: &Value, screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::from_definitions(definition).unwrap();
        Detector::new(&catalog, CodingAgent::Codex, Some(VERSION))
            .unwrap()
            .observe(screen)
            .unwrap()
    }

    fn permission(operation: &str) -> ScreenObservation {
        observation(
            &definitions(),
            &terminal(&lines(operation), 80, false).snapshot(),
        )
    }

    fn simple_definition(kind: &str, verified: bool) -> Value {
        let mut value = definitions();
        let rule = &mut value["profiles"][0]["rules"][0];
        rule["id"] = json!(format!("synthetic-{kind}"));
        rule["observation"] = json!(kind);
        rule["region"]["start_pattern"] = json!("^Synthetic state$");
        rule["region"]["end_pattern"] = json!("^State end$");
        rule["matchers"] = json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
        rule["extract"] = json!({"operation":null,"choices":[],"native_request_id":null});
        rule["verified_noninteraction"] = json!(verified);
        let declaration = json!({"rule_ids":[format!("synthetic-{kind}")],"fixture_ids":[FIXTURE]});
        value["profiles"][0]["capabilities"] = if kind.starts_with("composer_") {
            json!({"detection":null,"composer":declaration,"answers":[]})
        } else {
            json!({"detection":declaration,"composer":null,"answers":[]})
        };
        value
    }

    fn state_observation(kind: &str, verified: bool) -> ScreenObservation {
        observation(
            &simple_definition(kind, verified),
            &terminal(&["Synthetic state".into(), "State end".into()], 80, false).snapshot(),
        )
    }

    fn tracker() -> EpisodeTracker {
        EpisodeTracker::new(1, 1, EvidencePolicy::metadata()).unwrap()
    }

    fn open(
        tracker: &mut EpisodeTracker,
        value: &ScreenObservation,
        origin: Instant,
    ) -> EpisodeUpdate {
        let candidate = tracker.observe(value, origin, NOW).unwrap();
        assert!(candidate.transitions.is_empty());
        assert!(matches!(
            candidate.eligibility,
            InputEligibility::Hold(GateReason::Candidate)
        ));
        let update = tracker
            .observe(value, origin + Duration::from_millis(100), NOW)
            .unwrap();
        assert_eq!(update.transitions.len(), 1);
        assert_eq!(
            update.transitions[0].kind,
            RuntimeEventKind::InteractionOpened
        );
        update
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerCapability {
    backend: CodingAgent,
    native_version: String,
    profile_version: String,
    request_rule_id: String,
    answer_rule_id: String,
    decision: PermissionDecision,
    fixture_ids: Vec<String>,
}
impl AnswerCapability {
    pub fn backend(&self) -> CodingAgent {
        self.backend
    }
    pub fn native_version(&self) -> &str {
        &self.native_version
    }
    pub fn profile_version(&self) -> &str {
        &self.profile_version
    }
    pub fn request_rule_id(&self) -> &str {
        &self.request_rule_id
    }
    pub fn answer_rule_id(&self) -> &str {
        &self.answer_rule_id
    }
    pub fn decision(&self) -> PermissionDecision {
        self.decision
    }
    pub fn fixture_ids(&self) -> &[String] {
        &self.fixture_ids
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        backend: CodingAgent,
        native_version: &str,
        profile_version: &str,
        request_rule_id: &str,
        answer_rule_id: &str,
        decision: PermissionDecision,
        fixture_ids: &[String],
    ) -> Result<Self, CafleetError> {
        if [
            native_version,
            profile_version,
            request_rule_id,
            answer_rule_id,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || fixture_ids.is_empty()
            || fixture_ids.iter().any(|value| value.trim().is_empty())
        {
            return Err(CafleetError::Value(
                "answer capability requires complete profile provenance".into(),
            ));
        }
        Ok(Self {
            backend,
            native_version: native_version.into(),
            profile_version: profile_version.into(),
            request_rule_id: request_rule_id.into(),
            answer_rule_id: answer_rule_id.into(),
            decision,
            fixture_ids: fixture_ids.to_vec(),
        })
    }
}
