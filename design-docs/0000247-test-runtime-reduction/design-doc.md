# Test Runtime Reduction

**Status**: Approved
**Progress**: 0/45 tasks complete
**Last Updated**: 2026-11-27

## Overview

Bring the local Rust test suite from 156 s to under 20 s of wall time. Three changes get it there: parse the embedded profile fixtures once instead of on every lookup, remove or merge tests whose behavior another test already asserts, and run every time-dependent test on the injected `time::Clock` so that no verdict and no wait depends on wall-clock time. This design supersedes 0000222 (deterministic test timing) and folds its rules in.

## Success Criteria

- [ ] On an 8-CPU host, the suite's libtest `exec_time` at opt-level 0 with `RUST_TEST_THREADS=8` is under 20 s. The figure is the median of 3 runs of the § Measurement protocol.
- [ ] No test's `exec_time` exceeds 2 s in those runs.
- [ ] Every REMOVE, FOLD and MERGE row of § Redundancy inventory is applied. Each merged test is table-driven over a named case array and names its case in every assertion message.
- [ ] No test asserts a real elapsed-time bound, and no verdict depends on elapsed real time. No in-process test waits real time for a deadline, budget, retry interval or sleep. Every remaining real-time read or wait in test code is a § Sanctioned real-time sites entry.
- [ ] Every real-time cap in test code is `time::test_support::HANG_GUARD` (30 s), and a passing run never reaches it.
- [ ] Production timing constants and observable behavior are unchanged. Production builds read `SystemClock` (or `Instant::now()` inside the database clock) wherever this design adds a seam. `SPEC.md`, `README.md` and `assets/skills/` are untouched.
- [ ] `mise //:coverage` line coverage is reported before Step 2 and after Step 7, and every drop is justified in the PR that causes it.
- [ ] Every PR of § Implementation passes `mise //:test`, `mise //:lint` and the § Flakiness gate before merge.
- [ ] `.claude/rules/code-quality.md` and `docs/docs/contributing.md` state the clock and fixture-parse rules in § Rule and documentation text.
- [ ] `design-docs/0000222-deterministic-test-timing/design-doc.md` has Status `Aborted` and a changelog row recording the supersession.

---

## Background

### Measured state

These runs were measured on 2026-11-27 at HEAD `362c4acd` on an 8-CPU host that also ran unrelated workloads. Each is one run of the § Measurement protocol with `RUST_TEST_THREADS=8`.

| Run | Wall (`exec_time`) | Summed test time | `coding_agent::catalog` | `coding_agent::detector` | `runtime::input` | `runtime::service` | `runtime::observation` |
|---|---|---|---|---|---|---|---|
| HEAD, opt-level 0 (local default) | 156.4 s | 1237.8 s | 450.5 s | 380.5 s | 113.1 s | 114.6 s | 93.5 s |
| HEAD, `CARGO_PROFILE_TEST_OPT_LEVEL=1` (CI setting) | 68.4 s | 536.7 s | 175.6 s | 106.7 s | 71.7 s | 54.8 s | 38.7 s |
| HEAD + fixture-parse cache, opt-level 0 | 29.8 s | 231.8 s | 13.3 s | 2.7 s | 56.0 s | 66.7 s | 24.1 s |

The suite has 1473 tests: 1464 run and 9 are ignored child-process entries.

### Root causes

| Cause | Share | Evidence |
|---|---|---|
| Fixture lookups re-parse every embedded profile file | about 1000 s of the 1238 s summed | `catalog::test_support::bundled_files()` parses all five `assets/profiles/<backend>.json` files (about 4.4 MB) through the duplicate-key `Strict` visitor on each call. That visitor is crate code, so it runs at opt-level 0. `bundled_file`, `bundled_fixture`, the detector `document`/`bundled_screen` and `replay_as_captured` call it once or twice per screen, and they deep-clone the parsed file. The cost arrived with 0000243 (fixture-registry fold, 2026-11-23); 0000238 measured the suite at about 33 s on 2026-11-21. Caching the parse alone gives the third row above. |
| Real waits on production budgets | about 110 s summed across about 60 tests | Production code reads `Instant::now()` / `std::thread::sleep` directly instead of the injected clock. Examples: `Deadline::remaining`, `pane_lock::retry_until`, the `db.rs` busy, progress and grace handling, the `system.rs` `collect_until`, the member-create probe bounds, the Director sweep `storage_until` conversion and the tmux capture retry. Some tests also spawn real `sleep N` children to reach those bounds. § Real-time inventory lists every site. |
| Deadline-sensitive verdicts | about 15 tests | Tests assert upper or lower elapsed-time bounds as tight as ±100 ms. These can flake under load, as the incidents 0000222 recorded did. |
| CPU-bound virtual-time tests | remaining tests over 2 s after the cache | These are many virtual turns, each doing SQLite and detector work, with no real waits. § CPU residue lists them. |

### Relationship to 0000222

0000222 is Approved at 3/33 tasks, but none of its toolkit (`ThreadClock`, `HANG_GUARD`, `monotonic_now`, `writer_work_slice_ms`) exists in HEAD. This design keeps 0000222's timing rules: assert outcomes, expire by construction, wait for consequences, and assert absence at a barrier. It replaces 0000222's process-wide thread-local clock shim with explicit `Clock` injection, as § Clock injection describes. 0000222 is marked `Aborted`.

---

## Specification

### Timing rules

A test is deterministic when its verdict depends only on the order of events the code under test produces, never on how much real time those events took. Every test follows these rules, in order of preference:

1. **Assert outcomes, not durations.** Assert returned errors, persisted rows, event sequences, host call counts, hook ordinals or `TestClock` instants and recorded sleeps. Assert no real elapsed-time bound, upper or lower.
2. **Time moves only when the test moves it.** Code under test reads time and sleeps through the `Clock` it is given. The test passes a `TestClock`, whose `sleep` advances virtual time and records the duration.
3. **Expiry happens by construction.** A budget the test expects to expire expires because of one of the following:
   - virtual time advanced past it;
   - a boundary hook returned `CafleetError::DeadlineExceeded`;
   - the deadline was already past at entry;
   - for a budget that runs in another process, the awaited condition is impossible, such as a child that ignores `SIGTERM`.
4. **Wait for the consequence, not for time.** Block on a hook `Condvar` through `await_state`. Poll external state (rows, files, PTY output) through `await_until` only when no in-process signal exists.
5. **Assert absence at a barrier.** Check "X did not happen" after a later event that production orders after X, never after a sleep.
6. **One real-time cap.** The only real-time cap in test code is `HANG_GUARD`. Reaching it is a hang, reported by a panic that names what was awaited.

### Clock injection

`time::Clock` with `time::test_support::TestClock` is the mock. Rust has no runtime patching, so every call site that reads `Instant::now()` or calls `std::thread::sleep` needs an injection point. The existing trait already provides that point for the runner, writer, process, transport and input code. This design extends the same trait to the remaining call sites and adds no mock crate.

A crate such as `mock_instant` was not chosen for four reasons:
- It swaps `Instant` per call site under `cfg(test)`, but it does not cover `std::thread::sleep`, `poll(2)` timeouts or SQLite's busy and progress callbacks.
- Its global or thread-local mock clock leaks between libtest threads unless every test installs it first.
- It would duplicate the `Clock`/`TestClock` pair that runtime code already uses.
- It would add a dependency, which 0000238 worked to avoid.

Injection takes these forms:

| Form | When | Shape |
|---|---|---|
| Explicit parameter | The call path already carries a clock, a host trait or a deadline owner | `clock: &dyn Clock` parameter, or an `Arc<dyn Clock>` field set at construction. Production passes `&crate::time::SystemClock` or the clock it already holds. |
| Database clock | Only the `db.rs` budget machinery, whose deadlines SQLite's C busy and progress callbacks evaluate without any parameter | `time::database_now()` and `time::database_sleep(d)`. In production builds these are exactly `Instant::now()` and `std::thread::sleep(d)`. In test builds they read a clock installed on the calling thread when one is installed, and otherwise fall back to those same calls. |

The database clock covers every time read in `db.rs`, including:
- the entry checks of `with_database_budget`, `with_database_deadline` and `with_shared_database_deadline`;
- `past_deadline`;
- `remaining_database_wait`;
- `database_cancelled`;
- `wait_for_database_deadline`;
- the `complete_database_budget` grace window.

The entry and exit checks share the callbacks' clock so that one database scope never mixes two time sources.

### Shared test-time toolkit (`src/time.rs`)

`database_now` and `database_sleep` are production code. Everything else lives in the existing `#[cfg(test)] pub(crate) mod test_support`, which owns `TestClock`.

```rust
// time.rs (production; test builds consult the installed database clock first)
pub(crate) fn database_now() -> Instant;
pub(crate) fn database_sleep(duration: Duration);

// time::test_support
pub(crate) const HANG_GUARD: Duration = Duration::from_secs(30);

/// Polls external state until `ready`; panics "hang guard: {awaited} did not happen within 30s".
pub(crate) fn await_until(awaited: &str, ready: impl FnMut() -> bool);
/// Waits on `changed` until `ready(state)`, bounded by HANG_GUARD; returns the guard.
pub(crate) fn await_state<'a, T>(
    awaited: &str,
    lock: &'a Mutex<T>,
    changed: &Condvar,
    ready: impl FnMut(&mut T) -> bool,
) -> MutexGuard<'a, T>;

impl TestClock {
    /// Runs `action` once, when the `nth` sleep (counting from 1) is recorded.
    pub(crate) fn on_sleep(self, nth: usize, action: impl FnOnce() + Send + 'static) -> Self;
}

/// Real time with a configured shutdown unit: `now`, `wall_now`, `monotonic_sample` and
/// `sleep` delegate to `SystemClock`; `shutdown_unit` returns the configured unit.
pub(crate) struct RealClockWithUnit(pub(crate) Duration);

/// Installs `clock` as the calling thread's database clock until dropped; panics when one is
/// already installed on the thread.
pub(crate) struct DatabaseClock { /* !Send */ }
impl DatabaseClock {
    pub(crate) fn install(clock: Arc<dyn Clock>) -> Self;
    /// Installs `clock` when `clock.kind()` is `Deterministic`; does nothing for a real clock.
    pub(crate) fn install_deterministic(clock: &Arc<dyn Clock>) -> Option<Self>;
}
```

| Item | Contract |
|---|---|
| `HANG_GUARD` | 30 s, a real-time cap that is never a correctness criterion. |
| `await_until` / `await_state` | Return as soon as the condition holds. Otherwise panic after `HANG_GUARD` of real time, naming `awaited`. |
| `TestClock::on_sleep` | Makes "released within the wait" deterministic: the action, such as dropping a lock holder, runs at a chosen retry. |
| `RealClockWithUnit` | Replaces the `SystemClockWithUnit` defined inside `observation::tests` (observation.rs:3780) and serves every service child that needs a short unit (§ Production seams > `RunnerDependencies`). |
| `DatabaseClock` | Governs only the installing thread. A test installs it as its first statement, before it builds any deadline the database scope will read. Drop uninstalls it, so a thread that libtest reuses starts clean. |

`await_until` and `await_state` replace exactly these helpers. Each is deleted in the Step that migrates its last caller:
- `writer::test_support::{wait_until, wait_until_within, deadline, yield_until, yield_until_within}` (writer.rs:1527-1626);
- the `coding_agent::hermes` test `deadline()` (hermes.rs:593);
- `service::test_support` `await_writer` (service.rs:4918) and `wait_for` (service.rs:5866);
- `input::tests` `await_writer` (input.rs:10107);
- the ad-hoc `loop { ..; sleep(2ms) }` waits found by § Per-step audit.

#### Database scopes on spawned threads

A spawned thread that owns database scopes gets its database clock from the clock it was given. In test builds, its thread body begins with `let _database_clock = DatabaseClock::install_deterministic(&clock);`. A deterministic clock (`TestClock`, `YieldingClock`) is then also the database clock on that thread, and a real clock leaves the production path untouched. Two threads do this:

| Thread | Entry | Inventory rows that rely on it |
|---|---|---|
| Writer worker (`cafleet-events`, writer.rs:206) | First statement of the spawned closure, before `worker.serve()` | `runtime::writer::tests::step_bound::{writer_persist_waits_beyond_100_ms_under_a_held_lock_without_failing, writer_keeps_a_statement_cut_off_by_the_shutdown_deadline_at_the_front_unrejected}` |
| Director sweep thread (service.rs:1361) | First statement of the spawned closure | `runtime::service::tests::director_sweep::sweep_thread::*` |

A `YieldingClock` records no sleeps and does not advance on sleep, so a writer busy wait under it lasts until the test advances the shared `TestClock`. The writer rows therefore advance explicitly. Tests that call `DirectorSweep::tick` on the test thread (`storage_busy` ×3) install `DatabaseClock` on that thread.

`time::tests` covers each toolkit item with deterministic cases: `await_until` returning and panicking, `await_state` waking on notify, the `on_sleep` ordinal, `RealClockWithUnit` delegation, `DatabaseClock` install, refusal of a second install, uninstall on drop, `install_deterministic` skipping a real clock, and the production fallback when nothing is installed.

### Production seams

Each seam changes no production value or path. Each production caller passes `SystemClock`, or the clock it already holds.

