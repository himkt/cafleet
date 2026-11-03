pub(crate) use crate::cli::member::MemberCapture;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::AnyMultiplexer;
use crate::output::format_json;
use crate::runtime::resolve_mux;
use crate::runtime::system::read_stdin;
use clap::Parser;
use clap::Subcommand;
use serde_json::Value;
use std::ffi::OsString;

mod fleet;
mod member;
mod message;
mod runtime;
mod setup;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    /// Initialize the version's database and install embedded coding-agent skills/presets.
    Setup(setup::SetupArgs),
    /// Report independent environment, database and backend checks.
    Doctor(setup::DoctorArgs),
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
    run_args(std::env::args_os())
}

fn run_args(args: impl IntoIterator<Item = OsString>) -> Result<(), CafleetError> {
    let args = CliArgs::parse_from(args);
    refuse_director_only(&args.command, std::env::var_os("CAFLEET_MEMBER_ID"))?;
    let settings = Settings::from_env()?;
    dispatch(&settings, args.command)
}

fn director_only_name(command: &Command) -> Option<&'static str> {
    match command {
        Command::Member(member::MemberCommand::Prompt { .. }) => Some("member prompt"),
        Command::Member(member::MemberCommand::Exec { .. }) => Some("member exec"),
        Command::Member(member::MemberCommand::Press { .. }) => Some("member press"),
        Command::Member(member::MemberCommand::Recover { .. }) => Some("member recover"),
        Command::Permission(runtime::PermissionCommand::Respond { .. }) => {
            Some("permission respond")
        }
        Command::Input(runtime::InputCommand::Reconcile { .. }) => Some("input reconcile"),
        _ => None,
    }
}

fn refuse_director_only(
    command: &Command,
    member_id: Option<OsString>,
) -> Result<(), CafleetError> {
    match (director_only_name(command), member_id) {
        (Some(name), Some(member_id)) => Err(CafleetError::App(format!(
            "'cafleet {name}' is a Director-only command and this process runs inside CAFleet member {} (CAFLEET_MEMBER_ID is set); send the request to your Director.",
            member_id.to_string_lossy()
        ))),
        _ => Ok(()),
    }
}

fn invocation_connection(
    database_url: &str,
    deadline: crate::time::Deadline,
) -> Result<rusqlite::Connection, CafleetError> {
    let missing_database = || CafleetError::App("no cafleet database; run 'cafleet setup'".into());
    let validate = |conn: &mut rusqlite::Connection| {
        crate::db::with_database_budget(conn, deadline, |conn| {
            crate::db::classify_selected_schema(conn, database_url).require_current()
        })
    };
    let mut preflight =
        crate::db::open_existing(database_url, true)?.ok_or_else(missing_database)?;
    validate(&mut preflight)?;
    drop(preflight);
    let mut conn = crate::db::open_existing(database_url, false)?.ok_or_else(missing_database)?;
    validate(&mut conn)?;
    Ok(conn)
}

