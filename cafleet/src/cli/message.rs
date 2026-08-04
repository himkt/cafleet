//! The `message` group and its shared handler sequence (SPEC §6.3 *message
//! group*): handler → emit fork (JSON = the complete untruncated result;
//! text = truncation + compact rendering). The broker derives the fleet and
//! recipient from the subject row.

use clap::{Args, Subcommand};
use serde_json::Value;

use super::helpers::{CliNotifier, connect, resolve_body};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{format_indexed_list, format_message, truncate_message_text};

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct BodyArgs {
    /// Inline message body. Exactly one of TEXT / --file.
    #[arg(value_name = "TEXT")]
    text: Option<String>,
    /// UTF-8 file carrying the body (`-` = stdin).
    #[arg(long, value_name = "PATH")]
    file: Option<String>,
}

#[derive(Subcommand)]
pub enum MessageCommand {
    /// Send a unicast message.
    Send {
        /// Sender's member ID.
        #[arg(long = "from-member-id")]
        from_member_id: i64,
        /// Recipient member ID.
        #[arg(long = "to-member-id")]
        to_member_id: i64,
        #[command(flatten)]
        body: BodyArgs,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Broadcast a message to all fleet members.
    Broadcast {
        /// Sender's member ID.
        #[arg(long = "from-member-id")]
        from_member_id: i64,
        #[command(flatten)]
        body: BodyArgs,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Fetch un-acked incoming messages.
    Poll {
        /// Recipient whose inbox is fetched.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Acknowledge a received message.
    Ack {
        /// Message to acknowledge.
        #[arg(value_name = "MESSAGE_ID")]
        message_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Show one message.
    Show {
        /// Message to fetch.
        #[arg(value_name = "MESSAGE_ID")]
        message_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

/// The emit fork of the shared handler sequence: JSON is the complete,
/// untruncated machine form; text truncates then renders compactly.
fn emit_result(
    settings: &Settings,
    mut result: Value,
    json: bool,
    text: impl FnOnce(&Value) -> String,
) {
    if json {
        println!("{}", crate::output::format_json(&result));
    } else {
        truncate_message_text(&mut result, settings.max_text_len);
        println!("{}", text(&result));
    }
}

pub fn run(settings: &Settings, command: MessageCommand) -> Result<(), CafleetError> {
    match command {
        MessageCommand::Send {
            from_member_id,
            to_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref())?;
            let mut conn = connect(settings)?;
            let notifier = CliNotifier::new(settings);
            let result = broker::send_message(
                &mut conn,
                &notifier,
                settings.max_text_len,
                from_member_id,
                &to_member_id.to_string(),
                &text,
            )?;
            emit_result(settings, result, json, |result| {
                format!("Message sent.\n{}", format_message(result))
            });
            Ok(())
        }
        MessageCommand::Broadcast {
            from_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref())?;
            let mut conn = connect(settings)?;
            let notifier = CliNotifier::new(settings);
            let result = broker::broadcast_message(
                &mut conn,
                &notifier,
                settings.max_text_len,
                from_member_id,
                &text,
            )?;
            emit_result(settings, Value::Array(result), json, |result| {
                let envelope = &result[0];
                format!(
                    "broadcast id={} recipients={} delivered={}",
                    envelope["message"]["message_id"],
                    envelope["recipients"],
                    envelope["delivered"],
                )
            });
            Ok(())
        }
        MessageCommand::Poll { member_id, json } => {
            let conn = connect(settings)?;
            let result = broker::poll_messages(&conn, member_id)?;
            emit_result(settings, Value::Array(result), json, |result| {
                let items = result.as_array().expect("poll returns a list");
                format_indexed_list(items, format_message, "No messages found.")
            });
            Ok(())
        }
        MessageCommand::Ack { message_id, json } => {
            let mut conn = connect(settings)?;
            let result = broker::ack_message(&mut conn, message_id)?;
            emit_result(settings, result, json, |result| {
                format!("Message acknowledged.\n{}", format_message(result))
            });
            Ok(())
        }
        MessageCommand::Show { message_id, json } => {
            let conn = connect(settings)?;
            let result = broker::get_message(&conn, message_id)?;
            emit_result(settings, result, json, format_message);
            Ok(())
        }
    }
}
