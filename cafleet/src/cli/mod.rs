//! The `cafleet` command tree (SPEC §6.3, §10): clap parsing, the shared
//! option surface, the schema-version and stale-assets guard prologues, and
//! the per-group handlers. Orchestration glue only — it wires broker /
//! multiplexer / output / coding-agent.

pub(crate) mod creation;
mod doctor;
pub(crate) mod fleet;
pub(crate) mod helpers;
pub(crate) mod member;
mod message;
pub(crate) mod monitor;
mod server;
mod setup;

use clap::{Parser, Subcommand};
use rusqlite::Connection;

use crate::config::Settings;
use crate::diagnosis::{self, AssetMode};
use crate::error::CafleetError;

pub(crate) struct InvocationHooks<'a> {
    pub(crate) connect: &'a dyn Fn(&str) -> Result<Connection, CafleetError>,
    pub(crate) asset_env: crate::config_dir::EnvLookup<'a>,
}

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
    /// Migrate the database schema and install the coding-agent assets
    /// (skills and presets).
    Setup(setup::SetupArgs),
    /// Print the three-section environment diagnosis (multiplexer, database,
    /// coding agents).
    Doctor(doctor::DoctorArgs),
    /// Start the admin WebUI server.
    Server(server::ServerArgs),
    /// Fleet lifecycle.
    #[command(subcommand)]
    Fleet(fleet::FleetCommand),
    /// Member lifecycle and pane interaction.
    #[command(subcommand)]
    Member(member::MemberCommand),
    /// Message broker.
    #[command(subcommand)]
    Message(message::MessageCommand),
    /// Run the per-fleet scheduler loop in-process.
    Monitor(monitor::MonitorArgs),
}

/// Parse the argv and run the selected command. clap's own parse errors exit
/// 2 and `--help` / `--version` exit 0 before this returns.
pub fn run() -> Result<(), CafleetError> {
    let args = CliArgs::parse();
    let settings = Settings::from_env()?;
    dispatch(
        &settings,
        args,
        &InvocationHooks {
            connect: &crate::db::connect,
            asset_env: &|name| std::env::var(name).ok(),
        },
    )
}

#[cfg(test)]
pub(crate) fn run_with_hooks(
    settings: &Settings,
    argv: &[&str],
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    let args = CliArgs::try_parse_from(argv).map_err(|e| CafleetError::Usage(e.to_string()))?;
    dispatch(settings, args, hooks)
}

fn dispatch(
    settings: &Settings,
    args: CliArgs,
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    match args.command {
        Command::Setup(cmd) => setup::run(settings, cmd, hooks),
        Command::Doctor(cmd) => doctor::run(settings, cmd, hooks),
        command => {
            let conn = (hooks.connect)(&settings.database_url)?;
            let schema = diagnosis::classify_schema(&conn, crate::db::head_version());
            helpers::schema_guard(&schema)?;
            if !matches!(command, Command::Server(_)) {
                let home = std::path::PathBuf::from(
                    std::env::var("HOME")
                        .map_err(|_| CafleetError::App("HOME is not set".into()))?,
                );
                let assets = diagnosis::diagnose_assets(
                    Some(&conn),
                    hooks.asset_env,
                    &home,
                    VERSION,
                    AssetMode::Guard,
                );
                helpers::stale_assets_guard(&assets?, VERSION)?;
            }
            let mut slot = Some(conn);
            match command {
                Command::Fleet(cmd) => fleet::run(&mut slot, settings, cmd),
                Command::Member(cmd) => member::run(
                    slot.as_mut().expect("open invocation connection"),
                    settings,
                    cmd,
                ),
                Command::Message(cmd) => message::run(
                    slot.as_mut().expect("open invocation connection"),
                    settings,
                    cmd,
                ),
                Command::Monitor(cmd) => monitor::run(
                    slot.as_mut().expect("open invocation connection"),
                    settings,
                    cmd,
                ),
                Command::Server(cmd) => server::run(settings, cmd),
                Command::Setup(_) | Command::Doctor(_) => unreachable!("handled above"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{self, test_support as common};
    use std::cell::Cell;

    #[test]
    fn ack_reuses_the_guard_connection() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::from_lookup(|_| None).unwrap();
        let opens = Cell::new(0);
        let connect = |_: &str| {
            opens.set(opens.get() + 1);
            let mut conn = Connection::open_in_memory().unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            let (_, director) = common::create_fleet(&mut conn, "invocation");
            broker::record_asset_install(
                &mut conn,
                "claude",
                dir.path().to_str().unwrap(),
                VERSION,
            )
            .unwrap();
            let message = broker::send_message(
                &mut conn,
                &common::FakeNotifier::succeeding(),
                200,
                director,
                &director.to_string(),
                "ack",
            )
            .unwrap();
            assert_eq!(message.message.message_id, 1);
            Ok(conn)
        };
        run_with_hooks(
            &settings,
            &["cafleet", "message", "ack", "1", "--json"],
            &InvocationHooks {
                connect: &connect,
                asset_env: &|_| Some(dir.path().display().to_string()),
            },
        )
        .unwrap();
        assert_eq!(opens.get(), 1);
    }

    #[test]
    fn setup_reuses_open_connections_and_retries_failed_opens() {
        for initial in ["missing", "head", "ahead", "open failure"] {
            let dir = tempfile::tempdir().unwrap();
            let url = format!("sqlite:///{}", dir.path().join("database.db").display());
            let settings =
                Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone()))
                    .unwrap();
            let opens = Cell::new(0);
            let connect = |url: &str| {
                opens.set(opens.get() + 1);
                if initial == "open failure" && opens.get() == 1 {
                    return Err(CafleetError::App("first open failed".into()));
                }
                let mut conn = crate::db::connect(url).unwrap();
                if initial != "missing" {
                    crate::db::migrate_to_head(&mut conn).unwrap();
                }
                if initial == "ahead" {
                    conn.execute(
                        "UPDATE refinery_schema_history SET version=version+1 WHERE version=?1",
                        [crate::db::head_version()],
                    )
                    .unwrap();
                }
                Ok(conn)
            };
            let result = run_with_hooks(
                &settings,
                &["cafleet", "setup", "--coding-agent", "claude"],
                &InvocationHooks {
                    connect: &connect,
                    asset_env: &|name| {
                        assert_eq!(name, "CLAUDE_CONFIG_DIR");
                        Some(dir.path().display().to_string())
                    },
                },
            );
            assert_eq!(
                result.is_ok(),
                matches!(initial, "missing" | "head"),
                "{initial}"
            );
            assert_eq!(opens.get(), if initial == "open failure" { 2 } else { 1 });
            let conn = crate::db::connect(&url).unwrap();
            let rows = broker::list_asset_installs(&conn).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["coding_agent"], "claude");
            assert_eq!(rows[0]["cafleet_version"], VERSION);
            assert!(dir.path().join("skills/cafleet/SKILL.md").is_file());
        }
    }
}