fn dispatch(settings: &Settings, command: Command) -> Result<(), CafleetError> {
    match command {
        Command::Setup(cmd) => setup::run(settings, cmd),
        Command::Doctor(cmd) => setup::doctor(settings, cmd),
        Command::MemberRun { descriptor } => {
            let code = crate::runtime::run(settings, &descriptor)?;
            std::process::exit(code)
        }
        Command::Member(member::MemberCommand::Press {
            member_id,
            keys,
            json,
        }) => {
            let deadline = crate::time::Deadline::inherited_or_new();
            let keys = crate::multiplexer::PressKeys::parse(&keys)?;
            let mut conn =
                invocation_connection(&settings.database_url, deadline).map_err(|cause| {
                    CafleetError::WithDiagnostics {
                        cause: Box::new(cause),
                        diagnostics: vec![format!(
                            "keys were not dispatched to member {member_id}."
                        )],
                    }
                })?;
            member::press(&mut conn, settings, member_id, &keys, json, deadline)
        }
        Command::Permission(command) => {
            let deadline = crate::time::Deadline::new();
            runtime::permission_preflight(&command)
                .map_err(|source| runtime::permission_error(&command, source))?;
            let mut conn = invocation_connection(&settings.database_url, deadline)
                .map_err(|source| runtime::permission_error(&command, source))?;
            runtime::permission(&mut conn, settings, command, deadline)
        }
        mut command => {
            let mut conn =
                invocation_connection(&settings.database_url, crate::time::Deadline::new())?;
            if let Command::Member(command) = &mut command {
                member::prepare_input(command)?;
            }
            let deadline = crate::time::Deadline::new();
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

pub(crate) fn available_mux(
    settings: &Settings,
    deadline: Option<crate::time::Deadline>,
) -> Result<AnyMultiplexer, CafleetError> {
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    let mux = match deadline {
        Some(deadline) => mux.bounded(deadline),
        None => mux,
    };
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    Ok(mux)
}

pub(crate) fn checked(
    validate: fn(&str) -> Result<(), CafleetError>,
) -> impl Fn(&str) -> Result<String, String> + Clone + Send + Sync + 'static {
    move |value| {
        validate(value).map_err(|cause| cause.to_string())?;
        Ok(value.into())
    }
}

pub(crate) fn nonblank(
    message: &'static str,
) -> impl Fn(&str) -> Result<String, String> + Clone + Send + Sync + 'static {
    move |value| {
        if value.trim().is_empty() {
            Err(message.into())
        } else {
            Ok(value.into())
        }
    }
}

fn is_blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

pub(crate) fn resolve_body(
    inline: Option<&str>,
    file: Option<&str>,
    flag: &str,
) -> Result<String, CafleetError> {
    let body = read_body(inline, file, flag)?;
    if !is_blank(&body) {
        return Ok(body);
    }
    Err(match file {
        None => CafleetError::Usage("text may not be empty.".to_string()),
        Some("-") => CafleetError::App(format!("{flag} -: stdin is empty.")),
        Some(path) => CafleetError::App(format!("{flag} {path}: file is empty.")),
    })
}

pub(crate) fn resolve_command_body(
    inline: Option<&str>,
    file: Option<&str>,
) -> Result<String, CafleetError> {
    let mut body = read_body(inline, file, "--file")?;
    if file.is_some() {
        if body.ends_with("\r\n") {
            body.truncate(body.len() - 2);
        } else if body.ends_with('\n') {
            body.pop();
        }
    }
    if !crate::pane_lock::valid_input(&body) {
        return Err(CafleetError::Usage(
            "command must be one non-empty line without control characters.".into(),
        ));
    }
    Ok(body)
}

fn read_body(inline: Option<&str>, file: Option<&str>, flag: &str) -> Result<String, CafleetError> {
    match (inline, file) {
        (Some(text), None) => Ok(text.to_string()),
        (None, Some("-")) => {
            let bytes = read_stdin().map_err(|e| CafleetError::App(format!("{flag} -: {e}")))?;
            String::from_utf8(bytes)
                .map_err(|_| CafleetError::App(format!("{flag} -: file is not valid UTF-8.")))
        }
        (None, Some(path)) => {
            let bytes = std::fs::read(path).map_err(|e| {
                let message = match e.kind() {
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory => {
                        format!("{flag} {path}: file does not exist or is not a regular file.")
                    }
                    _ => format!("{flag} {path}: file is not readable."),
                };
                CafleetError::App(format!("{message} ({e})"))
            })?;
            String::from_utf8(bytes)
                .map_err(|_| CafleetError::App(format!("{flag} {path}: file is not valid UTF-8.")))
        }
        (Some(_), Some(_)) | (None, None) => {
            unreachable!("clap's required argument group supplies exactly one body source")
        }
    }
}

pub(crate) fn emit(json: bool, payload: &Value, text: impl FnOnce() -> String) {
    if json {
        println!("{}", format_json(payload));
    } else {
        println!("{}", text());
    }
}

#[cfg(test)]
pub(crate) use crate::cli::fleet::test_support::guarded_delete_fleet;
#[cfg(test)]
pub(crate) use crate::cli::member::substitute_spawn_placeholders;

#[cfg(test)]
pub(crate) mod test_support {
    use crate::cli::CliArgs;
    use crate::cli::Command;
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

    pub(super) fn isolated_cli(
        settings: &crate::config::Settings,
        args: &[&str],
    ) -> (std::process::Output, String) {
        isolated_cli_with(
            settings,
            args,
            &[(
                "tmux",
                "printf '%s\\n' \"$*\" >> \"$CAFLEET_TEST_PANE_LOG\"",
            )],
            &[],
        )
    }

    /// Runs the CLI with only `executables` (name, shell body) on PATH and `environment` added.
    pub(super) fn isolated_cli_with(
        settings: &crate::config::Settings,
        args: &[&str],
        executables: &[(&str, &str)],
        environment: &[(&str, &str)],
    ) -> (std::process::Output, String) {
        use crate::runtime::system::test_support::TestChild;
        use std::process::Command;
        use std::process::Stdio;
        use std::time::Duration;
        use std::time::Instant;

        let dir = tempfile::tempdir().unwrap();
        let pane_log = dir.path().join("pane.log");
        for (name, body) in executables {
            crate::coding_agent::test_support::executable_stub(dir.path(), name, body);
        }
        let stdout_path = dir.path().join("stdout");
        let stderr_path = dir.path().join("stderr");
        let started = Instant::now();
        let mut child = TestChild::new(
            Command::new(std::env::current_exe().unwrap())
                .envs(environment.iter().copied())
                .args([
                    "--exact",
                    crate::cli::tests::isolated_cli_child_path(),
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env(
                    "CAFLEET_TEST_ISOLATED_CLI",
                    serde_json::json!({"database_url":settings.database_url,"args":args})
                        .to_string(),
                )
                .env("CAFLEET_TEST_PANE_LOG", &pane_log)
                .env("PATH", dir.path())
                .env("TMUX", "cafleet-test,1,0")
                .env_remove("HERDR_ENV")
                .env("RUST_BACKTRACE", "0")
                .stdin(Stdio::null())
                .stdout(Stdio::from(std::fs::File::create(&stdout_path).unwrap()))
                .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()))
                .spawn()
                .unwrap(),
        );
        let status = child.poll_until(started + Duration::from_secs(10)).unwrap();
        if status.is_none() {
            child.terminate_until(started + Duration::from_secs(12));
        }
        let output = std::process::Output {
            status: status.unwrap_or_else(|| {
                panic!(
                    "isolated CLI exceeded its deadline: {:?}",
                    std::fs::read_to_string(&stderr_path).unwrap()
                )
            }),
            stdout: std::fs::read(stdout_path).unwrap(),
            stderr: std::fs::read(stderr_path).unwrap(),
        };
        let pane_calls = if pane_log.exists() {
            std::fs::read_to_string(pane_log).unwrap()
        } else {
            String::new()
        };
        (output, pane_calls)
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::test_support::*;
    use crate::cli::*;
    use clap::error::ErrorKind;

    pub(super) fn isolated_cli_child_path() -> &'static str {
        test_path!(isolated_cli_child)
    }

    #[test]
    #[ignore = "owned CLI subprocess entry"]
    fn isolated_cli_child() {
        let config: serde_json::Value = serde_json::from_str(
            &std::env::var("CAFLEET_TEST_ISOLATED_CLI")
                .expect("explicit isolated CLI configuration"),
        )
        .unwrap();
        let database = crate::db::database_path(config["database_url"].as_str().unwrap()).unwrap();
        let settings =
            crate::config::Settings::for_database(std::path::Path::new(database), Some("tmux"));
        let args = config["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect::<Vec<_>>();
        let command = parse_cli(&args).unwrap();
        match dispatch(&settings, command) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("Error: {error}");
                std::process::exit(error.exit_code());
            }
        }
    }

    #[test]
    fn ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes() {
        for mutation in [
            "PRAGMA application_id=42",
            "UPDATE refinery_schema_history SET version=99 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)",
            "ALTER TABLE messages RENAME COLUMN text TO wrong",
            "ALTER TABLE member_runtime_events RENAME COLUMN notification_state TO broken",
            "ALTER TABLE runtime_database_identity RENAME COLUMN instance_id TO broken",
            "DROP TABLE member_input_operations",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("registry.db");
            let settings = Settings::for_database(&path, None);
            let mut conn = crate::db::connect(&settings.database_url).unwrap();
            crate::db::initialize(&mut conn).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
            conn.execute_batch(mutation).unwrap();
            conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
                .unwrap();
            let before = crate::db::test_support::wal_snapshot(&path);
            for args in [
                &["fleet", "list", "--json"][..],
                &["member", "show", "1", "--json"],
                &["message", "poll", "1", "--json"],
                &["member", "recover", "1", "--json"],
                &["event", "show", "1", "--json"],
                &["input", "show", "1", "--json"],
            ] {
                assert!(
                    dispatch(&settings, parse(args)).is_err(),
                    "{mutation}: {args:?}"
                );
                for (file, bytes) in &before {
                    assert_eq!(
                        &std::fs::read(file).unwrap(),
                        bytes,
                        "{mutation}: {args:?}: {}",
                        file.display()
                    );
                }
            }
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }

    #[test]
    fn ordinary_commands_require_storage_without_creating_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.db");
        assert!(
            invocation_connection(
                &format!("sqlite:///{}", path.display()),
                crate::time::Deadline::new()
            )
            .is_err()
        );
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
                    (
                        "member",
                        Command::Member(member::MemberCommand::Create(member::CreateArgs {
                            fleet_id,
                            name,
                            description,
                            ..
                        })),
                    ) => assert_eq!(
                        (fleet_id, name.as_str(), description.as_str()),
                        (17, "worker", "work")
                    ),
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
    fn member_exec_help_describes_runner_delivery_and_orders_member_before_command() {
        use clap::CommandFactory;
        let help = subcommand_help(&["member", "exec"]);
        let lower = help.to_lowercase();
        assert!(
            lower.contains("runner"),
            "member exec help describes runner delivery: {help}"
        );
        assert!(
            lower.contains("fallback") || lower.contains("falls back"),
            "member exec help describes delivery fallback: {help}"
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
        crate::db::initialize(&mut conn).unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        let args = CliArgs::try_parse_from([
            "cafleet",
            "__member-run",
            "--descriptor",
            descriptor.to_str().unwrap(),
        ])
        .unwrap();
        assert!(crate::cli::dispatch(&settings, args.command).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn body_files_preserve_exact_bytes_and_report_invalid_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.txt");
        let name = path.to_str().unwrap();
        for text in ["界🙂\nnext\r\n", "{literal}\ttext", " padded "] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(resolve_body(None, Some(name), "--file").unwrap(), text);
        }
        for text in ["", " \t\n", "\u{1c}\u{1f}", "\u{2003}"] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                resolve_body(Some(text), None, "--file")
                    .unwrap_err()
                    .exit_code(),
                2
            );
            assert_eq!(
                resolve_body(None, Some(name), "--file")
                    .unwrap_err()
                    .exit_code(),
                1
            );
        }
        std::fs::write(&path, [0xff]).unwrap();
        assert!(
            resolve_body(None, Some(name), "--file")
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(
            resolve_body(None, Some(name), "--file")
                .unwrap_err()
                .to_string()
                .contains(name)
        );
        assert!(resolve_body(None, Some(dir.path().to_str().unwrap()), "--file").is_err());
    }

    #[test]
    fn native_command_files_strip_only_one_terminal_line_ending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("command.txt");
        let name = path.to_str().unwrap();
        for (source, expected) in [
            ("printf '界🙂'", "printf '界🙂'"),
            ("echo literal\n", "echo literal"),
            ("echo literal\r\n", "echo literal"),
            ("  echo literal  \n", "  echo literal  "),
        ] {
            std::fs::write(&path, source).unwrap();
            assert_eq!(resolve_command_body(None, Some(name)).unwrap(), expected);
        }
        for source in [
            "",
            "  ",
            "echo x\n\n",
            "echo x\r",
            "echo\tx",
            "one\ntwo",
            "\0",
            "\u{7f}",
        ] {
            std::fs::write(&path, source).unwrap();
            assert!(
                resolve_command_body(None, Some(name)).is_err(),
                "{source:?}"
            );
        }
        assert_eq!(
            resolve_command_body(Some("echo literal"), None).unwrap(),
            "echo literal"
        );
        assert!(resolve_command_body(Some("echo literal\n"), None).is_err());
    }

    mod director_only_guard {
        use crate::cli::test_support::parse_cli;
        use crate::cli::*;
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        const REQUEST_TOKEN: &str =
            "0000000000000000000000000000000000000000000000000000000000000000";

        fn director_only_commands() -> Vec<(&'static str, Vec<&'static str>)> {
            vec![
                ("member prompt", vec!["member", "prompt", "7", "hello"]),
                ("member exec", vec!["member", "exec", "7", "printf ok"]),
                ("member press", vec!["member", "press", "7", "Enter"]),
                ("member recover", vec!["member", "recover", "7"]),
                (
                    "permission respond",
                    vec![
                        "permission",
                        "respond",
                        "7",
                        "--event-id",
                        "1",
                        "--from-member-id",
                        "1",
                        "--expected-run-id",
                        "1",
                        "--expected-owner-epoch",
                        "1",
                        "--request-token",
                        REQUEST_TOKEN,
                        "--decision",
                        "allow",
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
                    ],
                ),
                (
                    "input reconcile",
                    vec![
                        "input",
                        "reconcile",
                        "7",
                        "--disposition",
                        "submitted",
                        "--evidence-ref",
                        "seen",
                    ],
                ),
            ]
        }

        fn unguarded_commands() -> Vec<Vec<&'static str>> {
            vec![
                vec![
                    "message",
                    "send",
                    "--from-member-id",
                    "1",
                    "--to-member-id",
                    "2",
                    "hi",
                ],
                vec!["message", "poll", "1"],
                vec!["member", "list", "1"],
                vec!["member", "capture", "1"],
                vec![
                    "member",
                    "create",
                    "--fleet-id",
                    "1",
                    "--name",
                    "worker",
                    "--description",
                    "work",
                    "prompt",
                ],
                vec![
                    "permission",
                    "inspect",
                    "7",
                    "--event-id",
                    "1",
                    "--from-member-id",
                    "1",
                ],
                vec!["input", "show", "7"],
                vec!["event", "show", "7"],
                vec!["doctor"],
                vec!["setup"],
            ]
        }

        fn parse(args: &[&str]) -> Command {
            parse_cli(args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
        }

        fn guard_message(name: &str, value: &str) -> String {
            format!(
                "'cafleet {name}' is a Director-only command and this process runs inside CAFleet member {value} (CAFLEET_MEMBER_ID is set); send the request to your Director."
            )
        }

        #[test]
        fn director_only_name_names_exactly_the_six_guarded_commands() {
            for (name, args) in director_only_commands() {
                assert_eq!(director_only_name(&parse(&args)), Some(name), "{args:?}");
            }
            for args in unguarded_commands() {
                assert_eq!(director_only_name(&parse(&args)), None, "{args:?}");
            }
        }

        #[test]
        fn director_only_commands_refuse_when_the_member_variable_is_present_with_any_value() {
            let values = [
                (OsString::from("34"), "34".to_string()),
                (OsString::from(""), String::new()),
                (
                    OsString::from_vec(vec![b'3', 0xff, b'4']),
                    "3\u{fffd}4".to_string(),
                ),
            ];
            for (name, args) in director_only_commands() {
                for (value, rendered) in &values {
                    let error = refuse_director_only(&parse(&args), Some(value.clone()))
                        .expect_err(&format!("{args:?} with {value:?} is refused"));
                    assert_eq!(error.exit_code(), 1, "{args:?} with {value:?}");
                    assert_eq!(
                        error.to_string(),
                        guard_message(name, rendered),
                        "{args:?} with {value:?}"
                    );
                }
            }
        }

        #[test]
        fn director_only_commands_pass_when_the_member_variable_is_unset() {
            for (_, args) in director_only_commands() {
                refuse_director_only(&parse(&args), None)
                    .unwrap_or_else(|error| panic!("{args:?}: {error}"));
            }
        }

        #[test]
        fn unguarded_commands_pass_when_the_member_variable_is_set() {
            for args in unguarded_commands() {
                for value in [OsString::from("34"), OsString::from("")] {
                    refuse_director_only(&parse(&args), Some(value.clone()))
                        .unwrap_or_else(|error| panic!("{args:?} with {value:?}: {error}"));
                }
            }
        }

        const CHILD_ARGV: &str = "CAFLEET_TEST_GUARD_ARGV";
        const CHILD_STDOUT_MARKER: &str = "cafleet-guard-child-stdout-begins";
        const SETTINGS_ERROR: &str =
            "Error: CAFLEET_MAX_TEXT_LEN must be a non-negative integer (got 'bogus')\n";

        #[test]
        #[ignore = "owned CLI subprocess entry"]
        fn guarded_run_child() {
            let argv: Vec<String> = serde_json::from_str(
                &std::env::var(CHILD_ARGV).expect("explicit guard child argv"),
            )
            .unwrap();
            println!("{CHILD_STDOUT_MARKER}");
            let args = std::iter::once(OsString::from("cafleet"))
                .chain(argv.into_iter().map(OsString::from))
                .collect::<Vec<_>>();
            match run_args(args) {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(error.exit_code());
                }
            }
        }

        struct ChildOutcome {
            code: Option<i32>,
            stdout: String,
            stderr: String,
        }

        fn run_guarded_child(args: &[&str], member_id: Option<&str>) -> ChildOutcome {
            use crate::runtime::system::test_support::TestChild;
            use std::process::Stdio;
            use std::time::Duration;
            use std::time::Instant;

            let database_dir = tempfile::tempdir().unwrap();
            let capture_dir = tempfile::tempdir().unwrap();
            let stdout_path = capture_dir.path().join("stdout");
            let stderr_path = capture_dir.path().join("stderr");
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    test_path!(guarded_run_child),
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env(CHILD_ARGV, serde_json::json!(args).to_string())
                .env("CAFLEET_MAX_TEXT_LEN", "bogus")
                .env(
                    "CAFLEET_DATABASE_URL",
                    format!(
                        "sqlite:///{}",
                        database_dir.path().join("absent.db").display()
                    ),
                )
                .env("RUST_BACKTRACE", "0")
                .stdin(Stdio::null())
                .stdout(Stdio::from(std::fs::File::create(&stdout_path).unwrap()))
                .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()));
            match member_id {
                Some(value) => command.env("CAFLEET_MEMBER_ID", value),
                None => command.env_remove("CAFLEET_MEMBER_ID"),
            };
            let started = Instant::now();
            let mut child = TestChild::new(command.spawn().unwrap());
            let status = child.poll_until(started + Duration::from_secs(10)).unwrap();
            if status.is_none() {
                child.terminate_until(started + Duration::from_secs(12));
            }
            let status = status.unwrap_or_else(|| {
                panic!(
                    "{args:?}: guard child exceeded its deadline: {:?}",
                    std::fs::read_to_string(&stderr_path).unwrap()
                )
            });
            assert_eq!(
                std::fs::read_dir(database_dir.path()).unwrap().count(),
                0,
                "{args:?}: the database directory stays empty"
            );
            let stdout = std::fs::read_to_string(stdout_path).unwrap();
            let (_, cli_stdout) = stdout
                .split_once(&format!("{CHILD_STDOUT_MARKER}\n"))
                .unwrap_or_else(|| panic!("{args:?}: child reached run_args: {stdout:?}"));
            ChildOutcome {
                code: status.code(),
                stdout: cli_stdout.to_string(),
                stderr: std::fs::read_to_string(stderr_path).unwrap(),
            }
        }

        #[test]
        fn run_args_refuses_director_only_commands_before_reading_settings() {
            for (name, args) in director_only_commands() {
                let outcome = run_guarded_child(&args, Some("34"));
                assert_eq!(outcome.code, Some(1), "{args:?}: {}", outcome.stderr);
                assert_eq!(
                    outcome.stderr,
                    format!("Error: {}\n", guard_message(name, "34")),
                    "{args:?}"
                );
                assert_eq!(outcome.stdout, "", "{args:?}");
            }
        }

        #[test]
        fn run_args_reads_the_rejected_settings_without_the_member_variable() {
            for (_, args) in director_only_commands() {
                let outcome = run_guarded_child(&args, None);
                assert_eq!(outcome.code, Some(1), "{args:?}: {}", outcome.stderr);
                assert_eq!(outcome.stderr, SETTINGS_ERROR, "{args:?}");
            }
        }

        #[test]
        fn run_args_lets_unguarded_commands_reach_settings_with_the_member_variable() {
            let outcome = run_guarded_child(&["message", "poll", "1"], Some("34"));
            assert_eq!(outcome.code, Some(1), "{}", outcome.stderr);
            assert_eq!(outcome.stderr, SETTINGS_ERROR);
        }

        #[test]
        fn run_args_reports_parse_errors_before_the_guard() {
            let outcome = run_guarded_child(&["member", "prompt", "7"], Some("34"));
            assert_eq!(outcome.code, Some(2), "{}", outcome.stderr);
            assert!(
                !outcome.stderr.contains("Director-only"),
                "{}",
                outcome.stderr
            );
            assert!(outcome.stderr.contains("TEXT"), "{}", outcome.stderr);
        }
    }
}
