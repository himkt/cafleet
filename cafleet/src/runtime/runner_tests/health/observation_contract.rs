use super::*;

fn wire(run: &RunRecord) -> serde_json::Value {
    json!({"identity":ControlIdentity::from_run(run),"runner":{"pid":run.runner_pid.unwrap(),"start_identity":run.runner_start_identity.as_ref().unwrap()},"child":{"pid":run.child_pid.unwrap(),"start_identity":run.child_start_identity.as_ref().unwrap()},"output_seq":17,"observation":{"visibility":"unknown","eligibility":{"state":"hold","reason":"unknown"},"pending_count":0,"pending_evidence":false,"gap_count":0,"degraded":false,"diagnostic":null}})
}

#[test]
fn runner_batch2_health_decode_validates_legacy_identity_without_granting_composer_proof() {
    let mut fixture = Fixture::new();
    let (_artifacts, _lock) = activate(&mut fixture);
    let mut value = wire(&fixture.run);
    value.as_object_mut().unwrap().remove("observation");
    let decoded = HealthSnapshot::decode(value, &fixture.run).unwrap();
    assert!(decoded.observation.is_none());
}

#[test]
fn runner_batch2_health_decode_accepts_bounded_hold_and_checked_empty_composer_states() {
    let mut fixture = Fixture::new();
    let (_artifacts, _lock) = activate(&mut fixture);
    for visibility in ["visible", "obscured", "not_visible", "unknown"] {
        let mut value = wire(&fixture.run);
        value["observation"]["visibility"] = json!(visibility);
        value["observation"]["pending_count"] = json!(130);
        value["observation"]["pending_evidence"] = json!(true);
        value["observation"]["degraded"] = json!(true);
        value["observation"]["diagnostic"] = json!("observation_gap");
        let decoded = HealthSnapshot::decode(value, &fixture.run).unwrap();
        decoded.observation.unwrap().validate().unwrap();
    }
    let mut value = wire(&fixture.run);
    value["observation"]["visibility"] = json!("not_visible");
    value["observation"]["eligibility"] = json!({"state":"composer_empty"});
    assert!(HealthSnapshot::decode(value, &fixture.run).is_ok());
}

#[test]
fn runner_batch2_health_decode_rejects_unknown_wire_fields_enums_unbounded_and_private_diagnostics()
{
    let mut fixture = Fixture::new();
    let (_artifacts, _lock) = activate(&mut fixture);
    for field in [
        "unknown_field",
        "visibility",
        "state",
        "reason",
        "pending",
        "negative",
        "float",
        "raw_diagnostic",
        "long_diagnostic",
        "unicode_diagnostic",
    ] {
        let mut value = wire(&fixture.run);
        let status = &mut value["observation"];
        match field {
            "unknown_field" => status["screen"] = json!("private screen"),
            "visibility" => status["visibility"] = json!("invented"),
            "state" => status["eligibility"]["state"] = json!("allow"),
            "reason" => status["eligibility"]["reason"] = json!("invented"),
            "pending" => status["pending_count"] = json!(131),
            "negative" => status["gap_count"] = json!(-1),
            "float" => status["pending_count"] = json!(0.5),
            "raw_diagnostic" => {
                status["diagnostic"] = json!("private exception configuration canary")
            }
            "long_diagnostic" => status["diagnostic"] = json!("x".repeat(129)),
            _ => status["diagnostic"] = json!("世界"),
        }
        assert!(
            HealthSnapshot::decode(value, &fixture.run).is_err(),
            "{field}"
        );
    }
}

#[test]
fn runner_batch2_health_decode_requires_hold_for_visibility_pressure_and_degradation() {
    let mut fixture = Fixture::new();
    let (_artifacts, _lock) = activate(&mut fixture);
    for field in [
        "visible",
        "obscured",
        "unknown",
        "pending_count",
        "pending_evidence",
        "degraded",
    ] {
        let mut value = wire(&fixture.run);
        value["observation"]["visibility"] = json!("not_visible");
        value["observation"]["eligibility"] = json!({"state":"composer_empty"});
        match field {
            "pending_count" => value["observation"][field] = json!(1),
            "pending_evidence" | "degraded" => value["observation"][field] = json!(true),
            _ => value["observation"]["visibility"] = json!(field),
        }
        assert!(
            HealthSnapshot::decode(value, &fixture.run).is_err(),
            "{field}"
        );
    }
}

#[test]
fn runner_batch2_health_decode_preserves_process_generation_and_control_identity_checks() {
    let mut fixture = Fixture::new();
    let (_artifacts, _lock) = activate(&mut fixture);
    for field in ["epoch", "nonce", "runner", "child"] {
        let mut value = wire(&fixture.run);
        match field {
            "epoch" => value["identity"]["owner_epoch"] = json!(fixture.run.owner_epoch + 1),
            "nonce" => value["identity"]["launch_nonce"] = json!("b".repeat(64)),
            "runner" => value["runner"]["start_identity"] = json!("stale generation"),
            _ => value["child"]["start_identity"] = json!("stale generation"),
        }
        assert!(
            HealthSnapshot::decode(value, &fixture.run).is_err(),
            "{field}"
        );
    }
}
