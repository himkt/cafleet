use rusqlite::Connection;

use clap::{Args, Subcommand};

use super::helpers::{emit, resolve_body};
use crate::broker;
use crate::broker::runtime::{DeliveryProjection, DeliveryStatus};
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
            let (input_id, delivery) = send_delivery(conn, &message, outcome.as_ref())?;
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

fn send_delivery(
    conn: &Connection,
    message: &crate::broker::records::MessageRecord,
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
    }
    let reason = match outcome {
        Some(AttemptOutcome {
            held_reason: Some(reason),
            ..
        }) => reason.as_str(),
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
    use crate::broker::runtime::DeliveryMethod;
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
    fn send_delivery_reports_self_sends_the_root_director_and_run_less_recipients() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "branches");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        for (from, to, expected) in [
            (
                director,
                director,
                projection(DeliveryStatus::NotApplicable, None, Some("self_send")),
            ),
            (
                worker,
                director,
                projection(DeliveryStatus::NotApplicable, None, Some("director_stdout")),
            ),
        ] {
            let message = broker::send_message(&mut conn, from, to, "branch").unwrap();
            assert_eq!(
                send_delivery(&conn, &message, None).unwrap(),
                (None, expected),
                "{from} -> {to}"
            );
        }
        let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()), None);
        let (output, pane_calls) = crate::cli::test_support::isolated_cli(
            &settings,
            &[
                "message",
                "send",
                "--from-member-id",
                &director.to_string(),
                "--to-member-id",
                &worker.to_string(),
                "committed run-less assignment",
                "--json",
            ],
        );
        assert_eq!(output.status.code(), Some(0));
        let stdout = String::from_utf8(output.stdout).unwrap();
        let payload: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("successful send JSON"),
        )
        .unwrap();
        let message_id = payload["message"]["message_id"].as_i64().unwrap();
        let message = broker::get_message(&conn, message_id).unwrap();
        assert_eq!(message.from_member_id, director);
        assert_eq!(message.to_member_id, worker);
        assert_eq!(message.text, "committed run-less assignment");
        assert_eq!(payload["message"], presentation::message(&message));
        assert_eq!(
            payload["delivery"],
            serde_json::json!({
                "status":"held", "method":null, "reason":"notification_failed",
            })
        );
        assert_eq!(payload["notification_sent"], false);
        assert_eq!(payload["input_id"], serde_json::Value::Null);
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "Message {message_id} was persisted. message {message_id} was not notified to member {worker}: member {worker} has no recorded run; the next send to member {worker} or the next message wait retries it.\n"
            ),
        );
        assert!(pane_calls.is_empty(), "{pane_calls}");
        assert_eq!(broker::poll_messages(&conn, worker).unwrap(), [message]);
        assert_eq!(
            conn.query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn message_wait_repeats_missing_run_causes_for_each_pending_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "run-less wait");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let messages = ["first", "second"]
            .map(|body| broker::send_message(&mut conn, director, worker, body).unwrap());
        let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()), None);
        let expected = messages.iter().map(|message| format!(
            "message {} was not notified to member {worker}: member {worker} has no recorded run; the next send to member {worker} or the next message wait retries it.\n",
            message.message_id,
        )).collect::<String>();
        for _ in 0..2 {
            let (output, pane_calls) = crate::cli::test_support::isolated_cli(
                &settings,
                &[
                    "message",
                    "wait",
                    &director.to_string(),
                    "--timeout",
                    "2",
                    "--json",
                ],
            );
            assert_eq!(output.status.code(), Some(0));
            assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
            assert!(pane_calls.is_empty(), "{pane_calls}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let payload: serde_json::Value = serde_json::from_str(
                stdout
                    .lines()
                    .find(|line| line.starts_with('{'))
                    .expect("wait JSON"),
            )
            .unwrap();
            assert_eq!(payload["member_id"], director);
            assert_eq!(payload["outcome"], "timeout");
            assert_eq!(payload["messages"], serde_json::json!([]));
            assert_eq!(
                broker::poll_messages(&conn, worker).unwrap(),
                messages.iter().rev().cloned().collect::<Vec<_>>()
            );
            assert_eq!(conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE to_member_id=?1 AND notified_at IS NOT NULL",
                [worker], |row| row.get::<_, i64>(0),
            ).unwrap(), 0);
        }
    }

    #[test]
    fn send_delivery_reports_a_wrapped_self_send_and_the_queued_preview_receipt() {
        let mut fixture = crate::runtime::runner::test_support::Fixture::new();
        let run = fixture.run.clone();
        let own =
            broker::send_message(&mut fixture.conn, run.member_id, run.member_id, "self").unwrap();
        assert_eq!(
            send_delivery(&fixture.conn, &own, None).unwrap(),
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
            send_delivery(&fixture.conn, &message, None).unwrap(),
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
        let (input_id, delivery) = send_delivery(&conn, &message, None)
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
