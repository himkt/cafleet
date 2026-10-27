use super::*;
use crate::coding_agent::{ComposerOperation, ComposerPlan, ProfileCatalog};

const EMPTY: &str = "synthetic-composer_empty";
const DRAFT: &str = "synthetic-composer_draft";
const PROMPT_FIXTURE: &str = "synthetic-prompt-plan";
const EXEC_FIXTURE: &str = "synthetic-exec-plan";
const VERSION_TWO: &str = "synthetic-0.0.2";

fn definition(prompt: Option<&str>, exec: Option<&str>) -> Value {
    let mut value = simple_definition("composer_empty", true);
    let draft = simple_definition("composer_draft", true)["profiles"][0]["rules"][0].clone();
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

fn reject(good: &Value, bad: &Value, operation: ComposerOperation, label: &str) -> CafleetError {
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
fn step5_composer_e1_observation_only_definition_preserves_observation_without_executable_plans() {
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
fn step5_composer_e2_nullable_operations_and_absent_composer_keep_executable_coverage_unknown() {
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
fn step5_composer_e3_prompt_plan_retains_exact_selected_metadata_and_independent_coverage() {
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
    let catalog = ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
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
fn step5_composer_e5_operation_provenance_and_exact_selection_do_not_authorize_other_profiles() {
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
fn step5_composer_e6_prompt_preserves_literal_utf8_between_empty_prefix_and_tested_submit_bytes() {
    let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
    let literal = "  literal 世界 '$x' `backticks` \\\"quotes\\\"  ";
    let bytes = plan(&catalog, ComposerOperation::Prompt)
        .encode(literal)
        .unwrap();
    assert_eq!(bytes, expected(ComposerOperation::Prompt, literal, b"~"));
    assert_eq!(&bytes[..literal.len()], literal.as_bytes());
}

#[test]
fn step5_composer_e7_exec_owns_exact_bang_space_prefix_without_changing_the_caller_literal() {
    let catalog = ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
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
        for invalid in
            values
                .into_iter()
                .chain([Value::Null, json!(17), json!(true), json!([]), json!({})])
        {
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
fn step5_composer_e11_optional_submission_and_operation_objects_have_exact_complete_fields() {
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
                [operation_name(operation)]["fixture_ids"] = json!(["foreign-operation-fixture"]);
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
        good["profiles"][0]["capabilities"]["composer"]["submission"][operation_name(operation)]
            ["fixture_ids"] = json!([fixture, format!("{fixture}-v2")]);
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
        bad["profiles"][0]["capabilities"]["composer"]["submission"][operation_name(operation)]["fixture_ids"] =
            json!([fixture]);
        reject(&good, &bad, operation, "uncovered operation version");
    }
}

#[test]
fn step5_composer_e14_composer_rule_references_keep_component_identity_and_declared_order() {
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
        bad["profiles"][0]["capabilities"]["composer"]["submission"][operation_name(operation)]["prefix_hex"] =
            match operation {
                ComposerOperation::Prompt => json!("00"),
                ComposerOperation::Exec => json!(""),
            };
        let definition_error = reject(&good, &bad, operation, "private submit with invalid prefix");
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
