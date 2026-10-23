mod common;

use common::{Cli, code, stderr, stdout, write_file};

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
    assert_eq!(cli.shim_count("send-keys"), 0);
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
    for (source, body) in [("file", "printf file\n"), ("stdin", "printf stdin\r\n")] {
        let (cli, _, worker) = fixture();
        let id = worker.to_string();
        let output = if source == "file" {
            let file = write_file(&cli.home.path().join("command.txt"), body.as_bytes());
            cli.run(&["member", "exec", &id, "--file", &file, "--json"])
        } else {
            cli.run_with_stdin(&["member", "exec", &id, "--file", "-", "--json"], body)
        };
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(payload["command"], body.trim_end_matches(['\r', '\n']));
        let invalid = cli.run_with_stdin(
            &["member", "exec", &id, "--file", "-"],
            "printf example\n\n",
        );
        assert_eq!(code(&invalid), 2);
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
