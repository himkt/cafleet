use super::*;
use crate::broker::records::Placement;
use crate::error::CafleetError;
use crate::input::Deadline;
use recording::{FakeRunner, argv, env, herdr_envelope};
use std::cell::RefCell;
use std::time::Instant;

fn parse(tokens: &[&str]) -> PressKeys {
    PressKeys::parse(
        &tokens
            .iter()
            .map(|token| (*token).into())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn usage(tokens: &[String], expected: &str) {
    let failure = match PressKeys::parse(tokens) {
        Ok(_) => panic!("the key vector is outside the common grammar"),
        Err(failure) => failure,
    };
    assert!(matches!(&failure, CafleetError::Usage(message) if message == expected));
    assert_eq!(failure.exit_code(), 2);
}

#[test]
fn step5_press_keys_validate_the_complete_common_grammar_and_byte_count_bounds() {
    let keys = parse(&[
        "A",
        "a",
        "+",
        "-",
        ";",
        "!",
        "~",
        "sPaCe",
        "ESCAPE",
        "enter",
        "Tab",
        "Backspace",
        "up",
        "down",
        "left",
        "right",
        "f1",
        "F12",
        "cTrL+A",
        "CTRL+Z",
        "sHiFt+tAb",
    ]);
    assert_eq!(
        keys.canonical(),
        argv(&[
            "A",
            "a",
            "+",
            "-",
            ";",
            "!",
            "~",
            "Space",
            "Escape",
            "Enter",
            "Tab",
            "Backspace",
            "Up",
            "Down",
            "Left",
            "Right",
            "F1",
            "F12",
            "Ctrl+a",
            "Ctrl+z",
            "Shift+Tab",
        ])
    );
    assert_eq!(
        PressKeys::parse(&vec!["a".into(); 64])
            .unwrap()
            .canonical()
            .len(),
        64
    );
    usage(&[], "press requires between 1 and 64 keys");
    usage(
        &vec!["a".into(); 65],
        "press requires between 1 and 64 keys",
    );
    let long = "é".repeat(17);
    usage(
        &["Enter".into(), long],
        "press key at position 2 exceeds 32 UTF-8 bytes",
    );
    for invalid in ["F0", "F13", "Ctrl+1", " ", "é"] {
        usage(
            &["a".into(), invalid.into()],
            &format!(
                "invalid press key at position 2: {}",
                serde_json::to_string(invalid).unwrap()
            ),
        );
    }
}

fn adapter(backend: &str, runner: Rc<FakeRunner>) -> AnyMultiplexer {
    match backend {
        "tmux" => AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            runner,
            env(&[("TMUX", "fixture"), ("TMUX_PANE", "%0")]),
        )),
        "herdr" => AnyMultiplexer::Herdr(HerdrMultiplexer::new(
            runner,
            env(&[("HERDR_ENV", "fixture")]),
        )),
        _ => panic!("fixture backend"),
    }
}

#[test]
fn step5_press_adapters_request_only_the_ordered_keys_and_qualify_stdout_acknowledgment() {
    let keys = parse(&[
        "A",
        "a",
        "+",
        "-",
        ";",
        "Space",
        "Escape",
        "Enter",
        "Tab",
        "Backspace",
        "Up",
        "F12",
        "Ctrl+A",
        "Shift+Tab",
    ]);
    for (backend, pane, expected) in [
        (
            "tmux",
            "%2",
            argv(&[
                "tmux",
                "send-keys",
                "-t",
                "%2",
                "0x41",
                "0x61",
                "0x2b",
                "0x2d",
                "0x3b",
                "Space",
                "Escape",
                "Enter",
                "Tab",
                "BSpace",
                "Up",
                "F12",
                "C-a",
                "BTab",
            ]),
        ),
        (
            "herdr",
            "w1:p2",
            argv(&[
                "herdr",
                "pane",
                "send-keys",
                "w1:p2",
                "A",
                "a",
                "plus",
                "minus",
                "semicolon",
                "space",
                "esc",
                "enter",
                "tab",
                "backspace",
                "up",
                "f12",
                "ctrl+a",
                "shift+tab",
            ]),
        ),
    ] {
        let runner = FakeRunner::with_binary(backend);
        runner.respond(Ok(" \n\t".into()));
        adapter(backend, runner.clone())
            .send_keys(pane, &keys, Deadline::new())
            .unwrap();
        let mut actual = runner.run_argvs();
        assert_eq!(actual.len(), 1);
        if backend == "tmux" {
            for value in &mut actual[0][4..9] {
                assert!(value.starts_with("0x") && value.len() == 4);
                value.make_ascii_lowercase();
            }
        }
        assert_eq!(actual, vec![expected]);
        assert!(
            runner
                .events()
                .iter()
                .all(|event| matches!(event, recording::Event::Run { .. }))
        );
        assert!(runner.responses.borrow().is_empty());

        let unreadable = FakeRunner::with_binary(backend);
        unreadable.respond(Ok("unexpected acknowledgment".into()));
        let failure = adapter(backend, unreadable.clone())
            .send_keys(pane, &keys, Deadline::new())
            .expect_err("non-whitespace stdout leaves actual effects uncertain");
        assert!(failure.submission_started());
        assert_eq!(unreadable.run_argvs().len(), 1);
    }
}

