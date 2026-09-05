//! Step 8 deadline/preparation contracts. Phase B connects this from lib.rs.
use crate::{
    broker::fleets::{BootstrapEvent, BootstrapHooks},
    cli::{
        self,
        creation::{
            CleanupEvent, CreationHooks, FleetCreateOptions, MemberCreateOptions, PreparedSpawn,
            SpawnPreparation,
            test_support::{Event, Fixture},
        },
    },
    coding_agent::{coding_agent, test_support::FakeProbe},
    error::CafleetError,
    multiplexer::{
        AnyMultiplexer, CommandRunner, HerdrMultiplexer, MultiplexerContext, PaneCleanup, RunError,
        TmuxMultiplexer,
        spawn::{
            Deadline, MonotonicClock, PaneSpawnRequest, SpawnExecution, SpawnMultiplexer,
            TimedCommandRunner,
        },
        test_support::herdr_envelope,
    },
};
use rusqlite::Connection;
use serde_json::{Value, json};
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
    fixture: Option<&'a Fixture>,
}
impl<'a> Script<'a> {
    fn new(clock: &'a Clock, steps: Vec<Step>, fixture: Option<&'a Fixture>) -> Self {
        Self {
            clock,
            steps: RefCell::new(steps.into()),
            calls: RefCell::new(Vec::new()),
            fixture,
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
        if let Some(f) = self.fixture {
            if step.op == "close" {
                assert_eq!(&argv[..4], ["herdr", "pane", "close", "w1:p9"]);
                let conn = f.conn();
                conn.busy_timeout(Duration::ZERO).unwrap();
                let available = match conn.execute_batch("BEGIN IMMEDIATE") {
                    Ok(()) => {
                        conn.execute_batch("ROLLBACK").unwrap();
                        true
                    }
                    Err(rusqlite::Error::SqliteFailure(e, _))
                        if e.code == rusqlite::ErrorCode::DatabaseBusy =>
                    {
                        false
                    }
                    other => panic!("unexpected real lock result {other:?}"),
                };
                f.events
                    .borrow_mut()
                    .push(Event::WriteLockAvailable(available));
            }
        }
        let result = if step.cost >= timeout {
            self.clock.advance(timeout);
            Err(RunError::Timeout)
        } else {
            self.clock.advance(step.cost);
            step.result
        };
        self.clock.jump.set(step.after);
        if let Some(f) = self.fixture {
            f.events.borrow_mut().push(Event::Command {
                operation: step.op.into(),
                succeeded: result.is_ok(),
            });
        }
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
        let script = Script::new(&clock, vec![], None);
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
    let script = Script::new(&clock, herdr_steps(375), None);
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
        None,
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
        None,
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
        let script = Script::new(&clock, steps, None);
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
    let script = Script::new(&clock, steps, None);
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
    let script = Script::new(&clock, steps, None);
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
struct Hooks<'a> {
    fixture: &'a Fixture,
    prepared: Cell<usize>,
    member: bool,
}
impl BootstrapHooks for Hooks<'_> {
    fn observe(&self, event: BootstrapEvent) {
        self.fixture.observe(event);
    }
    fn after_rollback(&self, fleet: Option<i64>) -> Result<(), CafleetError> {
        self.fixture.after_rollback(fleet)
    }
}
impl CreationHooks for Hooks<'_> {
    fn observe_cleanup(&self, event: CleanupEvent) {
        self.fixture.observe_cleanup(event);
    }
    fn prepared(&self, conn: &Connection, plan: &PreparedSpawn) {
        assert!(conn.is_autocommit(), "preparation must precede BEGIN");
        let count: i64 = conn
            .query_row("SELECT count(*) FROM members", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count,
            if self.member { 2 } else { 0 },
            "preparation must precede registration"
        );
        assert!(!plan.argv_prefix.is_empty());
        assert_eq!(plan.coding_agent, "claude");
        assert_eq!(
            plan.env,
            [(
                "CAFLEET_DATABASE_URL".to_owned(),
                "sqlite:///snapshot-1".to_owned()
            )]
        );
        assert_eq!(plan.cwd.as_deref(), Some(self.fixture.dir.path()));
        self.prepared.set(self.prepared.get() + 1);
    }
}
fn create(
    f: &Fixture,
    member: bool,
    script: &Script<'_>,
    prompt: &str,
) -> (Result<Value, CafleetError>, bool) {
    let hooks = Hooks {
        fixture: f,
        member,
        prepared: Cell::new(0),
    };
    let reads = Cell::new(0);
    let cwd_reads = Cell::new(0);
    let cwd = || {
        cwd_reads.set(cwd_reads.get() + 1);
        script.clock.advance(Duration::from_secs(60)); // preparation is outside spawn budget
        assert_eq!(f.count("members"), if member { 2 } else { 0 });
        Ok(f.dir.path().to_path_buf())
    };
    let env = |key: &str| {
        assert_eq!(key, "CAFLEET_DATABASE_URL");
        reads.set(reads.get() + 1);
        Some(format!("sqlite:///snapshot-{}", reads.get()))
    };
    let preparation = SpawnPreparation {
        cwd: &cwd,
        env: &env,
    };
    let execution = SpawnExecution {
        clock: script.clock,
        runner: script,
    };
    let probe = FakeProbe::with_binary("claude", f.dir.path());
    let mut slot = Some(f.conn());
    let result = if member {
        cli::member::create_with_options(
            slot.as_mut().unwrap(),
            &MemberCreateOptions {
                fleet_id: 1,
                name: "worker {member_id}",
                description: "work",
                explicit_agent: Some("claude"),
                model: None,
                effort: None,
                monitor: false,
                prompt: Some(prompt),
                file: None,
            },
            || Ok(mux(true)),
            &probe,
            &preparation,
            &execution,
            &hooks,
        )
    } else {
        let path = f.dir.path().join("monitor.md");
        std::fs::write(&path, prompt).unwrap();
        cli::fleet::create_with_options(
            &mut slot,
            &FleetCreateOptions {
                name: "fixture",
                agent_name: "claude",
                monitor_file: path.to_str().unwrap(),
                monitor_model: None,
            },
            || Ok(mux(true)),
            &probe,
            &preparation,
            &execution,
            &hooks,
        )
    };
    assert_eq!(reads.get(), 1);
    assert_eq!(cwd_reads.get(), 1);
    assert_eq!(hooks.prepared.get(), 1);
    (result, slot.is_some())
}
#[test]
fn known_run_timeout_closes_before_member_deregister_or_fleet_rollback_even_if_close_fails() {
    for member in [false, true] {
        for close in [ok("close", 125, ""), fail("close"), ok("close", 5_000, "")] {
            let close_failed = close.result.is_err() || close.cost >= Duration::from_secs(5);
            let close_timed_out = close.cost >= Duration::from_secs(5);
            let f = Fixture::new(member);
            let clock = Clock::new();
            let mut steps = herdr_steps(30_000);
            steps.push(close);
            let script = Script::new(&clock, steps, Some(&f));
            let (result, retained) = create(&f, member, &script, "F {fleet_id} M {member_id}");
            let error = result.unwrap_err();
            assert_eq!(error.exit_code(), 1);
            let detail = error.to_string();
            assert!(
                detail.starts_with(
                    "tmux split-window failed: herdr command timed out after 29.625s:"
                ),
                "{detail}"
            );
            assert_eq!(
                detail.matches("cleanup failed for pane w1:p9:").count(),
                usize::from(close_failed)
            );
            if close_timed_out {
                assert!(detail.contains("herdr command timed out after 5s:"));
            }
            assert_eq!(retained, member);
            let timeline = f.timeline();
            let close_index = timeline
                .iter()
                .position(|s| s.starts_with("close:"))
                .unwrap();
            let db_index = timeline
                .iter()
                .position(|s| {
                    s == if member {
                        "deregister:ok"
                    } else {
                        "rollback:ok"
                    }
                })
                .unwrap();
            assert!(close_index < db_index, "{timeline:?}");
            assert!(timeline.contains(&format!("write-lock:{member}")));
            assert!(
                !timeline.iter().any(|s| s.starts_with("cli-kill:")),
                "backend already owns cleanup"
            );
            if member {
                f.assert_deregistered();
            } else {
                f.assert_empty_bootstrap();
            }
            assert_eq!(
                script.budgets(),
                [30_000, 29_875, 29_625, 5_000].map(Duration::from_millis)
            );
            script.drained();
        }
    }
}
#[test]
fn unknown_split_timeout_performs_db_compensation_without_guessed_or_double_kill() {
    for member in [false, true] {
        let f = Fixture::new(member);
        let clock = Clock::new();
        let mut steps = herdr_steps(0);
        steps.pop();
        steps[1].cost = Duration::from_secs(30);
        let script = Script::new(&clock, steps, Some(&f));
        let (result, retained) = create(&f, member, &script, "prompt");
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(
            error
                .to_string()
                .starts_with("tmux split-window failed: herdr command timed out after 29.875s:")
        );
        assert!(
            error
                .to_string()
                .contains("pane ID unknown; pane cleanup unconfirmed")
        );
        assert_eq!(retained, member);
        assert_eq!(script.ops(), ["list", "split"]);
        if member {
            f.assert_deregistered();
        } else {
            f.assert_empty_bootstrap();
        }
        script.drained();
    }
}
#[test]
fn postcallback_sql_failure_rolls_back_and_closes_connection_before_bounded_cli_close() {
    for commit_failure in [false, true] {
        for close in [fail("close"), ok("close", 5_000, "")] {
            let f = Fixture::new(false);
            f.sql(if commit_failure {
                "CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE child(id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fail_commit AFTER INSERT ON member_placements WHEN NEW.member_id=2 BEGIN INSERT INTO child VALUES(999); END;"
            } else {"CREATE TRIGGER fail_placement BEFORE INSERT ON member_placements WHEN NEW.member_id=2 BEGIN SELECT RAISE(ABORT,'placement sentinel'); END;"});
            let clock = Clock::new();
            let mut steps = herdr_steps(125);
            steps.push(close);
            let script = Script::new(&clock, steps, Some(&f));
            let (result, retained) = create(&f, false, &script, "prompt");
            assert!(!retained, "owner slot must be taken and closed");
            let detail = result.unwrap_err().to_string();
            assert!(detail.contains(if commit_failure {
                "FOREIGN KEY constraint failed"
            } else {
                "placement sentinel"
            }));
            assert_eq!(detail.matches("cleanup failed for pane w1:p9:").count(), 1);
            let timeline = f.timeline();
            let rollback = timeline.iter().position(|s| s == "rollback:ok").unwrap();
            let close = timeline.iter().position(|s| s == "close:error").unwrap();
            assert!(rollback < close, "{timeline:?}");
            assert!(timeline.contains(&"write-lock:true".into()));
            assert_eq!(
                timeline
                    .iter()
                    .filter(|s| s.starts_with("cli-kill:"))
                    .count(),
                1
            );
            assert_eq!(timeline.iter().filter(|s| *s == "disarm:pane").count(), 1);
            assert_eq!(*script.budgets().last().unwrap(), Duration::from_secs(5));
            f.assert_empty_bootstrap();
            script.drained();
        }
    }
}
#[test]
fn malformed_placeholder_is_still_compensated_after_identity_allocation() {
    for member in [false, true] {
        let f = Fixture::new(member);
        let clock = Clock::new();
        let script = Script::new(&clock, vec![], Some(&f));
        let (result, retained) = create(&f, member, &script, "{unknown_step8}");
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("Unknown placeholder"));
        assert_eq!(retained, member);
        assert!(script.calls.borrow().is_empty());
        if member {
            f.assert_deregistered();
        } else {
            f.assert_empty_bootstrap();
        }
    }
}
#[test]
fn prepared_render_matches_all_three_builders_and_never_expands_other_argv_slots() {
    for agent in ["claude", "codex", "opencode"] {
        let backend = coding_agent(agent).unwrap();
        let mut prefix = backend.build_spawn_argv(
            "",
            "name {member_id}",
            Some("model/{fleet_id}"),
            Some("{coding_agent}"),
        );
        assert_eq!(prefix.pop().as_deref(), Some(""));
        let plan = PreparedSpawn {
            prompt_template:
                "F {fleet_id} M {member_id} D {director_member_id} A {coding_agent} {{literal}}"
                    .into(),
            argv_prefix: prefix,
            coding_agent: agent.into(),
            env: vec![],
            cwd: None,
        };
        assert_eq!(
            plan.render(11, 22, 33).unwrap(),
            backend.build_spawn_argv(
                &format!("F 11 M 22 D 33 A {agent} {{literal}}"),
                "name {member_id}",
                Some("model/{fleet_id}"),
                Some("{coding_agent}")
            )
        );
    }
}
#[test]
fn successful_create_keeps_owner_slot_and_forwards_one_environment_snapshot() {
    for member in [false, true] {
        let f = Fixture::new(member);
        let clock = Clock::new();
        let script = Script::new(&clock, herdr_steps(125), Some(&f));
        let (result, retained) = create(&f, member, &script, "F {fleet_id} M {member_id}");
        assert!(result.is_ok());
        assert!(retained);
        let calls = script.calls.borrow();
        assert!(
            calls[1]
                .0
                .windows(2)
                .any(|w| w == ["--env", "CAFLEET_DATABASE_URL=sqlite:///snapshot-1"])
        );
        if member {
            assert!(calls[2].0[4].contains("worker {member_id}"));
        }
        assert!(
            !f.timeline()
                .iter()
                .any(|s| s.starts_with("close:") || s.starts_with("deregister:"))
        );
        script.drained();
    }
}

