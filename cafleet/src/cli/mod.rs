pub(crate) mod creation;
mod doctor;
pub(crate) mod fleet;
pub(crate) mod helpers;
pub(crate) mod member;
mod message;
mod setup;

use clap::{Parser, Subcommand};

use crate::config::Settings;
use crate::error::CafleetError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "cafleet",
    version,
    about = "CAFleet — CLI for the message broker and member registry."
)]
struct CliArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Migrate storage and install embedded coding-agent skills/presets.
    Setup(setup::SetupArgs),
    /// Report independent environment, database and backend checks.
    Doctor(doctor::DoctorArgs),
    /// Fleet lifecycle.
    #[command(subcommand)]
    Fleet(fleet::FleetCommand),
    /// Member lifecycle and pane interaction.
    #[command(subcommand)]
    Member(member::MemberCommand),
    /// Durable point-to-point messages.
    #[command(subcommand)]
    Message(message::MessageCommand),
}

pub fn run() -> Result<(), CafleetError> {
    let args = CliArgs::parse();
    let settings = Settings::from_env()?;
    dispatch(&settings, args.command)
}

fn invocation_connection(database_url: &str) -> Result<rusqlite::Connection, CafleetError> {
    let path = crate::db::database_path(database_url)?;
    if !std::path::Path::new(path).try_exists().map_err(|cause| {
        CafleetError::App(format!("cannot inspect database at '{path}': {cause}"))
    })? {
        return Err(CafleetError::App(
            "no cafleet database; run 'cafleet setup'".into(),
        ));
    }
    let deadline = crate::input::Deadline::new();
    let validate = |conn: &mut rusqlite::Connection| {
        crate::input::with_database_budget(conn, deadline, |conn| {
            crate::diagnosis::classify_selected_schema(
                conn,
                crate::db::head_version(),
                database_url,
            )
            .require_current()
        })
    };
    let mut preflight = crate::db::connect_read_only(database_url)?;
    validate(&mut preflight)?;
    drop(preflight);
    let mut conn = crate::db::connect(database_url)?;
    validate(&mut conn)?;
    Ok(conn)
}

fn dispatch(settings: &Settings, command: Command) -> Result<(), CafleetError> {
    match command {
        Command::Setup(cmd) => setup::run(settings, cmd),
        Command::Doctor(cmd) => doctor::run(settings, cmd),
        mut command => {
            let mut conn = invocation_connection(&settings.database_url)?;
            if let Command::Member(command) = &mut command {
                member::prepare_input(command)?;
            }
            let deadline = crate::input::Deadline::new();
            match command {
                Command::Fleet(cmd) => fleet::run(&mut conn, settings, cmd, deadline),
                Command::Member(cmd) => member::run(&mut conn, settings, cmd, deadline),
                Command::Message(cmd) => message::run(&mut conn, settings, cmd),
                Command::Setup(_) | Command::Doctor(_) => unreachable!("handled above"),
            }
        }
    }
}

#[cfg(test)]
mod parser_contracts {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes() {
        for mutation in [
            "PRAGMA application_id=42",
            "UPDATE refinery_schema_history SET version=99 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)",
            "ALTER TABLE messages RENAME COLUMN text TO wrong",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("registry.db");
            let settings = Settings {
                database_url: format!("sqlite:///{}", path.display()),
                max_text_len: 200,
                multiplexer: None,
            };
            let mut conn = crate::db::connect(&settings.database_url).unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
            conn.execute_batch(mutation).unwrap();
            conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
                .unwrap();
            let before = ["", "-wal", "-shm"].map(|suffix| {
                let file = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
                let bytes = std::fs::read(&file).unwrap();
                (file, bytes)
            });
            for command in [
                Command::Fleet(fleet::FleetCommand::List { json: true }),
                Command::Member(member::MemberCommand::Show {
                    member_id: 1,
                    json: true,
                }),
                Command::Message(message::MessageCommand::Poll {
                    member_id: 1,
                    json: true,
                }),
            ] {
                assert!(dispatch(&settings, command).is_err(), "{mutation}");
                for (file, bytes) in &before {
                    assert_eq!(&std::fs::read(file).unwrap(), bytes, "{}", file.display());
                }
            }
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }

    #[test]
    fn ordinary_commands_require_storage_without_creating_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.db");
        assert!(invocation_connection(&format!("sqlite:///{}", path.display())).is_err());
        assert!(!path.exists());
    }

