use rusqlite::Connection;

use clap::{Args, Subcommand};

use super::helpers::{emit, resolve_body};
use crate::broker;
use crate::broker::runtime::{DeliveryMethod, DeliveryProjection, DeliveryStatus};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{
    format_message, format_message_acknowledged, format_message_sent, format_message_wait,
    format_messages,
};
use crate::presentation;
use crate::receive::{AttemptEnd, AttemptOutcome, BrokerReceiver, SystemClock};

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
            let outcome = if from_member_id == to_member_id {
                None
            } else {
                match crate::delivery::deliver_pending(
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
                        Some(outcome)
                    }
                    Err(error) => {
                        eprintln!("Message {message_id} was persisted. {}", error.message());
                        None
                    }
                }
            };
            let notification_sent = outcome
                .as_ref()
                .is_some_and(|outcome| outcome.sent.contains(&message_id));
            let (input_id, delivery) =
                send_delivery(conn, &message, notification_sent, outcome.as_ref())?;
            emit(
                json,
                &presentation::send_outcome(&message, notification_sent, input_id, &delivery),
                || format_message_sent(&message, settings.max_text_len, input_id, &delivery),
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
            let messages = &crate::receive::wait(
                &SystemClock,
                &mut receiver,
                std::time::Duration::from_secs(timeout),
            )?;
            let payload = serde_json::json!({
                "member_id":member_id,
                "outcome":if messages.is_empty() { "timeout" } else { "messages" },
                "messages":messages.iter().map(presentation::message).collect::<Vec<_>>(),
            });
            emit(json, &payload, || {
                format_message_wait(member_id, messages, settings.max_text_len)
            });
            Ok(())
        }
        MessageCommand::Listen { member_id, timeout } => {
            crate::listen::run(conn, member_id, timeout.map(std::time::Duration::from_secs))
        }
        MessageCommand::Ack { message_id, json } => {
            let message = broker::ack_message(conn, message_id)?;
            emit(json, &presentation::message_envelope(&message), || {
                format_message_acknowledged(&message, settings.max_text_len)
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

/// The newly sent message's delivery: broker-only for self-sends and the root Director, its
/// preview receipt for a wrapped recipient, and the multiplexer path otherwise.
fn send_delivery(
    conn: &Connection,
    message: &crate::broker::records::MessageRecord,
    notification_sent: bool,
    outcome: Option<&AttemptOutcome>,
) -> Result<(Option<i64>, DeliveryProjection), CafleetError> {
    let projection = |status, method, reason: Option<&str>| DeliveryProjection {
        status,
        method,
        reason: reason.map(str::to_owned),
    };
    if message.from_member_id == message.to_member_id {
        return Ok((
            None,
            projection(DeliveryStatus::NotApplicable, None, Some("self_send")),
        ));
    }
    let recipient = match broker::active_member_fleet(conn, message.to_member_id)? {
        Some(fleet) => broker::get_member(conn, message.to_member_id, fleet)?,
        None => None,
    };
    let Some(recipient) = recipient else {
        return Ok((
            None,
            projection(DeliveryStatus::Held, None, Some("notification_held")),
        ));
    };
    if recipient.kind == crate::broker::records::MemberKind::Director {
        return Ok((
            None,
            projection(DeliveryStatus::NotApplicable, None, Some("director_stdout")),
        ));
    }
    if let Some(runtime) = &recipient.runtime {
        let key = format!("preview:{}:{}", runtime.run.run_id, message.message_id);
        if let Some(input) = crate::broker::runtime::find_input(conn, runtime.run.run_id, &key)?
            && let Some(delivery) = input.delivery()
        {
            return Ok((Some(input.input_id), delivery));
        }
    } else if notification_sent {
        return Ok((
            None,
            projection(
                DeliveryStatus::Submitted,
                Some(DeliveryMethod::Unwrapped),
                None,
            ),
        ));
    }
    let reason = match outcome {
        Some(AttemptOutcome {
            held_reason: Some(reason),
            ..
        }) => reason.as_str(),
        Some(AttemptOutcome {
            end: AttemptEnd::BudgetExhausted,
            ..
        }) => "delivery_budget_exhausted",
        Some(AttemptOutcome {
            end: AttemptEnd::Failed(_),
            ..
        })
        | None => "notification_failed",
        Some(_) => "notification_held",
    };
    Ok((None, projection(DeliveryStatus::Held, None, Some(reason))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;

    fn projection(
        status: DeliveryStatus,
        method: Option<DeliveryMethod>,
        reason: Option<&str>,
    ) -> DeliveryProjection {
        DeliveryProjection {
            status,
            method,
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn send_delivery_reports_self_sends_the_root_director_and_unwrapped_notifications() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "branches");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        for (from, to, notification_sent, expected) in [
            (
                director,
                director,
                false,
                projection(DeliveryStatus::NotApplicable, None, Some("self_send")),
            ),
            (
                worker,
                director,
                false,
                projection(DeliveryStatus::NotApplicable, None, Some("director_stdout")),
            ),
            (
                director,
                worker,
                true,
                projection(
                    DeliveryStatus::Submitted,
                    Some(DeliveryMethod::Unwrapped),
                    None,
                ),
            ),
        ] {
            let message = broker::send_message(&mut conn, from, to, "branch").unwrap();
            assert_eq!(
                send_delivery(&conn, &message, notification_sent, None).unwrap(),
                (None, expected),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn send_delivery_reports_a_wrapped_self_send_and_the_queued_preview_receipt() {
        let mut fixture = crate::runtime::runner::test_support::Fixture::new();
        let run = fixture.run.clone();
        let own =
            broker::send_message(&mut fixture.conn, run.member_id, run.member_id, "self").unwrap();
        assert_eq!(
            send_delivery(&fixture.conn, &own, false, None).unwrap(),
            (
                None,
                projection(DeliveryStatus::NotApplicable, None, Some("self_send"))
            ),
            "a self-send precedes the wrapped recipient's own delivery"
        );
        let message = broker::send_message(
            &mut fixture.conn,
            run.director_member_id,
            run.member_id,
            "queued",
        )
        .unwrap();
        fixture
            .conn
            .execute(
                "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,message_id,created_at,updated_at,deferred_at,defer_reason) VALUES(?1,?2,'preview',?3,'ready',?4,?5,?5,?5,'interaction_open')",
                rusqlite::params![
                    run.run_id,
                    format!("preview:{}:{}", run.run_id, message.message_id),
                    "d".repeat(64),
                    message.message_id,
                    "2026-10-30T10:00:00Z",
                ],
            )
            .unwrap();
        let receipt = fixture.conn.last_insert_rowid();
        assert_eq!(
            send_delivery(&fixture.conn, &message, false, None).unwrap(),
            (
                Some(receipt),
                projection(DeliveryStatus::Queued, None, Some("interaction_open"))
            )
        );
    }

    #[test]
    fn a_send_whose_recipient_is_deregistered_after_the_commit_reports_it_held() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "deregistered");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "committed").unwrap();
        assert!(broker::deregister_member(&mut conn, worker).unwrap());
        let (input_id, delivery) = send_delivery(&conn, &message, false, None)
            .expect("a committed send reports its record after the recipient is gone");
        assert_eq!(input_id, None);
        assert_eq!(
            delivery,
            DeliveryProjection {
                status: DeliveryStatus::Held,
                method: None,
                reason: Some("notification_held".into()),
            }
        );
    }
}