#[test]
fn tmux_create_prepares_all_agent_argv_before_registration_without_reading_cwd() {
    struct Prepared {
        seen: Cell<usize>,
    }
    impl BootstrapHooks for Prepared {}
    impl CreationHooks for Prepared {
        fn prepared(&self, conn: &Connection, plan: &PreparedSpawn) {
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("SELECT count(*) FROM members", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert!(plan.cwd.is_none());
            assert!(plan.env.is_empty());
            self.seen.set(self.seen.get() + 1);
        }
    }
    for agent in ["claude", "codex", "opencode"] {
        let f = Fixture::new(true);
        let mut conn = f.conn();
        let mut probe = FakeProbe::with_binary(agent, f.dir.path());
        let preset = f.dir.path().join("opencode");
        std::fs::create_dir_all(preset.join("agents")).unwrap();
        std::fs::write(preset.join("agents/cafleet.md"), "fixture preset").unwrap();
        probe.env.insert(
            "OPENCODE_CONFIG_DIR".into(),
            preset.to_str().unwrap().into(),
        );
        let clock = Clock::new();
        let script = Script::new(
            &clock,
            vec![ok("split-window", 125, "%9"), ok("select-layout", 125, "")],
            None,
        );
        let reads = Cell::new(0);
        let env = |key: &str| {
            assert_eq!(key, "CAFLEET_DATABASE_URL");
            reads.set(reads.get() + 1);
            None
        };
        let prep = SpawnPreparation {
            cwd: &|| panic!("tmux must not read cwd"),
            env: &env,
        };
        let hooks = Prepared { seen: Cell::new(0) };
        let options = MemberCreateOptions {
            fleet_id: 1,
            name: "name {member_id}",
            description: "work",
            explicit_agent: Some(agent),
            model: Some("provider/{fleet_id}"),
            effort: None,
            monitor: false,
            prompt: Some("F {fleet_id} M {member_id} A {coding_agent}"),
            file: None,
        };
        let result = cli::member::create_with_options(
            &mut conn,
            &options,
            || Ok(mux(false)),
            &probe,
            &prep,
            &SpawnExecution {
                clock: &clock,
                runner: &script,
            },
            &hooks,
        )
        .unwrap();
        assert_eq!(result["member_id"], 3);
        let expected = coding_agent(agent).unwrap().build_spawn_argv(
            &format!("F 1 M 3 A {agent}"),
            "name {member_id}",
            Some("provider/{fleet_id}"),
            None,
        );
        assert_eq!(&script.calls.borrow()[0].0[8..], expected);
        assert_eq!(reads.get(), 1);
        assert_eq!(hooks.seen.get(), 1);
        script.drained();
    }
}
#[test]
fn cwd_failure_happens_before_any_member_registration_or_fleet_transaction() {
    for member in [false, true] {
        let f = Fixture::new(member);
        let clock = Clock::new();
        let script = Script::new(&clock, vec![], Some(&f));
        let preparation = SpawnPreparation {
            cwd: &|| Err(std::io::Error::other("cwd sentinel")),
            env: &|_| None,
        };
        let execution = SpawnExecution {
            clock: &clock,
            runner: &script,
        };
        let probe = FakeProbe::with_binary("claude", f.dir.path());
        let mut slot = Some(f.conn());
        let result = if member {
            cli::member::create_with_options(
                slot.as_mut().unwrap(),
                &MemberCreateOptions {
                    fleet_id: 1,
                    name: "worker",
                    description: "work",
                    explicit_agent: None,
                    model: None,
                    effort: None,
                    monitor: false,
                    prompt: Some("prompt"),
                    file: None,
                },
                || Ok(mux(true)),
                &probe,
                &preparation,
                &execution,
                &f,
            )
        } else {
            let path = f.dir.path().join("monitor.md");
            std::fs::write(&path, "prompt").unwrap();
            cli::fleet::create_with_options(
                &mut slot,
                &FleetCreateOptions {
                    name: "fleet",
                    agent_name: "claude",
                    monitor_file: path.to_str().unwrap(),
                    monitor_model: None,
                },
                || Ok(mux(true)),
                &probe,
                &preparation,
                &execution,
                &f,
            )
        };
        assert!(result.unwrap_err().to_string().contains("cwd sentinel"));
        assert!(slot.is_some());
        assert!(slot.as_ref().unwrap().is_autocommit());
        assert_eq!(f.count("members"), if member { 2 } else { 0 });
        assert!(f.events.borrow().is_empty());
        assert!(script.calls.borrow().is_empty());
    }
}
#[test]
fn create_precondition_order_remains_member_fleet_first_and_fleet_mux_first() {
    let f = Fixture::new(true);
    let clock = Clock::new();
    let script = Script::new(&clock, vec![], None);
    let prep = SpawnPreparation {
        cwd: &|| panic!("precondition must fail before cwd"),
        env: &|_| panic!("precondition must fail before env"),
    };
    let execution = SpawnExecution {
        clock: &clock,
        runner: &script,
    };
    let probe = FakeProbe::without_binaries(f.dir.path());
    let mut conn = f.conn();
    let mut options = MemberCreateOptions {
        fleet_id: 999,
        name: "worker",
        description: "work",
        explicit_agent: Some("claude"),
        model: None,
        effort: Some("invalid"),
        monitor: true,
        prompt: None,
        file: None,
    };
    let error = cli::member::create_with_options::<AnyMultiplexer>(
        &mut conn,
        &options,
        || panic!("fleet before mux"),
        &probe,
        &prep,
        &execution,
        &f,
    )
    .unwrap_err();
    assert!(error.to_string().contains("Fleet '999' not found"));
    options.fleet_id = 1;
    let error = cli::member::create_with_options::<AnyMultiplexer>(
        &mut conn,
        &options,
        || panic!("effort before mux"),
        &probe,
        &prep,
        &execution,
        &f,
    )
    .unwrap_err();
    assert!(error.to_string().contains("--effort"));
    options.effort = None;
    let error = cli::member::create_with_options::<AnyMultiplexer>(
        &mut conn,
        &options,
        || panic!("monitor role before body/mux"),
        &probe,
        &prep,
        &execution,
        &f,
    )
    .unwrap_err();
    assert!(error.to_string().contains("already has an active monitor"));
    let mut slot = Some(conn);
    let error = cli::fleet::create_with_options::<AnyMultiplexer>(
        &mut slot,
        &FleetCreateOptions {
            name: "fleet",
            agent_name: "claude",
            monitor_file: "missing-prompt",
            monitor_model: None,
        },
        || Err(crate::multiplexer::MultiplexerError::new("mux unavailable")),
        &probe,
        &prep,
        &execution,
        &f,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "cafleet fleet create must be run inside a tmux or herdr session"
    );
    assert!(slot.is_some());
    assert!(f.events.borrow().is_empty());
}
#[test]
fn expiry_between_subprocesses_skips_next_run_and_compensates_known_pane() {
    let clock = Clock::new();
    let mut steps = herdr_steps(0);
    steps.pop();
    steps[1].after = Duration::from_secs(30);
    steps.push(ok("close", 125, ""));
    let script = Script::new(&clock, steps, None);
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
            let script = Script::new(&clock, vec![missing], None);
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
