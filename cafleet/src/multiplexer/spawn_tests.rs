use crate::multiplexer::{
    AnyMultiplexer, CommandRunner, HerdrMultiplexer, MultiplexerContext, PaneCleanup, RunError,
    TmuxMultiplexer,
    spawn::{
        Deadline, MonotonicClock, PaneSpawnRequest, SpawnExecution, SpawnMultiplexer,
        TimedCommandRunner,
    },
    test_support::herdr_envelope,
};
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
struct Clock {
    base: Instant,
    elapsed: Cell<Duration>,
    jump: Cell<Duration>,
}
impl Clock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
            jump: Cell::new(Duration::ZERO),
        }
    }
    fn advance(&self, cost: Duration) {
        self.elapsed.set(self.elapsed.get() + cost);
    }
}
impl MonotonicClock for Clock {
    fn now(&self) -> Instant {
        self.advance(self.jump.replace(Duration::ZERO));
        self.base + self.elapsed.get()
    }
}
struct Step {
    op: &'static str,
    cost: Duration,
    result: Result<String, RunError>,
    after: Duration,
}
fn ok(op: &'static str, ms: u64, text: impl Into<String>) -> Step {
    Step {
        op,
        cost: Duration::from_millis(ms),
        result: Ok(text.into()),
        after: Duration::ZERO,
    }
}
fn fail(op: &'static str) -> Step {
    Step {
        result: Err(RunError::Failed {
            stderr: "close/layout sentinel".into(),
        }),
        ..ok(op, 250, "")
    }
}
fn operation(argv: &[String]) -> &str {
    if argv[0] == "herdr" {
        &argv[2]
    } else {
        &argv[1]
    }
}
struct Script<'a> {
    clock: &'a Clock,
    steps: RefCell<VecDeque<Step>>,
    calls: RefCell<Vec<(Vec<String>, Duration)>>,
}
impl<'a> Script<'a> {
    fn new(clock: &'a Clock, steps: Vec<Step>) -> Self {
        Self {
            clock,
            steps: RefCell::new(steps.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
    fn budgets(&self) -> Vec<Duration> {
        self.calls.borrow().iter().map(|(_, d)| *d).collect()
    }
    fn ops(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .map(|(a, _)| operation(a).into())
            .collect()
    }
    fn drained(&self) {
        assert!(
            self.steps.borrow().is_empty(),
            "missing expected subprocess"
        );
    }
}
impl TimedCommandRunner for Script<'_> {
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        assert!(
            !timeout.is_zero(),
            "expired deadline must not invoke runner"
        );
        let step = self
            .steps
            .borrow_mut()
            .pop_front()
            .expect("unexpected extra subprocess");
        assert_eq!(operation(argv), step.op, "{argv:?}");
        self.calls.borrow_mut().push((argv.to_vec(), timeout));
        let result = if step.cost >= timeout {
            self.clock.advance(timeout);
            Err(RunError::Timeout)
        } else {
            self.clock.advance(step.cost);
            step.result
        };
        self.clock.jump.set(step.after);
        result
    }
}
// Legacy calls are allowed only for precondition context discovery. Any spawn,
// layout or cleanup routed through Option<u64> fails immediately.
struct ContextOnly;
impl CommandRunner for ContextOnly {
    fn binary_exists(&self, _: &str) -> bool {
        true
    }
    fn sleep(&self, _: f64) {
        panic!("unexpected legacy sleep")
    }
    fn run(&self, argv: &[String], _: Option<u64>) -> Result<String, RunError> {
        match operation(argv) {
            "current" => Ok(herdr_envelope(
                json!({"pane":{"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1"}}),
            )),
            "display-message" => Ok("fixture|@1|%0".into()),
            _ => panic!("bounded creation escaped into legacy runner: {argv:?}"),
        }
    }
}
fn mux(herdr: bool) -> AnyMultiplexer {
    let runner = Rc::new(ContextOnly);
    if herdr {
        AnyMultiplexer::Herdr(HerdrMultiplexer::new(
            runner,
            HashMap::from([("HERDR_ENV".into(), "1".into())]),
        ))
    } else {
        AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            runner,
            HashMap::from([
                ("TMUX".into(), "fixture".into()),
                ("TMUX_PANE".into(), "%0".into()),
            ]),
        ))
    }
}
fn reference(herdr: bool) -> MultiplexerContext {
    MultiplexerContext {
        session: "fixture".into(),
        window_id: if herdr { "w1:t1" } else { "@1" }.into(),
        pane_id: if herdr { "w1:p1" } else { "%0" }.into(),
    }
}
fn herdr_steps(run_ms: u64) -> Vec<Step> {
    vec![
        ok(
            "list",
            125,
            herdr_envelope(json!({"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1"}]})),
        ),
        ok(
            "split",
            250,
            herdr_envelope(json!({"pane":{"pane_id":"w1:p9"}})),
        ),
        ok("run", run_ms, ""),
    ]
}
fn split(
    herdr: bool,
    clock: &Clock,
    script: &Script<'_>,
    budget: Duration,
) -> Result<String, crate::multiplexer::MultiplexerError> {
    let reference = reference(herdr);
    let argv = vec!["claude".into(), "prompt with spaces".into()];
    let env = vec![("CAFLEET_DATABASE_URL".into(), "sqlite:///fixture".into())];
    mux(herdr).split_prepared(
        &PaneSpawnRequest {
            reference: &reference,
            env: &env,
            command: &argv,
            cwd: Some(Path::new(env!("CARGO_MANIFEST_DIR"))),
        },
        &Deadline::after(clock, budget),
        &SpawnExecution {
            clock,
            runner: script,
        },
    )
}
#[test]
fn deadline_preserves_fractional_remaining_and_expires_at_exact_boundary() {
    let clock = Clock::new();
    let deadline = Deadline::after(&clock, Duration::from_secs(30));
    clock.advance(Duration::from_micros(125_007));
    assert_eq!(
        deadline.remaining(&clock),
        Ok(Duration::from_micros(29_874_993))
    );
    clock.advance(Duration::from_micros(29_874_993));
    assert_eq!(deadline.remaining(&clock), Err(RunError::Timeout));
    clock.advance(Duration::from_secs(5));
    assert_eq!(deadline.remaining(&clock), Err(RunError::Timeout));
}
#[test]
fn both_backends_skip_the_runner_when_deadline_is_already_exhausted() {
    for herdr in [false, true] {
        let clock = Clock::new();
        let script = Script::new(&clock, vec![]);
        let error = split(herdr, &clock, &script, Duration::ZERO).unwrap_err();
        assert!(error.to_string().starts_with(if herdr {
            "herdr spawn deadline exceeded:"
        } else {
            "tmux spawn deadline exceeded:"
        }));
        assert!(script.calls.borrow().is_empty());
    }
}
#[test]
fn herdr_forwards_one_decreasing_fractional_budget_and_prepared_cwd_env() {
    let clock = Clock::new();
    let script = Script::new(&clock, herdr_steps(375));
    assert_eq!(
        split(true, &clock, &script, Duration::from_secs(30)).unwrap(),
        "w1:p9"
    );
    assert_eq!(
        script.budgets(),
        [
            Duration::from_secs(30),
            Duration::from_millis(29_875),
            Duration::from_millis(29_625)
        ]
    );
    let calls = script.calls.borrow();
    assert!(
        calls[1]
            .0
            .windows(2)
            .any(|w| w == ["--cwd", env!("CARGO_MANIFEST_DIR")])
    );
    assert!(
        calls[1]
            .0
            .windows(2)
            .any(|w| w == ["--env", "CAFLEET_DATABASE_URL=sqlite:///fixture"])
    );
    assert_eq!(calls[2].0[4], "claude 'prompt with spaces'");
    script.drained();
}
#[test]
fn tmux_layout_ordinary_failure_is_best_effort_with_decreasing_budget() {
    let clock = Clock::new();
    let script = Script::new(
        &clock,
        vec![ok("split-window", 125, "%9\n"), fail("select-layout")],
    );
    assert_eq!(
        split(false, &clock, &script, Duration::from_secs(30)).unwrap(),
        "%9"
    );
    assert_eq!(
        script.budgets(),
        [Duration::from_secs(30), Duration::from_millis(29_875)]
    );
    let calls = script.calls.borrow();
    assert_eq!(
        &calls[0].0[..8],
        [
            "tmux",
            "split-window",
            "-t",
            "@1",
            "-P",
            "-F",
            "#{pane_id}",
            "-d"
        ]
    );
    assert_eq!(
        &calls[0].0[8..],
        [
            "-e",
            "CAFLEET_DATABASE_URL=sqlite:///fixture",
            "claude",
            "prompt with spaces"
        ]
    );
    script.drained();
}
#[test]
fn tmux_layout_timeout_fails_spawn_and_uses_independent_five_second_kill() {
    let clock = Clock::new();
    let script = Script::new(
        &clock,
        vec![
            ok("split-window", 125, "%9"),
            ok("select-layout", 30_000, ""),
            ok("kill-pane", 100, ""),
        ],
    );
    let error = split(false, &clock, &script, Duration::from_secs(30)).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("tmux command timed out after 29.875s:")
    );
    assert_eq!(
        error.pane_cleanup(),
        Some(&PaneCleanup::Attempted {
            pane_id: "%9".into(),
            error: None
        })
    );
    assert_eq!(
        script.budgets(),
        [
            Duration::from_secs(30),
            Duration::from_millis(29_875),
            Duration::from_secs(5)
        ]
    );
    assert_eq!(
        script.calls.borrow()[2].0,
        ["tmux", "kill-pane", "-t", "%9"]
    );
    script.drained();
}
#[test]
fn successful_last_subprocess_cannot_transfer_after_the_deadline_expires() {
    for herdr in [false, true] {
        let clock = Clock::new();
        let mut steps = if herdr {
            herdr_steps(29_000)
        } else {
            vec![
                ok("split-window", 125, "%9"),
                ok("select-layout", 29_000, ""),
            ]
        };
        steps.last_mut().unwrap().after = Duration::from_secs(1);
        steps.push(ok(if herdr { "close" } else { "kill-pane" }, 100, ""));
        let script = Script::new(&clock, steps);
        let error = split(herdr, &clock, &script, Duration::from_secs(30)).unwrap_err();
        assert!(error.to_string().starts_with(if herdr {
            "herdr spawn deadline exceeded"
        } else {
            "tmux spawn deadline exceeded"
        }));
        assert!(matches!(
            error.pane_cleanup(),
            Some(PaneCleanup::Attempted { error: None, .. })
        ));
        assert_eq!(*script.budgets().last().unwrap(), Duration::from_secs(5));
        script.drained();
    }
}
fn column_steps(resize: Step) -> Vec<Step> {
    vec![
        ok(
            "list",
            125,
            herdr_envelope(
                json!({"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1"},{"pane_id":"w1:p2","tab_id":"w1:t1"}]}),
            ),
        ),
        ok(
            "split",
            250,
            herdr_envelope(json!({"pane":{"pane_id":"w1:p9"}})),
        ),
        ok(
            "layout",
            375,
            herdr_envelope(json!({"layout":{"panes":[
            {"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":60,"height":30}},
            {"pane_id":"w1:p2","rect":{"x":60,"y":0,"width":60,"height":15}},
            {"pane_id":"w1:p9","rect":{"x":60,"y":15,"width":60,"height":15}}
        ],"splits":[{"direction":"right","rect":{"x":0,"y":0},"ratio":0.5},{"direction":"down","rect":{"x":60,"y":15},"ratio":0.7}]}})),
        ),
        resize,
    ]
}
#[test]
fn herdr_resize_ordinary_failure_continues_run_with_remaining_budget() {
    let clock = Clock::new();
    let mut steps = column_steps(fail("resize"));
    steps.push(ok("run", 125, ""));
    let script = Script::new(&clock, steps);
    assert_eq!(
        split(true, &clock, &script, Duration::from_secs(30)).unwrap(),
        "w1:p9"
    );
    assert_eq!(
        script.budgets(),
        [30_000, 29_875, 29_625, 29_250, 29_000].map(Duration::from_millis)
    );
    script.drained();
}
#[test]
fn herdr_resize_timeout_compensates_without_running_the_agent() {
    let clock = Clock::new();
    let mut steps = column_steps(ok("resize", 30_000, ""));
    steps.push(ok("close", 125, ""));
    let script = Script::new(&clock, steps);
    let error = split(true, &clock, &script, Duration::from_secs(30)).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("herdr command timed out after 29.25s:")
    );
    assert_eq!(
        error.pane_cleanup(),
        Some(&PaneCleanup::Attempted {
            pane_id: "w1:p9".into(),
            error: None
        })
    );
    assert_eq!(script.ops(), ["list", "split", "layout", "resize", "close"]);
    assert_eq!(*script.budgets().last().unwrap(), Duration::from_secs(5));
    script.drained();
}
#[test]
fn expiry_between_subprocesses_skips_next_run_and_compensates_known_pane() {
    let clock = Clock::new();
    let mut steps = herdr_steps(0);
    steps.pop();
    steps[1].after = Duration::from_secs(30);
    steps.push(ok("close", 125, ""));
    let script = Script::new(&clock, steps);
    let error = split(true, &clock, &script, Duration::from_secs(30)).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("herdr spawn deadline exceeded: herdr pane run")
    );
    assert_eq!(
        error.pane_cleanup(),
        Some(&PaneCleanup::Attempted {
            pane_id: "w1:p9".into(),
            error: None
        })
    );
    assert_eq!(script.ops(), ["list", "split", "close"]);
    assert_eq!(*script.budgets().last().unwrap(), Duration::from_secs(5));
    script.drained();
}
#[test]
fn bounded_close_is_direct_and_honors_missing_tolerance_without_relayout() {
    for herdr in [false, true] {
        for ignore in [false, true] {
            let clock = Clock::new();
            let mut missing = fail(if herdr { "close" } else { "kill-pane" });
            missing.result = Err(RunError::Failed {
                stderr: if herdr {
                    crate::multiplexer::test_support::herdr_error_stderr("pane_not_found")
                } else {
                    "can't find pane: %9".into()
                },
            });
            let script = Script::new(&clock, vec![missing]);
            let result = mux(herdr).kill_pane_with_deadline(
                if herdr { "w1:p9" } else { "%9" },
                ignore,
                &Deadline::after(&clock, Duration::from_secs(5)),
                &SpawnExecution {
                    clock: &clock,
                    runner: &script,
                },
            );
            assert_eq!(result.is_ok(), ignore);
            assert_eq!(script.budgets(), [Duration::from_secs(5)]);
            script.drained();
        }
    }
}
