pub(crate) use crate::cli::member::MemberCapture;
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
        Command::Member(M::Recover {
            member_id,
            director_member_id,
            ..
        }) => (
            "member recover",
            MEMBER_FLEET,
            member_id,
            director_member_id,
        ),
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
            runtime::permission_preflight(&command)
                .map_err(|source| runtime::permission_error(&command, source))?;
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
    /// Runs the libtest child `test` of this binary with `command`'s environment, capturing its
    /// output and bounding it to ten seconds.
    pub(super) fn run_test_child(
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
        crate::db::initialize(&mut conn).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        conn.execute_batch(mutation).unwrap();
        conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
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
                timeout: None,
                codex_thread: None,
            })
        ));
        match parse(&["message", "listen", "17", "--codex-thread", "THREAD"]) {
            Command::Message(message::MessageCommand::Listen {
                member_id: 17,
                timeout: None,
                codex_thread,
            }) => assert_eq!(codex_thread.as_deref(), Some("THREAD")),
            _ => panic!("expected a forwarding listener"),
        }
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
        for (argument, values) in [
            (
                "decision",
                crate::broker::runtime::PermissionDecision::VALUES,
            ),
            (
                "decision_source",
                crate::broker::runtime::DecisionSource::VALUES,
            ),
            (
                "director_mode",
                crate::broker::runtime::DirectorMode::VALUES,
            ),
        ] {
            assert_eq!(
                possible_values(&["permission", "respond"], argument),
                values,
                "{argument}"
            );
            assert!(
                respond.contains(&format!("[possible values: {}]", values.join(", "))),
                "{argument} help lists {values:?}: {respond}"
            );
        }
    }

    #[test]
    fn member_runner_dispatch_rejects_an_unallocated_descriptor_before_runtime_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let descriptor = dir.path().join("unallocated.json");
        std::fs::write(&descriptor, b"{\"schema_version\":1}").unwrap();
        let settings = crate::config::Settings::for_database(&path);
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
                    "member recover",
                    owned(&["member", "recover", subject, "--from-member-id", caller]),
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
        fn director_only_names_exactly_the_six_guarded_commands() {
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
                    .input(&owner, "prompt:guard", InputKind::Prompt, None)
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

        struct ChildOutcome {
            code: Option<i32>,
            stdout: String,
            stderr: String,
        }

        fn run_guarded_child(args: &[String]) -> ChildOutcome {
            let output = crate::cli::test_support::run_test_child(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .env(CHILD_ARGV, serde_json::json!(args).to_string()),
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
}