| Seam | Change | Unblocks |
|---|---|---|
| `db.rs` budget machinery (`remaining_database_wait`, `database_cancelled`, `wait_for_database_deadline`, `with_database_budget`, `with_database_deadline`, `with_shared_database_deadline`, `past_deadline`, `complete_database_budget`) | Every time read and sleep goes through `time::database_now` / `time::database_sleep` (§ Clock injection) | db scope tests, the lifecycle `runner_database_deadline_*` tests, the artifacts lock-transfer test, fleet unwind, Director `storage_busy`, the input contended host |
| `pane_lock::retry_until` | `retry_until(deadline, clock, attempt)`: it checks `deadline.remaining_at(clock.now())` and sleeps through `clock.sleep` | — |
| `LayoutLock::acquire_within`, `PaneLock::acquire_within`, `PaneLock::reacquire_within`, `lock_deletion` | Each gains `clock: &dyn Clock` and forwards it to `retry_until` | pane-lock and layout-lock tests, member and fleet busy-lock tests, input_client lock reacquire, press busy lock |
| `runtime::system::{collect_until, run_timed, run_until_exit}` | Each gains `clock: &dyn Clock`. They use it for the start instant and the per-iteration deadline check; the `poll(2)` wait stays `min(20 ms, left)` | `timed_runner_*`, `bounded_run::*` |
| `SystemRunner` | It keeps the `CommandRunner` impl on `SystemClock`. A new `run_process_on(argv, timeout, clock)` carries the shared body, and tests call it | timed-runner tests |
| `coding_agent::{run_bounded, run_bounded_output}`, `CodingAgent::probe_native_version`, `hermes::effective_home` (through `HermesCli`) | Each gains `clock: &dyn Clock`, forwarded to `run_until_exit` | native version probe tests, hermes config-path bound |
| `cli::member::create_with_layout_wait` | Gains `clock: &dyn Clock`, which it uses for the layout-lock deadline, the 10 s command deadline, the preflight start and both probes. A new pure function `preflight_deadlines(start, deadline) -> (probe, config_path)` returns `(deadline.min(start + 2 s), deadline.min(start + 5 s))`. | hermes probe-bounds test, member layout-lock tests |
| `cli::fleet::DeletionHost` and the deletion path | `DeletionHost` gains `fn clock(&self) -> &dyn Clock`. `end_member` reads `host.clock()`. `await_runner_exit` and `acquire_deletion_layout` gain a `clock: &dyn Clock` parameter. `hermes::settlement_deadline(clock)` builds its 20 s budget from `clock.now()`. The table below lists each implementation and caller; the `delete_fleet*` signatures are unchanged | fleet deletion and settlement tests, member deletion busy-lock tests |
| `multiplexer::BackendCore` / `DeadlineRunner` | `BackendCore` gains an `Arc<dyn Clock>` set at construction, `SystemClock` in production. `DeadlineRunner::remaining`, `check_deadline`, `press_run` and the tmux `capture_screen` retry sleep all use it. The public `bounded(deadline)` API is unchanged | tmux capture deadline and retry tests, Director sweep `attempt` |
| `service::DirectorSweep::tick` / `attempt` | Delete the `storage_until` and `real_deadline` conversions to real time. Database scopes receive the sweep-clock instant (identical under `SystemClock`), and the multiplexer is built on the sweep clock | `storage_busy` ×3, `sweep_thread` ×2 |
| `RunnerDependencies` (test fields only) and `run_configured` | Add `#[cfg(test)] sweep_interval: Duration`, which `run` sets to `DIRECTOR_SWEEP_INTERVAL`. `run_configured` (service.rs:3885) reads two optional keys from `observation-services.json`. With `shutdown_unit_ms` present, `clock` is `Arc::new(RealClockWithUnit(unit))`; when it is absent, `clock` stays `Arc::new(SystemClock)`. `sweep_interval_ms` sets `sweep_interval`, which defaults to `DIRECTOR_SWEEP_INTERVAL` when absent. A `#[cfg(test)]` sweep-pass hook appends one line per completed pass to `sweep-passes` in the fixture directory. | the service-child rows of § Real-time inventory > Director sweep and service child, and the `input_client` service-child rows |

The `DeletionHost::clock` seam reaches every implementation and caller as follows:

| Implementation or caller | Clock |
|---|---|
| `MultiplexerDeletionHost` (fleet.rs:344) | Gains a `clock: Arc<dyn Clock>` field that `clock()` returns. `new(mux)` keeps its signature and sets `SystemClock`, so `NativeFleetDeletionHost` (fleet.rs:404) and the fleet test at fleet.rs:2274 are unchanged. A new `with_clock(mux, clock)` sets the given clock. |
| `NativeFleetDeletionHost` (fleet.rs:413), the host behind `delete_fleet` | `clock()` returns `&SystemClock` |
| Test hosts `Host` (fleet.rs:976), `FixtureDeletionHost` (fleet.rs:1158) and `UntouchedHost` (fleet.rs:2334) | Each gains `clock()`, returning the test's `TestClock`. `UntouchedHost` returns a `TestClock` that no test advances. |
| Member deletion: `delete` (member.rs:694), `lock_member_deletion` (member.rs:644), `close_member_within` (member.rs:747) and the test-only `close_member` | Each gains `clock: &Arc<dyn Clock>`, and the CLI dispatch passes `Arc::new(SystemClock)` to `delete`. `lock_member_deletion` passes the clock to `PaneLock::acquire_within` and `lock_deletion`. `close_member_within` passes it to `acquire_deletion_layout` (member.rs:761) and builds `MultiplexerDeletionHost::with_clock(mux, clock.clone())` at member.rs:776. One clock therefore governs the pane-lock wait, the layout wait and `end_member`. |
| `delete_fleet_with_layout_wait` | Passes `host.clock()` to `acquire_deletion_layout` and `await_runner_exit` |

The parameter fits better than a host-only carrier because `close_member_within` builds its `MultiplexerDeletionHost` internally, so a test cannot inject a host. The busy pane-lock wait of `hermes_member_deletion_waits_for_busy_pane_input_only_within_the_command_deadline` also happens in `lock_member_deletion`, which `delete` calls before any host exists. Tests pass their `TestClock` to `delete` (the hermes busy-pane test), to `lock_member_deletion` and `close_member_within` (`a_busy_layout_lock_fails_member_delete_and_leaves_run_pane_and_row_unchanged`, member.rs:3290-3297), and to `close_member`.

`writer::StepBound` keeps its real-time step limit, so a virtual-time jump never expires a running step. Tests end a paused step through the hook in § Real-time inventory > Writer and observation.

### Real-time inventory

Seconds are the measured real wait in a passing run at HEAD. Each conversion follows a construct from this table:

| Construct | Meaning |
|---|---|
| Advance | Pass a `TestClock` to the seam. Production's own sleep or retry advances virtual time, so expiry follows by construction. |
| Gate | For a bounded call on a real child, the call runs on a spawned thread with `YieldingClock(Arc<TestClock>)`. That clock reads the shared `TestClock`, and its `sleep` yields without advancing, so the deadline cannot pass on its own even when the call sleeps through its clock (`PtySession::shutdown` sleeps at process.rs:621 and through `nap_until` at process.rs:645). The child's stub writes a marker file once it reaches the state the verdict needs, such as "flooding", "pipes closed" or "started". The test waits for that marker with `await_until`. When the verdict needs the exit already observed, the test also waits until the call has reaped the child: `kill(pid, 0)` returns `ESRCH` for the pid the stub records. Only then does the test call `advance` past the deadline. The call's next deadline check expires, and the test joins the thread. No read count is involved. A correct implementation always passes. A regression that returns early is caught whenever it acts before the advance, and it never makes a correct run fail. |
| Release-on-sleep | `TestClock::on_sleep(n, release)` releases the holder at the n-th retry. |
| Hook expiry | A writer hook returns `DeadlineExceeded` for a paused boundary. |
| DB clock | The test thread installs `DatabaseClock` with the test's `TestClock`, then advances it, or lets the busy handler's sleeps advance it. |
| Barrier | Assert absence after an ordered later event. |
| Certain expiry | In a service child, a short `shutdown_unit_ms` or `sweep_interval_ms` applies only where expiry is certain (rule 3) or the barrier counts passes. No verdict reads elapsed time. |
| Drop bound | Delete the elapsed-time assertion and keep the outcome assertions. |

`YieldingClock` moves from `writer::test_support` to `time::test_support`, unchanged. Its callers (`writer::test_support::start_on`, `start_on_with`) import it from there.

Every row also drops its elapsed-time assertions (Drop bound).

#### Locks, database scopes and time helpers (`src/pane_lock.rs`, `src/db.rs`, `src/time.rs`)

| Test | Real wait | Construct |
|---|---|---|
| `pane_lock::tests::pane_lock_wait_tests::a_lock_still_busy_at_the_reserved_deadline_returns_none` | 1.5 s | Advance. The busy result arrives after virtual time reaches the reserved deadline, and recorded sleeps sum to it. |
| `pane_lock::tests::pane_lock_wait_tests::a_lock_released_within_the_wait_is_acquired` | 0.3 s | Release-on-sleep |
| `pane_lock::tests::layout_lock::a_second_acquisition_blocks_until_the_holder_releases` | 0.3 s | Release-on-sleep |
| `pane_lock::tests::layout_lock::an_expired_wait_reports_busy_after_its_full_budget` | 0.3 s | Advance |
| `pane_lock::tests::deletion_locks::deletion_acquires_physical_keys_in_sorted_order` | 0.05 s | Advance; `recv_timeout` becomes `await_state` |
| `pane_lock::tests::*` uncontended and distinct-fleet cases | guard | Drop bound |
| `db::tests::database_scope_tests::expired_completion_reports_contention_after_bounded_grace` | 1.0 s | DB clock. The busy handler advances to the grace end. |
| `db::tests::database_scope_tests::a_late_error_other_than_an_interrupt_keeps_its_cause` | 0.4 s | DB clock: advance past the 50 ms deadline instead of 4 × 100 ms sleeps |
| `db::tests::database_scope_tests::nested_exit_restores_the_earliest_outer_deadline_on_every_exit` | 0.32 s | DB clock |
| `db::tests::database_scope_tests::shared_deadline_lowered_mid_scope_aborts_the_running_wait` | 0.2 s | DB clock: lower the shared deadline from the `update_hook` or authorizer callback |
| `db::tests::database_scope_tests::consecutive_sql_waits_share_one_absolute_deadline` | 0.2 s | DB clock |
| `db::tests::database_scope_tests::completion_waits_for_a_reader_to_release_commit_after_input_expiry` | 0.13 s | DB clock plus Release-on-sleep |
| `time::tests::deadline_reservation::reserving_moves_the_deadline_earlier_by_the_reserve` | 0 s (100 ms real window) | Assert through `remaining_at` on fixed instants |
| `pane_lock::test_support` `PaneLock::acquire_released` (pane_lock.rs:229-238, `RELEASED_LOCK_GRACE` 2 s), `runtime::artifacts` `RunLock::acquire_released`, `runtime::recovery` `recover_released` (2 s grace) | 0 s in a passing run (cap only) | Their callers, including `cli::member` and `cli::fleet` tests, wait for a flock that a concurrently forked child of another test shares until its `exec` closes it. That is external real state, so these helpers keep `SystemClock`. Their 2 s cap becomes `HANG_GUARD`, `RELEASED_LOCK_GRACE` is deleted, and they join § Sanctioned real-time sites. |

#### CLI deletion and creation (`src/cli/member.rs`, `src/cli/fleet.rs`)

| Test | Real wait | Construct |
|---|---|---|
| `cli::member::tests::hermes_member_create_bounds_config_path_by_five_seconds_and_the_version_probe_by_two` | 10 s | Split three ways: (1) a table test of `preflight_deadlines` on literal instants (start+2 s and start+5 s, each clipped by the command deadline); (2) one in-process test per probe (`probe_native_version` and `hermes::effective_home`), each driving a real stub that writes a `started` marker and then runs `exec /bin/sleep 30`, through Gate, asserting the exact `did not finish within its deadline` refusal and the native-version warning text; (3) the CLI child keeps the both-answer-at-once case. It merges with the hermes home-resolution cases (§ Redundancy inventory). |
| `cli::member::tests::deletion_reloads_a_published_pending_pane_and_retains_ownership_through_completion` | 0.51 s | Advance past the 500 ms deadline instead of `sleep(510ms)`, and give the no-sleep case a `HANG_GUARD` budget |
| `cli::member::tests::recovery_deletion::hermes_member_deletion_waits_for_busy_pane_input_only_within_the_command_deadline` | 0.3 s | Advance |
| `cli::member::tests::window_layout::a_busy_layout_lock_fails_member_create_and_registers_nothing` | 0.3 s | Advance |
| `cli::member::tests::window_layout::a_busy_layout_lock_fails_member_delete_and_leaves_run_pane_and_row_unchanged` | 0.3 s | Advance |
| `cli::member::tests::observation_surface::probe_observation_skips_unprobed_statuses…` | guard | Pass `TestClock` to `probe_observation` |
| `cli::fleet::tests::fleet::runner_review_fleet_changed_placement_is_relocked_before_any_destructive_effect` | 1.0 s | Advance; the helper's 700 ms poll becomes `await_state` |
| `cli::fleet::tests::fleet::runner_review_fleet_changed_run_on_the_same_pane…` | 0.1 s | Advance |
| `cli::fleet::tests::recovery::runner_recovery_deletion_unwind_restores_the_still_live_outer_scope` | 1.0 s | DB clock. Advance 620 ms instead of `sleep(inner + 20 ms)`, then advance past `outer` immediately before `SLOW_SUM`. `EmergencyInterrupt` stays armed at `HANG_GUARD`. |
| `cli::fleet::tests::pane_locks::fleet_deletion_reloads_new_placements_within_the_original_deadline` | 0.6 s | Advance plus Release-on-sleep |
| `cli::fleet::tests::deletion_waits_for_the_runner_a_settlement_left_running_only_within_its_deadline` | 0.4 s | Keep the real `sleep 0.3` child and Advance `await_runner_exit` to its 100 ms deadline. Assert the outcome, not the elapsed time. |
| `cli::fleet::tests::layout_lock::a_busy_layout_lock_fails_fleet_delete_and_leaves_every_member_unchanged` | 0.3 s | Advance |
| `cli::fleet::tests::fleet::hermes_member_deletion_settles_before_shutdown…` | 0.06 s | Advance `SETTLEMENT_TIME`; assert the extension against the clock's instants |
| `cli::fleet::tests::pane_locks::fleet_deletion_contention_preserves_rows_and_releases_preceding_guards` | 0.16 s | Advance |

