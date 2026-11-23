use crate::broker;
use crate::cli::emit;
use crate::cli::resolve_body;
use crate::error::CafleetError;
use crate::output::format_message;
use crate::output::format_message_sent;
use crate::output::format_messages;
use clap::Args;
use clap::Subcommand;
use rusqlite::Connection;
use std::borrow::Cow;

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
    /// Fetch and consume incoming messages.
    Poll {
        /// Recipient whose inbox is fetched.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
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

pub(crate) fn run(conn: &mut Connection, command: MessageCommand) -> Result<(), CafleetError> {
    match command {
        MessageCommand::Send {
            from_member_id,
            to_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref())?;
            let deadline = crate::time::Deadline::new();
            let message = crate::db::with_database_budget(conn, deadline, |conn| {
                broker::send_message(conn, from_member_id, to_member_id, &text)
            })?;
            emit(
                json,
                &crate::output::json::message_envelope(&message),
                || format_message_sent(&message),
            )
        }
        MessageCommand::Poll { member_id, json } => {
            let messages = broker::poll_messages(conn, member_id)?;
            let payload = serde_json::Value::Array(
                messages.iter().map(crate::output::json::message).collect(),
            );
            emit(json, &payload, || format_messages(&messages))
        }
        MessageCommand::Show { message_id, json } => {
            let message = broker::get_message(conn, message_id)?;
            emit(
                json,
                &crate::output::json::message_envelope(&message),
                || format_message(&message),
            )
        }
    }
}

/// The member's consuming poll, carrying the database URL as one shell word when given.
pub(crate) fn poll_command(member_id: i64, database_url: Option<&str>) -> String {
    let command = format!("cafleet message poll {member_id} --json");
    match database_url {
        Some(url) => format!("{command} --database-url {}", shell_word(url)),
        None => command,
    }
}

fn shell_word(value: &str) -> Cow<'_, str> {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./:~@%+=,-".contains(c);
    if value.chars().all(safe) {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(format!("'{}'", value.replace('\'', r"'\''")))
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::message::*;

    mod runner_pull {
        use crate::runtime::service::test_support::input_service::*;

        #[test]
        fn message_send_stores_the_message_and_writes_no_byte_to_the_recipients_child() {
            with_input_service(|fixture, _| {
                let run = fixture.fixture.run.clone();
                let url = format!("sqlite:///{}", fixture.fixture.conn.path().unwrap());
                let (from, to) = (
                    run.director_member_id.to_string(),
                    run.member_id.to_string(),
                );
                crate::cli::run_args(
                    [
                        "cafleet",
                        "--database-url",
                        &url,
                        "message",
                        "send",
                        "--from-member-id",
                        &from,
                        "--to-member-id",
                        &to,
                        "stored for the runner",
                    ]
                    .map(std::ffi::OsString::from),
                )
                .unwrap();
                assert!(input_service_records(fixture).is_empty());
                let (status, notified_at): (String, Option<String>) = fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT status_state,notified_at FROM messages WHERE to_member_id=?1 AND text='stored for the runner'",
                        [run.member_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(status, "input_required");
                assert_eq!(notified_at, None);
            });
        }
    }

    #[test]
    fn message_subcommands_outside_the_registry_are_unrecognized() {
        for args in [
            ["message", "ack", "44"],
            ["message", "wait", "17"],
            ["message", "listen", "1"],
        ] {
            let error = crate::cli::test_support::usage_error(&args);
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::InvalidSubcommand,
                "{args:?}: {error}"
            );
            assert!(error.to_string().contains(args[1]), "{args:?}: {error}");
        }
    }

    mod next_command {
        use crate::cli::message::tests::*;

        #[test]
        fn the_next_poll_command_appends_the_database_url_quoted_only_when_needed() {
            let every_safe_class = "sqlite:///Home/u_1/~x/a-b.c@d%e+f=g,h.db";
            for (name, member, url, expected) in [
                (
                    "omitted database URL",
                    7,
                    None,
                    "cafleet message poll 7 --json".to_owned(),
                ),
                (
                    "only safe characters",
                    7,
                    Some("sqlite:///tmp/fleet.db"),
                    "cafleet message poll 7 --json --database-url sqlite:///tmp/fleet.db"
                        .to_owned(),
                ),
                (
                    "every safe character class",
                    42,
                    Some(every_safe_class),
                    format!("cafleet message poll 42 --json --database-url {every_safe_class}"),
                ),
                (
                    "a space",
                    7,
                    Some("sqlite:///tmp/my fleet.db"),
                    "cafleet message poll 7 --json --database-url 'sqlite:///tmp/my fleet.db'"
                        .to_owned(),
                ),
                (
                    "a single quote closed, escaped and reopened",
                    7,
                    Some("sqlite:///tmp/it's.db"),
                    r"cafleet message poll 7 --json --database-url 'sqlite:///tmp/it'\''s.db'"
                        .to_owned(),
                ),
            ] {
                assert_eq!(poll_command(member, url), expected, "{name}");
            }
        }

        #[test]
        fn every_character_outside_the_safe_set_quotes_the_whole_url() {
            for unsafe_char in ['$', ';', '*', '"', '\\', '&', '|', '`', '#', '!', '\t', 'é'] {
                let url = format!("sqlite:///tmp/a{unsafe_char}b.db");
                assert_eq!(
                    poll_command(3, Some(&url)),
                    format!("cafleet message poll 3 --json --database-url '{url}'"),
                    "{unsafe_char:?}"
                );
            }
        }
    }
}
