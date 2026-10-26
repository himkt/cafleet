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
    let bytes = include_bytes!("synthetic-permission-screen.json");
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
        let unsupported = catalog.capabilities(backend, Some("synthetic-uninstalled-version"));
        assert_eq!(unsupported.detection, Coverage::Unknown);
        assert_eq!(unsupported.composer, Coverage::Unknown);
        assert!(unsupported.answer_decisions.is_empty());
    }
}

#[test]
fn runtime_profile_private_fingerprint_supports_comparison_without_debug_or_serialization() {
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