#### Subprocess bounds (`src/runtime/system.rs`, `src/coding_agent/mod.rs`, `src/multiplexer/*`)

Every row below uses Gate. `ProcessFixture` and the `bounded_run` / version-probe stubs record the stub's pid (`$$`) and write the named marker into the fixture directory. The test thread calls `advance` past the deadline only after the listed condition holds.

| Test | Real wait | Gate condition before `advance` |
|---|---|---|
| `runtime::system::tests::timed_runner_kills_and_reaps_a_sleeping_direct_child_within_five_seconds` | 2.0 s | Marker `started`, written immediately before `exec sleep 30` |
| `runtime::system::tests::timed_runner_checks_deadline_while_both_streams_keep_producing` | 2.0 s | Marker `flooding`, written after the first line on each stream, while both streams keep producing |
| `runtime::system::tests::timed_runner_applies_deadline_after_child_exit_with_inherited_pipes_open` | 2.0 s | The direct child is reaped (`kill(pid, 0)` returns `ESRCH`) while its background `sleep 6` holds both pipes open |
| `runtime::system::tests::timed_runner_waits_for_child_exit_even_after_both_pipes_close` | 2.0 s | Marker `closed`, written after `exec 1>&- 2>&-` and before `exec sleep 30` |
| `runtime::system::tests::bounded_run::the_deadline_kills_a_running_child_and_reports_whether_it_had_exited` | 0.6 s | Running case (`Deadline { exited: false }`): marker `started`. Held case (`Deadline { exited: true }`): the shell is reaped while its background `/bin/sleep 30` holds the pipes |
| `coding_agent::tests::native_version_probe::native_version_probe_kills_and_reaps_a_probe_running_at_its_bound` | 2.0 s | Marker `started` before the stub's `exec /bin/sleep 30` |
| `coding_agent::tests::native_version_probe::bounded_runs_name_an_unstartable_program_and_each_expired_deadline` | 0.6 s | Marker `started` for each sleeping stub |

| Test | Real wait | Construct |
|---|---|---|
| `multiplexer::tmux::tests::director_pane::capture_screen_fails_when_the_geometry_never_settles_within_the_deadline` | 0.3 s | Advance on the core clock |
| `multiplexer::tmux::tests::…capture_screen_retries_the_whole_capture_when_the_metadata_line_changes` | 0.05 s | Advance; assert the recorded 50 ms sleep |

Successful-path timed-runner tests (`timed_runner_drains_*`, `timed_runner_decodes_*`) keep `SystemClock`. Their 10 s budget is not a verdict, and its cap becomes `HANG_GUARD`.

#### Writer and observation (`src/runtime/writer.rs`, `src/runtime/observation.rs`)

`writer::test_support::Hooks` gains `expire(boundary)`. A boundary paused with `pause` returns `DeadlineExceeded` from `before` once the test calls `expire`. A paused boundary otherwise waits until released, bounded by `HANG_GUARD`, rather than by polling `bound.check()`.

| Test | Real wait | Construct |
|---|---|---|
| `runtime::observation::tests::controller_tests::controller_gap_recovery_reobserves_the_unchanged_screen_after_publishing_a_delivery` | 10 s | Hook expiry replaces the wait for the 5 s `WRITER_STEP_BOUND`. The writer starts on `start_on` (its `YieldingClock`) and waits through `await_until`. |
| `runtime::writer::tests::diagnostics_flush::writer_keeps_the_buffer_through_a_transient_flush_failure` | 1.5 s | `start_on`. Hook expiry ends the flush step, and the test advances virtual time past `DIAGNOSTIC_FLUSH_RETRY` instead of sleeping. |
| `runtime::writer::tests::diagnostics_flush::writer_drops_a_permanently_failing_flush…` | 0.2 s | Barrier: a later flush attempt recorded by the hook |
| `runtime::writer::tests::step_bound::writer_finish_joins_an_in_flight_step_at_the_shutdown_deadline` | 0.7 s | `start_on`; advance past the shutdown deadline |
| `runtime::writer::tests::step_bound::writer_persist_waits_beyond_100_ms_under_a_held_lock_without_failing` | 0.3 s | `start_on`; the worker's database clock is the shared `TestClock` (§ Database scopes on spawned threads). With the lock held, the test advances the clock 300 ms (past 100 ms, short of the busy deadline). Barrier: the hook's `Persist` call count shows the step still running with no failure recorded. The test then releases the lock and asserts the persisted event. |
| `runtime::writer::tests::step_bound::writer_keeps_a_statement_cut_off_by_the_shutdown_deadline_at_the_front_unrejected` | 0.3 s | `start_on` with the worker's database clock. With the statement blocked, the test advances past the shutdown deadline; the worker's busy handler sees it expired. |
| `runtime::observation::native_capture::exporter::tests::runner_collector_{encode_and_prepermit_manifest_stalls…, cancelled_manifest_worker…, exhausted_service_storage_budget…}` | 3.8 s at HEAD, 2.1–2.3 s after the parse cache | The stall cases already write `shutdown_unit_ms: 100` to `export-test-plan.json` (observation.rs:2734), so the 4-unit storage deadline is 400 ms; the unit does not explain the measured time. Step 6 first attributes it, using the helper record's `completed_ns` and the service record's `storage_deadline_ns` in `export-helper-returned.json` together with the fixture's spawn and stop instants. The dominant wait then gets a construct from this table: `await_until` for the helper record in place of the `wait_until_within` poll, and Certain expiry for any remaining child budget. The +100 ms tolerance in `assert_original_completion_bound` is dropped, and `completed_ns <= storage_deadline_ns` stays. Expected after conversion: child startup, the 400 ms storage deadline and teardown, well under 2 s. If attribution shows another real wait, the same PR updates this row with its construct. |
| `runtime::observation::native_capture::tests::service_capture::service_private_capture_publishes_manifest_last_and_keeps_native_work_running` | about 2 s | Wait for the manifest file through `await_until`, not for the 2000 ms capture timeout of `NativeCaptureArm::for_identity` |
| `runtime::observation::native_capture::verification::tests::runner_collector_common_service_arms_on_actual_resize…` | 1.3–3 s | Waits become `await_until`. Step 6 confirms it is under 2 s, or attributes and converts the remaining wait as for the exporter rows. |

#### Director sweep and service child (`src/runtime/service.rs`)

| Test | Real wait | Construct |
|---|---|---|
| `runtime::service::tests::director_sweep::storage_busy::a_busy_snapshot_holds_under_the_doubling_backoff_and_writes_nothing` | 10 s | DB clock with the sweep's `TestClock`; the busy handler's sleeps advance it to each `DIRECTOR_ATTEMPT` end |
| `runtime::service::tests::director_sweep::storage_busy::a_hold_continued_by_busy_storage_reports_storage_unavailable_after_three_minutes` | 5 s | DB clock |
| `runtime::service::tests::director_sweep::storage_busy::busy_storage_at_the_stamp_after_submission_is_an_uncertain_preview` | 4.5 s | DB clock |
| `runtime::service::tests::director_sweep::sweep_thread::stop_and_join_returns_by_the_shutdown_deadline_with_the_in_flight_attempt_clipped` | 0.6 s | The `HungPane` runner sleeps through the core clock (Advance); drop the +250 ms bound |
| `runtime::service::tests::director_sweep::sweep_thread::the_subprocess_clip_measures_the_shutdown_deadline_on_the_injected_clock` | 0.7 s | Same |
| `runtime::service::tests::preview_sweep::an_uncertain_prompt_stops_queued_input_and_the_sweep_until_reconciled_on_an_unverified_run` | 5.3 s | Barrier: after the clear, `await_until` two more lines in `sweep-passes` instead of `sleep(SWEEP_WINDOW)`, with `sweep_interval_ms` 50 |
| `runtime::service::tests::preview_sweep::an_unverified_run_stamps_an_uncertain_preview_before_publishing_it_and_never_retypes_it` | 4.2 s | Same barrier |
| `runtime::service::tests::control_service::control_creator_disconnect_before_start_exits_without_spawning` | 2.1 s | Certain expiry: `shutdown_unit_ms` 50 (no native child, so the child deadline always passes) |
| `runtime::service::tests::control_service::control_accepted_start_spawn_and_storage_failures_compensate` | 2.2 s | Certain expiry |
| `runtime::service::tests::injected_failures::runner_failure_with_a_live_native_child_persists_degraded_before_owned_cleanup_confirms_exit` | 2.0 s | Certain expiry (the child ignores `SIGTERM`) |
| `runtime::service::tests::input_service::service_input_queued_expiration_keeps_manual_progress_and_uses_fresh_output_for_exec` | 0.8 s | The command deadline is past at entry for the expiring submission; drop the window loop |
| `runtime::service::tests::input_service::service_input_expired_unreserved_attempt_releases_pending_work_for_a_fresh_key` | 0.4 s | Past-at-entry deadline; drop the ±100 ms assertion |

`ServiceFixture` caps (the 10 s startup deadline, the 2 s `read_until`, the 5 s `wait`, the 1 s `stop_for_disconnect`) become `HANG_GUARD`. Its polling uses `await_until`.

#### Input coordinator, input client, control and remaining runtime

| Test | Real wait | Construct |
|---|---|---|
| `runtime::input_client::tests::external_input_accepted_reply_succeeds_when_another_writer_takes_the_released_lock` | 5 s | `submit_wrapped_input*` gains the client's clock (the `NativeInputHost::clock` field). Its bookkeeping `reacquire_within` Advances to the deadline. |
| `runtime::input_client::tests::{external_input_borrowed_lock…, external_input_disconnected_reply…, live_receipt_client_binds_query…, permission_inspection_reply_binds…}` | 0.6–0.8 s each | `await_until` on the reply rows; the runner child's settle windows use `sweep_interval_ms`/`shutdown_unit_ms` only where expiry is certain. Step 6 confirms each is under 1 s. |
| `runtime::input::tests::ordinary_prompt::deferred_queue::queue_head_terminal_commit_survives_a_temporary_storage_failure_and_publishes_once` | 2 s | DB clock on the contended host |
| `runtime::control::tests::input_client_tests::expired_and_stalled_exchanges_keep_the_original_deadline` | 0.8 s | Pass `TestClock` to `exchange_input`. The peer thread blocks on a channel instead of sleeping, the deadline passes through Advance, and the +100 ms tolerance is dropped. |
| `runtime::lifecycle::tests::runner_database_deadline_review_a_retired_inner_scope_restores_the_live_outer_scope` and the six sibling `runner_database_deadline_*` tests | 0.06–0.8 s | DB clock |
| `runtime::lifecycle::tests::launch_readiness_tests::runner_readiness_*` | 0.1 s each | Pass `TestClock` to `NativeLaunchHost`; Advance |
| `runtime::process::tests::mechanisms_tests::runner_real_shutdown_{kills_and_reaps…, closes_a_slave…}` | 0.35 s each | Real children stay. `PtySession` receives Gate's `YieldingClock` in place of the `SystemClock` whose `start` is moved back 1.7 s. Shutdown runs on a spawned thread under Gate. The condition is marker `started` from the TERM-ignoring leader, or from the slave-retaining descendant. The test's single `advance` passes only the child deadline. The SIGKILL storage phase then ends on the observed reap (`try_wait` returns the status and no survivor remains), because the storage deadline never passes under the yielding clock. The test asserts the exit status and the complete cleanup and drops the `< 4750 ms` bound. |
| `runtime::press::tests::press_entry_outcomes_follow_the_multiplexer_and_release_the_pane_lock` (ExpireMetadata) | 0.3 s | The probe sleeps through the clock (Advance) |
| `runtime::press::tests::…busy_shared_lock…` | 0.1 s | Advance |
| `runtime::recovery::tests::unpublished_missing_paths::…revalidates_epoch…` | 0.08 s | Advance |
| `runtime::artifacts::tests::lock_transfer::…contention_uses_the_original_absolute_deadline` | 0.08 s | DB clock |
| `broker::messaging::tests::concurrent_polls_on_separate_connections_return_disjoint_sets_covering_the_pending_set`, `broker::messaging::tests::message_ids_from_interleaved_inserts_increase_in_commit_order` | ≤1.1 s, ≤0.3 s | Sanctioned: real concurrent connections contend under SQLite's built-in `busy_timeout`, and the verdict is set membership and ordering, not elapsed time |

### Sanctioned real-time sites

After migration, test code touches real time only at these sites:

