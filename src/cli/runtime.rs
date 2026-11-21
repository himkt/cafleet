use crate::broker::runtime;
use crate::cli::checked;
use crate::cli::emit;
use crate::cli::nonblank;
use crate::coding_agent::catalog::AnswerKey;
use crate::error::CafleetError;
use crate::runtime::validate_request_token;
use clap::Subcommand;
use clap::builder::PossibleValuesParser;
use clap::builder::TypedValueParser;
use rusqlite::Connection;

fn choice_id(value: &str) -> Result<String, String> {
    if crate::coding_agent::catalog::valid_choice_id(value) {
        Ok(value.into())
    } else {
        Err("a choice id is 1-64 bytes of lowercase hyphen-separated words".into())
    }
}

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
        /// The root Director issuing this Director-only command.
        #[arg(long = "from-member-id", value_name = "DIRECTOR_MEMBER_ID", value_parser = clap::value_parser!(i64).range(1..))]
        director_member_id: i64,
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
            emit(json, &crate::output::json::runtime_event(&view), || {
                crate::output::format_runtime_event(&view)
            })
        }
    }
}

pub(crate) fn input(
    conn: &mut Connection,
    settings: &crate::config::Settings,
    command: InputCommand,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    match command {
        InputCommand::Show { input_id, json } => {
            let input = runtime::get_input(conn, input_id)?;
            emit(json, &crate::output::json::input_operation(&input), || {
                crate::output::format_input_operation(&input)
            })
        }
        InputCommand::Reconcile {
            input_id,
            director_member_id: _,
            disposition,
            evidence_ref,
            json,
        } => {
            let disposition = match disposition {
                Disposition::Submitted => runtime::InputDisposition::Submitted,
                Disposition::NotSubmitted => runtime::InputDisposition::NotSubmitted,
                Disposition::AbandonedUncertain => runtime::InputDisposition::AbandonedUncertain,
            };
            let input = crate::runtime::reconcile_input(
                conn,
                settings,
                input_id,
                disposition,
                &evidence_ref,
                deadline,
            )
            .map_err(CafleetError::from)?;
            emit(json, &crate::output::json::input_operation(&input), || {
                crate::output::format_input_operation(&input)
            })
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
    /// Answer the inspected native permission request with a semantic decision, or the inspected
    /// question with a tested choice.
    #[command(group(clap::ArgGroup::new("answer").required(true).args(["decision", "choice"])))]
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
        /// The semantic answer to a permission request.
        #[arg(long, value_parser = stored_enum::<runtime::PermissionDecision>(runtime::PermissionDecision::VALUES))]
        decision: Option<runtime::PermissionDecision>,
        /// The tested choice that answers a question.
        #[arg(long, value_parser = choice_id)]
        choice: Option<String>,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn permission(
    conn: &mut Connection,
    settings: &crate::config::Settings,
    command: PermissionCommand,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    use crate::runtime::PermissionInspectInput;
    use crate::runtime::PermissionRespondInput;
    use crate::runtime::SubmissionStatus;
    match command {
        PermissionCommand::Inspect {
            interaction_id,
            event_id,
            director_member_id,
            json,
        } => {
            let inspection = crate::runtime::inspect_permission(
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
                &crate::output::json::permission_inspection(&inspection),
                || crate::output::format_permission_inspection(&inspection),
            )
        }
        PermissionCommand::Respond {
            interaction_id,
            event_id,
            director_member_id,
            expected_run_id,
            expected_owner_epoch,
            request_token,
            decision,
            choice,
            json,
        } => {
            let key = match (decision, choice) {
                (Some(decision), None) => AnswerKey::Decision(decision),
                (None, Some(choice)) => AnswerKey::Choice(choice),
                _ => unreachable!("clap requires exactly one answer argument"),
            };
            let response = crate::runtime::input_client::respond_permission_with_host(
                conn,
                settings,
                &PermissionRespondInput {
                    interaction_id,
                    event_id,
                    director_member_id,
                    expected_run_id,
                    expected_owner_epoch,
                    request_token,
                    key,
                },
                deadline,
                &mut crate::runtime::input_client::NativeInputHost,
            )
            .map_err(CafleetError::from)?;
            emit(
                json,
                &crate::output::json::permission_response_result(&response),
                || crate::output::format_permission_response_result(&response),
            )?;
            if response.submission.status == SubmissionStatus::Accepted {
                return Ok(());
            }
            let reason = response
                .submission
                .reason
                .as_deref()
                .expect("a nonaccepted permission response supplies its reason");
            Err(CafleetError::App(format!(
                "permission response {} for interaction {}: {}",
                response.submission.status.as_str(),
                interaction_id,
                reason
            )))
        }
    }
}

pub(crate) fn permission_error(command: &PermissionCommand, source: CafleetError) -> CafleetError {
    let (run_id, interaction_id) = match command {
        PermissionCommand::Inspect { interaction_id, .. } => (None, Some(*interaction_id)),
        PermissionCommand::Respond {
            expected_run_id,
            interaction_id,
            ..
        } => (Some(*expected_run_id), Some(*interaction_id)),
    };
    crate::runtime::InputContext {
        run_id,
        interaction_id,
        input_id: None,
        operation_key: None,
    }
    .failure(source)
    .into()
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime as storage;
    use crate::cli::CliArgs;
    use crate::cli::runtime::*;
    use crate::cli::test_support::assert_usage_rejected;
    use crate::cli::test_support::usage_error;
    use crate::config::Settings;
    use crate::pane_lock::PaneLock;
    use crate::time::Deadline;
    use clap::Parser;
    use std::time::Duration;
    use std::time::Instant;

    #[test]
    fn live_receipt_cli_reconciles_real_exec_history_and_preserves_repeat_evidence() {
        crate::runtime::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                let settings = Settings::for_database(std::path::Path::new(
                    fixture.fixture.conn.path().unwrap(),
                ));
                let original = crate::broker::runtime::test_support::reserve_input(
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
                storage::record_input_uncertain(
                    &mut fixture.fixture.conn,
                    &run.owner(),
                    original.input_id,
                    storage::StopCause::WriteFailed,
                )
                .unwrap();
                let until = outer.min(Instant::now() + Duration::from_secs(2));
                let command = |disposition, evidence: &str| InputCommand::Reconcile {
                    input_id: original.input_id,
                    director_member_id: run.director_member_id,
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
                        .placement;
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
                crate::runtime::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                    crate::runtime::service::test_support::input_service::input_service_records(
                        fixture
                    )
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
        use crate::cli::runtime::tests::*;

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
            assert_usage_rejected(&args[1..]);
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
                let mut json = fixture.clone();
                json.push("--json".into());
                accepted(&json);
                for field in fields {
                    let mut args = fixture.clone();
                    set(&mut args, field, "0");
                    rejected(&args);
                }
            }
            for command in ["event", "input"] {
                accepted(&["cafleet", command, "show", "481", "--json"].map(String::from));
                rejected(&["cafleet", command, "show", "0"].map(String::from));
            }
        }

        #[test]
        fn permission_respond_accepts_each_semantic_decision() {
            for decision in storage::PermissionDecision::VALUES {
                let mut args = respond();
                set(&mut args, "--decision", decision);
                accepted(&args);
            }
        }

        fn respond_choice(choice: &str) -> Vec<String> {
            let mut args = respond();
            let index = args
                .iter()
                .position(|arg| arg == "--decision")
                .expect("the fixture names its decision");
            args[index] = "--choice".into();
            args[index + 1] = choice.into();
            args
        }

        fn parsed(args: &[String]) -> PermissionCommand {
            match CliArgs::try_parse_from(args).unwrap().command {
                crate::cli::Command::Permission(command) => command,
                _ => panic!("a permission command"),
            }
        }

        fn parsed_answer(args: &[String]) -> (Option<storage::PermissionDecision>, Option<String>) {
            match parsed(args) {
                PermissionCommand::Respond {
                    decision, choice, ..
                } => (decision, choice),
                PermissionCommand::Inspect { .. } => panic!("a permission respond command"),
            }
        }

        #[test]
        fn permission_respond_accepts_a_choice_in_place_of_a_decision() {
            for choice in [
                "decline",
                "not-now",
                "dont-show-again",
                "a",
                &"a".repeat(64),
            ] {
                let args = respond_choice(choice);
                accepted(&args);
                assert_eq!(
                    parsed_answer(&args),
                    (None, Some(choice.to_owned())),
                    "{choice}"
                );
            }
            assert_eq!(
                parsed_answer(&respond()),
                (Some(storage::PermissionDecision::Allow), None)
            );
        }

        #[test]
        fn permission_respond_requires_exactly_one_of_a_decision_and_a_choice() {
            let mut both = respond();
            both.extend(["--choice".into(), "decline".into()]);
            rejected(&both);
            let mut both_choice_first = respond_choice("decline");
            both_choice_first.extend(["--decision".into(), "deny".into()]);
            rejected(&both_choice_first);
            let mut neither = respond();
            let index = neither.iter().position(|arg| arg == "--decision").unwrap();
            neither.drain(index..index + 2);
            rejected(&neither);
            let mut repeated = respond_choice("decline");
            repeated.extend(["--choice".into(), "not-now".into()]);
            rejected(&repeated);
        }

        #[test]
        fn permission_respond_validates_each_checked_flag() {
            rejected(&respond_choice("Decline"));
            for token in ["A".repeat(64), "a".repeat(63)] {
                let mut args = respond();
                set(&mut args, "--request-token", &token);
                rejected(&args);
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
            required(
                respond(),
                &[
                    "--event-id",
                    "--from-member-id",
                    "--expected-run-id",
                    "--expected-owner-epoch",
                    "--request-token",
                ],
            );
        }
    }

    #[test]
    fn positive_id_accepts_only_positive_i64_values() {
        for (value, id) in [("1", 1), ("9223372036854775807", i64::MAX)] {
            assert_eq!(positive_id(value), Ok(id));
        }
        for value in ["0", "-1", "unknown", "1.0", "9223372036854775808"] {
            assert_eq!(
                positive_id(value),
                Err("a positive assigned ID is required".to_owned()),
                "{value}"
            );
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
                    "--from-member-id",
                    "1",
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
            let args = [args, vec!["--from-member-id", "1"]].concat();
            assert_eq!(usage_error(&args).exit_code(), 2, "{args:?}");
        }
    }
}