fn placement(backend: &str) -> Placement {
    Placement {
        backend: backend.into(),
        mux_session: "main".into(),
        mux_window_id: "@1".into(),
        mux_pane_id: Some(if backend == "tmux" { "%2" } else { "w1:p2" }.into()),
        coding_agent: "codex".into(),
        created_at: "2026-10-27T10:33:00+00:00".into(),
    }
}

#[test]
fn step5_press_target_validation_rejects_a_wrong_tmux_tuple_and_ambiguous_herdr_target() {
    let tmux = FakeRunner::with_binary("tmux");
    tmux.respond(Ok("main|@1|%0\n".into()));
    tmux.respond(Ok("%2|@9|main\n".into()));
    assert!(
        adapter("tmux", tmux.clone())
            .validate_key_target(&placement("tmux"), Deadline::new())
            .is_err()
    );
    assert!(
        tmux.run_argvs()
            .iter()
            .all(|args| args[1] == "display-message")
    );

    let pane = serde_json::json!({"workspace_id":"main","tab_id":"@1","pane_id":"w1:p2"});
    let valid = FakeRunner::with_binary("herdr");
    valid.respond(Ok(herdr_envelope(
        serde_json::json!({"pane":{"workspace_id":"main","tab_id":"@1","pane_id":"w1:p0"}}),
    )));
    valid.respond(Ok(herdr_envelope(
        serde_json::json!({"panes":[pane.clone()]}),
    )));
    adapter("herdr", valid.clone())
        .validate_key_target(&placement("herdr"), Deadline::new())
        .unwrap();
    assert_eq!(
        valid.run_argvs(),
        vec![
            argv(&["herdr", "pane", "current"]),
            argv(&["herdr", "pane", "list"])
        ]
    );
    let duplicate = FakeRunner::with_binary("herdr");
    duplicate.respond(Ok(herdr_envelope(
        serde_json::json!({"pane":{"workspace_id":"main","tab_id":"@1","pane_id":"w1:p0"}}),
    )));
    duplicate.respond(Ok(herdr_envelope(
        serde_json::json!({"panes":[pane.clone(),pane]}),
    )));
    assert!(
        adapter("herdr", duplicate.clone())
            .validate_key_target(&placement("herdr"), Deadline::new())
            .is_err()
    );
    assert_eq!(duplicate.run_argvs().len(), 2);
}

struct AbsoluteRecorder(RefCell<Vec<Instant>>);

impl CommandRunner for AbsoluteRecorder {
    fn binary_exists(&self, _: &str) -> bool {
        true
    }
    fn run(&self, _: &[String], _: Option<u64>) -> Result<String, RunError> {
        panic!("absolute execution retains its Instant")
    }
    fn run_until(&self, _: &[String], deadline: Instant) -> Result<String, TimedRunError> {
        self.0.borrow_mut().push(deadline);
        Ok(String::new())
    }
    fn sleep(&self, _: f64) {
        panic!("absolute execution has no phase sleep")
    }
}

#[test]
fn step5_press_deadline_runner_clips_the_actual_absolute_call_and_refuses_expiry() {
    let recorder = Rc::new(AbsoluteRecorder(RefCell::default()));
    let original = Instant::now() + Duration::from_secs(2);
    let bounded = DeadlineRunner::wrap(recorder.clone(), Deadline::until(original));
    assert!(
        bounded
            .run_until(&argv(&["fixture"]), original + Duration::from_secs(5))
            .is_ok()
    );
    let earlier = original - Duration::from_secs(1);
    assert!(bounded.run_until(&argv(&["fixture"]), earlier).is_ok());
    assert_eq!(*recorder.0.borrow(), vec![original, earlier]);
    let expired = DeadlineRunner::wrap(recorder.clone(), Deadline::until(Instant::now()));
    let failure = match expired.run_until(&argv(&["fixture"]), original) {
        Ok(_) => panic!("expired work starts no underlying command"),
        Err(failure) => failure,
    };
    assert!(matches!(failure.source, RunError::NotStarted(_)));
    assert_eq!(recorder.0.borrow().len(), 2);
}