| Site | Why it stays |
|---|---|
| `time::test_support::{await_until, await_state}` | The hang guard itself |
| Fixture loops built on `await_until` that poll external state: `ServiceFixture` PTY reads, `TestChild::poll_until(hang_guard)`, CLI `run_test_child` | Polling external state, capped at `HANG_GUARD` |
| `PaneLock::acquire_released`, `RunLock::acquire_released` and `recover_released`, on `SystemClock` with a `HANG_GUARD` cap | They wait for another test's forked child to `exec` and close a shared flock, which is external real state |
| Real child processes, PTYs and sockets driven by Gate, Certain expiry or Barrier | Real children cannot run on virtual time, and no verdict reads elapsed time |
| `writer::StepBound`'s real step limit and the paused-hook wait | Production design; in tests it is a hang guard |
| `EmergencyInterrupt` watchdogs armed at `HANG_GUARD` | They fire only on a hang |
| SQLite's built-in busy handler between concurrent connections in `broker::messaging` concurrency tests | Real concurrency is the subject |
| Past-at-entry deadlines (`Instant::now() - 1 ms`) and `TestClock` / scripted clock data | Expiry by construction; injected time |
| `ETXTBSY` retries of freshly written executable stubs, rebuilt on `await_until` (capped at `HANG_GUARD`) in place of their 20 ms `std::thread::sleep` loops. These are `runtime::system::tests::bounded_run::run` (system.rs:788-798), `coding_agent::test_support` `run_stub` (mod.rs:722-735) and `coding_agent::hermes::tests::cli` `with_busy_retry` (hermes.rs:574-583). | Another test's forked child still holds the stub open for writing until its `exec`, which is external real state |

### Fixture parse (Step 2)

`catalog::test_support` holds the parsed profile files once per test process. Today `bundled_files()` returns a fresh owned `Value`, and `bundled_file(id)` returns an owned `(CodingAgent, Value)` clone of the whole file. Both change to borrow from the cache, and `bundled_screen_value` is new:

```rust
static BUNDLED_FILES: LazyLock<Value> = LazyLock::new(|| /* parse_bundled_text per file, as today */);

pub(crate) fn bundled_files() -> &'static Value;                       // changed: borrows
pub(crate) fn bundled_file(id: &str) -> (CodingAgent, &'static Value); // changed: borrows
pub(crate) fn bundled_screen_value(id: &str) -> &'static Value;        // new
pub(crate) fn bundled_fixture(id: &str) -> String; // unchanged signature; canonical JSON of bundled_screen_value(id)
```

- Callers that mutate a catalog (`without_ask_rules`, `with_synthetic_composer_evidence` inputs, the detector catalog builders) call `bundled_files().clone()`. Read-only callers borrow.
- `detector::test_support` adds `replay_screen(screen: &Value, change, alternate, cursor) -> ScreenSnapshot`. `replay_native_fixture(&str)` parses once and delegates to it. `replay_as_captured`, `document`, `fixture_lines`, `fixture_size`, `validate_pre_submit_fixtures` and `input::tests::…::after_native_submit` read the borrowed screen value directly, with no canonical-JSON round trip.
- `after_native_submit` takes each same-geometry screen's empty-composer classification from a `LazyLock<Mutex<HashMap<String, bool>>>` memo in `detector::test_support`, keyed by screen id. It no longer replays every same-geometry screen per fixture.
- A malformed bundled file still fails loudly: the `LazyLock` initializer panics with the guard message, and every test that reads it then fails. The bundled-catalog guard tests keep calling `parse_bundled_text` on the raw text.

### Redundancy inventory

Policy (user decision):
- REMOVE a test only when a retained test asserts the same behavior on the same inputs.
- FOLD means adding the listed assertions to the retained test first, then removing the folded test.
- MERGE turns near-identical tests into one table-driven test over a named case array, with the case name in every assertion message.
- Every-fixture sweeps stay.

Test paths drop the `tests::` segment after the file module where unambiguous. Line numbers refer to HEAD `362c4acd`.

#### REMOVE and FOLD

| Action | Test | Retained test | Evidence |
|---|---|---|---|
| REMOVE | `coding_agent::catalog::tests::composer_evidence::complete_operation_and_multiplexer_evidence_accepts_unrelated_additional_citations` (catalog.rs:3883) | `…::composer_evidence::every_witness_category_is_required_for_each_operation_on_each_multiplexer` | Same `validate_provenance(&measured())` on the same input. The retained test also checks the unrelated citation. |
| REMOVE | `coding_agent::catalog::tests::composer_evidence::explicitly_measured_null_plain_limits_are_valid` (catalog.rs:4054) | `…::composer_evidence::missing_plain_limit_key_is_rejected_even_when_the_plan_is_unlimited` | Same `validate_provenance(&unlimited()).unwrap()` |
| REMOVE | `runtime::input::tests::ordinary_prompt::composer_reliability::pre_submit::a_composer_rendering_the_written_literal_continues_to_the_submit_key` (input.rs:7221) | `…::composer_reliability::freshly_verified_empty_composer_confirms_the_written_submit_key` | Same steps and assertions; only the operation key string differs |
| REMOVE | `runtime::input::tests::ordinary_prompt::composer_reliability::pre_submit::a_key_typed_during_the_settle_records_literal_mismatch_without_a_submit_key` (input.rs:7291) | `…::pre_submit::an_appended_key_records_literal_mismatch_when_the_pre_submit_window_closes` | Same input sequence; the retained test's assertions are a superset |
| REMOVE | `coding_agent::tests::opencode_launch_policy_records_the_agent_for_the_omitted_and_explicit_modes` (mod.rs:1073) | `runtime_policy_omission_selects_each_backends_default_argv`, `runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode`, `required_preset_is_the_selected_opencode_agent_file_and_absent_for_other_backends`, `runtime_policy_help_lists_every_catalog_with_its_default` | Each assertion appears in a retained table row |
| REMOVE | `coding_agent::tests::pi::pi_launch_policy_records_the_gate_pair_for_the_omitted_and_explicit_modes` (mod.rs:1986) | The same omission, explicit and help tests | Same, for Pi |
| REMOVE | `runtime::service::tests::director_sweep::a_message_sent_while_no_runner_is_live_is_delivered_by_a_later_runner` (service.rs:8424) | `…::director_sweep::a_verified_empty_composer_receives_the_canonical_preview_then_its_message_is_stamped` | Same sweep and assertions. The only difference is a 60 s clock advance that target selection never reads. |
| REMOVE | `broker::runtime::events::tests::composer_reliability::immediate_submitted_inputs_publish_no_delivery_result` (events.rs:3040) | `broker::runtime::inputs::tests::stop_cause::immediate_stopped_results_remain_unpublished_until_their_first_durable_event` | Same `unpublished_delivery_results` call on the same setup; the retained test asserts a superset |
| FOLD | `cli::setup::tests::doctor_summary::doctor_reports_a_healthy_pi_backend_with_its_extension_path` (setup.rs:2686) | `…::doctor_summary::doctor_with_only_warnings_prints_ready_backends_and_exits_zero` | Add the `pi.extension` row assertion |
| FOLD | `cli::setup::tests::doctor_summary::doctor_process_prints_json_for_a_healthy_home` (setup.rs:2072) | `…::doctor_summary::doctor_process_prints_the_json_array_rows_as_text_in_check_order` | Add the empty-stderr, first-two-rows and all-ok assertions |
| FOLD | `runtime::service::tests::director_sweep::an_appended_key_records_literal_mismatch_when_the_pre_submit_window_closes` | `…::director_sweep::a_key_typed_during_the_settle_leaves_the_preview_unnotified_and_it_is_typed_again` | Add the `waited >= DIRECTOR_SETTLE + PRE_SUBMIT_WINDOW - DIRECTOR_VERIFICATION_POLL` (`TestClock` instants) and `pre_submit_captures > 1` checks after the first tick |
| FOLD | `runtime::service::tests::control_service::control_shutdown_outcome_diagnostics_are_stored_as_shutdown_rows` | `…::control_service::control_accepted_start_spawn_and_storage_failures_compensate` (`storage_failure` iteration) | Add the `source='shutdown'` row assertion; the number of child runs stays the same |
| FOLD | `runtime::control::tests::reply_inspection_supported_and_unknown_preserve_complete_public_object` (control.rs:3388) | `runtime::control::tests::reply_encoding_matches_independent_shapes_public_order_and_body_cap` (control.rs:3956) | The table's single inspection row uses `case.inspection(false)` (control.rs:3969). Add rows `inspection_unsupported` (`inspection(false)`) and `inspection_supported` (`inspection(true)`), each asserting `matches!(actual, InputControlReply::Inspection(_))` alongside the frame and key-order checks, and name every row (`submission_accepted`, `inspection_unsupported`, `inspection_supported`, `permission_response`, `receipt_missing`) |

#### MERGE — `src/coding_agent/catalog.rs`

| Merged test (module) | Members → case names |
|---|---|
| `plain_limit_diagnostics_include_the_prefix_and_measured_maximum_for_both_refusals` (`composer_reliability`) | `byte_plain_limit_diagnostics_include_the_prefix_and_measured_maximum_for_both_refusals` → `bytes`; `scalar_plain_limit_diagnostics_include_the_prefix_and_use_characters_for_both_refusals` → `scalars` |
| `literal_stages_frame_the_prefixed_utf8_literal_and_keep_the_submit_key` (`composer_reliability`) | `bracketed_literal_stages_preserve_the_plain_operation_prefix_and_utf8_literal` → `bracketed paste`; `plain_literal_stages_omit_framing_and_keep_the_selected_submit_key` → `plain` |
| `another_operation_or_multiplexer_claim_cannot_replace_the_selected_witness` (`composer_evidence`) | `another_operations_claim_cannot_replace_the_selected_operations_witness` → `operation`; `another_multiplexers_claim_cannot_replace_the_selected_multiplexers_witness` → `multiplexer` |
| `witness_values_must_match_the_selected_plan_values` (`composer_evidence`) | `framing_evidence_matches_each_selected_plan_value` → `framing`; `plain_limit_evidence_matches_the_unit_bound_and_null_declaration` → `plain_limit` |
| `composer_prefix_and_submit_hex_fields_are_validated` (`composer_submission`) | `composer_prefix_shape_hex_and_operation_identity_are_validated` → `prefix prompt "2120"`, `prefix exec ""`, `prefix exec 17`; `composer_submit_hex_requires_nonempty_even_lowercase_hex_and_string_type` → `submit ""`, `submit "0"`, `submit "ABCDEF"`, `submit null` |
| `bundled_composer_plans_submit_a_prompt_and_a_native_exec_in_separate_stages` (`bundled_backends`) | `hermes_composer_plans_submit_a_prompt_and_a_native_exec_in_separate_stages` → `hermes prompt`, `hermes exec`; `pi_composer_plans_submit_a_prompt_and_a_native_exec_in_separate_stages` → `pi prompt`, `pi exec` |
| `ask_user_question_captures_open_a_choiceless_question` (`bundled_claude_ask_user_question`) | `every_ask_user_question_form_capture_opens_a_choiceless_question` → `form captures`; `a_native_dialog_drawn_with_the_same_component_opens_the_same_choiceless_question` → `same-component native dialog`; `every_review_your_answers_capture_opens_a_choiceless_question` → `review-your-answers captures` (each case still sweeps its full fixture set) |
| `composer_plans_encode_the_exact_prefix_caller_literal_and_submit_bytes` (`tests`) | `composer_prompt_preserves_literal_utf8_between_empty_prefix_and_tested_submit_bytes` → `prompt`; `composer_exec_owns_exact_bang_space_prefix_without_changing_the_caller_literal` → `exec` |
| `a_rule_text_declaration_overrides_the_profile_for_its_regions` (`composer_text`) | `a_rule_text_prefixes_replace_the_profile_prefixes_for_its_regions` and `a_rule_text_wrap_overrides_the_profile_wrap_for_its_regions` → their existing case names, with a `text_wrap` column |
| `a_pre_submit_screen_without_an_unmasked_extractable_composer_is_rejected` (`pre_submit_fixtures`) | `a_pre_submit_screen_masks_no_row_of_its_composer_region` → `masked composer row`; `a_pre_submit_screen_without_composer_text_is_rejected` → `no composer text` |

#### MERGE — `src/coding_agent/detector.rs`, `hermes.rs`, `mod.rs`

