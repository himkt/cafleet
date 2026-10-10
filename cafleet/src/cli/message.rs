//! The `message` group and its shared handler sequence (SPEC §6.3 *message
//! group*): handler → emit fork (JSON = the complete untruncated result;
//! text = truncation + compact rendering). The broker derives the fleet and
//! recipient from the subject row.

use rusqlite::Connection;

use clap::{Args, Subcommand};
use serde_json::Value;

use super::helpers::{resolve_body, resolve_mux};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{format_indexed_list, format_message, truncate_message_text};
use crate::presentation;
use crate::runtime::{deliver_preview, ensure_loop_for_owed_work};

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

/// The fleet of a sender the broker has just validated as active.
fn sender_fleet(conn: &Connection, from_member_id: i64) -> Result<i64, CafleetError> {
    Ok(broker::active_member_fleet(conn, from_member_id)?
        .expect("the broker persisted a message from this active sender"))
}

pub fn run(
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

            let message =
                broker::send_message(conn, from_member_id, &to_member_id.to_string(), &text)?;
            let message_id = message.message_id;
            let mux = resolve_mux(settings).ok();
            let notification_sent =
                deliver_preview(conn, settings, mux.as_ref(), to_member_id, message_id)?;
            ensure_loop_for_owed_work(conn, settings, sender_fleet(conn, from_member_id)?)
                .map_err(|error| {
                    CafleetError::App(format!(
                        "Message {message_id} was persisted, but {}. \
                         Do not resend this message; run 'cafleet doctor'.",
                        error.message()
                    ))
                })?;
            emit_result(
                settings,
                presentation::send_outcome(&message, notification_sent),
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

            let rows = broker::broadcast_message(conn, from_member_id, &text)?;
            let summary_id = rows.summary.message_id;
            let recipients = rows.deliveries.len();
            let mux = resolve_mux(settings).ok();
            let mut delivered = 0;
            for (member_id, message_id) in rows.deliveries {
                if deliver_preview(conn, settings, mux.as_ref(), member_id, message_id)? {
                    delivered += 1;
                }
            }
            ensure_loop_for_owed_work(conn, settings, sender_fleet(conn, from_member_id)?)
                .map_err(|error| {
                    CafleetError::App(format!(
                        "Broadcast {summary_id} was persisted, but {}. \
                         Do not resend it; run 'cafleet doctor'.",
                        error.message()
                    ))
                })?;
            emit_result(
                settings,
                Value::Array(vec![presentation::broadcast_outcome(
                    &rows.summary,
                    recipients,
                    delivered,
                )]),
                json,
                |_| {
                    format!(
                        "broadcast id={summary_id} recipients={recipients} delivered={delivered}"
                    )
                },
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
