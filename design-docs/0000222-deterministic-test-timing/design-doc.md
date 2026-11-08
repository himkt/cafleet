# Deterministic Test Timing

**Status**: Approved
**Progress**: 3/33 tasks complete
**Last Updated**: 2026-11-08

## Overview

Make the Rust test suite deterministic by removing real elapsed time as a pass/fail criterion. Tests move time explicitly through a test-thread clock and existing clock traits, synchronize on events and boundary hooks instead of short polling caps, and keep one generous real-time cap only as a hang guard. The work lands in phases, ordered by observed flakiness, each an independently mergeable PR that leaves the suite green.

## Success Criteria

- [ ] The full suite passes 10 consecutive runs of `RUST_TEST_THREADS=16 taskset -c 0,1 mise //:test` (2 cores, 16 test threads).
- [ ] No test asserts a real elapsed-time bound: no assertion compares a real `Instant::now()`, `elapsed()` or `duration_since(..)` against a duration or deadline. Assertions on `ThreadClock` instants are injected-time assertions and allowed.
- [ ] Every real-time wait in test code is a sanctioned site listed in § Specification > Sanctioned real-time sites.
- [ ] Every deadline a test constructs is the hang guard, already past at entry, or data on a `ThreadClock` / scripted clock that only the test advances.
- [ ] `writer::test_support::{wait_until, wait_until_within, deadline}`, `service::test_support::wait_until`, `synthetic_with_unhurried_writer` and the `writer_work_slice_ms` config key no longer exist; their callers use `time::test_support`.
- [ ] Seam trait methods and test hooks are `#[cfg(test)]`; production builds keep every timing value and code path; production code carries only value-preserving plumbing (an extracted constant, a field holding it, `time::monotonic_now()` returning `Instant::now()`). `SPEC.md`, `docs/`, `README.md`, `skills/` and `.claude/rules/` are untouched.
- [ ] `mise //:test` and `mise //:lint` pass at the end of every phase.

---

## Background

Many tests treat "finished within N ms" as correctness. On a loaded CI runner (`cargo test --bin cafleet` with `RUST_TEST_THREADS` = `nproc`, capped at 32, on 4 cores) scheduler delay exhausts these budgets, and the tests fail without any related code change.

