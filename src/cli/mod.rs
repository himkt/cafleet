pub(crate) use crate::cli::member::MemberCapture;
pub(crate) use crate::cli::message::poll_command;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::format_json;
use crate::runtime::system::read_stdin;
use clap::Parser;
use clap::Subcommand;
use serde_json::Value;
use std::ffi::OsString;
use std::io::Write;

mod fleet;
pub(crate) mod member;
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
    /// SQLite database URL `sqlite:///<path>` [default: the running version's database under
    /// ~/.local/share/cafleet].
    #[arg(long = "database-url", value_name = "URL", global = true)]
    database_url: Option<String>,
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
    let settings = Settings::new(args.database_url)?;
    dispatch(&settings, args.command)
}

/// A Director-only command: its name, the query from its subject to the subject's fleet, the
/// subject id and its `--from-member-id`.
struct DirectorOnly {
    name: &'static str,
    fleet_query: &'static str,
    subject: i64,
    director_member_id: i64,
}

const MEMBER_FLEET: &str = "SELECT fleet_id FROM members WHERE member_id=?1 AND status='active'";
/// Recovery also reconciles a deregistered member's run, so its subject keeps its fleet.
const INPUT_FLEET: &str = "SELECT r.fleet_id FROM member_input_operations i JOIN member_runs r ON r.run_id=i.run_id WHERE i.input_id=?1";
const INTERACTION_FLEET: &str = "SELECT r.fleet_id FROM member_interactions i JOIN member_runs r ON r.run_id=i.run_id WHERE i.interaction_id=?1";

fn director_only(command: &Command) -> Option<DirectorOnly> {
    use member::MemberCommand as M;
    let (name, fleet_query, subject, director_member_id) = match command {
        Command::Member(M::Prompt {
            member_id,
            director_member_id,
            ..
        }) => ("member prompt", MEMBER_FLEET, member_id, director_member_id),
        Command::Member(M::Exec {
            member_id,
            director_member_id,
            ..
        }) => ("member exec", MEMBER_FLEET, member_id, director_member_id),
        Command::Member(M::Press {
            member_id,
            director_member_id,
            ..
        }) => ("member press", MEMBER_FLEET, member_id, director_member_id),
        Command::Permission(runtime::PermissionCommand::Respond {
            interaction_id,
            director_member_id,
            ..
        }) => (
            "permission respond",
            INTERACTION_FLEET,
            interaction_id,
            director_member_id,
        ),
        Command::Input(runtime::InputCommand::Reconcile {
            input_id,
            director_member_id,
            ..
        }) => ("input reconcile", INPUT_FLEET, input_id, director_member_id),
        _ => return None,
    };
    Some(DirectorOnly {
        name,
        fleet_query,
        subject: *subject,
        director_member_id: *director_member_id,
    })
}

