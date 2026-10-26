use clap::Subcommand;
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
        #[arg(value_parser = positive_id)]
        event_id: i64,
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
        #[arg(value_parser = positive_id)]
        input_id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Record an evidence-based input disposition under synchronized ownership.
    Reconcile {
        #[arg(value_parser = positive_id)]
        input_id: i64,
        #[arg(long, value_enum)]
        disposition: Disposition,
        #[arg(long, value_parser = evidence_reference)]
        evidence_ref: String,
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

pub fn input(conn: &Connection, command: InputCommand) -> Result<(), CafleetError> {
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
            let input = runtime::get_input(conn, input_id)?;
            let _request = (input, disposition, evidence_ref, json);
            Err(CafleetError::App(format!(
                "input_id={input_id}: reconciliation requires synchronized runner or exclusive recovery ownership"
            )))
        }
    }
}