    fn parse(args: &[&str]) -> Command {
        CliArgs::try_parse_from(std::iter::once("cafleet").chain(args.iter().copied()))
            .unwrap_or_else(|error| panic!("{args:?}: {error}"))
            .command
    }

    fn assert_error(args: &[&str], kind: ErrorKind, input: &str) {
        let error =
            match CliArgs::try_parse_from(std::iter::once("cafleet").chain(args.iter().copied())) {
                Err(error) => error,
                Ok(_) => panic!("{args:?} must reject {input}"),
            };
        assert_eq!(error.kind(), kind, "{args:?}: {error}");
        assert!(error.to_string().contains(input), "{args:?}: {error}");
    }

    #[test]
    fn command_options_are_scoped_to_their_subcommands() {
        let cases: &[(&[&str], &str)] = &[
            (&["--json", "fleet", "list"], "--json"),
            (&["setup", "--fleet-id", "1"], "--fleet-id"),
            (
                &[
                    "fleet",
                    "create",
                    "--fleet-id",
                    "1",
                    "--name",
                    "x",
                    "--coding-agent",
                    "claude",
                ],
                "--fleet-id",
            ),
            (&["fleet", "list", "--fleet-id", "1"], "--fleet-id"),
            (&["fleet", "show", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "list", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "show", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["message", "poll", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["message", "ack", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "show", "--member-id", "1"], "--member-id"),
            (&["message", "poll", "--member-id", "1"], "--member-id"),
            (&["message", "ack", "--message-id", "1"], "--message-id"),
            (&["message", "show", "--message-id", "1"], "--message-id"),
            (
                &[
                    "fleet",
                    "create",
                    "--name",
                    "x",
                    "--coding-agent",
                    "claude",
                    "--full",
                ],
                "--full",
            ),
            (&["fleet", "show", "1", "--full"], "--full"),
            (&["member", "show", "1", "--full"], "--full"),
            (&["member", "list", "1", "--full"], "--full"),
            (&["message", "show", "1", "--full"], "--full"),
            (&["message", "poll", "1", "--full"], "--full"),
            (
                &[
                    "message",
                    "send",
                    "--from-member-id",
                    "1",
                    "--to-member-id",
                    "2",
                    "hi",
                    "--quiet",
                ],
                "--quiet",
            ),
            (&["message", "ack", "1", "--quiet"], "--quiet"),
            (&["member", "capture", "1", "--no-ansi"], "--no-ansi"),
            (
                &[
                    "message",
                    "send",
                    "--from-member-id",
                    "1",
                    "--to-member-id",
                    "2",
                    "--text",
                    "hi",
                ],
                "--text",
            ),
            (
                &[
                    "member",
                    "create",
                    "--fleet-id",
                    "1",
                    "--name",
                    "w",
                    "--description",
                    "d",
                    "--text",
                    "prompt",
                ],
                "--text",
            ),
        ];
        for (args, input) in cases {
            assert_error(args, ErrorKind::UnknownArgument, input);
        }
        assert!(matches!(
            parse(&["fleet", "list", "--json"]),
            Command::Fleet(fleet::FleetCommand::List { json: true })
        ));
        assert!(matches!(
            parse(&["fleet", "show", "41", "--json"]),
            Command::Fleet(fleet::FleetCommand::Show {
                fleet_id: 41,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["member", "show", "42", "--json"]),
            Command::Member(member::MemberCommand::Show {
                member_id: 42,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["message", "poll", "43", "--json"]),
            Command::Message(message::MessageCommand::Poll {
                member_id: 43,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["message", "ack", "44", "--json"]),
            Command::Message(message::MessageCommand::Ack {
                message_id: 44,
                json: true
            })
        ));
    }

    #[test]
    fn fleet_creation_requires_named_options_and_integer_subjects() {
        for (args, kind, input) in [
            (
                vec!["fleet", "create", "--coding-agent", "claude"],
                ErrorKind::MissingRequiredArgument,
                "--name",
            ),
            (
                vec!["fleet", "create", "--name", "alpha"],
                ErrorKind::MissingRequiredArgument,
                "--coding-agent",
            ),
            (
                vec![
                    "fleet",
                    "create",
                    "--name",
                    "alpha",
                    "--coding-agent",
                    "python",
                ],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                vec!["fleet", "show"],
                ErrorKind::MissingRequiredArgument,
                "FLEET_ID",
            ),
            (
                vec!["fleet", "show", "abc"],
                ErrorKind::ValueValidation,
                "abc",
            ),
        ] {
            assert_error(&args, kind, input);
        }
        let Command::Fleet(fleet::FleetCommand::Create {
            name,
            coding_agent,
            json,
        }) = parse(&[
            "fleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "codex",
            "--json",
        ])
        else {
            panic!("expected fleet creation");
        };
        assert_eq!(name, "alpha");
        assert_eq!(coding_agent, crate::coding_agent::CodingAgent::Codex);
        assert!(json);
    }

    #[test]
    fn member_and_message_body_sources_are_exclusive() {
        let member = [
            "member",
            "create",
            "--fleet-id",
            "17",
            "--name",
            "worker",
            "--description",
            "work",
        ];
        let send = [
            "message",
            "send",
            "--from-member-id",
            "17",
            "--to-member-id",
            "18",
        ];
        let exec = ["member", "exec", "17"];
        for base in [member.as_slice(), send.as_slice(), exec.as_slice()] {
            assert_error(base, ErrorKind::MissingRequiredArgument, "--file");
            let both: Vec<_> = base
                .iter()
                .copied()
                .chain(["payload", "--file", "body.txt"])
                .collect();
            assert_error(&both, ErrorKind::ArgumentConflict, "--file");
            for source in [
                vec!["payload"],
                vec!["--file", "body.txt"],
                vec!["--file", "-"],
            ] {
                let args: Vec<_> = base.iter().copied().chain(source).collect();
                match (base[0], parse(&args)) {
                    ("member", Command::Member(member::MemberCommand::Create { fleet_id, .. })) => {
                        assert_eq!(fleet_id, 17)
                    }
                    ("member", Command::Member(member::MemberCommand::Exec { member_id, .. })) => {
                        assert_eq!(member_id, 17)
                    }
                    (
                        "message",
                        Command::Message(message::MessageCommand::Send {
                            from_member_id,
                            to_member_id,
                            ..
                        }),
                    ) => assert_eq!((from_member_id, to_member_id), (17, 18)),
                    _ => panic!("{args:?} must preserve its command variant"),
                }
            }
        }
    }

    #[test]
    fn receive_leases_validate_integer_bounds_and_optional_listening() {
        assert!(matches!(
            parse(&["message", "wait", "17"]),
            Command::Message(message::MessageCommand::Wait {
                member_id: 17,
                timeout: 300,
                ..
            })
        ));
        assert!(matches!(
            parse(&["message", "listen", "17"]),
            Command::Message(message::MessageCommand::Listen {
                member_id: 17,
                timeout: None
            })
        ));
        for command in ["wait", "listen"] {
            for seconds in ["1", "1800"] {
                let parsed = parse(&["message", command, "17", "--timeout", seconds]);
                match parsed {
                    Command::Message(message::MessageCommand::Wait { timeout, .. }) => {
                        assert_eq!(timeout, seconds.parse::<u64>().unwrap());
                    }
                    Command::Message(message::MessageCommand::Listen { timeout, .. }) => {
                        assert_eq!(timeout, Some(seconds.parse::<u64>().unwrap()));
                    }
                    _ => panic!("expected finite receive"),
                }
            }
            for seconds in ["0", "1801", "fraction", "1.5"] {
                assert_error(
                    &["message", command, "17", "--timeout", seconds],
                    ErrorKind::ValueValidation,
                    seconds,
                );
            }
        }
        assert_error(
            &["message", "listen", "17", "--json"],
            ErrorKind::UnknownArgument,
            "--json",
        );
    }

    #[test]
    fn setup_rejects_unknown_options_positionals_and_agent_values() {
        let cases: &[(&[&str], ErrorKind, &str)] = &[
            (&["setup", "extra"], ErrorKind::UnknownArgument, "extra"),
            (&["setup", "claude"], ErrorKind::UnknownArgument, "claude"),
            (
                &["setup", "--skip", "claude"],
                ErrorKind::UnknownArgument,
                "--skip",
            ),
            (
                &["setup", "--coding-agent", "python"],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["setup", "--coding-agent", "claude", "python"],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["setup", "--coding-agent", "claude", "extra"],
                ErrorKind::InvalidValue,
                "extra",
            ),
        ];
        for (args, kind, input) in cases {
            assert_error(args, *kind, input);
        }
    }
}
