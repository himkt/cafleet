mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Output;

use common::{Cli, code, stderr, stdout, write_executable, write_file};

#[derive(Clone, Copy, Debug)]
enum CommandSource {
    File,
    Stdin,
}

fn from_source(cli: &Cli, worker: i64, source: CommandSource, body: &[u8]) -> Output {
    let id = worker.to_string();
    match source {
        CommandSource::File => {
            let file = write_file(&cli.home.path().join("native command.txt"), body);
            cli.run(&["member", "exec", &id, "--file", &file, "--json"])
        }
        CommandSource::Stdin => {
            cli.run_with_stdin_bytes(&["member", "exec", &id, "--file", "-", "--json"], body)
        }
    }
}

fn files_under(root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            files.extend(files_under(&entry.path()));
        } else {
            files.insert(entry.path());
        }
    }
    files
}

fn fixture() -> (Cli, i64, i64) {
    let cli = Cli::new();
    let (fleet, director) = cli.seeded_fleet();
    let worker = cli.seed_member(fleet, "worker");
    cli.set_pane_capture(
        &format!("%{}", worker + 4),
        "Ready.\n────────────\n❯ \x1b[0m\n────────────\n? for shortcuts",
    );
    (cli, director, worker)
}

#[test]
fn native_exec_requires_exactly_one_source_and_one_nonempty_line() {
    let (cli, _, worker) = fixture();
    let file = write_file(&cli.home.path().join("command.txt"), b"printf file");
    for arguments in [
        vec![],
        vec!["printf inline", "--file", file.as_str()],
        vec![""],
        vec!["  "],
        vec!["a\nb"],
        vec!["a\rb"],
        vec!["a\tb"],
        vec!["a\u{1b}b"],
        vec!["a\u{7f}b"],
    ] {
        let id = worker.to_string();
        let mut args = vec!["member", "exec", id.as_str()];
        args.extend(arguments);
        let output = cli.run(&args);
        assert_eq!(code(&output), 2, "{}", stderr(&output));
    }
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn native_exec_dispatches_the_exact_command_and_reduced_metadata_once() {
    let (cli, _, worker) = fixture();
    let command = "  printf '%s' 'literal $HOME; !'  ";
    let output = cli.run(&["member", "exec", &worker.to_string(), command, "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(
        payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "member_id",
            "pane_id",
            "coding_agent",
            "command",
            "dispatched"
        ]
    );
    let pane = format!("%{}", worker + 4);
    assert_eq!(
        payload,
        serde_json::json!({"member_id":worker,"pane_id":pane,
        "coding_agent":"claude","command":command,"dispatched":true})
    );
    let writes: Vec<_> = cli
        .shim_calls()
        .into_iter()
        .filter(|line| line.starts_with("send-keys"))
        .collect();
    assert_eq!(
        writes,
        [
            format!("send-keys -t {pane} -l ! {command}"),
            format!("send-keys -t {pane} Enter")
        ]
    );
}

#[test]
fn native_exec_file_and_stdin_remove_at_most_one_terminal_line_ending() {
    let command = "  printf '%s' 'café 界 $HOME; !'  ";
    for source in [CommandSource::File, CommandSource::Stdin] {
        for ending in ["", "\n", "\r\n"] {
            let (cli, _, worker) = fixture();
            let body = format!("{command}{ending}");
            let output = from_source(&cli, worker, source, body.as_bytes());
            assert_eq!(code(&output), 0, "{source:?}: {}", stderr(&output));
            let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
            assert_eq!(payload["command"], command);
            let pane = format!("%{}", worker + 4);
            let writes: Vec<_> = cli
                .shim_calls()
                .into_iter()
                .filter(|line| line.starts_with("send-keys"))
                .collect();
            assert_eq!(
                writes,
                [
                    format!("send-keys -t {pane} -l ! {command}"),
                    format!("send-keys -t {pane} Enter"),
                ]
            );
        }
    }
}

#[test]
fn native_exec_file_and_stdin_reject_blank_multiline_and_control_bytes_before_io() {
    let mut bodies = [
        "",
        " \u{2003}",
        "\n",
        "\r\n",
        "printf example\r",
        "printf example\n\n",
        "printf example\r\n\r\n",
        "printf example\r\n\n",
        "first\nsecond",
    ]
    .map(|body| body.as_bytes().to_vec())
    .to_vec();
    bodies.extend((0..=31).chain([127]).map(|byte| vec![b'x', byte, b'y']));
    for source in [CommandSource::File, CommandSource::Stdin] {
        let (cli, _, worker) = fixture();
        for body in &bodies {
            let output = from_source(&cli, worker, source, body);
            assert_eq!(code(&output), 2, "{source:?} {body:?}: {}", stderr(&output));
            assert_eq!(stdout(&output), "");
            assert_eq!(
                stderr(&output),
                "Error: command must be one non-empty line without control characters.\n"
            );
        }
        assert!(cli.shim_calls().is_empty());
    }
}

#[test]
fn native_exec_rejects_invalid_utf8_from_file_and_stdin_with_source_context() {
    for source in [CommandSource::File, CommandSource::Stdin] {
        let (cli, _, worker) = fixture();
        let label = match source {
            CommandSource::File => cli
                .home
                .path()
                .join("native command.txt")
                .display()
                .to_string(),
            CommandSource::Stdin => "-".to_string(),
        };
        for bytes in [vec![0xff], vec![0xc3, 0x28], vec![0xf0, 0x9f]] {
            let output = from_source(&cli, worker, source, &bytes);
            assert_eq!(code(&output), 1, "{source:?}: {}", stderr(&output));
            assert_eq!(stdout(&output), "");
            assert_eq!(
                stderr(&output),
                format!("Error: --file {label}: file is not valid UTF-8.\n")
            );
        }
        assert!(cli.shim_calls().is_empty());
    }
}

#[test]
fn native_exec_reports_missing_or_directory_command_files_before_pane_io() {
    let (cli, _, worker) = fixture();
    for path in [
        cli.home.path().join("missing command.txt"),
        cli.home.path().to_path_buf(),
    ] {
        let path = path.to_str().unwrap();
        let output = cli.run(&[
            "member",
            "exec",
            &worker.to_string(),
            "--file",
            path,
            "--json",
        ]);
        assert_eq!(code(&output), 1);
        assert_eq!(stdout(&output), "");
        assert_eq!(
            stderr(&output),
            format!("Error: --file {path}: file does not exist or is not a regular file.\n")
        );
    }
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn native_exec_reports_unknown_deregistered_and_absent_placement_targets() {
    for state in ["unknown", "deregistered", "no-placement"] {
        let (cli, _, worker) = fixture();
        let (subject, cause) = match state {
            "unknown" => (999, "Member 999 not found".to_string()),
            "deregistered" => {
                assert!(cafleet::broker::deregister_member(&mut cli.sqlite(), worker).unwrap());
                (worker, format!("Member {worker} not found"))
            }
            "no-placement" => {
                assert_eq!(
                    cli.sqlite()
                        .execute("DELETE FROM member_placements WHERE member_id=?1", [worker])
                        .unwrap(),
                    1
                );
                (
                    worker,
                    format!(
                        "member {worker} has no placement row; it was not spawned via `cafleet member create`."
                    ),
                )
            }
            _ => unreachable!(),
        };
        let output = cli.run(&[
            "member",
            "exec",
            &subject.to_string(),
            "printf example",
            "--json",
        ]);
        assert_eq!(code(&output), 1);
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), format!("Error: {cause}\n"));
        assert!(cli.shim_calls().is_empty());
    }
}