| Merged test (module) | Members → case names |
|---|---|
| `operation_rows::operation_rows_extraction_cases` | `continuation_margin_rows_exempt_the_first_row_and_strip_every_later_margin` → `continuation-two-rows`, `continuation-three-rows`, `continuation-single-row`; `a_selected_row_without_its_margin_fails_the_match` → `continuation-missing-margin`, `all-margin-rows`, `default-margin-rows`; `the_default_margin_rows_keep_the_shipped_all_rows_extraction` → `default-single-space-margin`, `explicit-all`; `a_trailer_is_removed_with_its_preceding_whitespace_before_the_margin` → `trailer-padded`, `trailer-unpadded`, `trailer-mid-row`; `a_trailer_applies_to_a_single_row_operation_without_a_margin` → `trailer-single-row`; `a_blank_margined_row_inside_a_multi_line_command_still_matches` → `blank-margined-row` |
| `opencode_layouts::opencode_supported_layouts_report_every_displayed_command_row` | `opencode_split_layout_reports_every_displayed_command_row` → `split-wrapped`, `split-single`; `opencode_wide_layout_keeps_its_single_row_request` → `wide-single`; `opencode_compact_layout_reports_every_displayed_command_row` → `compact-055-shared`, `compact-052-own`, `compact-036-own`, `compact-055-single` |
| `opencode_layouts::opencode_forms_outside_the_supported_layouts_yield_no_request` | Itself (6 cases); `opencode_clipped_commands_yield_no_request_for_either_request_rule` (4 named cases); `opencode_compact_forms_outside_the_layout_yield_no_request` (6, prefixed `compact,`) |
| `opencode_shell_mode::opencode_composer_is_empty_only_above_the_normal_mode_footer` | `a_composer_above_the_normal_mode_footer_is_an_empty_composer` → `normal-cafleet`, `normal-cafleet-ask`; `a_shell_mode_composer_with_the_normal_glyph_is_not_an_empty_composer` → `shell-footer` (each × `VERSIONS`) |
| `claude_ask_user_question::the_live_form_with_a_foreign_row_written_into_it_is_no_interaction` | `the_live_form_with_a_live_composer_row_written_into_it_is_no_interaction` → `composer-row-no-cursor`, `composer-row-cursor-on-it`; `the_live_form_with_the_transcript_viewer_footer_written_into_it_is_no_interaction` → `transcript-footer` |
| `claude_2_1_312_guards::forms_outside_their_live_frame_are_no_interaction` | `a_dialog_a_quoted_above_a_live_composer_is_no_interaction` → `nudge-quoted-no-cursor`, `nudge-quoted-cursor`; `a_pending_notice_outside_its_live_frame_or_margin_is_no_interaction` (5, prefixed `notice,`); `a_batched_heading_outside_its_live_form_is_no_interaction` (4, prefixed `batched,`) |
| `tests::bundled_claude_operation_rows_report_each_displayed_row` | `bundled_claude_plain_operation_rows_keep_their_line_break_wherever_a_row_ends` → `split-path`, `full-row-mid-word`, `full-row-at-space`, `wide-cells-28`, `wide-cells-27`; `bundled_claude_operation_rows_remove_exactly_one_margin_space` → `indented-continuation`, `plain-gutter-glyph`; `bundled_claude_operation_is_only_the_rows_between_the_dashed_rules` → `full-width-description`; `bundled_claude_boxed_row_reaching_the_last_usable_column_keeps_its_line_break` → `boxed-col-119-of-120`, `boxed-col-51-of-52` |
| `tests::bundled_claude_written_padding_keeps_each_request_identity` | `bundled_claude_auto_tip_form_with_written_padding_rows_keeps_its_request_identity` → `auto-tip-synthetic`; `bundled_claude_read_outside_dialog_with_trailing_repaint_padding_keeps_its_request_identity` → `read-outside-fixture` (per-case extra assertions kept) |
| `question_answers::marker_focus_resolves_the_choice_on_the_marker_row` | `marker_focus_resolves_the_one_choice_on_the_marker_row` → `accept`, `decline`, `later`; `marker_focus_resolves_no_choice_when_the_marker_row_holds_no_choice_label` → `unlisted-option` |
| `focus_resolution::focus_style_background_resolves_the_focused_choice_by_background` | `focus_style_background_resolves_no_choice_for_none_several_or_an_unlisted_option` (4 names); `focus_style_background_compares_backgrounds_not_theme_colours` (4 themes × allow/deny/unfocused) |
| `focus_resolution::focus_row_background_is_the_majority_background` | `focus_row_background_is_the_majority_background_not_the_left_edge` → `edge-highlight`, `last-column`; `focus_resolves_no_choice_without_a_background_covering_more_than_half_the_row` → `40-of-80`, `51-painted` |
| `tests::runtime_detector_observes_a_permission_only_on_a_complete_live_form` | `runtime_detector_every_required_live_component_is_indispensable` → `baseline`, `row-N-replaced`; `runtime_detector_missing_or_empty_configured_capture_keeps_the_rule_incomplete` → `capture-present`, `capture-empty`, `capture-unmatched-branch`; `runtime_detector_duplicate_matchers_or_ambiguous_live_regions_remain_incomplete` → `duplicate-matcher`, `cursor-any-one`, `cursor-any-two`; `runtime_detector_region_limits_screen_selection_and_cursor_are_enforced` → `max-rows-6`, `max-rows-5`, `alternate-on-primary`, `alternate-on-alternate`, `cursor-away`, `cursor-inside-away`; `runtime_detector_cursor_endpoint_uses_wide_display_cells` → `wide-end-at-13`, `wide-end-moved`; `runtime_detector_title_transcript_scrollback_and_forged_actions_supply_no_live_request` → `title-and-forged`, `scrollback`, `transcript-viewer`; `runtime_detector_lower_priority_rule_cannot_override_the_stronger_permission_form` → `lower-priority-question` |
| `panel_clipping::panel_separator_clipping_cases` | `a_shared_separator_column_clips_every_panel_row_for_a_declaring_rule` (4 names); `differing_separator_columns_clip_no_row` → `col-81-on-row-2`, `col-60-on-last-row`, `rows-above-col-81`; `a_separator_glyph_on_its_rows_background_is_never_a_separator` → `plain`, `faint`, `row-background`, `boxed-gutter`; `the_separator_column_lies_from_column_40_before_the_last_column` → `col-39`, `last-col`, `col-40`, `col-118` |
| `hermes_profile::hermes_forms_outside_a_live_supported_layout_yield_no_request` | `hermes_forms_past_their_layout_limit_or_clipped_yield_no_request` → `survey-036`, `survey-027`, `bottomless-escalate`, `bottomless-override`, `truncated-command`; `hermes_form_text_outside_a_live_form_is_no_request` → `closed-above-idle`, `quoted-log`, `footerless`, `unboxed` |
| `composer_reliability::measured_working_editors_are_verified_empty_after_submit` | `measured_pi_working_editors_are_verified_empty_after_submit` → `pi-tmux`, `pi-herdr`; `measured_hermes_working_placeholders_verify_only_the_live_editor` → `hermes-tmux`, `hermes-herdr` |
| `composer_reliability::working_editors_with_input_or_an_outside_cursor_remain_unverified` | `pi_working_editors_with_typed_or_pasted_input_remain_unverified` → `pi-x`, `pi-paste`, `pi-retained`; `pi_working_editor_requires_the_cursor_inside_its_live_region` → `pi-cursor-0-0`; `hermes_working_editor_rejects_literal_suffixes_and_nonprefix_cursor_positions` → `hermes-x`, `hermes-paste`, `hermes-retained`, `hermes-cursor-0-0` (confirm its added `verified_noninteraction` check holds) |
| `pi_profile::pi_gate_forms_outside_a_live_supported_layout_yield_no_request` | `pi_form_with_any_row_below_its_single_operation_row_yields_no_request` (3 row sets); `pi_command_line_equal_to_a_choice_label_yields_no_request` → `choice-label-row`; `pi_forms_past_their_layout_limits_yield_no_request` → `wrapped-command`, `taller-than-max-rows`, `wrapped-hint-row`; `pi_gate_form_text_outside_a_live_form_is_no_request` → `above-idle-editor`, `quoted`, `primary-screen` |
| `pi_profile::pi_answer_plans_stage_gated_bytes_without_escape` | `pi_allow_plan_is_one_enter_gated_on_the_allow_once_focus` → `allow`; `pi_deny_plan_moves_with_j_then_enters_without_any_escape_byte` → `deny` |
| `cursor_free::a_capture_without_a_cursor_or_screen_mode_relaxes_those_conditions` | `a_capture_without_a_cursor_treats_every_cursor_condition_as_any` → `cursor-at_end`, `cursor-after_prompt`, `cursor-inside`; `a_capture_without_a_cursor_or_screen_mode_ignores_the_screen_selector` → `selector-primary`, `selector-alternate` |
| `tests::answer_matches_and_fingerprints_have_no_debug_or_serialized_representation` | `permission_inspection_answer_matches_have_no_debug_or_serialized_representation` → `AnswerMatch`; `runtime_profile_private_fingerprint_supports_comparison_without_debug_or_serialization` → `SemanticFingerprint` |
| `hermes::tests::settlement::composer_reliability::composer_empty_classifies_each_composer_region` | `composer_region_uses_sgr_to_distinguish_placeholders_from_typed_text` (10 rows); `composer_region_with_multiple_rows_including_blank_rows_is_nonempty` (5); `composer_region_selects_the_live_borders_instead_of_historical_prompt_rows` → `history-above-live`, `two-frames-last-drafted`, `titled-border` |
| `hermes::tests::settlement::settlement_reports_each_unanswered_control_within_the_deadline` | Itself (4 causes) plus `settlement_waits_for_a_new_stop_report_rather_than_one_already_on_screen` → `stale-stop-report-on-screen` (confirm the added deadline and `recorded` checks hold) |
| `coding_agent::tests::runtime_policy_unknown_choice_fails_with_the_exact_usage_error` | `runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error` → `codex-auto`; `pi::pi_unknown_launch_mode_fails_with_the_exact_usage_error` → `pi-auto` |
| `coding_agent::tests::forwarded_config_forwards_only_the_selected_variable` | `forwarded_config_returns_only_the_selected_variable_with_the_creator_exact_value` → `every-variable-set`; `forwarded_config_is_absent_when_the_selected_variable_is_unset` → `only-others-set` |
| `coding_agent::tests::hermes::hermes_version_rule_records_only_a_canonical_parenthesized_release_date` | Itself (11 lines) plus `hermes_version_rule_ends_the_package_token_only_at_ascii_whitespace` (4 named) |
| `coding_agent::tests::native_version_probe::native_launch_identity_records_each_parsed_version_with_its_profile_coverage` | `native_launch_identity_launches_every_parsed_version_with_the_backends_profile` (5 versions); `native_launch_identity_launches_the_lowest_parsed_version_with_the_bundled_profile` (per backend at 0.0.1); `native_launch_identity_without_any_profile_for_the_backend_records_unknown_silently` → `no-profile-claude`, `no-profile-opencode` |

#### MERGE — `src/runtime/input.rs`, `src/runtime/input_client.rs`

| Merged test (module) | Members → case names |
|---|---|
| `clock_capture_computes_the_expected_expiration` (`tests`) | `clock_capture_uses_later_instant_remaining_without_bracket_extension` → `later instant remaining`; `clock_capture_equal_bracket_preserves_one_nanosecond` → `equal bracket one nanosecond`, `origin one nanosecond`; `clock_capture_accepts_exact_unsigned_expiration_upper_bound` → `exact u64 upper bound` |
| `clock_capture_rejects_invalid_samples_and_deadlines` (`tests`) | `clock_capture_rejects_unsigned_expiration_overflow` → `u64 overflow`; `clock_capture_rejects_equal_or_expired_deadline_without_resampling` → `equal deadline`, `expired deadline`; `clock_capture_rejects_reversed_instant_bracket` → `reversed bracket`; `clock_capture_source_failure_consumes_one_sample_without_fallback` → `source failure` |
| `clock_receive_computes_the_conservative_deadline` (`tests`) | `clock_receive_uses_earlier_instant_for_conservative_deadline` → `earlier instant`; `clock_receive_caps_expiration_at_original_five_second_admission` → `five-second cap`; `clock_receive_preserves_positive_single_nanosecond` → `single nanosecond` |
| `clock_receive_rejects_invalid_samples` (`tests`) | `clock_receive_rejects_equal_or_past_monotonic_expiration` → `equal expiration`, `past expiration`; `clock_receive_rejects_bracket_delay_equal_to_or_past_projected_deadline` → `delay equal`, `delay past`, `late bracket`; `clock_receive_rejects_reversed_instant_bracket` → `reversed bracket`; `clock_receive_source_failure_consumes_one_sample_without_fallback` → `source failure` |
| `immediate_over_limit_input_is_refused_before_receipt_or_bytes` (`ordinary_prompt::composer_reliability`) | `immediate_over_limit_prompt_is_refused_before_receipt_or_bytes` → `prompt`; `immediate_over_limit_exec_is_refused_before_receipt_or_bytes` → `exec` |
| `a_stop_inside_bracketed_paste_records_short_write_and_closes_only_an_opened_paste` (`ordinary_prompt::composer_reliability`) | `a_stop_inside_a_utf8_scalar_closes_paste_and_remains_uncertain` → `inside a UTF-8 scalar`; `a_stop_inside_the_opening_marker_does_not_emit_a_close_marker` → `inside the opening marker`; `a_stop_inside_the_closing_marker_closes_paste_and_keeps_short_write_uncertainty` → `inside the closing marker` |
| `post_submit_verification_settles_by_composer_confirmation` (`ordinary_prompt::composer_reliability`) | `an_expired_post_submit_verification_records_uncertain_submit_unconfirmed` → `expired verification`; `freshly_verified_empty_composer_confirms_the_written_submit_key` → `freshly verified empty composer` |
| `an_attempt_deadline_short_of_the_next_write_records_deadline_expired` (`ordinary_prompt::composer_reliability`) | `a_chunk_deadline_at_a_complete_prefix_records_partial_deadline_expired` → `chunk deadline at a complete prefix`; `submit_key_requires_the_full_verification_window_before_writing` → `submit key without the full verification window` (confirm the "no `~` offer" assertion holds for the first case) |
| `a_written_immediate_prompt_failing_in_advance_records_uncertain_write_failed` (`ordinary_prompt::staged_delivery::immediate_failure_bookkeeping`) | `…_after_its_literal_records_uncertain_write_failed` → `after its literal`; `…_after_its_submit_key_records_uncertain_write_failed` → `after its submit key` |
| `a_question_answer_of_the_wrong_kind_or_untested_is_rejected_with_zero_bytes` (`permission_response_contracts::question`) | `a_decision_on_a_question_is_rejected_with_zero_bytes` → `allow decision`, `deny decision`; `a_choice_without_a_tested_plan_is_rejected_with_zero_bytes` → `untested accept choice`, `missing choice` |
| `input_client::tests::uncertain_reason::uncertain_reply_reason_must_equal_the_receipt_cause` | `uncertain_reply_must_carry_the_receipt_cause` → `a recorded cause`; `uncertain_reply_for_a_causeless_receipt_carries_no_reason` → `a causeless receipt` |