#[cfg(test)]
mod parser_contracts {
    use super::*;
    use clap::error::ErrorKind;

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
            (&["monitor", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "show", "--member-id", "1"], "--member-id"),
            (&["member", "ping", "--member-id", "1"], "--member-id"),
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
                    "broadcast",
                    "--from-member-id",
                    "1",
                    "hi",
                    "--full",
                ],
                "--full",
            ),
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
            (&["member", "ping", "1", "--quiet"], "--quiet"),
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
                    "message",
                    "broadcast",
                    "--from-member-id",
                    "1",
                    "--text-file",
                    "f.txt",
                ],
                "--text-file",
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
        let cases: &[(&[&str], ErrorKind, &str)] = &[
            (
                &[
                    "fleet",
                    "create",
                    "--coding-agent",
                    "claude",
                    "--monitor-file",
                    "prompt.md",
                ],
                ErrorKind::MissingRequiredArgument,
                "--name",
            ),
            (
                &[
                    "fleet",
                    "create",
                    "--name",
                    "x",
                    "--monitor-file",
                    "prompt.md",
                ],
                ErrorKind::MissingRequiredArgument,
                "--coding-agent",
            ),
            (
                &["fleet", "create", "--name", "x", "--coding-agent", "claude"],
                ErrorKind::MissingRequiredArgument,
                "--monitor-file",
            ),
            (
                &[
                    "fleet",
                    "create",
                    "--name",
                    "x",
                    "--coding-agent",
                    "python",
                    "--monitor-file",
                    "prompt.md",
                ],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["fleet", "show"],
                ErrorKind::MissingRequiredArgument,
                "FLEET_ID",
            ),
            (&["fleet", "show", "abc"], ErrorKind::ValueValidation, "abc"),
        ];
        for (args, kind, input) in cases {
            assert_error(args, *kind, input);
        }
        let Command::Fleet(fleet::FleetCommand::Create {
            name,
            coding_agent,
            monitor_file,
            monitor_model,
            json,
        }) = parse(&[
            "fleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "codex",
            "--monitor-file",
            "-",
            "--monitor-model",
            "chosen",
            "--json",
        ])
        else {
            panic!("expected fleet creation")
        };
        assert_eq!(name, "alpha");
        assert_eq!(coding_agent, "codex");
        assert_eq!(monitor_file, "-");
        assert_eq!(monitor_model.as_deref(), Some("chosen"));
        assert!(json);
    }

    #[test]
    fn body_sources_are_exclusive_and_monitor_is_the_creation_role() {
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
        let broadcast = ["message", "broadcast", "--from-member-id", "17"];
        for base in [member.as_slice(), send.as_slice(), broadcast.as_slice()] {
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
                    (
                        "member",
                        Command::Member(member::MemberCommand::Create { fleet_id, role, .. }),
                    ) => {
                        assert_eq!(fleet_id, 17);
                        assert_eq!(role, None);
                    }
                    (
                        "message",
                        Command::Message(message::MessageCommand::Send {
                            from_member_id,
                            to_member_id,
                            ..
                        }),
                    ) => {
                        assert_eq!((from_member_id, to_member_id), (17, 18));
                    }
                    (
                        "message",
                        Command::Message(message::MessageCommand::Broadcast {
                            from_member_id, ..
                        }),
                    ) => assert_eq!(from_member_id, 17),
                    _ => panic!("{args:?} must preserve its command variant"),
                }
            }
        }
        let invalid: Vec<_> = member
            .iter()
            .copied()
            .chain(["--role", "builder", "prompt"])
            .collect();
        assert_error(&invalid, ErrorKind::InvalidValue, "builder");
        let valid: Vec<_> = member
            .iter()
            .copied()
            .chain(["--role", "monitor", "prompt"])
            .collect();
        let Command::Member(member::MemberCommand::Create { role, .. }) = parse(&valid) else {
            panic!("expected member creation");
        };
        assert_eq!(role.as_deref(), Some("monitor"));
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