#[test]
fn native_exec_rejects_director_and_pending_placement_without_typing() {
    let (cli, director, worker) = fixture();
    let output = cli.run(&["member", "exec", &director.to_string(), "printf example"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("cannot exec in the Director's own pane"),
        "{}",
        stderr(&output)
    );
    cli.sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [worker],
        )
        .unwrap();
    let output = cli.run(&["member", "exec", &worker.to_string(), "printf example"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "member {worker} has no pane yet (pending placement) — nothing to exec."
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(cli.shim_count("send-keys"), 0);
}

#[test]
fn native_exec_frames_each_backend_command_through_both_transports() {
    for (agent, capture) in [
        (
            "claude",
            include_str!("fixtures/pane-readiness/claude-placeholder.ansi"),
        ),
        (
            "codex",
            include_str!("fixtures/pane-readiness/codex-placeholder.ansi"),
        ),
        (
            "opencode",
            include_str!("fixtures/pane-readiness/opencode-placeholder.ansi"),
        ),
    ] {
        for transport in ["tmux", "herdr"] {
            let (mut cli, _, worker) = fixture();
            let pane = if transport == "tmux" {
                format!("%{}", worker + 4)
            } else {
                "w1:p2".to_string()
            };
            cli.sqlite().execute(
                "UPDATE member_placements SET backend=?1,mux_pane_id=?2,coding_agent=?3 WHERE member_id=?4",
                rusqlite::params![transport, pane, agent, worker],
            ).unwrap();
            cli.set_pane_capture(&pane, capture);
            if transport == "herdr" {
                cli.set_env("CAFLEET_MULTIPLEXER", "herdr");
                cli.set_env("HERDR_ENV", "native-exec-test");
                write_executable(
                    &cli.shim_dir.join("herdr"),
                    r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
if [ "$1" = pane ] && [ "$2" = read ]; then
    if [ ! -f "$CAFLEET_TEST_TMUX_CAPTURE_DIR/$3" ]; then exit 1; fi
    while IFS= read -r line || [ -n "$line" ]; do
        printf '%s\n' "$line"
    done < "$CAFLEET_TEST_TMUX_CAPTURE_DIR/$3"
fi
"#,
                );
            }
            let command = "  printf '%s' 'café $HOME; !'  ";
            let output = cli.run(&["member", "exec", &worker.to_string(), command, "--json"]);
            assert_eq!(code(&output), 0, "{agent}/{transport}: {}", stderr(&output));
            let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
            assert_eq!(payload["coding_agent"], agent);
            assert_eq!(payload["command"], command);
            assert_eq!(payload["dispatched"], true);
            let calls = cli.shim_calls();
            let writes: Vec<_> = calls
                .into_iter()
                .filter(|line| line.starts_with("send-keys") || line.starts_with("pane send-"))
                .collect();
            let expected = if transport == "tmux" {
                vec![
                    format!("send-keys -t {pane} -l ! {command}"),
                    format!("send-keys -t {pane} Enter"),
                ]
            } else {
                vec![
                    format!("pane send-text {pane} ! {command}"),
                    format!("pane send-keys {pane} enter"),
                ]
            };
            assert_eq!(writes, expected, "{agent}/{transport}");
        }
    }
}

#[test]
fn native_exec_text_reports_submission_with_unchanged_broker_and_only_input_artifacts() {
    let (cli, _, worker) = fixture();
    let mut locks = cli.db_path().into_os_string();
    locks.push(".pane-locks");
    let locks = PathBuf::from(locks);
    let stable_files = |files: BTreeSet<PathBuf>| {
        files
            .into_iter()
            .filter(|path| path != &cli.shim_log && !path.starts_with(&locks))
            .collect::<BTreeSet<_>>()
    };
    let before_files = stable_files(files_under(cli.home.path()));
    let before_database = std::fs::read(cli.db_path()).unwrap();
    let command = format!(
        "printf native-output > '{}'",
        cli.home.path().join("native-result.txt").display()
    );
    let output = cli.run_in(
        cli.home.path(),
        &["member", "exec", &worker.to_string(), &command],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    let pane = format!("%{}", worker + 4);
    assert_eq!(
        stdout(&output),
        format!(
            "Dispatched native command to member {worker} ({pane}). Inspect the pane for its result.\n"
        )
    );
    assert_eq!(std::fs::read(cli.db_path()).unwrap(), before_database);
    assert_eq!(stable_files(files_under(cli.home.path())), before_files);
    assert_eq!(cli.shim_count("send-keys"), 2);
}

#[test]
fn native_exec_uses_the_standard_parser_for_undeclared_options() {
    let cli = Cli::new();
    let output = cli.run(&["member", "exec", "1", "printf example", "--wait"]);
    assert_eq!(code(&output), 2);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("unexpected argument '--wait' found"),
        "{}",
        stderr(&output)
    );
    assert!(!cli.db_path().exists());
    assert!(cli.shim_calls().is_empty());
}
