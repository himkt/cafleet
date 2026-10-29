use clap::Subcommand;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use rusqlite::Connection;

use super::helpers::emit;
use crate::broker::runtime;
use crate::error::CafleetError;

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

fn evidence_reference(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("a nonempty evidence reference is required".into())
    } else {
        Ok(value.into())
    }
}

#[derive(Subcommand)]
pub enum EventCommand {
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
pub enum Disposition {
    Submitted,
    NotSubmitted,
    AbandonedUncertain,
}

#[derive(Subcommand)]
pub enum InputCommand {
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
        #[arg(long, value_parser = evidence_reference)]
        evidence_ref: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub fn event(conn: &Connection, command: EventCommand) -> Result<(), CafleetError> {
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

pub fn input(
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

fn permission_reference(value: &str) -> Result<String, String> {
    crate::runtime::runner::validate_permission_reference(value)
        .map_err(|cause| cause.to_string())?;
    Ok(value.into())
}
fn request_token(value: &str) -> Result<String, String> {
    crate::runtime::runner::validate_request_token(value).map_err(|cause| cause.to_string())?;
    Ok(value.into())
}
fn operation_key(value: &str) -> Result<String, String> {
    crate::runtime::runner::validate_operation_key(value).map_err(|cause| cause.to_string())?;
    Ok(value.into())
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
pub enum PermissionCommand {
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
        #[arg(long, value_parser = request_token)]
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
        #[arg(long, value_parser = permission_reference)]
        mode_evidence_ref: String,
        /// Evidence reference for the decision.
        #[arg(long, value_parser = permission_reference)]
        decision_evidence_ref: String,
        /// Idempotency key of this response.
        #[arg(long, value_parser = operation_key)]
        operation_key: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub fn permission(
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
    use crate::config::Settings;
    use crate::input::{Deadline, PaneLock};
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
}
