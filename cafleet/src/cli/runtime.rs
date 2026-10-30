use clap::Subcommand;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use rusqlite::Connection;

use super::helpers::{checked, emit, nonblank};
use crate::broker::runtime;
use crate::error::CafleetError;
use crate::runtime::runner::{
    validate_operation_key, validate_permission_reference, validate_request_token,
};

fn positive_id(value: &str) -> Result<i64, String> {
    let id = value
        .parse::<i64>()
        .map_err(|_| "a positive assigned ID is required".to_string())?;
    if id > 0 {
        Ok(id)
    } else {
        Err("a positive assigned ID is required".into())
    }
}

#[derive(Subcommand)]
pub(crate) enum EventCommand {
    /// Read a durable runtime event and its current run/interaction state.
    Show {
        /// The runtime event to read.
        #[arg(value_parser = positive_id)]
        event_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum Disposition {
    Submitted,
    NotSubmitted,
    AbandonedUncertain,
}

#[derive(Subcommand)]
pub(crate) enum InputCommand {
    /// Read a durable programmatic-input receipt.
    Show {
        /// The programmatic-input receipt to read.
        #[arg(value_parser = positive_id)]
        input_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Record an evidence-based input disposition under synchronized ownership.
    Reconcile {
        /// The programmatic-input receipt to settle.
        #[arg(value_parser = positive_id)]
        input_id: i64,
        /// The observed input disposition to record.
        #[arg(long, value_enum)]
        disposition: Disposition,
        /// Reference to the Director's recorded evidence for the disposition.
        #[arg(long, value_parser = nonblank("a nonempty evidence reference is required"))]
        evidence_ref: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn event(conn: &Connection, command: EventCommand) -> Result<(), CafleetError> {
    match command {
        EventCommand::Show { event_id, json } => {
            let view = runtime::get_event(conn, event_id)?;
            emit(json, &crate::presentation::runtime_event(&view), || {
                crate::output::format_runtime_event(&view)
            });
            Ok(())
        }
    }
}

pub(crate) fn input(
    conn: &mut Connection,
    settings: &crate::config::Settings,
    command: InputCommand,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    match command {
        InputCommand::Show { input_id, json } => {
            let input = runtime::get_input(conn, input_id)?;
            emit(json, &crate::presentation::input_operation(&input), || {
                crate::output::format_input_operation(&input)
            });
            Ok(())
        }
        InputCommand::Reconcile {
            input_id,
            disposition,
            evidence_ref,
            json,
        } => {
            let disposition = match disposition {
                Disposition::Submitted => runtime::InputDisposition::Submitted,
                Disposition::NotSubmitted => runtime::InputDisposition::NotSubmitted,
                Disposition::AbandonedUncertain => runtime::InputDisposition::AbandonedUncertain,
            };
            let input = crate::runtime::runner::reconcile_input(
                conn,
                settings,
                input_id,
                disposition,
                &evidence_ref,
                deadline,
            )
            .map_err(CafleetError::from)?;
            emit(json, &crate::presentation::input_operation(&input), || {
                crate::output::format_input_operation(&input)
            });
            Ok(())
        }
    }
}

fn stored_enum<T>(values: &'static [&'static str]) -> impl TypedValueParser<Value = T>
where
    T: for<'a> TryFrom<&'a str, Error = CafleetError> + Clone + Send + Sync + 'static,
{
    PossibleValuesParser::new(values.iter().copied()).map(|value| {
        T::try_from(value.as_str()).expect("possible values are the stored enum values")
    })
}

#[derive(Subcommand)]
pub(crate) enum PermissionCommand {
    /// Read the current native permission request and its answer capability.
    Inspect {
        /// The interaction carrying the request.
        #[arg(value_parser = positive_id)]
        interaction_id: i64,
        /// The runtime event delivered to the Director's inbox.
        #[arg(long, value_parser = positive_id)]
        event_id: i64,
        /// The pinned Director's member ID.
        #[arg(long = "from-member-id", value_parser = positive_id)]
        director_member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Answer the inspected native permission request with a semantic decision.
    Respond {
        /// The interaction carrying the request.
        #[arg(value_parser = positive_id)]
        interaction_id: i64,
        /// The runtime event delivered to the Director's inbox.
        #[arg(long, value_parser = positive_id)]
        event_id: i64,
        /// The pinned Director's member ID.
        #[arg(long = "from-member-id", value_parser = positive_id)]
        director_member_id: i64,
        /// The run ID reported by inspection.
        #[arg(long, value_parser = positive_id)]
        expected_run_id: i64,
        /// The owner epoch reported by inspection.
        #[arg(long, value_parser = positive_id)]
        expected_owner_epoch: i64,
        /// The request token reported by inspection.
        #[arg(long, value_parser = checked(validate_request_token))]
        request_token: String,
        /// The semantic answer.
        #[arg(long, value_parser = stored_enum::<runtime::PermissionDecision>(runtime::PermissionDecision::VALUES))]
        decision: runtime::PermissionDecision,
        /// Who made the decision.
        #[arg(long, value_parser = stored_enum::<runtime::DecisionSource>(runtime::DecisionSource::VALUES))]
        decision_source: runtime::DecisionSource,
        /// The Director session's effective approval mode.
        #[arg(long, value_parser = stored_enum::<runtime::DirectorMode>(runtime::DirectorMode::VALUES))]
        director_mode: runtime::DirectorMode,
        /// Evidence reference for the Director mode.
        #[arg(long, value_parser = checked(validate_permission_reference))]
        mode_evidence_ref: String,
        /// Evidence reference for the decision.
        #[arg(long, value_parser = checked(validate_permission_reference))]
        decision_evidence_ref: String,
        /// Idempotency key of this response.
        #[arg(long, value_parser = checked(validate_operation_key))]
        operation_key: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn permission(
    conn: &mut Connection,
    settings: &crate::config::Settings,
    command: PermissionCommand,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    use crate::runtime::runner::{
        PermissionInspectInput, PermissionRespondInput, SubmissionStatus,
    };
    match command {
        PermissionCommand::Inspect {
            interaction_id,
            event_id,
            director_member_id,
            json,
        } => {
            let inspection = crate::runtime::runner::inspect_permission(
                conn,
                settings,
                &PermissionInspectInput {
                    interaction_id,
                    event_id,
                    director_member_id,
                },
                deadline,
            )
            .map_err(CafleetError::from)?;
            emit(
                json,
                &crate::presentation::permission_inspection(&inspection),
                || crate::output::format_permission_inspection(&inspection),
            );
            Ok(())
        }
        PermissionCommand::Respond {
            interaction_id,
            event_id,
            director_member_id,
            expected_run_id,
            expected_owner_epoch,
            request_token,
            decision,
            decision_source,
            director_mode,
            mode_evidence_ref,
            decision_evidence_ref,
            operation_key,
            json,
        } => {
            let response = crate::runtime::runner::respond_permission(
                conn,
                settings,
                &PermissionRespondInput {
                    interaction_id,
                    event_id,
                    director_member_id,
                    expected_run_id,
                    expected_owner_epoch,
                    request_token,
                    decision,
                    decision_source,
                    director_mode,
                    mode_evidence_ref,
                    decision_evidence_ref,
                    operation_key,
                },
                deadline,
            )
            .map_err(CafleetError::from)?;
            emit(
                json,
                &crate::presentation::permission_response_result(&response),
                || crate::output::format_permission_response_result(&response),
            );
            if response.submission.status == SubmissionStatus::Accepted {
                return Ok(());
            }
            let reason = response
                .submission
                .reason
                .as_deref()
                .expect("a nonaccepted historical projection supplies its reason");
            let mut message = format!(
                "permission response {} for interaction {}: {}",
                response.submission.status.as_str(),
                interaction_id,
                reason
            );
            if let Some(input_id) = response.submission.input_id {
                message.push_str(&format!(" (input_id={input_id})"));
            }
            Err(CafleetError::App(message))
        }
    }
}

pub(crate) fn permission_preflight(command: &PermissionCommand) -> Result<(), CafleetError> {
    if let PermissionCommand::Respond {
        decision_source,
        director_mode,
        ..
    } = command
    {
        crate::runtime::runner::validate_permission_attribution(*decision_source, *director_mode)?;
    }
    Ok(())
}
pub(crate) fn permission_error(command: &PermissionCommand, source: CafleetError) -> CafleetError {
    let (run_id, interaction_id, operation_key) = match command {
        PermissionCommand::Inspect { interaction_id, .. } => (None, Some(*interaction_id), None),
        PermissionCommand::Respond {
            expected_run_id,
            interaction_id,
            operation_key,
            ..
        } => (
            Some(*expected_run_id),
            Some(*interaction_id),
            Some(operation_key.clone()),
        ),
    };
    crate::runtime::runner::InputFailure {
        source,
        run_id,
        interaction_id,
        input_id: None,
        operation_key,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::runtime as storage;
    use crate::cli::CliArgs;
    use crate::cli::test_support::usage_error;
    use crate::config::Settings;
    use crate::input::{Deadline, PaneLock};
    use clap::Parser;
    use std::time::{Duration, Instant};

    #[test]
    fn live_receipt_cli_reconciles_real_exec_history_and_preserves_repeat_evidence() {
        crate::runtime::runner::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                let settings = Settings::for_database(
                    std::path::Path::new(fixture.fixture.conn.path().unwrap()),
                    None,
                );
                let original = storage::reserve_input(
                    &mut fixture.fixture.conn,
                    &run.owner(),
                    &storage::NewInput {
                        operation_key: "exec:cli-receipt-history".into(),
                        kind: storage::InputKind::Exec,
                        payload_digest: "d".repeat(64),
                        message_id: None,
                    },
                )
                .unwrap();
                storage::record_input_state(
                    &mut fixture.fixture.conn,
                    &run.owner(),
                    original.input_id,
                    storage::InputState::InProgress,
                    storage::InputState::Uncertain,
                )
                .unwrap();
                let until = outer.min(Instant::now() + Duration::from_secs(2));
                let command = |disposition, evidence: &str| InputCommand::Reconcile {
                    input_id: original.input_id,
                    disposition,
                    evidence_ref: evidence.into(),
                    json: true,
                };
                input(
                    &mut fixture.fixture.conn,
                    &settings,
                    command(
                        Disposition::AbandonedUncertain,
                        "fixture:cli-owned-resolution",
                    ),
                    Deadline::until(until),
                )
                .unwrap();
                let settled = storage::get_input(&fixture.fixture.conn, original.input_id).unwrap();
                assert_eq!(settled.state, storage::InputState::Settled);
                assert_eq!(
                    settled.disposition,
                    Some(storage::InputDisposition::AbandonedUncertain)
                );
                assert_eq!(
                    settled.evidence_ref.as_deref(),
                    Some("fixture:cli-owned-resolution")
                );
                let placement =
                    crate::broker::get_member(&fixture.fixture.conn, run.member_id, run.fleet_id)
                        .unwrap()
                        .unwrap()
                        .placement
                        .unwrap();
                let held = PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .unwrap();
                let changes = fixture.fixture.conn.total_changes();
                input(
                    &mut fixture.fixture.conn,
                    &settings,
                    command(
                        Disposition::AbandonedUncertain,
                        "fixture:replacement-evidence",
                    ),
                    Deadline::until(until),
                )
                .unwrap();
                assert!(
                    input(
                        &mut fixture.fixture.conn,
                        &settings,
                        command(
                            Disposition::Submitted,
                            "fixture:conflicting-cli-disposition"
                        ),
                        Deadline::until(until)
                    )
                    .is_err()
                );
                assert_eq!(fixture.fixture.conn.total_changes(), changes);
                assert_eq!(
                    storage::get_input(&fixture.fixture.conn, original.input_id).unwrap(),
                    settled
                );
                held.validate_binding(&settings.database_url, &placement)
                    .unwrap();
                drop(held);
                crate::runtime::runner::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                crate::runtime::runner::service::test_support::input_service::input_service_records(fixture)
                    .iter()
                    .map(|record| record["line"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
                ["barrier"],
                "receipt-only CLI work sends no native input"
            );
            },
        );
    }

    mod permission_entry {
        use super::*;

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

        const INSPECT_IDS: &[&str] = &["interaction", "--event-id", "--from-member-id"];
        const RESPOND_IDS: &[&str] = &[
            "interaction",
            "--event-id",
            "--from-member-id",
            "--expected-run-id",
            "--expected-owner-epoch",
        ];

        #[test]
        fn permission_commands_accept_positive_ids_with_json_and_reject_every_other_id() {
            for (fixture, fields) in [(inspect(), INSPECT_IDS), (respond(), RESPOND_IDS)] {
                accepted(&fixture);
                for field in fields {
                    for value in ["1", "9223372036854775807"] {
                        for json in [false, true] {
                            let mut args = fixture.clone();
                            set(&mut args, field, value);
                            if json {
                                args.push("--json".into());
                            }
                            accepted(&args);
                        }
                    }
                    for value in [
                        "0",
                        "-1",
                        "unknown",
                        "1.0",
                        "9223372036854775808",
                        "18446744073709551616",
                    ] {
                        let mut args = fixture.clone();
                        set(&mut args, field, value);
                        rejected(&args);
                    }
                }
            }
        }
        #[test]
        fn permission_respond_accepts_each_semantic_decision_and_valid_source_mode_pair() {
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
        fn required(mut args: Vec<String>, flags: &[&str]) {
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
        fn permission_inspect_requires_its_interaction_event_and_director() {
            required(inspect(), &["--event-id", "--from-member-id"]);
        }
        #[test]
        fn permission_respond_requires_each_snapshot_decision_and_provenance_field() {
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
        fn permission_response_accepts_exact_lowercase_hex_tokens() {
            for token in ["0".repeat(64), "f".repeat(64), "0123456789abcdef".repeat(4)] {
                let mut args = respond();
                set(&mut args, "--request-token", &token);
                accepted(&args);
            }
        }
        #[test]
        fn permission_response_rejects_uppercase_nonhex_and_wrong_length_tokens() {
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
        fn permission_response_references_accept_nonblank_utf8_at_exact_byte_boundaries() {
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
        fn permission_response_references_reject_blank_oversize_and_every_control() {
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
        fn permission_response_operation_key_accepts_literal_nonblank_values_and_rejects_blank() {
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
        fn permission_response_rejects_invalid_decision_source_and_mode_enums() {
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
    }

    #[test]
    fn runtime_read_commands_accept_positive_assigned_ids_and_json_selection() {
        for command in ["event", "input"] {
            for id in ["1", "481", "9223372036854775807"] {
                for json in [false, true] {
                    let mut args = vec!["cafleet", command, "show", id];
                    if json {
                        args.push("--json");
                    }
                    assert!(CliArgs::try_parse_from(&args).is_ok(), "{args:?}");
                }
            }
        }
    }

    #[test]
    fn runtime_read_ids_reject_zero_negative_malformed_and_overflowing_values() {
        for command in ["event", "input"] {
            for id in ["0", "-1", "unknown", "1.0", "9223372036854775808"] {
                assert_eq!(usage_error(&[command, "show", id, "--json"]).exit_code(), 2);
            }
            assert_eq!(usage_error(&[command, "show"]).exit_code(), 2);
        }
    }

    #[test]
    fn input_reconcile_accepts_exact_cli_dispositions_with_evidence() {
        for disposition in ["submitted", "not-submitted", "abandoned-uncertain"] {
            assert!(
                CliArgs::try_parse_from([
                    "cafleet",
                    "input",
                    "reconcile",
                    "17",
                    "--disposition",
                    disposition,
                    "--evidence-ref",
                    "audit:inspected-receipt",
                    "--json",
                ])
                .is_ok(),
                "{disposition}"
            );
        }
    }

    #[test]
    fn input_reconcile_requires_valid_disposition_and_nonempty_evidence() {
        for args in [
            vec!["input", "reconcile", "17"],
            vec!["input", "reconcile", "17", "--disposition", "submitted"],
            vec![
                "input",
                "reconcile",
                "17",
                "--evidence-ref",
                "audit:missing-choice",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "approved",
                "--evidence-ref",
                "audit:x",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "not_submitted",
                "--evidence-ref",
                "audit:x",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "submitted",
                "--evidence-ref",
                "",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "submitted",
                "--evidence-ref",
                "   ",
            ],
            vec![
                "input",
                "reconcile",
                "0",
                "--disposition",
                "submitted",
                "--evidence-ref",
                "audit:x",
            ],
        ] {
            assert_eq!(usage_error(&args).exit_code(), 2, "{args:?}");
        }
    }
}
