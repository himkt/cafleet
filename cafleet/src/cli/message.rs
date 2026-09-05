//! The `message` group and its shared handler sequence (SPEC §6.3 *message
//! group*): handler → emit fork (JSON = the complete untruncated result;
//! text = truncation + compact rendering). The broker derives the fleet and
//! recipient from the subject row.

use clap::{Args, Subcommand};
use serde_json::Value;

use super::helpers::{connect, resolve_body};
use crate::broker;
use crate::broker::records::NotificationAttempt;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{format_indexed_list, format_message, truncate_message_text};
use crate::presentation;
use crate::runtime::RuntimeNotifier;

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
            let text = resolve_body(body.text.as_deref(), body.file.as_deref(), "--file")?;
            let mut conn = connect(settings)?;
            let notifier = RuntimeNotifier::new(settings);
            let outcome = broker::send_message_record(
                &mut conn,
                &notifier,
                settings.max_text_len,
                from_member_id,
                &to_member_id.to_string(),
                &text,
            )?;
            if let NotificationAttempt::Failed { error: raw } = &outcome.notification {
                let message_id = outcome.message.message_id;
                return Err(CafleetError::App(format!(
                    "Message {message_id} was persisted, but pane notification failed: {raw}. \
                     Do not resend this message. Recover the recipient pane, then run \
                     'cafleet member ping {to_member_id}' or have the recipient run \
                     'cafleet message poll {to_member_id}'."
                )));
            }
            emit_result(
                settings,
                presentation::send_outcome(&outcome),
                json,
                |result| format!("Message sent.\n{}", format_message(result)),
            );
            Ok(())
        }
        MessageCommand::Broadcast {
            from_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref(), "--file")?;
            let mut conn = connect(settings)?;
            let notifier = RuntimeNotifier::new(settings);
            let result = broker::broadcast_message_record(
                &mut conn,
                &notifier,
                settings.max_text_len,
                from_member_id,
                &text,
            )?;
            emit_result(
                settings,
                Value::Array(vec![presentation::broadcast_outcome(&result)]),
                json,
                |_| {
                    format!(
                        "broadcast id={} recipients={} delivered={}",
                        result.message.message_id, result.recipients, result.delivered,
                    )
                },
            );
            Ok(())
        }
        MessageCommand::Poll { member_id, json } => {
            let conn = connect(settings)?;
            let result = broker::poll_message_records(&conn, member_id)?
                .iter()
                .map(presentation::message)
                .collect();
            emit_result(settings, Value::Array(result), json, |result| {
                let items = result.as_array().expect("poll returns a list");
                format_indexed_list(items, format_message, "No messages found.")
            });
            Ok(())
        }
        MessageCommand::Ack { message_id, json } => {
            let mut conn = connect(settings)?;
            let result =
                presentation::message_envelope(&broker::ack_message_record(&mut conn, message_id)?);
            emit_result(settings, result, json, |result| {
                format!("Message acknowledged.\n{}", format_message(result))
            });
            Ok(())
        }
        MessageCommand::Show { message_id, json } => {
            let conn = connect(settings)?;
            let result =
                presentation::message_envelope(&broker::get_message_record(&conn, message_id)?);
            emit_result(settings, result, json, format_message);
            Ok(())
        }
    }
}
