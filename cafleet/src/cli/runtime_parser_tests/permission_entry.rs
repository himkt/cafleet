use clap::Parser;

use super::CliArgs;

fn inspect() -> Vec<String> {
    [
        "cafleet",
        "permission",
        "inspect",
        "17",
        "--event-id",
        "19",
        "--from-member-id",
        "23",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}
fn respond() -> Vec<String> {
    [
        "cafleet",
        "permission",
        "respond",
        "17",
        "--event-id",
        "19",
        "--from-member-id",
        "23",
        "--expected-run-id",
        "29",
        "--expected-owner-epoch",
        "3",
        "--request-token",
        &"a".repeat(64),
        "--decision",
        "allow",
        "--decision-source",
        "director-auto",
        "--director-mode",
        "auto",
        "--mode-evidence-ref",
        "fixture:mode",
        "--decision-evidence-ref",
        "fixture:decision",
        "--operation-key",
        "fixture:response-1",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}
fn set(args: &mut [String], field: &str, value: &str) {
    let index = if field == "interaction" {
        3
    } else {
        args.iter()
            .position(|arg| arg == field)
            .expect("the fixture contains the selected flag")
            + 1
    };
    args[index] = value.into();
}
fn accepted(args: &[String]) {
    if let Err(error) = CliArgs::try_parse_from(args) {
        panic!("valid permission command parses: {error}");
    }
}
fn rejected(args: &[String]) {
    let error = match CliArgs::try_parse_from(args) {
        Ok(_) => panic!("invalid permission command produces a usage error"),
        Err(error) => error,
    };
    assert_eq!(error.exit_code(), 2);
    assert_ne!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn step5_a0_permission_inspect_accepts_positive_id_boundaries_and_json_selection() {
    accepted(&inspect());
    for field in ["interaction", "--event-id", "--from-member-id"] {
        for value in ["1", "9223372036854775807"] {
            for json in [false, true] {
                let mut args = inspect();
                set(&mut args, field, value);
                if json {
                    args.push("--json".into());
                }
                accepted(&args);
            }
        }
    }
}
#[test]
fn step5_a0_permission_respond_accepts_positive_id_epoch_boundaries_and_json_selection() {
    accepted(&respond());
    for field in [
        "interaction",
        "--event-id",
        "--from-member-id",
        "--expected-run-id",
        "--expected-owner-epoch",
    ] {
        for value in ["1", "9223372036854775807"] {
            for json in [false, true] {
                let mut args = respond();
                set(&mut args, field, value);
                if json {
                    args.push("--json".into());
                }
                accepted(&args);
            }
        }
    }
}
#[test]
fn step5_a0_permission_respond_accepts_each_semantic_decision_and_valid_source_mode_pair() {
    accepted(&respond());
    for decision in ["allow", "deny"] {
        for (source, mode) in [
            ("director-auto", "auto"),
            ("user", "auto"),
            ("user", "manual"),
            ("user", "unknown"),
        ] {
            let mut args = respond();
            set(&mut args, "--decision", decision);
            set(&mut args, "--decision-source", source);
            set(&mut args, "--director-mode", mode);
            accepted(&args);
        }
    }
}
#[test]
fn step5_a0_permission_inspect_rejects_invalid_assigned_ids() {
    accepted(&inspect());
    for field in ["interaction", "--event-id", "--from-member-id"] {
        for value in [
            "0",
            "-1",
            "unknown",
            "1.0",
            "9223372036854775808",
            "18446744073709551616",
        ] {
            let mut args = inspect();
            set(&mut args, field, value);
            rejected(&args);
        }
    }
}
#[test]
fn step5_a0_permission_respond_rejects_invalid_assigned_ids_and_epochs() {
    accepted(&respond());
    for field in [
        "interaction",
        "--event-id",
        "--from-member-id",
        "--expected-run-id",
        "--expected-owner-epoch",
    ] {
        for value in [
            "0",
            "-1",
            "unknown",
            "1.0",
            "9223372036854775808",
            "18446744073709551616",
        ] {
            let mut args = respond();
            set(&mut args, field, value);
            rejected(&args);
        }
    }
}
fn required(mut args: Vec<String>, flags: &[&str]) {
    accepted(&args);
    let mut missing_id = args.clone();
    missing_id.remove(3);
    rejected(&missing_id);
    for flag in flags {
        let index = args
            .iter()
            .position(|arg| arg == flag)
            .expect("required fixture flag");
        let mut missing = args.clone();
        missing.drain(index..index + 2);
        rejected(&missing);
    }
    args.push("--json".into());
    accepted(&args);
}
#[test]
fn step5_a0_permission_inspect_requires_its_interaction_event_and_director() {
    required(inspect(), &["--event-id", "--from-member-id"]);
}
#[test]
fn step5_a0_permission_respond_requires_each_snapshot_decision_and_provenance_field() {
    required(
        respond(),
        &[
            "--event-id",
            "--from-member-id",
            "--expected-run-id",
            "--expected-owner-epoch",
            "--request-token",
            "--decision",
            "--decision-source",
            "--director-mode",
            "--mode-evidence-ref",
            "--decision-evidence-ref",
            "--operation-key",
        ],
    );
}
#[test]
fn step5_a0_permission_response_accepts_exact_lowercase_hex_tokens() {
    accepted(&respond());
    for token in ["0".repeat(64), "f".repeat(64), "0123456789abcdef".repeat(4)] {
        let mut args = respond();
        set(&mut args, "--request-token", &token);
        accepted(&args);
    }
}
#[test]
fn step5_a0_permission_response_rejects_uppercase_nonhex_and_wrong_length_tokens() {
    accepted(&respond());
    for token in [
        String::new(),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        format!("{}A", "a".repeat(63)),
        "g".repeat(64),
        " ".repeat(64),
        format!("{} ", "a".repeat(63)),
        "é".repeat(32),
        "０".repeat(64),
    ] {
        let mut args = respond();
        set(&mut args, "--request-token", &token);
        rejected(&args);
    }
}
#[test]
fn step5_a0_permission_response_references_accept_nonblank_utf8_at_exact_byte_boundaries() {
    accepted(&respond());
    for field in ["--mode-evidence-ref", "--decision-evidence-ref"] {
        for value in [
            "x".into(),
            "x".repeat(2048),
            "é".repeat(1024),
            format!("{}é", "界".repeat(682)),
            format!("{}x", " ".repeat(2047)),
        ] {
            assert!(value.len() <= 2048);
            let mut args = respond();
            set(&mut args, field, &value);
            accepted(&args);
        }
    }
}
#[test]
fn step5_a0_permission_response_references_reject_blank_oversize_and_every_control() {
    accepted(&respond());
    for field in ["--mode-evidence-ref", "--decision-evidence-ref"] {
        for value in [
            String::new(),
            " ".into(),
            "\t".into(),
            "x".repeat(2049),
            format!("{}x", "é".repeat(1024)),
        ] {
            let mut args = respond();
            set(&mut args, field, &value);
            rejected(&args);
        }
        for control in (0u8..=31).chain(std::iter::once(127)) {
            let value = format!("fixture:{}:reference", char::from(control));
            let mut args = respond();
            set(&mut args, field, &value);
            rejected(&args);
        }
    }
}
#[test]
fn step5_a0_permission_response_operation_key_accepts_literal_nonblank_values_and_rejects_blank() {
    accepted(&respond());
    for key in ["fixture:key", "fixture:literal key 世界"] {
        let mut args = respond();
        set(&mut args, "--operation-key", key);
        accepted(&args);
    }
    for key in ["", " ", "\t\n"] {
        let mut args = respond();
        set(&mut args, "--operation-key", key);
        rejected(&args);
    }
}
#[test]
fn step5_a0_permission_response_rejects_invalid_decision_source_and_mode_enums() {
    accepted(&respond());
    for (field, values) in [
        ("--decision", ["", "approved", "yes", "Allow"]),
        ("--decision-source", ["", "director_auto", "member", "User"]),
        ("--director-mode", ["", "AUTO", "never", "on-request"]),
    ] {
        for value in values {
            let mut args = respond();
            set(&mut args, field, value);
            rejected(&args);
        }
    }
}
