use super::support::*;
use super::*;
use crate::coding_agent::ProfileCatalog;

fn prepared(fixture: &mut CaptureFixture) {
    fixture
        .fixture
        .conn
        .execute(
            "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
            [fixture.fixture.run.run_id],
        )
        .unwrap();
    fixture.fixture.run =
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
}
fn producer(fixture: &CaptureFixture) -> NativeCaptureProducer {
    read_native_capture_producer(&fixture.producer_path, &NativeCaptureLimits::default()).unwrap()
}
fn channel(
    fixture: &CaptureFixture,
    catalog: &ProfileCatalog,
) -> (NativeCapturePort, NativeCaptureReceiver) {
    native_capture_channel(
        &fixture.fixture.run,
        NativeCapturePolicy::from_launch(&fixture.spec, &[], catalog).unwrap(),
        NativeCaptureLimits::default(),
    )
    .unwrap()
}
fn select_catalog(fixture: &mut CaptureFixture, bundled: bool) -> ProfileCatalog {
    if bundled {
        let selected = ProfileCatalog::bundled().unwrap().capabilities(
            crate::coding_agent::CodingAgent::Codex,
            fixture.fixture.run.native_version.as_deref(),
        );
        fixture
            .fixture
            .conn
            .execute(
                "UPDATE member_runs SET profile_version=?1,profile_match=?2,coverage=?3 WHERE run_id=?4",
                rusqlite::params![
                    selected.profile_version,
                    selected.profile_match.map(|value| value.as_str()),
                    selected.detection.as_str(),
                    fixture.fixture.run.run_id
                ],
            )
            .unwrap();
        fixture.fixture.run =
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        ProfileCatalog::bundled().unwrap()
    } else {
        catalog()
    }
}

#[test]
fn runner_collector_f1_actual_permission_options_match_the_recorded_emitted_policy_before_binding()
{
    for mutation in [
        "approval",
        "sandbox",
        "missing_approval",
        "missing_sandbox",
        "repeat_approval",
        "repeat_sandbox",
    ] {
        let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let (mut port, receiver) = channel(&fixture, &catalog());
        match mutation {
            "approval" => fixture.spec.argv[1] = "--full-auto".into(),
            "sandbox" => fixture.spec.argv[3] = "read-only".into(),
            "missing_approval" => {
                fixture.spec.argv.remove(1);
            }
            "missing_sandbox" => {
                fixture.spec.argv.drain(2..4);
            }
            "repeat_approval" => fixture.spec.argv.push("--approve-for-me".into()),
            "repeat_sandbox" => fixture
                .spec
                .argv
                .extend(["--sandbox".into(), "danger-full-access".into()]),
            _ => unreachable!(),
        }
        prepared(&mut fixture);
        assert!(
            port.bind_prepared(
                &fixture.fixture.run,
                &fixture.placement,
                &fixture.spec,
                producer(&fixture),
                &fixture.clock
            )
            .is_err(),
            "{mutation}"
        );
        assert!(receiver.finalized_identity().is_none());
        assert!(receiver.request(fixture.request("unbound-policy")).is_err());
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        assert!(run.child_pid.is_none());
    }
}

#[test]
fn runner_collector_f1_flag_like_prompt_is_opaque_and_preserves_truthful_permission_source() {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    *fixture.spec.argv.last_mut().unwrap() =
        "explain --ask-for-approval on-request and --sandbox read-only".into();
    let (mut port, receiver) = channel(&fixture, &catalog());
    fixture.prepare(&mut port);
    fixture.finalize(&mut port);
    let lease = fixture.pair(&mut port, &receiver, b"safe source control");
    let result = value(&lease);
    assert_eq!(
        result["source"]["permission_argv"],
        fixture.fixture.run.permission_argv_json
    );
    assert_eq!(result["source"]["producer"]["catalog_origin"], "synthetic");
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("explain --ask-for-approval")
    );
}

#[test]
fn runner_collector_f1_producer_origin_matches_the_selected_catalog_constructor_after_clone() {
    for bundled in [false, true] {
        let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let actual = select_catalog(&mut fixture, bundled);
        let selected = actual.clone();
        let (mut port, receiver) = channel(&fixture, &selected);
        fixture.producer_json["catalog_origin"] = json!(if bundled {
            "synthetic"
        } else {
            "bundled_unknown"
        });
        std::fs::write(
            &fixture.producer_path,
            serde_json::to_vec(&fixture.producer_json).unwrap(),
        )
        .unwrap();
        prepared(&mut fixture);
        assert!(
            port.bind_prepared(
                &fixture.fixture.run,
                &fixture.placement,
                &fixture.spec,
                producer(&fixture),
                &fixture.clock
            )
            .is_err(),
            "bundled={bundled}"
        );
        assert!(receiver.finalized_identity().is_none());
        assert!(receiver.request(fixture.request("unbound-origin")).is_err());
        assert!(fixture.fixture.run.child_pid.is_none());
    }
}

