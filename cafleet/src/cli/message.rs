use rusqlite::Connection;

use clap::{Args, Subcommand};

use super::helpers::{emit, resolve_body};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{format_message, format_messages};
use crate::presentation;
use crate::receive::{AttemptEnd, BrokerReceiver, SystemClock};

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
pub(crate) enum MessageCommand {
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
        /// Maximum seconds to wait.
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=1800))]
        timeout: u64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Stream full pending inbox records as JSONL without consuming them.
    Listen {
        /// Active recipient whose inbox is streamed.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Optional monotonic lease in seconds.
        #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=1800))]
        timeout: Option<u64>,
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

pub(crate) fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: MessageCommand,
) -> Result<(), CafleetError> {
    match command {
        MessageCommand::Send {
            from_member_id,
            to_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref(), "--file")?;
            let deadline = crate::input::Deadline::new();
            let message = crate::input::with_database_budget(conn, deadline, |conn| {
                broker::send_message(conn, from_member_id, to_member_id, &text)
            })?;
            let message_id = message.message_id;
            let notification_sent = if from_member_id == to_member_id {
                false
            } else {
                match crate::runtime::deliver_pending(
                    conn,
                    settings,
                    to_member_id,
                    None,
                    deadline,
                    &std::collections::BTreeSet::new(),
                ) {
                    Ok(outcome) => {
                        if let AttemptEnd::Failed(error) = &outcome.end {
                            eprintln!("Message {message_id} was persisted. {}", error.error);
                        }
                        outcome.sent.contains(&message_id)
                    }
                    Err(error) => {
                        eprintln!("Message {message_id} was persisted. {}", error.message());
                        false
                    }
                }
            };
            emit(
                json,
                &presentation::send_outcome(&message, notification_sent),
                || {
                    format!(
                        "Message sent.\n{}",
                        format_message(&message, settings.max_text_len)
                    )
                },
            );
            Ok(())
        }
        MessageCommand::Poll { member_id, json } => {
            let messages = broker::poll_messages(conn, member_id)?;
            let payload =
                serde_json::Value::Array(messages.iter().map(presentation::message).collect());
            emit(json, &payload, || {
                format_messages(&messages, settings.max_text_len)
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
            let messages = &outcome.messages;
            let payload = serde_json::json!({
                "member_id":member_id,
                "outcome":if messages.is_empty() { "timeout" } else { "messages" },
                "messages":messages.iter().map(presentation::message).collect::<Vec<_>>(),
            });
            emit(json, &payload, || {
                if messages.is_empty() {
                    format!("Timed out waiting for member {member_id}.")
                } else {
                    format!(
                        "Messages available for member {member_id}.\n{}",
                        format_messages(messages, settings.max_text_len)
                    )
                }
            });
            Ok(())
        }
        MessageCommand::Listen { member_id, timeout } => {
            crate::listen::run(conn, member_id, timeout.map(std::time::Duration::from_secs))
        }
        MessageCommand::Ack { message_id, json } => {
            let message = broker::ack_message(conn, message_id)?;
            emit(json, &presentation::message_envelope(&message), || {
                format!(
                    "Message acknowledged.\n{}",
                    format_message(&message, settings.max_text_len)
                )
            });
            Ok(())
        }
        MessageCommand::Show { message_id, json } => {
            let message = broker::get_message(conn, message_id)?;
            emit(json, &presentation::message_envelope(&message), || {
                format_message(&message, settings.max_text_len)
            });
            Ok(())
        }
    }
}
