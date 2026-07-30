//! The `message` group and its shared handler sequence (SPEC §6.3 *message
//! group*): fleet read → fleet-gate → handler → truncation + rendering →
//! json-vs-text emit.

use clap::{Args, Subcommand};
use serde_json::Value;

use super::FleetIdArg;
use super::helpers::{CliNotifier, connect, require_fleet_id, resolve_text_body};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{
    format_indexed_list, format_message, render_messages_in_result, truncate_message_text,
};

#[derive(Args)]
pub(crate) struct BodyArgs {
    /// Inline message body. Exactly one of --text / --text-file.
    #[arg(long)]
    text: Option<String>,
    /// UTF-8 file carrying the body (`-` = stdin).
    #[arg(long = "text-file", value_name = "PATH")]
    text_file: Option<String>,
}

#[derive(Subcommand)]
pub enum MessageCommand {
    /// Send a unicast message.
    Send {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Sender's member ID.
        #[arg(long = "from-member-id")]
        from_member_id: i64,
        /// Recipient member ID.
        #[arg(long = "to-member-id")]
        to_member_id: i64,
        #[command(flatten)]
        body: BodyArgs,
        /// Emit the untruncated verbose form.
        #[arg(long)]
        full: bool,
        /// Print only the bare message_id.
        #[arg(long)]
        quiet: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Broadcast a message to all fleet members.
    Broadcast {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Sender's member ID.
        #[arg(long = "from-member-id")]
        from_member_id: i64,
        #[command(flatten)]
        body: BodyArgs,
        /// Emit the untruncated verbose form.
        #[arg(long)]
        full: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Fetch un-acked incoming messages.
    Poll {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Emit the untruncated verbose form.
        #[arg(long)]
        full: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Acknowledge a received message.
    Ack {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Message to acknowledge.
        #[arg(long = "message-id")]
        message_id: i64,
        /// Emit the untruncated verbose form.
        #[arg(long)]
        full: bool,
        /// Print only the bare message_id.
        #[arg(long)]
        quiet: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Show one message.
    Show {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Message to fetch.
        #[arg(long = "message-id")]
        message_id: i64,
        /// Emit the untruncated verbose form.
        #[arg(long)]
        full: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

fn fleet_gate(
    conn: &rusqlite::Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<(), CafleetError> {
    if broker::verify_member_fleet(conn, member_id, fleet_id)? {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "member {member_id} is not in fleet {fleet_id}."
        )))
    }
}

/// Truncate + render per the shared sequence, then emit.
fn emit_result(
    settings: &Settings,
    mut result: Value,
    full: bool,
    json: bool,
    text: impl FnOnce(&Value) -> String,
) {
    truncate_message_text(&mut result, full, settings.max_text_len);
    if json {
        println!(
            "{}",
            crate::output::format_json(&render_messages_in_result(&result, full))
        );
    } else {
        println!("{}", text(&result));
    }
}

pub fn run(settings: &Settings, command: MessageCommand) -> Result<(), CafleetError> {
    match command {
        MessageCommand::Send {
            fleet,
            from_member_id,
            to_member_id,
            body,
            full,
            quiet,
            json,
        } => {
            let fleet_id = require_fleet_id(fleet.fleet_id)?;
            let text = resolve_text_body(body.text.as_deref(), body.text_file.as_deref())?;
            let mut conn = connect(settings)?;
            fleet_gate(&conn, from_member_id, fleet_id)?;
            let notifier = CliNotifier::new(settings);
            let result = broker::send_message(
                &mut conn,
                &notifier,
                settings.max_text_len,
                fleet_id,
                from_member_id,
                &to_member_id.to_string(),
                &text,
            )?;
            if quiet && !json {
                println!("{}", result["message"]["message_id"]);
                return Ok(());
            }
            emit_result(settings, result, full, json, |result| {
                format!("Message sent.\n{}", format_message(result, full))
            });
            Ok(())
        }
        MessageCommand::Broadcast {
            fleet,
            from_member_id,
            body,
            full,
            json,
        } => {
            let fleet_id = require_fleet_id(fleet.fleet_id)?;
            let text = resolve_text_body(body.text.as_deref(), body.text_file.as_deref())?;
            let mut conn = connect(settings)?;
            let notifier = CliNotifier::new(settings);
            let result = broker::broadcast_message(
                &mut conn,
                &notifier,
                settings.max_text_len,
                fleet_id,
                from_member_id,
                &text,
            )?;
            emit_result(settings, Value::Array(result), full, json, |result| {
                let envelope = &result[0];
                if full {
                    format_message(envelope, true)
                } else {
                    format!(
                        "broadcast id={} recipients={} delivered={}",
                        envelope["message"]["message_id"],
                        envelope["recipients"],
                        envelope["delivered"],
                    )
                }
            });
            Ok(())
        }
        MessageCommand::Poll {
            fleet,
            member_id,
            full,
            json,
        } => {
            let fleet_id = require_fleet_id(fleet.fleet_id)?;
            let conn = connect(settings)?;
            fleet_gate(&conn, member_id, fleet_id)?;
            let result = broker::poll_messages(&conn, member_id)?;
            emit_result(settings, Value::Array(result), full, json, |result| {
                let items = result.as_array().expect("poll returns a list");
                format_indexed_list(items, |m| format_message(m, full), "No messages found.")
            });
            Ok(())
        }
        MessageCommand::Ack {
            fleet,
            member_id,
            message_id,
            full,
            quiet,
            json,
        } => {
            let fleet_id = require_fleet_id(fleet.fleet_id)?;
            let mut conn = connect(settings)?;
            fleet_gate(&conn, member_id, fleet_id)?;
            let result = broker::ack_message(&mut conn, member_id, message_id)?;
            if quiet && !json {
                println!("{}", result["message"]["message_id"]);
                return Ok(());
            }
            emit_result(settings, result, full, json, |result| {
                format!("Message acknowledged.\n{}", format_message(result, full))
            });
            Ok(())
        }
        MessageCommand::Show {
            fleet,
            member_id,
            message_id,
            full,
            json,
        } => {
            let fleet_id = require_fleet_id(fleet.fleet_id)?;
            let conn = connect(settings)?;
            fleet_gate(&conn, member_id, fleet_id)?;
            let result = broker::get_message(&conn, fleet_id, message_id)?;
            emit_result(settings, result, full, json, |result| {
                format_message(result, full)
            });
            Ok(())
        }
    }
}
