mod catalog;
mod composer;
mod detector;
use super::CodingAgent;
use crate::broker::runtime::PermissionDecision;
#[cfg(test)]
use crate::error::CafleetError;
#[cfg(test)]
pub(crate) use catalog::inspection_tests::{
    inspection_fixture_definition, inspection_fixture_form,
};
pub(crate) use catalog::{AnswerPlan, AnswerStage};
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
                let refresh = tracker
                    .observe(&redraw, origin + Duration::from_millis(200), NOW)
                    .unwrap()
                    .latest_evidence
                    .unwrap();
                assert!(!refresh.evidence_json.to_string().contains(secret));
            }
        }

        #[test]
        fn runtime_episode_metadata_removes_each_screen_field_before_materializing_events_or_refreshes()
         {
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