| Family | Location | Observed failure | Timing dependency |
|---|---|---|---|
| `permission_response_actual_service_returns_typed_durable_audit_and_one_selected_answer` | `src/runtime/input_client.rs` | `supported_decisions.contains(&Allow)` failed (run 42450379682, issue #580) | A single inspection raced request readiness |
| `service_shutdown_uses_the_active_controller_to_close_before_truthful_exit` | `src/runtime/observation.rs` | Two unexpected `runner_degraded` events (run 42378770248, issue #580) | The production 100 ms writer `WORK_SLICE` expired under load |
| `writer::test_support::wait_until` callers | Every file that imports it | `bounded worker condition did not complete` | 2 s / 3 s polling caps |
| `cli::fleet::tests::recovery::runner_recovery_deletion_*` | `src/cli/fleet.rs` | Recurring CI failures | 300 ms – 2 s operation deadlines, `start.elapsed() < 900ms`, SQL entry before a 300 ms deadline |
| `runtime::recovery::tests::*` | `src/runtime/recovery.rs` | Reproduced locally under 2-core / 16-thread overload | 300 ms – 7 s deadlines; expiry manufactured with `sleep(deadline - now)` |
| `permission_response_contracts` (formerly `permission_admission_requires_a_settled_observation_and_survives_an_evidence_only_redraw` and siblings) | `src/runtime/input.rs` | `a pending observation update is named` (runs 40623045925, 40622515629, 40621945055) | Observation settling raced a real-time budget |
| `permission_receipt_postcommit_expiry_preserves_claim_context_and_consumed_history` | `src/runtime/input_client.rs` | Class member (not yet observed failing) | `host.calls == [1, 1, 3]` must happen inside a 300 ms expiry; asserts `Instant::now() >= original_expiry` |

Existing seams this design builds on:

| Seam | Location | Today |
|---|---|---|
| `time::Clock` + `time::test_support::TestClock` | `src/time.rs` | Single-threaded manual clock used by delivery, message and observation code |
| `ObservationClock` (with the `#[cfg(test)]` `ObservationClockKind::{Real, Deterministic}` returned by `kind`) | `src/runtime/observation.rs` | Deterministic test clocks (`tests` `Clock`, `Timeline`) with absolute `set(millis)` |
| `InputDeadlineClock` / `RealInputDeadlineClock` / `ScriptClock` | `src/runtime/input.rs` | Maps deadline instants to monotonic nanoseconds; scripted samples in tests |
| `ShutdownClock::with_unit`, `RunnerDependencies::shutdown_unit` | `src/runtime/lifecycle.rs`, `src/runtime/service.rs` | Child, display and storage deadlines at 2, 3 and 4 units |
| `WriterHooks::before` (+ `writer::test_support::Hooks` with `Condvar`) | `src/runtime/writer.rs` | Pause, fail-once and call counting per writer boundary |
| `WriterHooks::work_slice` | `src/runtime/writer.rs` | Added in Step 1 |
| `RecoveryHost::checkpoint`, `DeletionHost`, `InputCommandHost` | `src/runtime/recovery.rs`, `src/cli/fleet.rs`, `src/runtime/input_client.rs` | Production host traits that tests implement |
| `InputHooks::checkpoint` | `src/runtime/input.rs` | `#[cfg(test)]` input boundary hook |
| `*_through_barrier` helpers (e.g. `native_answers_through_barrier`) | `src/runtime/input_client.rs` | Assert absence at a later ordered event |

Service tests run the runner in a re-executed child process (`service::test_support::run_configured` reads `observation-services.json`). In-process signalling and test-thread clocks cannot cross that boundary; those tests synchronize on database rows, files and PTY output, and the child's budgets are set through that configuration file.

---

## Specification

### Timing rules

A test is deterministic when its verdict depends only on the order of events the code under test produces, never on how much real time those events took. Every test follows these rules:

1. **Assert outcomes, not durations.** Assert the returned error, persisted rows, event sequence, host call counts, hook-recorded ordinals or `ThreadClock` instants. Assert no real elapsed-time bound, upper or lower.
2. **A budget the test expects to hold is the hang guard, or never moves.** A deadline handed to code expected to succeed is `hang_guard()` / `hang_guard_deadline()`, or a shorter value on an installed `ThreadClock`, which passes only when the test advances it.
3. **A budget the test expects to expire expires by construction.** Pick the first that applies:
   1. Production checks the deadline on the test thread: install a `ThreadClock` and call `advance_past(deadline)` at the chosen point; production's own check then fails exactly as after real expiry.
   2. The expiry is the subject of a boundary hook: the hook returns `CafleetError::DeadlineExceeded` (or `Hooks::end_pass`).
   3. The deadline must be expired on entry: pass a deadline already past.
   4. The budget runs in another thread or process: make the awaited condition impossible (e.g. a child that ignores `SIGTERM`), so expiry is certain regardless of the budget's length.
4. **Wait for the consequence, not for time.** Block on a `Condvar`/channel signalled from a hook when a hook exists; poll external state (database rows, files, PTY output) with `await_until` only when no in-process signal exists.
5. **Assert absence at a barrier.** "X did not happen" is checked after a later event that production orders after X (the existing `*_through_barrier` pattern), never after a sleep.
6. **Production budgets that load can exhaust are unhurried in tests.** In-process writers and service children run on a `HANG_GUARD` work slice; a test that exercises exhaustion injects it (`Hooks::fail_once`, `Hooks::end_pass`, the `fail-sync` file).

### Pattern catalog

| Pattern | Example today | Replacement |
|---|---|---|
| P1 Elapsed-time assertion | `assert!(start.elapsed() < Duration::from_millis(900))`, `assert!(Instant::now() >= original_expiry)` | Delete; assert the outcome (rule 1). A lower bound on a production window becomes a `ThreadClock` step assertion. |
| P2 Manufactured expiry | `std::thread::sleep(deadline.saturating_duration_since(Instant::now()))` in a hook or before a call | `advance_past(deadline)` under an installed `ThreadClock`, the rest unchanged (rule 3.1). |
| P3 Short real operation budget | `Instant::now() + Duration::from_millis(300)`, `let start = Instant::now(); Deadline::until(start + ..)` | `hang_guard()` / `hang_guard_deadline()`, or the same value on a `ThreadClock` (rule 2). |
| P4 Short polling cap | `wait_until(..)` (2 s), service `wait_until` (3 s), `ServiceFixture::read_until` (2 s), ad-hoc `loop { ..; sleep(2ms) }` | `await_state` on a hook `Condvar`, or `await_until` for external state (rule 4). |
| P5 Sleep to let something happen | `sleep(host.stage_settle + Duration::from_millis(50))`, `sleep(Duration::from_millis(150))` | `advance(..)` on a `ThreadClock`, or wait for the observable consequence (P4). |
| P6 Negative assertion over a window | "no event within N ms" | Assert at a barrier (rule 5). |
| P7 Exhaustible production budget | Writer `WORK_SLICE` (100 ms), service `shutdown_unit` (1 s) | Unhurried test value; inject exhaustion where it is the subject (rule 6). |

### Shared test-time toolkit

Added to `src/time.rs`. `monotonic_now` is production code; everything else lives in the existing `#[cfg(test)] pub(crate) mod test_support`, which already owns `TestClock`:

```rust
pub(crate) fn monotonic_now() -> Instant {
    #[cfg(test)]
    if let Some(now) = test_support::thread_clock_now() {
        return now;
    }
    Instant::now()
}

// time::test_support
pub(crate) const HANG_GUARD: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// A deadline no correct test reaches; reaching it means the test hung.
pub(crate) fn hang_guard() -> Instant {
    monotonic_now() + HANG_GUARD
}
pub(crate) fn hang_guard_deadline() -> Deadline {
    Deadline::until(hang_guard())
}

/// Polls external state until `ready`; panics naming `awaited` after `HANG_GUARD` of real time.
pub(crate) fn await_until(awaited: &str, ready: impl FnMut() -> bool) {
    await_until_by(awaited, Instant::now() + HANG_GUARD, ready)
}
fn await_until_by(awaited: &str, until: Instant, mut ready: impl FnMut() -> bool) {
    while !ready() {
        assert!(
            Instant::now() < until,
            "hang guard: {awaited} did not happen within {HANG_GUARD:?}"
        );
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Blocks on `changed` until `ready(state)`; panics naming `awaited` after `HANG_GUARD`.
pub(crate) fn await_state<'a, T>(
    awaited: &str,
    lock: &'a Mutex<T>,
    changed: &Condvar,
    ready: impl FnMut(&mut T) -> bool,
) -> MutexGuard<'a, T>;

/// Freezes `monotonic_now()` on the installing thread until dropped.
pub(crate) struct ThreadClock { /* !Send */ }
impl ThreadClock {
    pub(crate) fn install() -> Self;
}
pub(crate) fn advance(by: Duration);
pub(crate) fn advance_past(deadline: Instant);
```

| Item | Contract |
|---|---|
| `monotonic_now` | Production builds: exactly `Instant::now()`. Test builds: the installed `ThreadClock`'s instant on this thread, otherwise `Instant::now()`. |
| `HANG_GUARD` | 30 s, the only real-time cap in test code. Never a correctness criterion: reaching it is a hang, reported by a panic. |
| `hang_guard` | Built on `monotonic_now`, so a hang-guard deadline taken after `advance` is live on the thread clock. |
| `await_until` | Real-time cap independent of any `ThreadClock`; returns as soon as `ready()` is true; panics `hang guard: <awaited> did not happen within 30s`. |
| `await_state` | `Condvar::wait_timeout_while` with `HANG_GUARD`; same panic message; returns the guard so the caller asserts on the state it waited for. |
| `ThreadClock::install` | Freezes the thread's `monotonic_now()` at `Instant::now() + HANG_GUARD`, so every thread-clock instant lies ahead of real time while installed (§ Thread clock scope and production plumbing). Panics if a clock is already installed on the thread. Drop uninstalls, so a test thread reused by libtest starts clean. |
| `advance` / `advance_past` | Move the installed clock forward (`advance_past(d)` sets it to `d + 1ms` when it is not already past `d`). Panic when no clock is installed on the calling thread. |
| Frozen-clock hang guard | `thread_clock_now()` panics `hang guard: thread clock read <HANG_GUARD> after install; a loop on this thread waits for time the test never advances` once `HANG_GUARD` of real time has passed since install. |

`time::test_support`'s own tests cover `await_until_by` returning on a ready condition and panicking for an already-past `until` (`#[should_panic(expected = "hang guard: ")]`), `await_state` waking on notify, `ThreadClock` freezing, advancing, uninstalling on drop, refusing a second install and refusing `advance` without a clock. None waits on real time.

#### Thread clock scope and production plumbing

Production deadline checks live in shared helpers (`time::Deadline`, `runtime::check_deadline` callers, database scopes, pane-lock loops, `ShutdownClock` capture), which no per-subsystem trait reaches. These checks read `time::monotonic_now()` instead of `Instant::now()`. No trait is threaded through call graphs and no signature changes; existing clock traits stay as they are.

- **Converted production reads**: every `Instant::now()` that constructs, clips or compares a deadline in code that a converted test drives on its own thread. That includes `Deadline::{new, remaining}`, `check_deadline(Instant::now(), ..)` call sites, `database_cancelled`, `complete_database_budget`'s grace window, `ShutdownClock::new(Instant::now())` at recovery entry, remaining-time sleeps (`until.saturating_duration_since(Instant::now())`), the input coordinator's `settle_until` and pending-deadline checks, and input-client command and control-socket deadlines. Each phase converts the reads its tests drive. Production `Instant::now()` reads that measure diagnostics or feed `RealObservationClock` stay.
- **A missed read fails loudly**: because `install()` freezes the clock `HANG_GUARD` ahead of real time and the frozen-clock hang guard ends the test before real time catches up, an unconverted real read sees every thread-clock deadline as live. A test that expects expiry at that read therefore fails deterministically there (the expected `DeadlineExceeded` never arrives), and an unconverted remaining-time sleep toward a thread-clock instant overruns into the frozen-clock hang guard. The failing test names the read to convert; no missed read can reintroduce a load-dependent pass or fail.

- **Install first**: a test that uses a `ThreadClock` installs it as its first statement, before constructing any fixture, harness or deadline. Every deadline it then builds comes from `monotonic_now()` / `hang_guard()`. Harness constructors that capture a deadline (input `Host::new` and its `until`, recovery `Host::new` / `new_host` / `Case`, `ServiceFixture`'s `startup_deadline` and `wait`) build it with `monotonic_now()`, so the same constructor serves tests with and without a clock. The ordering matters because `install()` moves the thread to `now + HANG_GUARD`: a deadline captured on the real scale before install, or by test code that still writes `Instant::now() + ..`, lies behind the installed clock, and every converted read reports it as expired. That failure is deterministic too and names the capture to fix.
- **`RealInputDeadlineClock`**: a `#[cfg(test)]` branch keeps `InputExpiration` consistent under an installed clock. It takes the real bracketed sample, then computes a signed offset `offset_ns: i128` = `monotonic_now()` minus the real `before`, positive when the thread clock is ahead. It shifts `before` and `after` by that offset (`checked_add` of the magnitude when positive, `checked_sub` when negative) and `monotonic_ns` by the same signed nanoseconds (`i128` arithmetic, then `u64::try_from`), each overflow panicking. Under `install()` the offset is always positive; the negative branch serves no installed state and exists only so the arithmetic is total. Shifting the bracket and the nanoseconds equally leaves the nanosecond position of every deadline instant unchanged: a service child sees a thread-clock deadline on its real scale, at least the remaining hang-guard margin ahead, while capture-time expiry checks compare against thread-clock `before`/`after`.
- **Thread boundary**: a `ThreadClock` governs only its installing thread. A budget handed to another thread (the `EventWriter` worker, an `EmergencyInterrupt` watchdog) or to a service child is a hang guard or follows rule 3.4.

#### Construct per listed site

| Site | Construct |
|---|---|
| `runner_recovery_expired_original_budget_prevents_actual_unlink_dispatch` (`src/runtime/recovery.rs`) | Install a `ThreadClock` first, before `described_creator()`; then `Host::with_unlink(&case, hang_guard(), false)`; the `before_finish` action calls `advance_past(deadline)` and returns `Ok`. Production's next check, the `with_database_deadline` entry before `descriptor.remove`, fails. `unlink.calls == 0`, `incomplete(..)` and unchanged descriptor bytes stay; `Instant::now() >= host.deadline` is deleted. |
| `runner_recovery_expiry_after_a_commit_retains_counts_and_next_pass_counts_only_new_work` and every other `sleep(deadline - now)` in recovery hooks | Install a `ThreadClock` first, before `Case::new()`; the host deadline is `hang_guard()` (or clock data such as `monotonic_now() + 300ms`); replace the sleep with `advance_past(deadline)`; the hook keeps its return value. |
| `SocketHost::expire_retirement` (`src/runtime/input_client.rs`) | The test installs a `ThreadClock` first, before `ServiceFixture::configured`, and `SocketHost::new` receives a post-install `hang_guard()` capture. In the `Retire` exchange, after the durable-reread assertion, `advance_past(deadline)` replaces the sleep and `assert!(Instant::now() >= deadline)`. The host's `ControlClient::connect` entry check (`connect_in_phase` in `src/runtime/control.rs`, converted in Step 4) returns `DeadlineExceeded`, after `calls[2]` reached 3; no exchange with the child happens for `Retire`. |
| `permission_receipt_postcommit_expiry_preserves_claim_context_and_consumed_history` | Installs the clock as its first statement, before `ServiceFixture::configured(false, response_services)`. `until` and `original_expiry` are each one post-install `hang_guard()` capture shared by every deadline of their command. Because the clock moves only in `Retire`, exactly three exchanges happen, so `host.calls == [1, 1, 3]` holds by construction. `assert!(Instant::now() >= original_expiry)` is deleted. |
| `ExpiringReservedHooks::checkpoint` (`src/runtime/input.rs`) | Install a `ThreadClock` once as the test's first statement, before the `for fail` loop and every `with_host` call, so `Host::new` builds `h.until` (5 s) on the thread clock; `deadline = monotonic_now() + 150ms` stays as clock data and `assert!(deadline < h.until)` holds; the hook asserts `monotonic_now() < deadline`, then calls `advance_past(deadline)` instead of sleeping, and keeps its `fail` branch. |
| `permission_receipt_cacheless_ready_proof_…` ready expiry (`src/runtime/input.rs`) | Install a `ThreadClock` first, before `with_host`, so `h.until` is on the thread clock; `expired = monotonic_now() + 500ms` (`expired < h.until` holds); `advance_past(expired)` replaces `sleep(expired - now)` and its `Instant::now() >= expired` assertion before `try_advance`. |
| Stage-settle tests (`assert_settled_gaps`, `sleep(host.stage_settle + ..)`) | Install a `ThreadClock` first, before the host or fake PTY is built; the fake PTY records stage writes at `monotonic_now()`. A settle test advances to `settle - 1ms` and asserts no stage write, then advances past `settle` and asserts the write. `gap >= stage_settle` compares thread-clock instants. |

### Database scopes under a thread clock

`database_cancelled`, `remaining_database_wait` (through `Deadline::remaining`) and `complete_database_budget` read `monotonic_now()`. An advanced thread clock is therefore indistinguishable from real elapsed time for every frame. Inner frames expire independently of outer ones, a `Completion` frame computes its grace window from the advanced instant, and a popped frame stops participating. No database-specific seam is added.

Callback points that move the clock mid-statement run on the statement's thread:

| Statement kind | Callback point |
|---|---|
| A statement that writes (directly or through a trigger) | `Connection::update_hook` on the written table |
| A statement that writes nothing | `Connection::authorizer` (rusqlite `hooks` feature, already enabled), which runs during statement preparation |
| A statement issued directly by the test with no entry deadline check | `advance_past` immediately before issuing it; the first progress-handler tick interrupts it |

`runner_recovery_deletion_actual_replay_sql_is_cancelled_within_the_supplied_remaining_budget` (`src/cli/fleet.rs`):

1. Install a `ThreadClock` as the first statement, before `Case::new()`; `deadline = hang_guard_deadline()`; `EmergencyInterrupt::arm(&conn, Instant::now() + HANG_GUARD)` (real watchdog thread).
2. The `recovery_sql_probe` update hook records entry and calls `advance_past(deadline.instant())`; `SLOW_SUM` in the trigger is interrupted by the progress handler.
3. Assert: the probe entered, `!emergency_fired`, `before_replay` reached, and the same error the unconverted test produces. Advancing the thread clock places the deadline behind `now` exactly as real expiry did, so production follows today's path. Step 3 records today's `result` value before conversion and pins it with `assert_eq!` on its `Display` text. The candidates are `database error: interrupted` (the interrupted statement through `From<rusqlite::Error>`) and `member recovery remains incomplete; inspect with cafleet doctor` (recovery reporting a non-`Recovered` outcome).
4. `!host.calls.contains(&"close")`, `events(&case).is_empty()`, the rolled-back `recovery_sql_probe` count, unchanged spool bytes, `assert_registry_active` and the restored `busy_timeout` stay unchanged. The `entered < deadline`, `returned >= deadline` and `start.elapsed() < 900ms` assertions are deleted.

`runner_recovery_deletion_unwind_restores_the_still_live_outer_scope`:

1. Install a `ThreadClock` as the first statement, before `Case::new()`; `start = monotonic_now()`, `inner = start + 600ms`, `outer = start + 1000ms` (clock data); `EmergencyInterrupt` armed at `Instant::now() + HANG_GUARD`.
2. Inside the outer `with_database_deadline`, after the injected `before_compact` panic unwinds deletion, `advance(Duration::from_millis(620))` replaces `sleep(inner + 20ms)`; `assert!(monotonic_now() > inner && monotonic_now() < outer)` replaces the real-time bracket.
3. `SELECT 42` still succeeds (the retired inner frame no longer cancels). Then `advance_past(outer)` immediately before `conn.query_row(SLOW_SUM, ..)`; the outer cancellable frame's progress handler interrupts it.
4. Assert `!emergency.finish()`, `result.is_err()`, the restored `busy_timeout` and the final `SELECT 42`. `Instant::now() >= outer` is deleted and `start.elapsed() < 1400ms` is deleted.

### Recovery clock module

`runtime::recovery::tests::clock` proves that cleanup carries the one `ShutdownClock` captured at coordinator entry. Under a `ThreadClock` that intent becomes exact:

| Element | Today | Converted |
|---|---|---|
| `until` | `Instant::now() + cap` (7 s / 3.3 s) | `clock_case` installs a `ThreadClock` as its first statement, before `Case::new()`; `until = monotonic_now() + cap`, same caps; the 3.3 s cap still clips the 4-unit storage deadline because the clock moves only by `advance` |
| Pre-entry wait | `sleep(100ms)` | `advance(100ms)` |
| `before_entry` / `after_entry` | Real instants | `monotonic_now()`; nothing advances between them, so they are equal and the bounds in `assert_entry_clock` collapse to one exact value per boundary |
| `ClockProbe` delay at `AfterSnapshot` | `assert!(resume < deadline)` with real `resume`, then `sleep(delay)` | `assert!(monotonic_now() + delay < deadline)`, then `advance(delay)`; a re-captured clock would be off by `delay` and fail `assert_entry_clock` |
| Production entry capture | `ShutdownClock::new(Instant::now())` | `ShutdownClock::new(monotonic_now())` |

No recovery-specific `now()` seam is needed.

### Writer and service budgets

| Writer | Work slice in tests | Exhaustion when it is the subject |
|---|---|---|
| In-process (`DefaultWriterHooks`, `writer::test_support::Hooks`, every other test `WriterHooks`) | `work_slice()` returns `HANG_GUARD` | `Hooks::end_pass(boundary)`: the next `before(boundary)` returns `DeadlineExceeded` once (`Receive` ends the pass; other boundaries record a gap), or `Hooks::fail_once` |
| Service child (`run_configured`) | Fixed `HANG_GUARD`; the `writer_work_slice_ms` key is removed | The `fail-sync` file, or an equivalent `Hooks::before` file trigger added when a converted test needs one |

No test runs on the production `WORK_SLICE`. A paused boundary in `writer::test_support::Hooks::before` now waits until released, bounded by a `HANG_GUARD` slice. In-process tests that relied on a pause expiring to end a pass call `end_pass` instead.

| Config key | Field | Default in tests | Short value written by |
|---|---|---|---|
| `shutdown_unit_ms` (`run_configured`) | `RunnerDependencies::shutdown_unit` | 7 500 ms: the 4-unit storage deadline, the latest `ShutdownClock` boundary, equals `HANG_GUARD` | A test whose child ignores `SIGTERM` or stalls a shutdown phase (rule 3.4); found through the `observation-services.json` keys its fixture writes |
| `shutdown_unit_ms` (export plan of the native-capture verification child in `src/runtime/observation.rs`) | The same `ShutdownClock` unit for a different child | Non-stalled plans: 7 500 ms instead of 1 000 | Stalled plans keep 100 ms (expiry certain) |

`ServiceFixture` waits use the hang guard: `read_until` polls PTY output through `await_until` (its 2 s cap and the 10 s `startup_deadline` become `HANG_GUARD`), and `wait` passes `hang_guard()` to `poll_until`.

### Removed helpers and their replacements

| Removed | Replacement |
|---|---|
| `writer::test_support::wait_until` (2 s), `wait_until_within` | `await_until`, or `Hooks::wait` / `await_state` where a writer hook signals the condition |
| `writer::test_support::deadline()` (2 s), `broker::runtime` test `deadline()` (1 s), `coding_agent::hermes` test `deadline()` | `hang_guard()` |
| `service::test_support::wait_until` (3 s) | `await_until` |
| `service::test_support::observation_services::synthetic_with_unhurried_writer` | `synthetic`, whose writer slice is `HANG_GUARD` |
| `writer_work_slice_ms` config key | Fixed `HANG_GUARD` slice in `run_configured` |
| `response_command`'s two 2 ms loops (the `interaction_opened` row wait and the `inspect_permission` until-`Allow` loop) | `await_until("interaction_opened row", ..)` and `await_until("opened request becomes answerable", ..)` |

Each command in `input_client` service tests captures `let until = hang_guard();` once and derives every deadline from it — `response_command(.., until, ..)`, `Deadline::until(until)` and `SocketHost::new(.., until)` — because `SocketHost` asserts `deadline == self.until` in `lock`, `health` and `exchange`.

Per the repository's removal rule, each removed helper disappears in the phase that migrates its last caller, with no alias.

### Sanctioned real-time sites

After migration, test code touches real time only at these sites:

| Site | Why it stays |
|---|---|
| `time::test_support` (`await_until`, `await_state`, the frozen-clock hang guard) | The hang guard itself |
| Fixture loops built on `await_until` that poll external state (`ServiceFixture::read_until` PTY reads, child `poll_until(hang_guard())`) | Polling external state with the hang-guard cap (rule 4) |
| Hook-side waits bounded by the deadline the hook receives (the `block-persist` loop in `service::test_support::Hooks::before`, the pause wait in `writer::test_support::Hooks::before`) | The incoming deadline is a `HANG_GUARD` slice or hang guard; the wait ends on a test action |
| Watchdog threads armed at `Instant::now() + HANG_GUARD` (`EmergencyInterrupt` in `src/cli/fleet.rs`) | Fires only on a hang; the test asserts it did not fire |
| Past-at-entry deadlines (`Instant::now() - Duration::from_millis(1)`) | Expiry by construction |
| `ThreadClock` instants and scripted clock data (`ScriptClock` samples, observation `Clock` / `Timeline` `set` values) | Injected time, moved only by the test |

The observation tests' deterministic `Clock` and `Timeline` stay as they are: they already inject time and remove no real-time dependency by migrating.

### Per-phase audit

Because the convention has no machine enforcement, each phase PR records an audit of the files it converted. For each file, every hit of these searches inside `#[cfg(test)]` code is reviewed one by one and is either removed or one of the sanctioned sites above:

```text
grep -n "thread::sleep" <file>
grep -n "Instant::now()" <file>
grep -nE "Duration::from_(nanos|micros|millis|secs)\(" <file>
grep -nE "elapsed\(\)|duration_since" <file>
grep -nE "recv_timeout|wait_timeout|poll_until|read_until" <file>
grep -nE "wait_until(_within)?\(" <file>
```

`Instant::now()` without the `+` catches captured-instant deadlines (`start + ..`) and expressions wrapped across lines; `Duration::from_*` catches every budget literal. For production code in the same files, each `Instant::now()` outside `#[cfg(test)]` that feeds deadline arithmetic is converted to `monotonic_now()`; the remaining hits are diagnostics or observation samples.

### Phase verification

| Phase PR | Required before merge |
|---|---|
| Every phase | `mise //:test`, `mise //:lint`, and the phase's audit |
| Step 2 | `RUST_TEST_THREADS=16 taskset -c 0,1 mise //:test -- -- time::tests runtime::writer::tests runtime::service::tests` passes 10 consecutive runs |
| Step 3 | `RUST_TEST_THREADS=16 taskset -c 0,1 mise //:test -- -- db::tests runtime::recovery::tests cli::fleet::tests` passes 10 consecutive runs |
| Step 4 | `RUST_TEST_THREADS=16 taskset -c 0,1 mise //:test -- -- runtime::input::tests runtime::input_client::tests` passes 10 consecutive runs |
| Steps 5–6 | `RUST_TEST_THREADS=16 taskset -c 0,1 mise //:test` (full suite) passes 10 consecutive runs |

Filters are libtest substring filters. Each phase confirms its filter set selects exactly the converted modules with `mise //:test -- -- --list <filters>` before the stress runs. These are manual verification commands; no CI job or mise task is added. A converted test keeps its intent: it asserts the same outcome as before, minus the real-time bound.

### Out of scope

- Production timing behavior, constants and crate-external signatures.
- A clock trait threaded through call graphs, mock-time crates or any new dependency.
- Migrating the observation tests' deterministic clocks.
- Machine enforcement (clippy `disallowed-methods`, source scans) and rule-file text; review applies this document's rules.
- CI configuration (`RUST_TEST_THREADS` selection, job layout).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Targeted #580 fixes (this branch)

Delivered with this design document on `feat/0000222-deterministic-test-timing`; the PR closes #580.

- [x] Poll `inspect_permission` in `input_client` tests' `response_command` until `Allow` is supported, under the command budget <!-- completed: 2026-11-08T06:30 -->
- [x] Add `writer::WORK_SLICE`, `WriterHooks::work_slice` and the `writer_work_slice_ms` service config key <!-- completed: 2026-11-08T06:30 -->
- [x] Run `service_shutdown_uses_the_active_controller_to_close_before_truthful_exit` on `synthetic_with_unhurried_writer` <!-- completed: 2026-11-08T06:30 -->
- [ ] Verify `mise //:test` and `mise //:lint`, then commit the fixes and this design document and open the PR <!-- completed: -->

### Step 2: Shared toolkit, writer and service (PR 2)

- [ ] Add `time::monotonic_now` and, in `time::test_support`, `HANG_GUARD`, `hang_guard`, `hang_guard_deadline`, `await_until`, `await_state`, `ThreadClock`, `advance` and `advance_past`, with their deterministic tests <!-- completed: -->
- [ ] Convert `Deadline::{new, remaining}` to `monotonic_now()` <!-- completed: -->
- [ ] Rebuild `writer::test_support::Hooks::wait` on `await_state`; add `Hooks::end_pass` <!-- completed: -->
- [ ] Return `HANG_GUARD` from `work_slice()` in `DefaultWriterHooks`, `writer::test_support::Hooks` and every other test `WriterHooks`; convert in-process tests that relied on pause expiry to `end_pass` <!-- completed: -->
- [ ] Migrate every caller of `writer::test_support::{wait_until, wait_until_within, deadline}` and `service::test_support::wait_until` (found with `grep -rnE "wait_until(_within)?\(" src`), then delete those helpers <!-- completed: -->
- [ ] Replace the `broker::runtime` and `coding_agent::hermes` test `deadline()` helpers with `hang_guard()` and delete them <!-- completed: -->
- [ ] Fix the `run_configured` writer slice at `HANG_GUARD`, delete the `writer_work_slice_ms` key and `synthetic_with_unhurried_writer`, and return its caller to `synthetic` <!-- completed: -->
- [ ] Convert any service test that relied on real slice expiry to an injected gap (`fail-sync` or a `Hooks::before` file trigger) <!-- completed: -->
- [ ] Add `shutdown_unit_ms` (default 7 500) to `run_configured`; write a short unit for every service test whose child ignores `SIGTERM` or stalls a shutdown phase <!-- completed: -->
- [ ] Rebuild `ServiceFixture::read_until` on `await_until` with the `HANG_GUARD` cap, make `startup_deadline` and `wait`'s `poll_until` deadline `hang_guard()` (built on `monotonic_now()` per § Thread clock scope and production plumbing > Install first) <!-- completed: -->
- [ ] Convert the remaining P1–P7 sites in `src/time.rs`, `src/runtime/writer.rs` and `src/runtime/service.rs` tests <!-- completed: -->
- [ ] Run the audit and the Step 2 stress gate <!-- completed: -->

### Step 3: Recovery and fleet deletion (PR 3)

- [ ] Convert to `monotonic_now()` every production deadline read that the Step 3 tests reach on the test thread, shared helpers included (`check_deadline` callers, `nap_until`, pane-lock loops, descriptor removal, the recovery-entry `ShutdownClock` capture), using each converted test's loud failure at a missed read to find the rest, with a `db::tests` case showing an advanced thread clock interrupts a cancellable statement and leaves a completion frame live <!-- completed: -->
- [ ] Build the deadlines captured by recovery `Host::new` / `new_host` / `Case` and `DeletionTestHost` with `monotonic_now()`, then convert `runtime::recovery::tests::*` per § Construct per listed site and § Recovery clock module <!-- completed: -->
- [ ] Record today's `result` of the replay-SQL cancellation test, then convert it and the unwind test per § Database scopes under a thread clock <!-- completed: -->
- [ ] Convert the remaining `cli::fleet::tests::recovery::runner_recovery_deletion_*` tests <!-- completed: -->
- [ ] Convert the remaining P1–P7 sites in `src/runtime/recovery.rs`, `src/cli/fleet.rs` and `src/db.rs` tests <!-- completed: -->
- [ ] Run the audit and the Step 3 stress gate <!-- completed: -->

### Step 4: Input coordinator and input client (PR 4)

- [ ] Convert the production deadline reads in `src/runtime/input.rs`, `src/runtime/input_client.rs` and the control client to `monotonic_now()`, and add the `#[cfg(test)]` lag-shift branch to `RealInputDeadlineClock` <!-- completed: -->
- [ ] Build the input `Host::new` `until` with `monotonic_now()`, then convert the stage-settle tests and `ExpiringReservedHooks` / cacheless-ready expiry per § Construct per listed site <!-- completed: -->
- [ ] Convert the `permission_response_contracts` module <!-- completed: -->
- [ ] Convert `SocketHost::expire_retirement` and `permission_receipt_postcommit_expiry_preserves_claim_context_and_consumed_history` per § Construct per listed site <!-- completed: -->
- [ ] Replace both `response_command` loops with `await_until`, and capture one `hang_guard()` per command for every deadline it derives <!-- completed: -->
- [ ] Convert the remaining P1–P7 sites in `src/runtime/input.rs` and `src/runtime/input_client.rs` tests <!-- completed: -->
- [ ] Run the audit and the Step 4 stress gate <!-- completed: -->

### Step 5: Remaining modules (PR 5)

- [ ] Convert the P1–P7 sites in every remaining file whose `#[cfg(test)]` code has a hit for any § Per-phase audit search, including the observation export child's `shutdown_unit_ms` per § Writer and service budgets <!-- completed: -->
- [ ] Run the audit over those files and the full-suite stress gate <!-- completed: -->

### Step 6: Final verification and completion (PR 5)

- [ ] Run every § Per-phase audit search across `src/` and confirm each remaining test hit is a sanctioned site <!-- completed: -->
- [ ] Confirm every Success Criterion and mark this document Complete <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-08 | Initial draft |
| 2026-11-08 | Reviewer round: test-thread clock read by production deadline checks; per-site expiry constructs; writer slice and shutdown unit defaults; recovery clock and database-scope conversions; full audit searches and exact phase filters |
| 2026-11-08 | Thread clock installs `HANG_GUARD` ahead of real time so a missed production read fails loudly; signed `RealInputDeadlineClock` offset; `SocketHost::expire_retirement` fails at the host's `ControlClient::connect` entry check |
| 2026-11-08 | Install-first rule for `ThreadClock`; harness constructors capture deadlines with `monotonic_now()`; ordering stated per site |