#### MERGE — `src/runtime/{service,observation,control,transport,process}.rs`

| Merged test (module) | Members → case names |
|---|---|
| `service::tests::pacing_tests::tick_sleep_paces_the_next_tick` | `pacing_starts_the_next_tick_at_once_while_outer_input_moves` → `outer bytes read`, `positive master write`, `both` (each `/ no deadline`, `/ deadline in 1 s`); `pacing_keeps_the_idle_sleep_without_outer_input` → `idle, no deadline`, `idle, later deadline`, `idle, earlier deadline shortens the sleep`, `idle, deadline passed` |
| `observation::tests::bundled_claude_tracker::bundled_claude_replays_open_continue_and_close_their_episodes` (over `BUNDLED_CLAUDE_REPLAYS`) | `bundled_claude_auto_tracker::tracker_opens_one_permission_episode_for_the_tip_form_and_closes_it_once_the_command_runs` → `2.1.302 tip form closes once the command runs`; `bundled_claude_2_1_312_tracker::tracker_replaces_the_pending_notice_with_dialog_a_after_the_draft_is_cleared` → `2.1.312 cleared draft replaces notice with Dialog A`; `…::tracker_closes_the_pending_notice_without_dialog_a_after_the_draft_is_sent` → `2.1.312 sent draft closes notice without Dialog A`; `…::tracker_closes_dialog_a_once_its_decline_returns_the_composer` → `2.1.312 Dialog A decline closes on composer`; `…::tracker_supersedes_the_first_batched_read_with_the_second_after_its_allow` → `2.1.312 batched read supersedes`; `bundled_claude_ask_user_question_tracker::tracker_keeps_one_question_episode_across_the_form_and_closes_it_on_the_idle_composer` → `2.1.322 ask form keeps one episode`; `…::tracker_opens_the_review_screen_as_the_same_question_class` → `2.1.322 review screen opens as question`; `…::tracker_continues_the_pending_notice_into_the_form_in_one_episode` → `2.1.322 notice continues into form`. Optional expected fields are `None` where the original asserted nothing. |
| `control::tests::input_client_tests::exchange_input_decodes_preloaded_replies` | `typed_exchange_sends_an_independent_flat_request_with_the_original_expiry` → `accepted`; `textual_error_remains_error_and_structured_uncertainty_remains_result` → `textual_error`, `uncertain_partial`, `uncertain_unknown`; the `[duplicate, wrong]` loop of `exchange_validates_fragmented_duplicate_and_wrong_kind_replies` → `duplicate_input_id`, `wrong_result_kind`; the `[extra, partial]` loop of `exchange_rejects_extra_reply_trailing_partial_and_premature_eof` → `extra_reply`, `trailing_partial`. The remainders are renamed `exchange_reassembles_a_fragmented_reply` and `exchange_rejects_a_premature_eof`. |
| `transport::tests::drain_incomplete_mid_observer_fault_feeds_the_retained_remainder_on_the_next_drain` | `…_mid_observer_expiry_…` → `deadline expiry`; `…_mid_observer_error_before_expiry_…` → `observer error before expiry`; `…_mid_observer_error_after_expiry_…` → `observer error after expiry` |
| `transport::tests::drain_incomplete_final_step_refusal_reobserves_the_snapshot_before_sealing` | `drain_incomplete_final_step_deadline_guard_expiry_reobserves_the_snapshot_before_sealing` → `deadline guard expiry`; `drain_incomplete_final_step_observer_error_reobserves_the_snapshot_before_sealing` → `observer error` |
| `transport::tests::classification::reply_classifier_input_split_at_every_boundary_classifies_only_when_complete` | `pointer::pointer_reports_split_at_every_boundary_classify_only_when_complete` → `pointer reports`; `classification::reply_classifier_reply_split_at_every_boundary_counts_zero` → `terminal replies` |
| `transport::tests::runner_transport_one_tick_forwards_outer_input_unchanged_and_counts_only_manual_bytes` | `runner_transport_forwards_data_read_before_outer_eof_and_reports_eof` → `data read before outer EOF`; `runner_transport_forwards_a_terminal_reply_burst_unchanged_without_manual_generation` → `reply burst`; `runner_transport_counts_only_manual_bytes_around_a_reply` → `text around a reply`; `runner_transport_outer_eof_counts_a_pending_reply_prefix_as_manual` → `pending prefix at outer EOF` |
| `transport::tests::capture_replay::a_capture_replays_every_row_at_its_position` | `a_tmux_capture_replays_every_row_at_its_position_with_its_cursor_and_screen_mode` → `tmux primary`, `tmux alternate`; `a_full_width_row_leaves_the_next_row_at_its_own_position` → `full-width row`; `a_body_without_its_trailing_blank_rows_replays_at_the_viewport_height` → `body without trailing blank rows` |
| `process::tests::process_table_tests::stat_parser::parse_stat_parent_cases` | `reads_the_second_field_after_the_last_closing_parenthesis` → `parenthesized_comm`; `missing_parent_field_is_malformed` → `missing_parent`; `non_integer_parent_field_is_malformed` → `non_integer_parent`; `stat_without_a_closing_parenthesis_is_malformed` → `no_closing_paren` |
| `process::tests::process_table_tests::cmdline_parser::parse_cmdline_cases` | `splits_on_nul_and_drops_the_trailing_empty_segment` → `trailing_nul`; `keeps_the_last_argument_without_a_trailing_nul` → `no_trailing_nul`; `keeps_an_empty_argument_before_the_end` → `empty_inner_arg`; `empty_cmdline_is_an_empty_argv` → `empty` |
| `process::tests::process_table_tests::nspid_parser::parse_nspid_nested_cases` | `a_single_pid_is_not_nested` → `single_pid`; `several_pids_are_nested` → `several_pids`; `an_absent_nspid_line_is_not_nested` → `absent_line`; `a_non_integer_field_is_malformed_with_the_line` → `non_integer_field` |
| `process::tests::process_table_tests::ps_parser::parse_ps_output_cases` | `reads_the_parent_and_the_whitespace_split_argv` → `parent_and_argv`; `reads_the_first_non_blank_line` → `leading_blank_lines`; `blank_output_is_none` → `empty`, `whitespace_only`; `a_non_integer_parent_is_no_parent` → `non_integer_parent`; `a_parent_without_a_command_is_no_command` → `parent_without_command` |
| `process::tests::ownership_refresh_tests::reaped_stat::stat_read_outcomes` (linux) | `runner_process_esrch_stat_read_is_a_vanished_process` → `reaped_esrch`; `runner_process_other_stat_read_errors_still_fail` → `denied_eacces`; `runner_process_live_stat_text_keeps_its_identity_and_running_state` → `live_self` |
| `process::tests::ownership_refresh_tests::survivors_with_cases` | `survivors_report_a_live_group_member_the_closed_refresh_never_pinned` → `live_unpinned_member`; `survivors_ignore_a_zombie_group_member_once_owned_processes_are_gone` → `stopped_member` |
| `process::tests::ownership_refresh_tests::refresh_owned_with_cases` | `refresh_matching_anchor_adopts_only_original_active_generations` → `matching_anchor_adopts`; `refresh_initial_empty_closes_with_one_query_and_retains_map_and_clock` → `initial_empty_closes`; `refresh_stale_active_then_fresh_empty_closes_without_adoption_or_more_reads` → `stale_active_then_fresh_empty_closes`; `refresh_fresh_remaining_members_preserve_exact_refusal_without_adoption` → `fresh_members_refuse`; `refresh_same_pid_replacement_cannot_supply_an_anchor_or_be_adopted` → `same_pid_replacement_refuses`; `refresh_original_observation_errors_stop_at_the_actual_first_failure` → `members_error`, `identity_error`, `stopped_error`, `anchor_identity_error`; `refresh_anchor_fold_still_reads_later_pins_after_a_match_and_propagates_error` → `later_anchor_read_error`; `refresh_fresh_query_error_propagates_without_closure_or_followup_reads` → `fresh_query_error`; `refresh_anchored_pid_mismatch_retains_fence_without_fresh_query_fallback` → `anchored_pid_mismatch`; `refresh_absent_and_stopped_members_close_before_any_anchor_fold` → `absent_and_stopped_close` |

#### MERGE — remaining modules

Merges marked † drive real time today. They land in the Step that converts them (Steps 4–6), so that sequential cases never add real waits.