#[test]
fn runner_collector_f1_matching_synthetic_and_bundled_origins_publish_only_their_actual_provenance()
{
    for bundled in [false, true] {
        let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let selected = select_catalog(&mut fixture, bundled);
        let origin = if bundled {
            "bundled_unknown"
        } else {
            "synthetic"
        };
        fixture.producer_json["catalog_origin"] = json!(origin);
        std::fs::write(
            &fixture.producer_path,
            serde_json::to_vec(&fixture.producer_json).unwrap(),
        )
        .unwrap();
        let (mut port, receiver) = channel(&fixture, &selected.clone());
        fixture.prepare(&mut port);
        fixture.finalize(&mut port);
        let lease = fixture.pair(&mut port, &receiver, b"safe matching origin");
        let result = value(&lease);
        assert_eq!(result["source"]["producer"]["catalog_origin"], origin);
        assert_eq!(
            result["source"]["profile_version"],
            json!(fixture.fixture.run.profile_version)
        );
        assert_eq!(
            result["source"]["permission_argv"],
            fixture.fixture.run.permission_argv_json
        );
    }
}

fn duplicate_object(
    object: &serde_json::Map<String, Value>,
    key: &str,
    first: &Value,
    last: &Value,
) -> String {
    assert!(object.contains_key(key));
    let mut fields: Vec<String> = object
        .iter()
        .filter(|(name, _)| name.as_str() != key)
        .map(|(name, value)| {
            format!(
                "{}:{}",
                serde_json::to_string(name).unwrap(),
                serde_json::to_string(value).unwrap()
            )
        })
        .collect();
    let name = serde_json::to_string(key).unwrap();
    fields.push(format!("{name}:{}", serde_json::to_string(first).unwrap()));
    fields.push(format!("{name}:{}", serde_json::to_string(last).unwrap()));
    format!("{{{}}}", fields.join(","))
}
fn with_native_document(producer: &Value, native: &str) -> String {
    let fields: Vec<String> = producer
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| {
            let value = if name == "native_executable" {
                native.to_owned()
            } else {
                serde_json::to_string(value).unwrap()
            };
            format!("{}:{value}", serde_json::to_string(name).unwrap())
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}
fn assert_duplicate_fields_rejected(nested: bool, conflicting: bool) {
    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    assert!(
        read_native_capture_producer(&fixture.producer_path, &NativeCaptureLimits::default())
            .is_ok()
    );
    let object = if nested {
        &fixture.producer_json["native_executable"]
    } else {
        &fixture.producer_json
    };
    for (key, valid) in object.as_object().unwrap() {
        for invalid_last in [true, false] {
            let invalid = if conflicting {
                json!([])
            } else {
                valid.clone()
            };
            let (first, last) = if invalid_last {
                (valid, &invalid)
            } else {
                (&invalid, valid)
            };
            let duplicate = duplicate_object(object.as_object().unwrap(), key, first, last);
            let bytes = if nested {
                with_native_document(&fixture.producer_json, &duplicate)
            } else {
                duplicate
            };
            let path = fixture
                .fixture
                ._dir
                .path()
                .join(format!("duplicate-{key}-{invalid_last}.json"));
            private_write(&path, bytes.as_bytes());
            assert!(
                read_native_capture_producer(&path, &NativeCaptureLimits::default()).is_err(),
                "nested={nested}, conflicting={conflicting}, field={key}, invalid_last={invalid_last}"
            );
        }
    }
}

#[test]
fn runner_collector_f4_top_level_identical_duplicate_fields_fail_before_normalization() {
    assert_duplicate_fields_rejected(false, false);
}
#[test]
fn runner_collector_f4_nested_identical_duplicate_fields_fail_before_normalization() {
    assert_duplicate_fields_rejected(true, false);
}
#[test]
fn runner_collector_f4_top_level_conflicting_duplicates_reject_both_field_orders() {
    assert_duplicate_fields_rejected(false, true);
}
#[test]
fn runner_collector_f4_nested_conflicting_duplicates_reject_both_field_orders() {
    assert_duplicate_fields_rejected(true, true);
}
#[test]
fn runner_collector_f4_nullable_native_version_requires_presence_and_accepts_explicit_null() {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    fixture.producer_json["native_executable"]["version"] = Value::Null;
    let null = fixture.fixture._dir.path().join("null-version.json");
    private_write(&null, &serde_json::to_vec(&fixture.producer_json).unwrap());
    assert!(read_native_capture_producer(&null, &NativeCaptureLimits::default()).is_ok());
    fixture.producer_json["native_executable"]
        .as_object_mut()
        .unwrap()
        .remove("version")
        .unwrap();
    let missing = fixture.fixture._dir.path().join("missing-version.json");
    private_write(
        &missing,
        &serde_json::to_vec(&fixture.producer_json).unwrap(),
    );
    assert!(read_native_capture_producer(&missing, &NativeCaptureLimits::default()).is_err());
}