/// Requires a Director-only command's `--from-member-id` to be the root Director of the live
/// fleet that owns its subject. An unknown subject passes, so the command reports its own
/// not-found error.
fn require_director(
    conn: &rusqlite::Connection,
    guarded: Option<&DirectorOnly>,
) -> Result<(), CafleetError> {
    use rusqlite::OptionalExtension;
    let Some(guarded) = guarded else {
        return Ok(());
    };
    let Some(fleet_id) = conn
        .query_row(guarded.fleet_query, [guarded.subject], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?
    else {
        return Ok(());
    };
    let is_root_director = conn
        .query_row(
            "SELECT 1 FROM fleets WHERE fleet_id=?1 AND deleted_at IS NULL AND director_member_id=?2",
            [fleet_id, guarded.director_member_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if is_root_director {
        return Ok(());
    }
    Err(CafleetError::App(format!(
        "'cafleet {}' is a Director-only command and member {} is not the root Director of fleet {fleet_id}; send the request to your Director.",
        guarded.name, guarded.director_member_id
    )))
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
    let director = director_only(&command);
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
            ..
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
            require_director(&conn, director.as_ref())?;
            member::press(&mut conn, settings, member_id, &keys, json, deadline)
        }
        Command::Permission(command) => {
            let deadline = crate::time::Deadline::new();
            let mut conn = invocation_connection(&settings.database_url, deadline)
                .map_err(|source| runtime::permission_error(&command, source))?;
            require_director(&conn, director.as_ref())?;
            runtime::permission(&mut conn, settings, command, deadline)
        }
        mut command => {
            let mut conn =
                invocation_connection(&settings.database_url, crate::time::Deadline::new())?;
            require_director(&conn, director.as_ref())?;
            if let Command::Member(command) = &mut command {
                member::prepare_input(command)?;
            }
            let deadline = crate::time::Deadline::new();
            match command {
                Command::Fleet(cmd) => fleet::run(&mut conn, settings, cmd, deadline),
                Command::Member(cmd) => member::run(&mut conn, settings, cmd, deadline),
                Command::Message(cmd) => message::run(&mut conn, cmd),
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

pub(crate) fn resolve_body(
    inline: Option<&str>,
    file: Option<&str>,
) -> Result<String, CafleetError> {
    let body = read_body(inline, file)?;
    if !body.chars().all(char::is_whitespace) {
        return Ok(body);
    }
    Err(match file {
        None => CafleetError::Usage("text may not be empty.".to_string()),
        Some("-") => CafleetError::App("--file -: stdin is empty.".to_owned()),
        Some(path) => CafleetError::App(format!("--file {path}: file is empty.")),
    })
}

pub(crate) fn resolve_command_body(
    inline: Option<&str>,
    file: Option<&str>,
) -> Result<String, CafleetError> {
    let mut body = read_body(inline, file)?;
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

fn read_body(inline: Option<&str>, file: Option<&str>) -> Result<String, CafleetError> {
    match (inline, file) {
        (Some(text), None) => Ok(text.to_string()),
        (None, Some("-")) => {
            let bytes = read_stdin().map_err(|e| CafleetError::App(format!("--file -: {e}")))?;
            String::from_utf8(bytes)
                .map_err(|_| CafleetError::App("--file -: file is not valid UTF-8.".to_owned()))
        }
        (None, Some(path)) => {
            let bytes = std::fs::read(path).map_err(|e| {
                let message = match e.kind() {
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory => {
                        format!("--file {path}: file does not exist or is not a regular file.")
                    }
                    _ => format!("--file {path}: file is not readable."),
                };
                CafleetError::App(format!("{message} ({e})"))
            })?;
            String::from_utf8(bytes)
                .map_err(|_| CafleetError::App(format!("--file {path}: file is not valid UTF-8.")))
        }
        (Some(_), Some(_)) | (None, None) => {
            unreachable!("clap's required argument group supplies exactly one body source")
        }
    }
}

pub(crate) fn emit(
    json: bool,
    payload: &Value,
    text: impl FnOnce() -> String,
) -> Result<(), CafleetError> {
    let line = if json { format_json(payload) } else { text() };
    write_output(&line)
}

pub(crate) fn write_output(line: &str) -> Result<(), CafleetError> {
    output_result(writeln!(std::io::stdout().lock(), "{line}"))
}

/// A reader that closed the pipe has consumed all the output it wants, so a broken pipe ends
/// the output without an error.
pub(crate) fn output_result(result: std::io::Result<()>) -> Result<(), CafleetError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(CafleetError::App(format!(
            "failed to write output: {error}"
        ))),
    }
}

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

    /// Asserts that the parser itself refuses `args` as a usage error, rather than failing to
    /// recognize the subcommand or an option.
    pub(super) fn assert_usage_rejected(args: &[impl AsRef<str>]) {
        let args = args.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        let error = usage_error(&args);
        assert_eq!(error.exit_code(), 2, "{args:?}: {error}");
        assert!(
            !matches!(
                error.kind(),
                ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument
            ),
            "the parser validates these arguments: {args:?}: {error}"
        );
    }

    pub(super) fn assert_error(args: &[&str], kind: ErrorKind, input: &str) {
        let error = usage_error(args);
        assert_eq!(error.kind(), kind, "{args:?}: {error}");
        assert!(error.to_string().contains(input), "{args:?}: {error}");
    }

    /// Runs the CLI with only `executables` (name, shell body) on PATH and `environment` added.
    /// Runs the libtest child `test` of this binary with `command`'s environment, capturing its
    /// output and bounding it to ten seconds.
    pub(crate) fn run_test_child(
        command: &mut std::process::Command,
        test: &str,
    ) -> std::process::Output {
        use crate::runtime::system::test_support::TestChild;
        use std::process::Stdio;
        use std::time::Duration;
        use std::time::Instant;

        let capture = tempfile::tempdir().unwrap();
        let stdout_path = capture.path().join("stdout");
        let stderr_path = capture.path().join("stderr");
        command
            .args([
                "--exact",
                test,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
                "--quiet",
            ])
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::fs::File::create(&stdout_path).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()));
        let started = Instant::now();
        let mut child = TestChild::new(command.spawn().unwrap());
        let status = child.poll_until(started + Duration::from_secs(10)).unwrap();
        if status.is_none() {
            child.terminate_until(started + Duration::from_secs(12));
        }
        std::process::Output {
            status: status.unwrap_or_else(|| {
                panic!(
                    "{test} exceeded its deadline: {:?}",
                    std::fs::read_to_string(&stderr_path).unwrap()
                )
            }),
            stdout: std::fs::read(stdout_path).unwrap(),
            stderr: std::fs::read(stderr_path).unwrap(),
        }
    }

    pub(super) fn isolated_cli_with(
        settings: &crate::config::Settings,
        args: &[&str],
        executables: &[(&str, &str)],
        environment: &[(&str, &str)],
    ) -> (std::process::Output, String) {
        let dir = tempfile::tempdir().unwrap();
        let pane_log = dir.path().join("pane.log");
        for (name, body) in executables {
            crate::coding_agent::test_support::executable_stub(dir.path(), name, body);
        }
        let output = run_test_child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .envs(environment.iter().copied())
                .env(
                    "CAFLEET_TEST_ISOLATED_CLI",
                    serde_json::json!({"database_url":settings.database_url,"args":args})
                        .to_string(),
                )
                .env("CAFLEET_TEST_PANE_LOG", &pane_log)
                .env("PATH", dir.path())
                .env("TMUX", "cafleet-test,1,0")
                .env_remove("HERDR_ENV"),
            crate::cli::tests::isolated_cli_child_path(),
        );
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
        let settings = crate::config::Settings::for_database(std::path::Path::new(database));
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
        let mutation = "PRAGMA application_id=42";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let settings = Settings::for_database(&path);
        let mut conn = crate::db::connect(&settings.database_url).unwrap();
        crate::db::initialize(&mut conn, &settings.database_url).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        conn.execute_batch(mutation).unwrap();
        conn.execute_batch("BEGIN; SELECT * FROM cafleet_database")
            .unwrap();
        let before = crate::db::test_support::wal_snapshot(&path);
        for args in [
            &["fleet", "list", "--json"][..],
            &["member", "press", "1", "--from-member-id", "1", "Enter"],
            &[
                "permission",
                "inspect",
                "1",
                "--event-id",
                "1",
                "--from-member-id",
                "1",
            ],
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

    #[test]
    fn ordinary_commands_give_a_non_sqlite_file_the_incompatible_refusal_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foreign.db");
        let bytes = b"not a sqlite database file\n".repeat(64);
        std::fs::write(&path, &bytes).unwrap();
        let settings = Settings::for_database(&path);
        let expected = format!(
            "database format is incompatible at '{}'; choose an empty database path and run 'cafleet setup'.",
            path.display()
        );
        for args in [
            &["fleet", "list", "--json"][..],
            &["message", "poll", "1"],
            &["member", "press", "1", "--from-member-id", "1", "Enter"],
            &[
                "permission",
                "inspect",
                "1",
                "--event-id",
                "1",
                "--from-member-id",
                "1",
            ],
        ] {
            let error = dispatch(&settings, parse(args)).expect_err("a foreign file is refused");
            let message = error.message();
            assert!(
                message.lines().any(|line| line == expected),
                "{args:?}: {message}"
            );
            assert_eq!(error.exit_code(), 1, "{args:?}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{args:?}");
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
            (&["setup", "--unknown"], "--unknown"),
            (&["fleet", "list", "--unknown"], "--unknown"),
            (&["member", "show", "1", "--unknown"], "--unknown"),
            (&["message", "poll", "1", "--unknown"], "--unknown"),
            (&["event", "show", "1", "--unknown"], "--unknown"),
            (&["input", "show", "1", "--unknown"], "--unknown"),
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
            pane,
            mux_server,
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
        assert_eq!((pane, mux_server), (None, None));
        assert!(json);
        let Command::Fleet(fleet::FleetCommand::Create {
            pane, mux_server, ..
        }) = parse(&[
            "fleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "hermes",
            "--pane",
            "%3",
            "--mux-server",
            "/tmp/tmux-0/default",
        ])
        else {
            panic!("expected fleet creation");
        };
        assert_eq!(pane.as_deref(), Some("%3"));
        assert_eq!(mux_server.as_deref(), Some("/tmp/tmux-0/default"));
        assert_error(
            &[
                "fleet",
                "create",
                "--name",
                "alpha",
                "--coding-agent",
                "codex",
                "--mux-server",
                "/tmp/tmux-0/default",
            ],
            ErrorKind::MissingRequiredArgument,
            "--pane",
        );
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
        let exec = ["member", "exec", "17", "--from-member-id", "1"];
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
        assert_eq!(
            usage_error(&["__member-run", "--descriptor"]).exit_code(),
            2
        );
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
    fn help_describes_every_argument_of_the_runtime_read_input_and_press_commands() {
        use clap::CommandFactory;
        let command = CliArgs::command();
        for path in [
            &["event", "show"][..],
            &["input", "show"],
            &["input", "reconcile"],
            &["member", "press"],
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
    fn help_describes_command_groups_and_derives_choices_from_their_registries() {
        use crate::coding_agent::CodingAgent;
        use clap::CommandFactory;
        let catalog = CodingAgent::all()
            .iter()
            .map(|agent| {
                let default = agent.resolve_policy(None).unwrap().resolved_mode();
                format!(
                    "{}: {} (default {default})",
                    agent.name(),
                    agent.policy_choices().join(", ")
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
        let respond = subcommand_help(&["permission", "respond"]);
        let values = crate::broker::runtime::PermissionDecision::VALUES;
        assert_eq!(
            possible_values(&["permission", "respond"], "decision"),
            values
        );
        assert!(
            respond.contains(&format!("[possible values: {}]", values.join(", "))),
            "decision help lists {values:?}: {respond}"
        );
    }

    #[test]
    fn member_runner_dispatch_rejects_an_unallocated_descriptor_before_runtime_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let descriptor = dir.path().join("unallocated.json");
        std::fs::write(&descriptor, b"{\"schema_version\":1}").unwrap();
        let settings = crate::config::Settings::for_database(&path);
        let mut conn = crate::db::connect(&settings.database_url).unwrap();
        crate::db::initialize(&mut conn, &settings.database_url).unwrap();
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
            assert_eq!(resolve_body(None, Some(name)).unwrap(), text);
        }
        std::fs::write(&path, [0xff]).unwrap();
        assert!(
            resolve_body(None, Some(name))
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(
            resolve_body(None, Some(name))
                .unwrap_err()
                .to_string()
                .contains(name)
        );
        assert!(resolve_body(None, Some(dir.path().to_str().unwrap())).is_err());
    }

    struct BodyCase {
        name: &'static str,
        text: &'static str,
    }

    const BLANK_BODIES: [BodyCase; 9] = [
        BodyCase {
            name: "empty text",
            text: "",
        },
        BodyCase {
            name: "ASCII spaces and controls with White_Space",
            text: " \t\n\u{b}\u{c}\r",
        },
        BodyCase {
            name: "next line",
            text: "\u{85}",
        },
        BodyCase {
            name: "no-break space",
            text: "\u{a0}",
        },
        BodyCase {
            name: "ogham space mark",
            text: "\u{1680}",
        },
        BodyCase {
            name: "en quad through hair space",
            text: "\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}",
        },
        BodyCase {
            name: "line and paragraph separators",
            text: "\u{2028}\u{2029}",
        },
        BodyCase {
            name: "narrow no-break, medium mathematical and ideographic spaces",
            text: "\u{202f}\u{205f}\u{3000}",
        },
        BodyCase {
            name: "mixed White_Space scalars",
            text: " \u{3000}\n\u{a0}\t",
        },
    ];

    const NONBLANK_BODIES: [BodyCase; 8] = [
        BodyCase {
            name: "file separator",
            text: "\u{1c}",
        },
        BodyCase {
            name: "group separator",
            text: "\u{1d}",
        },
        BodyCase {
            name: "record separator",
            text: "\u{1e}",
        },
        BodyCase {
            name: "unit separator",
            text: "\u{1f}",
        },
        BodyCase {
            name: "information separators among White_Space",
            text: " \u{1c}\n\u{1f}\t",
        },
        BodyCase {
            name: "zero width space",
            text: "\u{200b}",
        },
        BodyCase {
            name: "byte order mark",
            text: "\u{feff}",
        },
        BodyCase {
            name: "one visible scalar among White_Space",
            text: "\u{3000}x\u{2029}",
        },
    ];

    #[test]
    fn a_body_is_blank_exactly_when_every_scalar_has_white_space() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.txt");
        let name = path.to_str().unwrap();
        for case in BLANK_BODIES {
            std::fs::write(&path, case.text).unwrap();
            let inline = resolve_body(Some(case.text), None).unwrap_err();
            assert!(
                matches!(&inline, CafleetError::Usage(message) if message == "text may not be empty."),
                "{}: {inline:?}",
                case.name
            );
            assert_eq!(inline.exit_code(), 2, "{}", case.name);
            let file = resolve_body(None, Some(name)).unwrap_err();
            assert_eq!(
                file.message(),
                format!("--file {name}: file is empty."),
                "{}",
                case.name
            );
            assert_eq!(file.exit_code(), 1, "{}", case.name);
        }
        for case in NONBLANK_BODIES {
            std::fs::write(&path, case.text).unwrap();
            assert_eq!(
                resolve_body(Some(case.text), None).unwrap(),
                case.text,
                "{}",
                case.name
            );
            assert_eq!(
                resolve_body(None, Some(name)).unwrap(),
                case.text,
                "{}",
                case.name
            );
        }
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
        for source in ["", "echo x\n\n", "echo x\r"] {
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

    #[test]
    fn press_cli_invalid_tail_is_usage_before_missing_database_validation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("absent.db");
        let settings = crate::config::Settings::for_database(&path);
        let command = CliArgs::try_parse_from([
            "cafleet",
            "member",
            "press",
            "--from-member-id",
            "1",
            "42",
            "Enter",
            "Ctrl+1",
            "--json",
        ])
        .unwrap_or_else(|cause| panic!("raw key arguments reach typed validation: {cause}"))
        .command;
        let failure =
            dispatch(&settings, command).expect_err("the complete key vector is validated first");
        assert!(matches!(&failure, CafleetError::Usage(message)
        if message == "invalid press key at position 2: \"Ctrl+1\""));
        assert_eq!(failure.exit_code(), 2);
        assert!(!path.exists());
    }

    mod director_only_guard {
        use crate::broker::runtime::InputKind;
        use crate::broker::runtime::test_support::Fixture;
        use crate::cli::test_support::parse_cli;
        use crate::cli::*;

        const REQUEST_TOKEN: &str =
            "0000000000000000000000000000000000000000000000000000000000000000";

        fn director_only_commands(subject: &str, caller: &str) -> Vec<(&'static str, Vec<String>)> {
            let owned = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect();
            vec![
                (
                    "member prompt",
                    owned(&[
                        "member",
                        "prompt",
                        subject,
                        "hello",
                        "--from-member-id",
                        caller,
                    ]),
                ),
                (
                    "member exec",
                    owned(&[
                        "member",
                        "exec",
                        subject,
                        "printf ok",
                        "--from-member-id",
                        caller,
                    ]),
                ),
                (
                    "member press",
                    owned(&[
                        "member",
                        "press",
                        subject,
                        "Enter",
                        "--from-member-id",
                        caller,
                    ]),
                ),
                (
                    "permission respond",
                    owned(&[
                        "permission",
                        "respond",
                        subject,
                        "--event-id",
                        "1",
                        "--from-member-id",
                        caller,
                        "--expected-run-id",
                        "1",
                        "--expected-owner-epoch",
                        "1",
                        "--request-token",
                        REQUEST_TOKEN,
                        "--decision",
                        "allow",
                    ]),
                ),
                (
                    "input reconcile",
                    owned(&[
                        "input",
                        "reconcile",
                        subject,
                        "--disposition",
                        "submitted",
                        "--evidence-ref",
                        "seen",
                        "--from-member-id",
                        caller,
                    ]),
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

        fn parse(args: &[impl AsRef<str>]) -> Command {
            let args = args.iter().map(AsRef::as_ref).collect::<Vec<_>>();
            parse_cli(&args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
        }

        fn guard_message(name: &str, caller: i64, fleet: i64) -> String {
            format!(
                "'cafleet {name}' is a Director-only command and member {caller} is not the root Director of fleet {fleet}; send the request to your Director."
            )
        }

        #[test]
        fn director_only_names_exactly_the_guarded_commands() {
            for (name, args) in director_only_commands("7", "1") {
                assert_eq!(
                    director_only(&parse(&args)).map(|guarded| guarded.name),
                    Some(name),
                    "{args:?}"
                );
            }
            for args in unguarded_commands() {
                assert!(director_only(&parse(&args)).is_none(), "{args:?}");
            }
        }

        #[test]
        fn director_only_commands_require_a_positive_from_member_id() {
            for (_, args) in director_only_commands("7", "1") {
                let position = args
                    .iter()
                    .position(|arg| arg == "--from-member-id")
                    .unwrap();
                let mut missing = args.clone();
                missing.drain(position..position + 2);
                let missing = missing.iter().map(String::as_str).collect::<Vec<_>>();
                let error = parse_cli(&missing)
                    .err()
                    .unwrap_or_else(|| panic!("{missing:?} parses without its caller"));
                assert_eq!(error.exit_code(), 2, "{missing:?}");
                let mut zero = args.clone();
                zero[position + 1] = "0".into();
                let zero = zero.iter().map(String::as_str).collect::<Vec<_>>();
                let error = parse_cli(&zero)
                    .err()
                    .unwrap_or_else(|| panic!("{zero:?} parses with a zero caller"));
                assert_eq!(error.exit_code(), 2, "{zero:?}");
            }
        }

        struct Subjects {
            fixture: Fixture,
            input: i64,
            interaction: i64,
        }

        impl Subjects {
            fn new() -> Self {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                let input = fixture
                    .input(&owner, "prompt:guard", InputKind::Prompt)
                    .input_id;
                let interaction = fixture
                    .open(&owner)
                    .interaction_id
                    .expect("the opening event creates an interaction");
                Self {
                    fixture,
                    input,
                    interaction,
                }
            }

            fn commands(&self, caller: i64) -> Vec<(&'static str, Vec<String>)> {
                let caller = caller.to_string();
                director_only_commands("0", &caller)
                    .into_iter()
                    .map(|(name, mut args)| {
                        args[2] = match name {
                            "permission respond" => self.interaction,
                            "input reconcile" => self.input,
                            _ => self.fixture.member,
                        }
                        .to_string();
                        (name, args)
                    })
                    .collect()
            }

            fn check(&self, args: &[String]) -> Result<(), CafleetError> {
                require_director(&self.fixture.conn, director_only(&parse(args)).as_ref())
            }
        }

        #[test]
        fn require_director_admits_the_root_director_of_each_subjects_fleet() {
            let subjects = Subjects::new();
            for (name, args) in subjects.commands(subjects.fixture.director) {
                subjects
                    .check(&args)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
            }
            for args in unguarded_commands() {
                require_director(
                    &subjects.fixture.conn,
                    director_only(&parse(&args)).as_ref(),
                )
                .unwrap_or_else(|error| panic!("{args:?}: {error}"));
            }
        }

        #[test]
        fn require_director_refuses_any_other_caller_naming_the_subjects_fleet() {
            let mut subjects = Subjects::new();
            let (other_fleet, other_director) =
                crate::broker::fleets::test_support::create_fleet_fixture(
                    &mut subjects.fixture.conn,
                    "other-fleet",
                );
            assert_ne!(other_fleet, subjects.fixture.fleet);
            for caller in [subjects.fixture.member, other_director, 999_999] {
                for (name, args) in subjects.commands(caller) {
                    let error = subjects
                        .check(&args)
                        .expect_err(&format!("{name} refuses member {caller}"));
                    assert_eq!(error.exit_code(), 1, "{name}");
                    assert_eq!(
                        error.to_string(),
                        guard_message(name, caller, subjects.fixture.fleet),
                        "{name}"
                    );
                }
            }
        }

        #[test]
        fn require_director_refuses_the_director_of_a_deleted_fleet() {
            let subjects = Subjects::new();
            subjects
                .fixture
                .conn
                .execute(
                    "UPDATE fleets SET deleted_at='2026-11-07T00:00:00+00:00' WHERE fleet_id=?1",
                    [subjects.fixture.fleet],
                )
                .unwrap();
            for (name, args) in subjects.commands(subjects.fixture.director) {
                if name.starts_with("member ") {
                    continue;
                }
                assert_eq!(
                    subjects.check(&args).unwrap_err().to_string(),
                    guard_message(name, subjects.fixture.director, subjects.fixture.fleet),
                    "{name}"
                );
            }
        }

        #[test]
        fn require_director_leaves_an_unknown_subject_to_the_commands_own_not_found_error() {
            let subjects = Subjects::new();
            for (name, args) in
                director_only_commands("999999", &subjects.fixture.member.to_string())
            {
                subjects
                    .check(&args)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
            }
        }

        const CHILD_ARGV: &str = "CAFLEET_TEST_GUARD_ARGV";
        const CHILD_STDOUT_MARKER: &str = "cafleet-guard-child-stdout-begins";

        #[test]
        #[ignore = "owned CLI subprocess entry"]
        fn guarded_run_child() {
            let argv: Vec<String> = serde_json::from_str(
                &std::env::var(CHILD_ARGV).expect("explicit guard child argv"),
            )
            .unwrap();
            println!("{CHILD_STDOUT_MARKER}");
            let args = std::iter::once(std::ffi::OsString::from("cafleet"))
                .chain(argv.into_iter().map(std::ffi::OsString::from))
                .collect::<Vec<_>>();
            match run_args(args) {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(error.exit_code());
                }
            }
        }

        pub(super) struct ChildOutcome {
            pub(super) code: Option<i32>,
            pub(super) stdout: String,
            pub(super) stderr: String,
        }

        fn run_guarded_child(args: &[String]) -> ChildOutcome {
            run_guarded_child_with(args, |_| {})
        }

        /// Runs `run_args` on `args` in a child whose command `configure` adjusts first.
        pub(super) fn run_guarded_child_with(
            args: &[String],
            configure: impl FnOnce(&mut std::process::Command),
        ) -> ChildOutcome {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.env(CHILD_ARGV, serde_json::json!(args).to_string());
            configure(&mut command);
            let output = crate::cli::test_support::run_test_child(
                &mut command,
                test_path!(guarded_run_child),
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            let (_, cli_stdout) = stdout
                .split_once(&format!("{CHILD_STDOUT_MARKER}\n"))
                .unwrap_or_else(|| panic!("{args:?}: child reached run_args: {stdout:?}"));
            ChildOutcome {
                code: output.status.code(),
                stdout: cli_stdout.to_string(),
                stderr: String::from_utf8(output.stderr).unwrap(),
            }
        }

        #[test]
        fn run_args_refuses_a_member_caller_with_exit_one_and_nothing_on_stdout() {
            let subjects = Subjects::new();
            let url = format!("sqlite:///{}", subjects.fixture.conn.path().unwrap());
            for (name, args) in subjects.commands(subjects.fixture.member) {
                let mut argv = vec!["--database-url".to_owned(), url.clone()];
                argv.extend(args);
                let outcome = run_guarded_child(&argv);
                assert_eq!(outcome.code, Some(1), "{name}: {}", outcome.stderr);
                assert_eq!(
                    outcome.stderr,
                    format!(
                        "Error: {}\n",
                        guard_message(name, subjects.fixture.member, subjects.fixture.fleet)
                    ),
                    "{name}"
                );
                assert_eq!(outcome.stdout, "", "{name}");
            }
        }

        #[test]
        fn run_args_reports_parse_errors_before_the_guard() {
            let outcome = run_guarded_child(&["member".into(), "prompt".into(), "7".into()]);
            assert_eq!(outcome.code, Some(2), "{}", outcome.stderr);
            assert!(
                !outcome.stderr.contains("Director-only"),
                "{}",
                outcome.stderr
            );
            assert!(outcome.stderr.contains("TEXT"), "{}", outcome.stderr);
        }
    }

    mod validation_order {
        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::broker::members::test_support::register;
        use crate::cli::test_support::parse_cli;
        use crate::cli::*;
        use crate::db::test_support::initialized_file_conn;

        const NO_DATABASE: &str = "no cafleet database; run 'cafleet setup'";

        struct Registry {
            dir: tempfile::TempDir,
            initialized: Settings,
            director: String,
            worker: String,
        }

        impl Registry {
            fn new() -> Self {
                let dir = tempfile::tempdir().unwrap();
                let mut conn = initialized_file_conn(&dir);
                let (fleet, director) = create_fleet_fixture(&mut conn, "validation-order");
                let worker = register(&mut conn, fleet, "worker", Some("%7"));
                let initialized =
                    Settings::for_database(std::path::Path::new(conn.path().unwrap()));
                Self {
                    dir,
                    initialized,
                    director: director.to_string(),
                    worker: worker.to_string(),
                }
            }

            fn absent_path(&self) -> std::path::PathBuf {
                self.dir.path().join("absent.db")
            }

            fn settings(&self, database: Database) -> Settings {
                match database {
                    Database::Missing => Settings::for_database(&self.absent_path()),
                    Database::Initialized => self.initialized.clone(),
                    Database::Url(url) => Settings::new(Some(url.into())).unwrap(),
                }
            }
        }

        #[derive(Clone, Copy)]
        enum Database {
            Missing,
            Initialized,
            Url(&'static str),
        }

        enum Expected {
            Exact(String),
            Prefix(String),
            Contains(&'static str),
        }

        struct OrderCase {
            name: &'static str,
            database: Database,
            args: fn(&Registry) -> Vec<String>,
            expected: fn(&Registry) -> Expected,
            exit_code: i32,
        }

        fn owned(args: &[&str]) -> Vec<String> {
            args.iter().map(|arg| (*arg).to_owned()).collect()
        }

        fn guard_message(name: &str, caller: &str, registry: &Registry) -> String {
            let conn = crate::db::connect(&registry.initialized.database_url).unwrap();
            let fleet: i64 = conn
                .query_row(
                    "SELECT fleet_id FROM members WHERE member_id=?1",
                    [caller.parse::<i64>().unwrap()],
                    |row| row.get(0),
                )
                .unwrap();
            format!(
                "'cafleet {name}' is a Director-only command and member {caller} is not the root Director of fleet {fleet}; send the request to your Director."
            )
        }

        fn assert_order(cases: &[OrderCase]) {
            for case in cases {
                let name = case.name;
                let registry = Registry::new();
                let settings = registry.settings(case.database);
                let args = (case.args)(&registry);
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                let command =
                    parse_cli(&args).unwrap_or_else(|error| panic!("{name}: {args:?}: {error}"));
                let error = dispatch(&settings, command).expect_err(name);
                let message = error.message();
                let first_line = message.lines().next().unwrap_or("");
                match (case.expected)(&registry) {
                    Expected::Exact(expected) => assert_eq!(first_line, expected, "{name}"),
                    Expected::Prefix(prefix) => {
                        assert!(first_line.starts_with(&prefix), "{name}: {message}")
                    }
                    Expected::Contains(text) => {
                        assert!(message.contains(text), "{name}: {message}")
                    }
                }
                assert_eq!(error.exit_code(), case.exit_code, "{name}: {message}");
                assert!(
                    !registry.absent_path().exists(),
                    "{name}: no database is created"
                );
            }
        }

        #[test]
        fn message_send_checks_fail_in_order() {
            assert_order(&[
                OrderCase {
                    name: "the database guard precedes body validation",
                    database: Database::Missing,
                    args: |_| {
                        owned(&[
                            "message",
                            "send",
                            "--from-member-id",
                            "1",
                            "--to-member-id",
                            "2",
                            " ",
                        ])
                    },
                    expected: |_| Expected::Exact(NO_DATABASE.into()),
                    exit_code: 1,
                },
                OrderCase {
                    name: "a blank body precedes the sender and recipient checks",
                    database: Database::Initialized,
                    args: |_| {
                        owned(&[
                            "message",
                            "send",
                            "--from-member-id",
                            "999",
                            "--to-member-id",
                            "998",
                            " \u{2028}",
                        ])
                    },
                    expected: |_| Expected::Exact("text may not be empty.".into()),
                    exit_code: 2,
                },
                OrderCase {
                    name: "a --file read error precedes the sender and recipient checks",
                    database: Database::Initialized,
                    args: |registry| {
                        let path = registry.dir.path().join("absent.txt");
                        owned(&[
                            "message",
                            "send",
                            "--from-member-id",
                            "999",
                            "--to-member-id",
                            "998",
                            "--file",
                            path.to_str().unwrap(),
                        ])
                    },
                    expected: |registry| {
                        let path = registry.dir.path().join("absent.txt");
                        Expected::Prefix(format!(
                            "--file {}: file does not exist or is not a regular file. (",
                            path.display()
                        ))
                    },
                    exit_code: 1,
                },
            ]);
        }

        #[test]
        fn director_only_pane_input_checks_fail_in_order() {
            assert_order(&[
                OrderCase {
                    name: "prompt: the database guard precedes the Director-only guard",
                    database: Database::Missing,
                    args: |registry| {
                        owned(&[
                            "member",
                            "prompt",
                            &registry.worker,
                            "two\nlines",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |_| Expected::Exact(NO_DATABASE.into()),
                    exit_code: 1,
                },
                OrderCase {
                    name: "prompt: the Director-only guard precedes text validation",
                    database: Database::Initialized,
                    args: |registry| {
                        owned(&[
                            "member",
                            "prompt",
                            &registry.worker,
                            "two\nlines",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |registry| {
                        Expected::Exact(guard_message("member prompt", &registry.worker, registry))
                    },
                    exit_code: 1,
                },
                OrderCase {
                    name: "prompt: text validation precedes the target checks",
                    database: Database::Initialized,
                    args: |registry| {
                        owned(&[
                            "member",
                            "prompt",
                            &registry.director,
                            "two\nlines",
                            "--from-member-id",
                            &registry.director,
                        ])
                    },
                    expected: |_| {
                        Expected::Exact(
                            "text must be one non-empty line without control characters.".into(),
                        )
                    },
                    exit_code: 2,
                },
                OrderCase {
                    name: "exec: the database guard precedes the Director-only guard",
                    database: Database::Missing,
                    args: |registry| {
                        owned(&[
                            "member",
                            "exec",
                            &registry.worker,
                            "two\nlines",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |_| Expected::Exact(NO_DATABASE.into()),
                    exit_code: 1,
                },
                OrderCase {
                    name: "exec: the Director-only guard precedes command validation",
                    database: Database::Initialized,
                    args: |registry| {
                        owned(&[
                            "member",
                            "exec",
                            &registry.worker,
                            "two\nlines",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |registry| {
                        Expected::Exact(guard_message("member exec", &registry.worker, registry))
                    },
                    exit_code: 1,
                },
                OrderCase {
                    name: "exec: command validation precedes the target checks",
                    database: Database::Initialized,
                    args: |registry| {
                        owned(&[
                            "member",
                            "exec",
                            &registry.director,
                            "two\nlines",
                            "--from-member-id",
                            &registry.director,
                        ])
                    },
                    expected: |_| {
                        Expected::Exact(
                            "command must be one non-empty line without control characters.".into(),
                        )
                    },
                    exit_code: 2,
                },
            ]);
        }

        #[test]
        fn member_press_checks_fail_in_order() {
            assert_order(&[
                OrderCase {
                    name: "key parsing precedes the database guard",
                    database: Database::Missing,
                    args: |_| {
                        owned(&[
                            "member",
                            "press",
                            "2",
                            "Enter",
                            "Ctrl+1",
                            "--from-member-id",
                            "1",
                        ])
                    },
                    expected: |_| {
                        Expected::Exact("invalid press key at position 2: \"Ctrl+1\"".into())
                    },
                    exit_code: 2,
                },
                OrderCase {
                    name: "key parsing precedes the database URL scheme check",
                    database: Database::Url("mysql://cafleet"),
                    args: |_| {
                        owned(&[
                            "member",
                            "press",
                            "2",
                            "Enter",
                            "Ctrl+1",
                            "--from-member-id",
                            "1",
                        ])
                    },
                    expected: |_| {
                        Expected::Exact("invalid press key at position 2: \"Ctrl+1\"".into())
                    },
                    exit_code: 2,
                },
                OrderCase {
                    name: "the database guard precedes the Director-only guard",
                    database: Database::Missing,
                    args: |registry| {
                        owned(&[
                            "member",
                            "press",
                            &registry.director,
                            "Enter",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |_| Expected::Exact(NO_DATABASE.into()),
                    exit_code: 1,
                },
                OrderCase {
                    name: "the Director-only guard precedes the target checks",
                    database: Database::Initialized,
                    args: |registry| {
                        owned(&[
                            "member",
                            "press",
                            &registry.director,
                            "Enter",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |registry| {
                        Expected::Exact(guard_message("member press", &registry.worker, registry))
                    },
                    exit_code: 1,
                },
            ]);
        }

        #[test]
        fn runtime_record_commands_check_the_database_before_the_director_only_guard() {
            assert_order(&[
                OrderCase {
                    name: "permission respond",
                    database: Database::Missing,
                    args: |registry| {
                        owned(&[
                            "permission",
                            "respond",
                            "1",
                            "--event-id",
                            "1",
                            "--from-member-id",
                            &registry.worker,
                            "--expected-run-id",
                            "1",
                            "--expected-owner-epoch",
                            "1",
                            "--request-token",
                            "0000000000000000000000000000000000000000000000000000000000000000",
                            "--decision",
                            "allow",
                        ])
                    },
                    expected: |_| Expected::Contains(NO_DATABASE),
                    exit_code: 1,
                },
                OrderCase {
                    name: "input reconcile",
                    database: Database::Missing,
                    args: |registry| {
                        owned(&[
                            "input",
                            "reconcile",
                            "1",
                            "--disposition",
                            "submitted",
                            "--evidence-ref",
                            "seen",
                            "--from-member-id",
                            &registry.worker,
                        ])
                    },
                    expected: |_| Expected::Exact(NO_DATABASE.into()),
                    exit_code: 1,
                },
            ]);
        }

        #[test]
        fn the_default_database_path_is_derived_before_command_validation() {
            struct HomeCase {
                name: &'static str,
                args: &'static [&'static str],
            }
            let cases = [
                HomeCase {
                    name: "before press key parsing",
                    args: &[
                        "member",
                        "press",
                        "2",
                        "Enter",
                        "Ctrl+1",
                        "--from-member-id",
                        "1",
                    ],
                },
                HomeCase {
                    name: "before message body validation",
                    args: &[
                        "message",
                        "send",
                        "--from-member-id",
                        "1",
                        "--to-member-id",
                        "2",
                        " ",
                    ],
                },
            ];
            for case in cases {
                let outcome = super::director_only_guard::run_guarded_child_with(
                    &owned(case.args),
                    |command| {
                        command.env_remove("HOME");
                    },
                );
                assert_eq!(outcome.code, Some(1), "{}: {}", case.name, outcome.stderr);
                assert_eq!(
                    outcome.stderr,
                    "Error: HOME is not set; cannot derive the default database path\n",
                    "{}",
                    case.name
                );
                assert_eq!(outcome.stdout, "", "{}", case.name);
            }
        }
    }
}