| Merged test (module) | Members → case names |
|---|---|
| `broker::runtime::inputs::tests::director_preview_stamp::a_director_preview_stamp_refusal_leaves_notified_at_unset` | `a_director_preview_stamp_refuses_a_message_to_another_recipient` → `another_recipient`; `a_director_preview_stamp_requires_the_current_run_owner` → `superseded_owner` |
| `broker::members::tests::idle_seconds_measures_from_the_latest_activity` | `idle_clamps_future_activity_to_zero` → `future_consumption_clamps_to_zero`; `idle_measures_from_a_consumption_later_than_every_send_and_receipt` → `consumption_later_than_send_and_receipt`; `idle_selects_raw_lexicographic_maximum_before_parsing_offsets` → `raw_lexicographic_maximum_before_offsets`; `idle_preserves_fractional_second_truncation` → `fractional_send_truncates_to_zero`, `fractional_consumption_truncates_to_one` |
| `broker::runtime::events::tests::persist_transition_rejects_a_misshapen_or_conflicting_transition_before_commit` | `event_shape_regression_rejects_a_misshapen_transition_before_commit` (5 cases); `transition_outside_reserved_sequences_or_with_invalid_payload_is_rejected_atomically` → `an opening outside the reserved sequences`, `an opening without evidence`; `each_conflicting_original_transition_field_rejects_sequence_reuse` → `a reused sequence with a different observed time`, `… episode`, `… kind`, `… classification` |
| `broker::runtime::events::tests::composer_reliability::distinct_stalled_records_are_each_published` | `blocked_records_naming_different_blockers_are_each_published` → `two blocked records naming different blockers`; `a_head_record_and_a_blocked_record_do_not_deduplicate_each_other` → `a blocked record followed by a head record` |
| `output::json::tests::placement_json::a_placement_orders_its_window_and_slot_keys_before_the_pane_and_nulls_pending_ids` | Itself → `concrete placement`, `pending placement`; `column_sides_render_as_their_stored_values` → `concrete right-column placement` |
| `runtime::recovery::tests::unpublished_history::runner_recovery_machine_missing_paths_keep_terminal_bookkeeping` | `runner_recovery_machine_missing_paths_preserve_original_exited_and_failed_history` → `original exited history`, `original failed history`; `runner_recovery_machine_missing_paths_keep_committed_native_exit_code_signal_and_message` → `committed native exit signal` |
| `runtime::lifecycle::tests::launch_readiness_tests::runner_readiness_startup_interval_becomes_ready_within_the_original_deadline` † | `runner_readiness_bound_socket_becomes_listening_within_the_original_deadline` → `bound socket becomes listening`; `runner_readiness_pre_private_socket_waits_without_sending_prepare` → `pre-private socket waits without sending prepare` |
| `runtime::lifecycle::tests::runner_creation_unwind_during_launch_compensates_every_owned_effect_once` | `runner_creation_unwind_after_split_closes_the_owned_pane_once` → `prepared`; `runner_creation_unwind_after_spawn_stops_the_owned_child_and_closes_the_pane_once` → `started` |
| `runtime::system::tests::timed_runner_times_out_and_reaps_the_direct_child` † | `timed_runner_kills_and_reaps_a_sleeping_direct_child_within_five_seconds` → `sleeping direct child`; `timed_runner_checks_deadline_while_both_streams_keep_producing` → `both streams keep producing`; `timed_runner_applies_deadline_after_child_exit_with_inherited_pipes_open` → `child exited with inherited pipes open`; `timed_runner_waits_for_child_exit_even_after_both_pipes_close` → `both pipes closed before exit` |
| `runtime::system::tests::timed_runner_returns_decoded_stdout_on_success_and_stderr_on_failure` | `timed_runner_drains_simultaneous_megabyte_streams_on_success` → `megabyte streams, success`; `timed_runner_drains_simultaneous_megabyte_streams_on_failure` → `megabyte streams, exit 9`; `timed_runner_decodes_successful_stdout_as_lossy_utf8` → `invalid UTF-8 stdout, success`; `timed_runner_decodes_failed_stderr_as_lossy_utf8` → `invalid UTF-8 stderr, exit 1` |
| `runtime::system::tests::drain_pipe_reads_the_64_kib_budget_and_keeps_at_most_the_cap` | `per_stream_budget_is_64_kib_even_with_short_reads` → `3-byte reads, no cap`, `8192-byte reads, no cap`; `a_drained_stream_keeps_at_most_its_cap_while_still_reading_the_budget` → `8192-byte reads, cap 100` |
| `multiplexer::caller::tests::walk_termination::walk_stops_at_the_4096_entry_cap` | `a_chain_longer_than_4096_entries_stops_at_the_last_recorded_pid` → `a chain longer than the cap`; `a_parent_cycle_terminates_at_the_cap` → `a parent cycle` |
| `multiplexer::caller::tests::evidence_fixtures::herdr_recorded_sessions_reach_their_expected_row` | `herdr_hermes_with_the_pane_shell_refuses_at_row_1` → `hermes gateway with the pane shell`; `herdr_codex_daemon_pane_a_refuses_at_row_1` → `Codex daemon, pane A`; `herdr_real_director_session_verifies_its_pane_shell` → `real Director session` |
| `multiplexer::tests::caller_server_resolves_each_backend_server_from_the_environment_or_its_default` | `tmux_caller_server_is_the_tmux_socket_path_before_its_last_two_commas` → `tmux socket before the last two commas`; `tmux_caller_server_without_tmux_asks_the_default_server_for_its_socket` → `empty TMUX asks the default server`, `a default-server failure finds no server`, `an empty default-server reply finds no server`; `herdr_caller_server_prefers_the_session_then_the_matching_socket_then_default` → `HERDR_SESSION wins over the socket`, `an empty HERDR_SESSION uses the default session`, `a socket matching a listed session`, `a socket matching no listed session` |
| `time::tests::parse_lenient_accepts_rfc3339_spellings_and_rejects_garbage` | `parse_accepts_missing_fractional_part` → `missing fractional part`; `parse_accepts_z_offset_spelling` → `Z offset spelling`; `parse_rejects_garbage` → `prose`, `empty string` |
| `cli::member::tests::hermes_member_create_probes_config_path_and_version_before_registration` † | `hermes_member_create_fails_before_registration_when_the_hermes_home_cannot_be_resolved` → `config path prints a relative path`, `config path exits non-zero`; the CLI-child remainder of `hermes_member_create_bounds_config_path_by_five_seconds_and_the_version_probe_by_two` → `both probes answer at once` (its bound cases move as § Real-time inventory states) |
| `cli::member::tests::wrapped_delivery_refusal_names_status_cause_and_method` | `wrapped_uncertain_prompt_and_exec_errors_name_the_recorded_cause_and_method` (4 cases); `possible_io_refusals_keep_their_category_instead_of_claiming_no_dispatch` (12 generated cases) |
| `cli::member::tests::capture_snapshot_from_raw_renders_content_and_hash_per_ansi_mode` | `snapshot_strips_csi_and_cr_rewrites_before_hashing_utf8` → `plain`; `snapshot_ansi_preserves_every_raw_byte_and_hashes_that_content` → `ansi` |
| `cli::member::tests::launch_policy::pi_launch_inputs_fail_before_any_creation_effect` | `pi_prompt_beginning_with_an_at_sign_fails_before_any_creation_effect` → `inline prompt with leading @`, `file prompt with leading @`; `pi_model_with_a_thinking_suffix_fails_before_any_creation_effect` → `model with thinking suffix` |
| `multiplexer::tmux::tests::director_pane::type_literal_sends_literal_chunks_of_at_most_256_characters_in_order` | Itself → `long literal in 256-character chunks`; `type_literal_sends_a_short_literal_in_one_process` → `short literal in one process` |
| `multiplexer::tmux::tests::close::close_pane_then_restore_after_close_equalizes_the_recorded_window` | `close_pane_closes_only_and_restoration_equalizes_the_recorded_window_later` → `columns to equalize`; `restoring_a_closed_window_does_nothing_more` → `closed window` |
| `multiplexer::herdr::tests::spawn::spawn_member_pane_runs_names_and_equalizes_each_target` | `a_column_with_members_splits_its_live_bottom_pane_then_runs_names_and_equalizes` → `a column with members splits its live bottom pane`; `the_director_windows_first_member_splits_the_lone_left_pane_rightward` → `the Director window's first member splits rightward`; `a_new_window_is_a_detached_labelled_tab_whose_root_pane_is_the_member` → `a new window is a detached labelled tab` |
| `multiplexer::herdr::tests::spawn::an_empty_column_beside_a_stacked_column_rebuilds_the_other_chain` | `an_empty_right_column_beside_a_stacked_left_column_rebuilds_the_left_chain` → `right column beside 2 left panes`, `right column beside 4 left panes`; `an_empty_left_column_moves_the_right_chain_beside_the_new_pane` → `left column beside 1 right pane`, `left column beside 3 right panes` |
| `multiplexer::herdr::tests::director_pane::capture_screen_takes_the_geometry_from_the_snapshot_and_the_body_from_the_visible_read` | `capture_screen_takes_the_width_and_rows_from_the_snapshot_and_the_body_from_the_visible_read` → `three viewport rows`; `capture_screen_keeps_the_viewport_rows_when_the_visible_read_drops_trailing_blank_rows` → `viewport rows below the last drawn row` |
| `multiplexer::herdr::tests::equalize::restore_after_close_equalizes_each_column_with_its_own_down_chain` | `restoration_equalizes_every_column_of_the_recorded_tab` → `two columns with their own down chains`; `a_column_without_its_own_down_chain_is_left_untouched` → `a split of another width never matches` |
| `pane_lock::tests::pane_lock_wait_tests::acquire_within_waits_for_the_holder_until_the_reserved_deadline` † | `an_uncontended_lock_is_acquired_at_once` → `uncontended`; `a_lock_released_within_the_wait_is_acquired` → `released within the wait`; `a_lock_still_busy_at_the_reserved_deadline_returns_none` → `busy at the reserved deadline` |
| `pane_lock::tests::layout_lock::layout_lock_acquire_within_waits_only_for_the_same_fleet_holder` † | `a_second_acquisition_blocks_until_the_holder_releases` → `holder releases during the wait`; `an_expired_wait_reports_busy_after_its_full_budget` → `holder never releases`; `distinct_fleets_do_not_contend` → `another fleet holds its lock`; `a_released_layout_lock_is_free_for_the_next_holder` → `holder released before the wait` |
| `cli::fleet::tests::fleet::runner_review_fleet_owner_change_stops_deletion_and_preserves_registry` | `runner_review_fleet_owner_change_during_shutdown_stops_before_pane_close` → `after_shutdown`; `runner_review_fleet_final_identity_change_preserves_registry_without_repeating_effects` → `after_close` |
| `cli::fleet::tests::create::an_explicit_pane_reads_no_process_table_and_ignores_the_other_multiplexer_environment` | `an_explicit_tmux_pane_reads_no_process_table_and_ignores_a_herdr_environment` → `tmux_pane_with_herdr_env`; `an_explicit_herdr_pane_reads_no_process_table_and_ignores_a_tmux_environment` → `herdr_pane_with_tmux_env` |
| `cli::fleet::tests::create::a_verified_env_pane_registers_its_context` | `a_verified_tmux_env_pane_registers_its_context` → `tmux_env_pane`; `a_verified_herdr_env_pane_registers_its_context` → `herdr_env_pane` |
| `cli::fleet::tests::create::a_failed_window_name_read_registers_nothing` | `a_failed_tmux_window_name_read_registers_nothing` → `tmux_empty_reply`, `tmux_window_without_name`, `tmux_other_window`; `a_failed_herdr_window_name_read_registers_nothing` → `herdr_tab_not_found`, `herdr_missing_label` |
| `cli::fleet::tests::create::post_rename_registration_failures_keep_the_applied_name_with_a_naming_diagnostic` | `registration_insert_failures_leave_the_applied_name_and_director_pane_in_place` (6 cases); `commit_failure_keeps_naming_diagnostics_and_performs_no_terminal_compensation` → `tmux_commit_refused`, `herdr_commit_refused` |
| `cli::message::tests::next_command::poll_command_quotes_the_database_url_only_when_it_holds_an_unsafe_character` | `the_next_poll_command_appends_the_database_url_quoted_only_when_needed` (5 cases); `every_character_outside_the_safe_set_quotes_the_whole_url` → generated `unsafe {c:?}` |
| `error::tests::usage_and_app_errors_map_to_their_exit_codes` | `usage_error_maps_to_exit_code_2` → `usage`; `app_error_maps_to_exit_code_1` → `app` |
| `assets::tests::default_installation_writes_every_asset_under_the_native_directory` | `opencode_default_installation_uses_the_native_config_directory_for_every_asset` → `opencode`; `pi::pi_default_installation_writes_skills_extension_and_record_under_the_pi_agent_directory` → `pi` |
| `assets::tests::blocked_extension_target_fails_installation_after_the_completed_lines` | `opencode_extension_failure_fails_installation` → `opencode`; `pi::pi_extension_failure_fails_installation` → `pi` |
| `db::tests::existing_file_opener::an_existing_connection_keeps_pragmas_and_honours_its_write_mode` | `an_existing_writable_connection_keeps_pragmas_and_persists_writes` → `writable`; `an_existing_read_only_connection_keeps_pragmas_and_refuses_writes` → `read-only` |
| `cli::setup::tests::extension_check_reports_each_extension_and_record_state` | `opencode_extension_check_reports_each_plugin_and_record_state` (7 cases `opencode …`); `pi::pi_extension_check_reports_each_extension_and_record_state` (8 cases `pi …`); `pi::pi_extension_check_reads_a_relocated_base` → `pi relocated base installed` |
| `cli::setup::tests::extension_check_is_unavailable_without_the_executable_or_configuration` | The `opencode_…` and `pi::pi_…` tests of that name → `opencode`, `pi` |
| `cli::setup::tests::doctor_summary::doctor_reports_a_tampered_extension_on_its_row` | `doctor_reports_a_tampered_opencode_extension_on_its_row` → `opencode`; `doctor_reports_a_tampered_pi_extension_on_its_row` → `pi` |
| `cli::setup::tests::doctor_summary::doctor_process_warns_on_the_caller_pane_and_exits_zero` | `doctor_process_under_a_codex_app_server_warns_on_the_caller_pane_and_exits_zero` → `codex app-server ancestor`; `doctor_process_with_tmux_pane_unset_warns_on_the_caller_pane_and_exits_zero` → `TMUX_PANE unset`; `doctor_process_with_a_non_ancestor_pane_warns_unverified_and_exits_zero` → `non-ancestor pane` |

Also drop the duplicated `(["setup","--unknown"], UnknownArgument)` case from `cli::setup::tests::setup_rejects_unknown_options_positionals_and_agent_values`, because `cli::tests::command_options_are_scoped_to_their_subcommands` keeps it.

#### Counts

FOLD rows count as removals. "Merged away" counts whole member tests of each MERGE table, including a table's own pre-existing test. In the `exchange_input_decodes_preloaded_replies` row, the two tests that keep a trimmed remainder (`exchange_reassembles_a_fragmented_reply`, `exchange_rejects_a_premature_eof`) are not counted. "Added" counts the tests the hermes probe-bounds split creates: the `preflight_deadlines` table test and one in-process test each for `probe_native_version` and `hermes::effective_home`.

| Scope | Before | Removed or folded | Merged away | Added | After |
|---|---|---|---|---|---|
| `src/coding_agent/catalog.rs` | 173 | 2 | 21 into 10 | 0 | 160 |
| `src/coding_agent/{detector,hermes,mod}.rs` | 207 | 2 | 73 into 26 | 0 | 158 |
| `src/runtime/{input,input_client}.rs` | 179 | 2 | 29 into 11 | 0 | 159 |
| `src/runtime/{service,observation,writer,control,transport,process}.rs` | 332 | 4 | 58 into 15 | 0 | 285 |
| Every other file | 584 | 3 | 98 into 41 | 3 | 527 |
| Total (`#[test]` functions, including ignored child entries) | 1475 | 13 | 279 into 103 | 3 | 1289 |

The Before total counts `#[test]` attributes. libtest on Linux lists 1473, because 2 of them sit in `#[cfg(target_os = …)]` code for another platform. No row of this inventory touches those 2, so Step 3 expects 1289 `#[test]` functions and a Linux libtest count of 1287.

### CPU residue

After the fixture-parse cache, these tests exceed 2 s or come close without waiting on real time. Each is solved in order: (1) remove repeated work in test helpers, (2) skip idle virtual turns, (3) split a table test into per-backend or per-fixture-set table tests, only when steps (1) and (2) leave it above 2 s.

| Test (post-cache `exec_time`) | Known cause | Remedy |
|---|---|---|
| `runtime::input::tests::ordinary_prompt::composer_reliability::every_authoritative_retained_native_snapshot_leaves_submit_unconfirmed` (11.1 s) | `after_native_submit` replays every same-geometry screen per fixture | The memo in § Fixture parse |
| `runtime::input::tests::ordinary_prompt::deferred_queue::queue_attempt_deadline_with_a_busy_pane_lock_settles_not_submitted_and_publishes_once` (7.5 s) | `turns_until` advances 5 ms per turn across a 5 s virtual budget | `turns_until` skips idle turns to the coordinator's next wake while no bytes are in flight, the same rule as the service scenario harness's `skip_idle_turns` |
| `runtime::input::tests::ordinary_prompt::deferred_queue::queued_stopped_inputs_publish_once_and_resume_only_after_reconciliation` (2.5 s), `…::composer_reliability::pre_submit::a_mismatched_prompt_or_exec_stays_uncertain_and_stops_the_fifo` (2.0 s), `…::permission_response_contracts::a_prompt_and_exec_during_dialog_a_queue_with_zero_bytes_and_each_drains_once` (2.0 s) | Virtual turns | Idle-turn skipping, then measure |
| `runtime::service::tests::scenarios::{a_head_that_stalls_again_after_an_interaction_archives_the_repeat_and_later_events_persist (4.0 s), a_control_prompt_queues_behind_an_in_flight_sweep_or_an_open_interaction (3.0 s), an_uncertain_prompt_stops_queued_input_and_the_sweep_until_reconciled (2.6 s), an_obstacle_defers_the_sweep_with_zero_bytes_until_it_clears (2.4 s)}` | Per-turn SQLite and detector work across many turns | Count turns per step; extend `skip_idle_turns` to the waits that hold no bytes; then measure |
| `coding_agent::catalog::tests::bundled_backend_files::{the_explicit_class_guard_names_the_profile_rule_and_construct_in_every_pattern_field (3.2 s), the_explicit_class_guard_admits_every_escape_of_the_pattern_syntax (2.5 s)}` | Repeated catalog validation per mutated pattern | Reuse one parsed bundled catalog per case (§ Fixture parse) and build each mutation from `bundled_files().clone()` once per case, not once per pattern; then measure |

