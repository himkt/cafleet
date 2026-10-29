mod doctor;
pub(crate) mod fleet;
pub(crate) mod helpers;
pub(crate) mod member;
mod message;
mod runtime;
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
    #[command(name = "__member-run", hide = true)]
    MemberRun {
        #[arg(long, value_name = "ABSOLUTE_PATH")]
        descriptor: std::path::PathBuf,
    },
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
    /// Read a durable runtime event and current state.
    #[command(subcommand)]
    Event(runtime::EventCommand),
    /// Read / settle a durable input receipt.
    #[command(subcommand)]
    Input(runtime::InputCommand),
    /// Inspect / answer the current native permission request.
    #[command(subcommand)]
    Permission(runtime::PermissionCommand),
}

pub fn run() -> Result<(), CafleetError> {
    let args = CliArgs::parse();
    let settings = Settings::from_env()?;
    dispatch(&settings, args.command)
}

fn invocation_connection(database_url: &str) -> Result<rusqlite::Connection, CafleetError> {
    invocation_connection_with_deadline(database_url, crate::input::Deadline::new())
}

fn invocation_connection_with_deadline(
    database_url: &str,
    deadline: crate::input::Deadline,
) -> Result<rusqlite::Connection, CafleetError> {
    let path = crate::db::database_path(database_url)?;
    if !std::path::Path::new(path).try_exists().map_err(|cause| {
        CafleetError::App(format!("cannot inspect database at '{path}': {cause}"))
    })? {
        return Err(CafleetError::App(
            "no cafleet database; run 'cafleet setup'".into(),
        ));
    }
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
        Command::MemberRun { descriptor } => {
            let code = crate::runtime::runner::run(settings, &descriptor)?;
            std::process::exit(code)
        }
        Command::Member(member::MemberCommand::Press {
            member_id,
            keys,
            json,
        }) => {
            let deadline = crate::input::Deadline::inherited_or_new();
            let keys = crate::multiplexer::PressKeys::parse(&keys)?;
            let mut conn = invocation_connection_with_deadline(&settings.database_url, deadline)
                .map_err(|cause| CafleetError::WithDiagnostics {
                    cause: Box::new(cause),
                    diagnostics: vec![format!("keys were not dispatched to member {member_id}.")],
                })?;
            member::press(&mut conn, settings, member_id, &keys, json, deadline)
        }
        Command::Permission(command) => {
            let deadline = crate::input::Deadline::new();
            runtime::permission_preflight(&command)
                .map_err(|source| runtime::permission_error(&command, source))?;
            let mut conn = invocation_connection_with_deadline(&settings.database_url, deadline)
                .map_err(|source| runtime::permission_error(&command, source))?;
            runtime::permission(&mut conn, settings, command, deadline)
        }
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
                Command::Event(cmd) => runtime::event(&conn, cmd),
                Command::Input(cmd) => runtime::input(&mut conn, settings, cmd, deadline),
                Command::Setup(_)
                | Command::Doctor(_)
                | Command::MemberRun { .. }
                | Command::Permission(_) => {
                    unreachable!("handled above")
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{CliArgs, Command};
    use clap::Parser;
    use clap::error::ErrorKind;

    pub(super) fn parse_cli(args: &[&str]) -> Result<Command, clap::Error> {
        CliArgs::try_parse_from(std::iter::once("cafleet").chain(args.iter().copied()))
            .map(|cli| cli.command)
    }

    pub(super) fn usage_error(args: &[&str]) -> clap::Error {
        match parse_cli(args) {
            Ok(_) => panic!("expected usage failure for {args:?}"),
            Err(error) => error,
        }
    }

    pub(super) fn assert_error(args: &[&str], kind: ErrorKind, input: &str) {
        let error = usage_error(args);
        assert_eq!(error.kind(), kind, "{args:?}: {error}");
        assert!(error.to_string().contains(input), "{args:?}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
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
            let settings = Settings::for_database(&path, None);
            let mut conn = crate::db::connect(&settings.database_url).unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
            conn.execute_batch(mutation).unwrap();
            conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
                .unwrap();
            let before = crate::db::test_support::wal_snapshot(&path);
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
        parse_cli(args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
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
    fn member_runner_internal_command_accepts_an_absolute_descriptor_and_stays_hidden() {
        use clap::CommandFactory;

        CliArgs::try_parse_from([
            "cafleet",
            "__member-run",
            "--descriptor",
            "/private/tmp/allocated-run/descriptor.json",
        ])
        .unwrap();
        let help = CliArgs::command().render_long_help().to_string();
        assert!(!help.contains("__member-run"));
    }

    fn flattened(text: impl ToString) -> String {
        text.to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn subcommand_help(path: &[&str]) -> String {
        use clap::CommandFactory;
        let mut command = CliArgs::command();
        let mut selected = &mut command;
        for name in path {
            selected = selected
                .find_subcommand_mut(name)
                .unwrap_or_else(|| panic!("registered subcommand {name}"));
        }
        flattened(selected.render_long_help())
    }

    fn possible_values(path: &[&str], argument: &str) -> Vec<String> {
        use clap::CommandFactory;
        let command = CliArgs::command();
        let mut selected = &command;
        for name in path {
            selected = selected.find_subcommand(name).unwrap();
        }
        selected
            .get_arguments()
            .find(|candidate| candidate.get_id() == argument)
            .unwrap_or_else(|| panic!("{path:?} declares {argument}"))
            .get_possible_values()
            .iter()
            .map(|value| value.get_name().to_owned())
            .collect()
    }

    #[test]
    fn help_describes_every_argument_of_the_runtime_read_input_press_and_recover_commands() {
        use clap::CommandFactory;
        let command = CliArgs::command();
        for path in [
            &["event", "show"][..],
            &["input", "show"],
            &["input", "reconcile"],
            &["member", "press"],
            &["member", "recover"],
        ] {
            let mut selected = &command;
            for name in path {
                selected = selected
                    .find_subcommand(name)
                    .unwrap_or_else(|| panic!("registered subcommand {name}"));
            }
            let mut described = 0;
            for argument in selected.get_arguments() {
                let id = argument.get_id().as_str();
                if ["help", "version"].contains(&id) {
                    continue;
                }
                let help = argument
                    .get_help()
                    .map(|help| help.to_string())
                    .unwrap_or_default();
                assert!(
                    !help.trim().is_empty(),
                    "{path:?} argument {id} has a help description"
                );
                described += 1;
            }
            assert!(described > 0, "{path:?} declares its arguments");
        }
    }

    #[test]
    fn member_exec_help_describes_wrapped_and_unwrapped_input_and_orders_member_before_command() {
        use clap::CommandFactory;
        let help = subcommand_help(&["member", "exec"]);
        let lower = help.to_lowercase();
        for term in ["unwrapped", "escape"] {
            assert!(
                lower.contains(term),
                "member exec help mentions {term}: {help}"
            );
        }
        assert!(
            lower
                .split("unwrapped")
                .next()
                .is_some_and(|before| before.contains("wrapped")),
            "member exec help describes wrapped members before the unwrapped exception: {help}"
        );
        let mut command = CliArgs::command();
        let usage = flattened(
            command
                .find_subcommand_mut("member")
                .unwrap()
                .find_subcommand_mut("exec")
                .unwrap()
                .render_usage(),
        );
        let member = usage.find("MEMBER_ID").expect("usage names MEMBER_ID");
        let body = usage.find("COMMAND").expect("usage names COMMAND");
        assert!(member < body, "MEMBER_ID precedes COMMAND: {usage}");
        match CliArgs::try_parse_from(["cafleet", "member", "exec", "17", "printf ok"])
            .unwrap()
            .command
        {
            crate::cli::Command::Member(crate::cli::member::MemberCommand::Exec {
                member_id,
                ..
            }) => {
                assert_eq!(member_id, 17)
            }
            _ => panic!("member exec parses MEMBER_ID then COMMAND"),
        }
    }

    #[test]
    fn help_describes_command_groups_and_derives_choices_from_their_registries() {
        use crate::coding_agent::CodingAgent;
        use clap::CommandFactory;
        let catalog = CodingAgent::all()
            .iter()
            .map(|agent| {
                let default = agent.resolve_policy(None, None).unwrap().resolved_mode;
                format!(
                    "{}: {} (default {default})",
                    agent.name(),
                    agent.policy_choices(None).join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let create = subcommand_help(&["member", "create"]);
        assert!(create.contains(&catalog), "{catalog} in {create}");
        let command = CliArgs::command();
        for group in ["event", "input", "permission"] {
            let about = command
                .find_subcommand(group)
                .unwrap_or_else(|| panic!("registered group {group}"))
                .get_about()
                .map(|about| about.to_string())
                .unwrap_or_default();
            assert!(
                !about.trim().is_empty(),
                "{group} has a top-level description"
            );
        }
        for (argument, values) in [
            ("decision", vec!["allow", "deny"]),
            ("decision_source", vec!["director-auto", "user"]),
            ("director_mode", vec!["auto", "manual", "unknown"]),
        ] {
            assert_eq!(
                possible_values(&["permission", "respond"], argument),
                values,
                "{argument}"
            );
            let respond = subcommand_help(&["permission", "respond"]);
            assert!(
                respond.contains(&format!("[possible values: {}]", values.join(", "))),
                "{argument} help lists {values:?}: {respond}"
            );
        }
        assert_eq!(
            possible_values(&["input", "reconcile"], "disposition"),
            ["submitted", "not-submitted", "abandoned-uncertain"]
        );
        assert_eq!(
            possible_values(&["member", "create"], "event_evidence"),
            ["metadata", "redacted"]
        );
        let invalid = usage_error(&[
            "permission",
            "respond",
            "1",
            "--event-id",
            "1",
            "--from-member-id",
            "1",
            "--expected-run-id",
            "1",
            "--expected-owner-epoch",
            "1",
            "--request-token",
            &"0".repeat(64),
            "--decision",
            "maybe",
            "--decision-source",
            "user",
            "--director-mode",
            "manual",
            "--mode-evidence-ref",
            "mode",
            "--decision-evidence-ref",
            "decision",
            "--operation-key",
            "key",
        ]);
        assert_eq!(invalid.exit_code(), 2);
        assert!(flattened(invalid.render()).contains("allow, deny"));
    }

    #[test]
    fn member_runner_internal_command_requires_its_descriptor_argument() {
        assert_eq!(usage_error(&["__member-run"]).exit_code(), 2);
        assert_eq!(
            usage_error(&["__member-run", "--descriptor"]).exit_code(),
            2
        );
    }

    #[test]
    fn member_runner_dispatch_rejects_an_unallocated_descriptor_before_runtime_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let descriptor = dir.path().join("unallocated.json");
        std::fs::write(&descriptor, b"{\"schema_version\":1}").unwrap();
        let settings = crate::config::Settings::for_database(&path, None);
        let mut conn = crate::db::connect(&settings.database_url).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        let args = CliArgs::try_parse_from([
            "cafleet",
            "__member-run",
            "--descriptor",
            descriptor.to_str().unwrap(),
        ])
        .unwrap();
        assert!(super::dispatch(&settings, args.command).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
