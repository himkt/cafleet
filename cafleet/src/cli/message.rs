//! The `message` group and its shared handler sequence (SPEC §6.3 *message
//! group*): handler → emit fork (JSON = the complete untruncated result;
//! text = truncation + compact rendering). The broker derives the fleet and
//! recipient from the subject row.

use rusqlite::Connection;

use clap::{Args, Subcommand};
use serde_json::Value;

use super::helpers::resolve_body;
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{format_indexed_list, format_message, truncate_message_text};
use crate::presentation;
use crate::receive::{AttemptOutcome, BrokerReceiver, Delivery, SystemClock};

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
    /// Fetch un-acked incoming messages.
    Poll {
        /// Recipient whose inbox is fetched.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Receive incoming messages and retry pending ordinary-member previews.
    Wait {
        /// Recipient whose inbox is received.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Maximum seconds; use the default for a host-native background task.
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=1800))]
        timeout: u64,
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

pub fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: MessageCommand,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    match command {
        MessageCommand::Send {
            from_member_id,
            to_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref(), "--file")?;

            let message = crate::input::with_database_budget(conn, deadline, |conn| {
                broker::send_message(conn, from_member_id, &to_member_id.to_string(), &text)
            })?;
            let message_id = message.message_id;
            let notification_sent =
                match crate::runtime::attempt_preview(conn, settings, to_member_id, None, deadline)
                {
                    Ok(AttemptOutcome::Sent {
                        message_id: notified,
                    }) => notified == message_id,
                    Ok(
                        AttemptOutcome::Held
                        | AttemptOutcome::NoPending
                        | AttemptOutcome::Settled { .. },
                    ) => false,
                    Ok(AttemptOutcome::Failed(error)) => {
                        eprintln!("Message {message_id} was persisted. {}", error.error);
                        false
                    }
                    Err(error) => {
                        eprintln!("Message {message_id} was persisted. {}", error.message());
                        false
                    }
                };
            emit_result(
                settings,
                presentation::send_outcome(&message, notification_sent),
                json,
                |result| format!("Message sent.\n{}", format_message(result)),
            );
            Ok(())
        }
        MessageCommand::Poll { member_id, json } => {
            let result = broker::poll_messages(conn, member_id)?
                .iter()
                .map(presentation::message)
                .collect();
            emit_result(settings, Value::Array(result), json, |result| {
                let items = result.as_array().expect("poll returns a list");
                format_indexed_list(items, format_message, "No messages found.")
            });
            Ok(())
        }
        MessageCommand::Wait {
            member_id,
            timeout,
            json,
        } => {
            let mut receiver = BrokerReceiver::new(conn, settings, member_id);
            let outcome = crate::receive::wait(
                &SystemClock,
                &mut receiver,
                std::time::Duration::from_secs(timeout),
            )?;
            let messages: Vec<_> = outcome.messages.iter().map(presentation::message).collect();
            let errors: Vec<_> = outcome.notification_errors.iter().map(|error| serde_json::json!({
                "member_id": error.member_id,
                "message_id": error.message_id,
                "delivery": match error.delivery { Delivery::NotSent => "not_sent", Delivery::Uncertain => "uncertain" },
                "error": error.error,
            })).collect();
            if !json {
                for error in &outcome.notification_errors {
                    eprintln!("{}", error.error);
                }
            }
            emit_result(
                settings,
                serde_json::json!({
                    "member_id": member_id,
                    "outcome": if messages.is_empty() { "timeout" } else { "messages" },
                    "messages": messages,
                    "notification_errors": errors,
                }),
                json,
                |result| {
                    let items = result["messages"]
                        .as_array()
                        .expect("wait returns a message list");
                    if items.is_empty() {
                        format!("Timed out waiting for member {member_id}.")
                    } else {
                        format!(
                            "Messages available for member {member_id}.\n{}",
                            format_indexed_list(items, format_message, "No messages found.")
                        )
                    }
                },
            );
            Ok(())
        }
        MessageCommand::Ack { message_id, json } => {
            let result = presentation::message_envelope(&broker::ack_message(conn, message_id)?);
            emit_result(settings, result, json, |result| {
                format!("Message acknowledged.\n{}", format_message(result))
            });
            Ok(())
        }
        MessageCommand::Show { message_id, json } => {
            let result = presentation::message_envelope(&broker::get_message(conn, message_id)?);
            emit_result(settings, result, json, format_message);
            Ok(())
        }
    }
}