Step 7 re-measures the suite and adds any remaining test over 2 s to this table, with its cause and remedy, in its PR.

### Measurement protocol

1. `mise //:test -- --no-run` builds the test binary and prints its path.
2. `RUSTC_BOOTSTRAP=1 RUST_TEST_THREADS=8 <test-binary> -Z unstable-options --format json --report-time` runs from the repository root. It emits one JSON line per test (`exec_time`) and a final suite line (`exec_time` = wall).
3. Record the median wall of 3 runs and every test over 2 s. Note concurrent host load.

`RUSTC_BOOTSTRAP=1` is set only on the test binary, which reads it at run time to accept `-Z unstable-options`. It is never set on the build, so it cannot change build fingerprints.

### Flakiness gate

Every PR of § Implementation passes 10 consecutive runs of `RUST_TEST_THREADS=16 taskset -c 0,1 mise //:test` (2 cores, 16 test threads) before merge, recorded in the PR description. This is a manual verification command; no CI job or mise task is added.

### Per-step audit

Each of Steps 4–6 reviews every hit of these searches inside `#[cfg(test)]` code of the files it converted. Each hit is either removed or an entry of § Sanctioned real-time sites:

```text
grep -n "thread::sleep" <file>
grep -n "Instant::now()" <file>
grep -nE "elapsed\(\)|duration_since" <file>
grep -nE "recv_timeout|wait_timeout|poll_until|read_until" <file>
grep -nE "Duration::from_(millis|secs)\(" <file>
```

Production hits in the same files that feed a deadline either go through the seam from § Production seams or are diagnostics.

### Rule and documentation text

`.claude/rules/code-quality.md` § Test placement replaces its runner-clock bullet with:

```markdown
- Production code that reads a deadline, budget, retry interval or sleep takes time from an injected `time::Clock`. This covers runner code in `src/runtime/` and `src/broker/runtime/`, pane and layout locks, subprocess bounds, multiplexer runners, native probes and fleet and member deletion. It takes the clock as a parameter, a field or the deletion host's `clock()`. The `db.rs` budget machinery reads the thread's database clock (`time::database_now` / `time::database_sleep`), because SQLite's busy and progress callbacks take no parameter.
- Every test whose code under test reads a deadline, budget, retry interval or sleep passes it an injected `TestClock` and moves time explicitly, so no verdict depends on elapsed real time. A test that drives a real process, PTY, socket or lock expires its bounds through the injected clock once the child reaches the state the verdict needs. It waits for consequences with `time::test_support::await_until` or `await_state`, and it asserts absence at a later ordered event. A database scope reads the test's clock after `time::test_support::DatabaseClock::install` on the thread that runs it. Real time remains only where it is the subject or external state, such as successful real-process runs, concurrent SQLite connections and a service child's own clock. Even there no verdict reads elapsed time, and every real-time cap is `HANG_GUARD`, which a passing run never reaches.
- Shared fixture data is parsed once per test process: helpers that read the embedded profile files borrow from one cached parse and never re-parse per lookup.
```

The bullet's Forbidden patterns list gains `A real sleep, real deadline or elapsed-time assertion in a test verdict` and `A test helper that re-parses embedded data on every call`.

The sentence in `docs/docs/contributing.md` ("runner tests take time from the injected clock and use real time only when they drive real processes, PTYs, sockets or locks") becomes: "code that reads a deadline, budget, retry or sleep takes time from the injected clock, and its tests drive that clock and expire bounds through it even when they drive real processes, PTYs, sockets or locks; no verdict depends on elapsed real time, and every real-time cap is a hang guard a passing run never reaches".

`SPEC.md`, `README.md` and `assets/skills/` describe no test mechanics and stay unchanged.

### Out of scope

- The test build profile. Local runs stay at opt-level 0; CI keeps `CARGO_PROFILE_TEST_OPT_LEVEL=1`.
- Machine enforcement, such as clippy `disallowed-methods` or a per-test duration budget.
- Trimming every-fixture sweeps.
- Test placement fixes the survey found:
  - `runtime::lifecycle::tests::runner_database_deadline_*` belong in `db.rs`;
  - `runtime::process::tests::runner_real_raw_terminal_guard_restores_only_its_owned_fixture_terminal` belongs in `transport.rs`;
  - loops that do not name their case.
- Production timing constants and crate-external behavior.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Each step from Step 2 on is one PR that leaves the suite green and passes the § Flakiness gate. Step 1 lands in the Step 2 PR.

### Step 1: Documentation and rules (lands in the first PR)

- [ ] Replace the runner-clock bullet and extend Forbidden patterns in `.claude/rules/code-quality.md` § Test placement per § Rule and documentation text <!-- completed: -->
- [ ] Update the test sentence in `docs/docs/contributing.md` per § Rule and documentation text and run `mise //docs:build` <!-- completed: -->
- [ ] Set `design-docs/0000222-deterministic-test-timing/design-doc.md` Status to `Aborted`, refresh Last Updated, and add the changelog row "User decision: superseded by 0000247-test-runtime-reduction; its timing rules carry over with explicit clock injection" <!-- completed: -->

### Step 2: Baseline and fixture parse (PR 1)

- [ ] Record the § Measurement protocol baseline and the `mise //:coverage` summary in the PR description <!-- completed: -->
- [ ] Cache the profile-file parse in `catalog::test_support`, add `bundled_screen_value` and change `bundled_files`/`bundled_file` to borrow; clone only at mutating call sites <!-- completed: -->
- [ ] Add `detector::test_support::replay_screen` and move `replay_as_captured`, `document`, `fixture_lines`, `fixture_size`, `validate_pre_submit_fixtures` and `after_native_submit` onto borrowed screen values <!-- completed: -->
- [ ] Add the empty-composer classification memo and use it in `after_native_submit` <!-- completed: -->
- [ ] Re-measure and record the wall time and every test over 2 s <!-- completed: -->

### Step 3: Redundancy removal (PR 2)

- [ ] Apply every REMOVE and FOLD row of § Redundancy inventory, adding each fold's assertions before deleting the folded test <!-- completed: -->
- [ ] Apply every MERGE row not marked † for `src/coding_agent/*` <!-- completed: -->
- [ ] Apply every MERGE row not marked † for `src/runtime/*` <!-- completed: -->
- [ ] Apply every MERGE row not marked † for the remaining modules, and drop the duplicated `setup --unknown` case <!-- completed: -->
- [ ] Confirm the three merges flagged "confirm … holds" in their rows by running them <!-- completed: -->
- [ ] Record the test count, the `mise //:coverage` summary and the justification of any coverage drop in the PR description <!-- completed: -->

### Step 4: Time toolkit, database clock and locks (PR 3)

- [ ] Move `YieldingClock` to `time::test_support`; add `time::database_now`, `time::database_sleep` and, in `time::test_support`, `HANG_GUARD`, `await_until`, `await_state`, `TestClock::on_sleep`, `RealClockWithUnit` and `DatabaseClock` (including `install_deterministic`), with their `time::tests` cases <!-- completed: -->
- [ ] Route every time read and sleep of the `db.rs` budget machinery through the database clock <!-- completed: -->
- [ ] Add the `clock` parameter to `retry_until`, `LayoutLock::acquire_within`, `PaneLock::acquire_within`, `PaneLock::reacquire_within` and `lock_deletion`, passing the caller's clock or `SystemClock` in production <!-- completed: -->
- [ ] Convert the § Real-time inventory rows for `pane_lock`, `db`, `time`, and the lock and database rows of `runtime::lifecycle`, `runtime::artifacts`, `runtime::press` and `runtime::input` (contended host). The `cli::member` and `cli::fleet` rows convert in Step 5, once the deletion clock exists. <!-- completed: -->
- [ ] Cap `PaneLock::acquire_released`, `RunLock::acquire_released` and `recover_released` at `HANG_GUARD` on `SystemClock` and delete `RELEASED_LOCK_GRACE` <!-- completed: -->
- [ ] Apply the † merges for `pane_lock` <!-- completed: -->
- [ ] Run the § Per-step audit over the converted files <!-- completed: -->

### Step 5: Subprocess and multiplexer bounds (PR 4)

- [ ] Add the `clock` parameter to `collect_until`, `run_timed`, `run_until_exit`, `run_bounded`, `run_bounded_output`, `probe_native_version` and `hermes::effective_home`; add `run_process_on` <!-- completed: -->
- [ ] Add the clock to `BackendCore` and route `DeadlineRunner::remaining`, `check_deadline`, `press_run` and the tmux `capture_screen` retry sleep through it <!-- completed: -->
- [ ] Add the clock parameter to `create_with_layout_wait` and extract `preflight_deadlines` <!-- completed: -->
- [ ] Add `DeletionHost::clock` (`MultiplexerDeletionHost` carries an injected clock with `new` on `SystemClock` and `with_clock`; `NativeFleetDeletionHost` returns `SystemClock`; the three test hosts return their `TestClock`). Add the `clock` parameter to `delete`, `lock_member_deletion`, `close_member_within` and `close_member`, with the CLI dispatch passing `SystemClock`. Read the clock in `await_runner_exit`, `end_member`, `acquire_deletion_layout` and `hermes::settlement_deadline(clock)`. <!-- completed: -->
- [ ] Add marker and pid recording to the `ProcessFixture`, `bounded_run` and version-probe stubs, then convert the § Real-time inventory rows for `runtime::system`, `coding_agent`, `multiplexer`, the hermes probe-bounds split, and every `cli::member` and `cli::fleet` row, using Gate for every real-child bound <!-- completed: -->
- [ ] Rebuild the `ETXTBSY` retries in `bounded_run::run`, `run_stub` and `with_busy_retry` on `await_until` <!-- completed: -->
- [ ] Apply the † merges for `runtime::system` and `cli::member` <!-- completed: -->
- [ ] Run the § Per-step audit over the converted files <!-- completed: -->

### Step 6: Runner, writer and service child (PR 5)

- [ ] Install the database clock with `DatabaseClock::install_deterministic` at the entry of the writer worker and Director sweep threads (§ Database scopes on spawned threads) <!-- completed: -->
- [ ] Add `Hooks::expire`, rebuild the paused-boundary wait on `await_state`, and convert the writer and observation rows of § Real-time inventory <!-- completed: -->
- [ ] Attribute the exporter and resize-verification test time from their records, convert the dominant wait, and update their § Real-time inventory rows with the result <!-- completed: -->
- [ ] Delete the `storage_until` and `real_deadline` conversions in `DirectorSweep` and convert the `storage_busy` and `sweep_thread` rows <!-- completed: -->
- [ ] Add the `#[cfg(test)] sweep_interval` dependency, the sweep-pass hook and `run_configured`'s optional `shutdown_unit_ms` (`RealClockWithUnit`, `SystemClock` when absent) and `sweep_interval_ms` keys; replace `observation::tests`' `SystemClockWithUnit` with `RealClockWithUnit`; then convert the service-child rows <!-- completed: -->
- [ ] Give `submit_wrapped_input*` the client's clock and convert the `input_client`, `control`, `process`, `lifecycle` readiness and `recovery` rows <!-- completed: -->
- [ ] Move every `ServiceFixture` cap and wait onto `HANG_GUARD` and `await_until` <!-- completed: -->
- [ ] Migrate every caller of the § Shared test-time toolkit's replaced helpers, then delete those helpers <!-- completed: -->
- [ ] Apply the † merge for `runtime::lifecycle` <!-- completed: -->
- [ ] Run the § Per-step audit over the converted files <!-- completed: -->

### Step 7: CPU residue and completion (PR 6)

- [ ] Add idle-turn skipping to `input::tests` `turns_until` and to the remaining scenario waits that hold no bytes <!-- completed: -->
- [ ] Apply the § CPU residue remedies, measuring after each, until no test exceeds 2 s <!-- completed: -->
- [ ] Run every § Per-step audit search across `src/` and confirm every remaining test hit is a § Sanctioned real-time sites entry <!-- completed: -->
- [ ] Run the § Measurement protocol 3 times and record the median wall time (under 20 s) and the maximum test time (2 s or less) <!-- completed: -->
- [ ] Record the final `mise //:coverage` summary against the baseline, with a justification for every drop <!-- completed: -->
- [ ] Confirm every Success Criterion and mark this document Complete <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-27 | Initial draft |
| 2026-11-27 | Reviewer round 1: Step vocabulary; database clock on writer and sweep threads; Gate construct for real children; `RealClockWithUnit` carrier; `DeletionHost::clock`; released-lock helpers sanctioned; exporter attribution; control fold rows; corrected counts; scoped rule text |
| 2026-11-27 | Reviewer round 2: `DeletionHost::clock` for every host and the member layout wait; Gate on `YieldingClock` with the PTY storage phase ending on the observed reap; `ETXTBSY` retries sanctioned on `await_until`; orphaned table header removed |
| 2026-11-27 | Reviewer round 3: member deletion takes an injected clock (`delete`, `lock_member_deletion`, `close_member_within`, `MultiplexerDeletionHost::with_clock`); CLI clock rows move to Step 5 |
