# Doctor and setup speed, bounded refactor, test pruning and Hermes configuration

**Status**: Complete
**Progress**: 72/72 tasks complete
**Last Updated**: 2026-11-07

## Overview

`cafleet doctor` and `cafleet setup` spend 96–99 % of their wall time in serially spawned `hermes` CLI processes, and `mise //:test` spends 95 % of its 190 s in one quadratic test. This change makes doctor and setup run their Hermes reads concurrently and only as often as needed, removes every user-facing CAFleet environment variable (`CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID`, `CAFLEET_MULTIPLEXER`, `CAFLEET_MAX_TEXT_LEN`) and every read of the caller's multiplexer variables outside `fleet create` and doctor's `multiplexer` row (literal IDs, the recorded Director placement and an explicit database option replace them, so `terminal.client_env_passthrough` is no longer written), rebuilds the slow tests, prunes redundant tests and lands a bounded set of high-value refactors. Everything ships on one feature branch as one PR.

## Success Criteria

Reference host: the 64-core research host (Hermes Agent 0.22.4, herdr session, 12 running members). Each wall time is the median of three warm runs measured with `time`.

- [x] `cafleet doctor` finishes in ≤ 1.0 s with Hermes on `PATH` and 12 running members (today 3.4–3.5 s), and in ≤ 0.1 s with Hermes absent from `PATH`. Measured 0.748 s with Hermes at 0 members and 0.009 s without; the 12-member figure is bounded by the implementation, because the member probes overlap the Hermes batch and 12 runs fit in one 100 ms probe wave.
- [x] `cafleet setup` (every backend) on an already configured home finishes in ≤ 1.0 s (today 4.4–4.9 s); a first Hermes install finishes in ≤ 3.0 s (today 7.3 s). Measured 0.737 s and, after fleet teardown, 2.703 s.
- [x] `cafleet member create --coding-agent hermes` spends ≤ 0.8 s in Hermes processes before the pane launches (today ~2 s). Measured 0.615 s for the concurrent batch.
- [x] No code outside `#[cfg(test)]` reads or exports a `CAFLEET_*` environment variable, and no CAFleet command reads `TMUX`, `TMUX_PANE` or `HERDR_*` except `fleet create` and `doctor`'s `multiplexer` row; setup writes no `terminal.client_env_passthrough`.
- [x] An idle member runner's main thread uses ≤ 3 % of one core over 60 s (today 78–86 %; 2.4 % measured with the § A11 fix on the profiling host), for a Claude member and a Hermes member. Measured 1.8 % (Claude) and 2.0 % (Hermes).
- [x] The doctor member probe pass finishes in ≤ 0.2 s for 50 healthy members, and 16 unresponsive runners cost ≤ 0.2 s instead of 1.6 s. Bounded by the implementation rather than measured live: 16 probe workers, each run capped at `min(100 ms, remaining)`, give one ≤ 100 ms wave for 16 unresponsive runners and four ~8 ms waves for 50 healthy ones.
- [x] `mise //:test` finishes in ≤ 20 s wall (today ~190 s) with 0 failures at the task's default thread count; the serial (`--test-threads=1`) CPU total is ≤ 300 s (today ~597 s). Measured 15.39 s and 77.7 s.
- [x] No test takes more than 3 s when run alone, except the one test § C3 Accepted long tests names as longer than 3 s.
- [x] Every test the § C5 ledger deletes names its surviving cover; every other deleted test is a § D6 removed-behavior test or a § C5 Rule-violation fixes entry.
- [x] Every refactor item B1–B22 has landed, and the implementation-region line count (§ Line-count method) drops by ≥ 900 lines net from the branch point, Workstream D's added code included. Measured 31,499 → 30,577 (−922) at the branch head.
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass; `SPEC.md`, `docs/`, `skills/` and `README.md` describe the shipped behavior with no stale command sequence.

**Line-count method.** Baseline: `main` at the branch point (`git merge-base HEAD main`). For each `src/**/*.rs` file at the baseline and at the branch head, drop every item annotated `#[cfg(test)]` (a module, `impl`, `fn`, `const`, `use` or `struct`, through its matching closing brace or terminating `;`), then count the remaining lines that are neither blank nor only a `//` comment. The script lives under the design folder's `.scripts/` (local, not committed) and runs on `git archive` copies of both commits.

---

## Background

### Measured cost (installed 0.32.0, reference host)

| Command | Wall | Hermes processes | Non-Hermes work |
|---|---|---|---|
| `cafleet doctor` (12 members) | 3.4–3.5 s | 5 serial, 0.62–0.71 s each | ~0.12 s (member probes ~8 ms each) |
| `cafleet setup`, configured home | 4.4–4.9 s | 7 serial | ~26 ms for every backend's files and records |
| `cafleet setup --coding-agent hermes`, first install | 7.3 s | 11 serial | — |
| `cafleet member create --coding-agent hermes` | ~2 s before the pane launches | 2 serial reads plus `hermes --version` | — |
| `mise //:test` | ~190 s | — | One test is 181 s; the other 1,746 tests finish in 33 s |

A `hermes config …` call costs ~0.65 s of Python start-up (argparse builds ~426 sub-parsers, a runtime venv check and provider discovery run on every command); CAFleet cannot shorten one call, only make fewer calls and stop serializing them. The doctor regression dates to `b2d9d35c` (2026-11-05), which added `hermes.extension` and `hermes.plugin_enabled` (1 → 5 doctor calls, 2 → 7 setup calls). The test regression is `catalog::tests::bundled_guard_rejects_every_unreviewed_change_to_each_bundled_fixture`: 191 fixtures × 18 mutations = 3,438 full bundled-catalog loads of ~52 ms, which grows with the square of the fixture count.

### Where the steady-state Hermes calls go

| Surface | Calls in order today | After this change |
|---|---|---|
| Setup | `config path`, passthrough get, `plugins.disabled` get, `plugins.enabled` get, injection get, `plugins.enabled` re-get, injection re-get | `config path` and one `plugins` get, concurrently (the passthrough goes with D; the re-gets run only after a write) |
| Doctor | passthrough get, `config path`, three plugin gets | `config path` and one `plugins` get, concurrently |
| `member create` | passthrough get, `config path`, `hermes --version` | `config path` and `hermes --version`, concurrently |

---

## Specification

### Workstream A — doctor and setup speed

#### A1. One Hermes CLI seam

Replace `SystemHermes`, `DoctorHermes` (`src/cli/setup.rs`) and the `config path` call in `hermes_settlement::effective_home` with one type in the new module `src/coding_agent/hermes.rs` (§ B7); `SystemProbe::hermes_client_env_passthrough` is deleted with the passthrough (§ D6):

```rust
/// Runs `hermes` with `HERMES_HOME=<base>`.
pub(crate) struct HermesCli {
    executable: PathBuf,
    base: PathBuf,
}

/// Each call runs until `deadline`; calls of one concurrent batch pass the same deadline.
pub(crate) trait HermesConfig: Sync {
    fn config_path(&self, deadline: Instant) -> Result<String, String>;                  // trimmed stdout of `config path`
    fn get(&self, key: &str, deadline: Instant) -> Result<Option<Value>, String>;        // `config get --json KEY`; None = *not set*
    fn set(&self, key: &str, value: &str, deadline: Instant) -> Result<(), String>;      // `config set KEY VALUE`
}
```

- `HermesCli` resolves the executable and the base once at construction; callers that find no `hermes` on `PATH` construct none. `HermesConfigRunner::binary_available` and the `Option<runner>` hand-off in `install_selected` become an explicit `enum HermesSetup { Available(&dyn HermesConfig), MissingRequired, Absent }` resolved once in `cli::setup::run`: setup behaves the same whether `hermes` was selected explicitly or found by plain setup, `MissingRequired` is an explicit `--coding-agent hermes` without `hermes` on `PATH` (`failed to install extension into <base>: binary hermes not found on PATH`), and holding `&dyn HermesConfig` lets tests drive the install with a fake.
- Methods take `&self`, so independent reads run on `std::thread::scope` workers. Test fakes implement `HermesConfig` with a `Mutex` around their call log and config map.
- Budgets, each computed when its phase starts:

| Surface | Phase | Deadline |
|---|---|---|
| Setup | The read batch (A2) | One `now + 10 s` shared by both reads |
| Setup | Each serial `config set` | Its own `now + 10 s` |
| Setup | The post-write `plugins` re-read | Its own `now + 10 s` |
| Doctor | The Hermes batch | One `now + 2 s` |
| `member create` | The `config path` + version-probe batch | One `min(now + 2 s, creation deadline)` |
- The *not set* rule (exit 1 with one trimmed stderr line equal to `Config key not set: <key>`) and `HERMES_HOME` handling stay as `SPEC.md` states them today; their home moves to § Hermes plugin enablement.

#### A2. Concurrent read batches

Every surface gathers its independent Hermes reads in one concurrent batch, then evaluates the results in the current specified order, so each error message and its precedence are unchanged:

```rust
let (path, plugins) = std::thread::scope(|scope| {
    let path = scope.spawn(|| hermes.config_path(deadline));
    let plugins = scope.spawn(|| hermes.get(PLUGINS_KEY, deadline));
    (join(path), join(plugins))
});
```

| Surface | Batch | Then |
|---|---|---|
| Setup | `config path`, `config get --json plugins` | Install the extension, plan the plugin writes in Rust, run the needed `config set` calls serially (each set rewrites `config.yaml`), then re-read `plugins` once only when something was written |
| Doctor | `config path`, `config get --json plugins`, spawned before the multiplexer, database, OpenCode and member checks so they overlap (A4) | Build `hermes.extension` and `hermes.plugin_enabled` from the results |
| `member create` | `config path` (effective home for settlement), `hermes --version` (native version probe) | Evaluate the config-path failure first, then the version probe, as today |

Each surface thus makes at most two concurrent Hermes calls in steady state (~0.7 s wall); a first install adds at most two serial sets and one re-read.

#### A3. One `plugins` subtree read, verify only after a write

- `hermes config get --json plugins` returns the resolved subtree (`{"enabled": […], "entries": {"cafleet": {"allow_gateway_injection": true}}, …}`; an absent member is missing from the object). A missing `disabled`, `enabled` or `entries.cafleet.allow_gateway_injection` member is that key's *not set* state; a *not set* `plugins` key means all three are *not set*; each present member keeps today's type check (`plugin_list`).
- `read_plugin_state(&dyn HermesConfig) -> Result<PluginState, String>` with `PluginState { disabled_lists_cafleet: bool, enabled: Vec<String>, injection_allowed: bool }` and `fn missing(&self) -> Vec<&'static str>` is the single definition setup and doctor share.
- Setup re-reads only when it wrote: steady state performs the two concurrent reads and no verification; a first install performs the two reads, up to two sets, then one `plugins` re-read. The re-read stays, because a `config set` can exit 0 without the value sticking under a managed scope.

#### A4. Doctor gathers, then renders

`doctor_with` becomes `gather(&DoctorInputs) -> Report` plus `render(&Report, Mode) -> String`, with `DoctorInputs { mux, settings, probe: &dyn SpawnProbe, hermes: Option<&dyn HermesConfig>, registry }`. The Hermes batch (A2) starts first inside a `std::thread::scope`; the multiplexer, database, installation, OpenCode preset/extension and member checks run on the main thread meanwhile; rows are assembled after the join, so row order is unchanged. Doctor wall time becomes max(Hermes batch ≈ 0.7 s, everything else ≈ 0.12 s).

The gather/render split also lets the doctor scenario tests run in-process (§ C1.9).

#### A5. Concurrent member probe pass

`member_checks` probes the registry-pass leftovers on `std::thread::scope` workers, at most 16 at a time (bounds file descriptors). Each worker opens its own read-only connection from the database path (`Connection` is `!Sync`; `diagnose_runtime` already reopens from `conn.path()`). The 5 s pass budget and the per-run cap `min(100 ms, remaining)` keep their meaning; the budget check stays at probe start; results are collected in registry order.

`SPEC.md` § Member runtime checks, Probe pass, becomes: "Only runs the registry pass leaves unclassified are probed, up to 16 at a time: … One five-second budget covers the pass, and each run is bounded by the smaller of 100 ms and the remaining budget when its probe starts."

#### A6. No polling tail in bounded subprocesses

`run_bounded_output` (`src/coding_agent/mod.rs`) becomes an adapter over the `poll(2)`-based engine in `src/runtime/system.rs` (§ B6), which returns as soon as the child exits and both pipes close. The 10 ms `try_wait` sleep loop and its reader threads are deleted. User-visible error strings (`could not start: …`, `did not finish within its deadline`, `output did not close within its deadline`) are preserved by the adapter.

#### A7. Doctor resolves each backend once

- `agent_checks` resolves `probe.resolve_binary(name)` once per backend and passes `Option<&Path>` to every row builder; `hermes_inputs` folds into `extension_base`.
- `Check::fixable(name, status, detail, phrase)` builds the seven `…; run 'cafleet setup --coding-agent X'` rows; `Check::failure(name, detail)` builds the four rows whose detail is also their phrase.
- A typed `CheckStatus { Ok, Info, Warn, Error }` deriving `Ord` (severity order) replaces the `&'static str` statuses, `severity()` and its `unreachable!`; `MemberState::status()` returns it (§ B11).
- `format_id_ranges` sorts and dedups once and groups with `slice::chunk_by`.

#### A8. Doctor creates no Hermes home

A `hermes` call against a missing `HERMES_HOME` makes Hermes create a full home (`SOUL.md`, `cache/`, `logs/`, …), which contradicts doctor's read-only contract. When the resolved Hermes base does not exist, doctor runs no `hermes` process, and each Hermes row that needs one reports `info` `unavailable: no Hermes home at <base>` with the summary phrase `no Hermes home at <base>` and the setup fix. The backend summary line lists an identical phrase once, so it reads `info hermes: no Hermes home at <base> — run 'cafleet setup --coding-agent hermes'`. Setup, which is allowed to create the home, is unchanged.

#### A9. Rejected alternatives

| Alternative | Reason |
|---|---|
| Parse `<HERMES_HOME>/config.yaml` in Rust (permitted by the user) | Locating the file means re-implementing Hermes's profile resolution (a `HERMES_HOME` whose parent is `profiles`, the root's sticky `active_profile`, live-profile markers, symlink spelling), and the resolved `plugins` value also merges `DEFAULT_CONFIG` and the managed scope (`$HERMES_MANAGED_DIR` or `/etc/hermes`). A direct parse answers a different question than the one the gateway applies, for ~0.7 s saved; A2's two concurrent calls already meet the targets. |
| Derive `config path` in Rust | Duplicates Hermes's profile rules, which `SPEC.md` delegates to `hermes config path`. |
| One Python snippet against Hermes's private API | Couples to unversioned internals; only ~0.45 s better than A2+A3. |
| Cache readings across invocations | Rejected by the user; invalidation cannot see environment, `.env` or managed-scope changes. |
| `--deep` opt-in for Hermes checks | Unnecessary once doctor is under 1 s. |

#### A10. Contract updates

| Surface | Change |
|---|---|
| `SPEC.md` § Hermes plugin enablement | Restate as D7's sequence: one concurrent read batch (`config path`, `plugins`) under one ten-second deadline, plan, serial writes with ten seconds each, one `plugins` re-read with ten seconds only after a write. Keep the plugin error strings and output lines. |
| `SPEC.md` § Extension checks, `hermes.plugin_enabled` | "reads `disabled`, `enabled` and `entries.cafleet.allow_gateway_injection` from one bounded `hermes config get --json plugins` call"; `hermes.extension` and `hermes.plugin_enabled` share one concurrent batch under one 2 s deadline; add the A8 `no Hermes home` row to both tables and the summary phrase table. |
| `SPEC.md` § Member runtime checks | The A5 sentence. |
| `SPEC.md` § `member create` | `hermes config path` and the native version probe run concurrently under the 2 s deadline. |
| `skills/cafleet/reference/cli.md` § `setup` and `doctor`, § `member create` | Mirror the command sequence and the new `info` row. |

#### A11. Idle runners wait on events

Measured on the branch and on the installed 0.32.0 (60 s, `/proc/<pid>/stat` utime+stime): every idle member runner uses 78–86 % of one core while its native child stays below 1 %. The runner's main thread accounts for it: `RunnerLoop::serve` (`src/runtime/service.rs`) sleeps `tick_sleep`, at most `IDLE_TICK_SLEEP` = 2 ms, between ticks, and each tick spends ~8 ms of user CPU while nothing changes; the `cafleet-events` writer thread uses 0.5 %. The cost predates this branch.

| Runner | Multiplexer | Idle CPU |
|---|---|---|
| Claude member | tmux | 78.2 % |
| Hermes member | tmux | 84.8 % |
| Claude member | herdr | 85.5 % |
| Hermes member | herdr | 77.6 % |

**Root cause.** A per-phase timing profile of an idle Claude runner (host with 5,983 processes) puts 77–78 % of the main thread's wall time in `observe_exit` → `PtySession::try_wait` → `refresh_owned` → `group_members` (8.1–8.3 ms per tick) and 19–20 % in the 2 ms sleep; the screen snapshot, the transport tick, the database `metadata` check, the controller, the detector and the clients take ≤ 1.2 % each. On Linux `group_members` runs `read_dir("/proc")` plus one `getpgid` per PID, about 6,000 syscalls per tick on that host, to re-pin the native process group every tick. The cost therefore scales with the host's process count, not with the member's activity.

**Mechanism.** `PtySession::try_wait` (`src/runtime/process.rs`) checks the native leader first with the existing `stopped(pid)` (one `/proc/<pid>/stat` read on Linux, `kernel_process` on macOS):

| Leader state | `try_wait` |
|---|---|
| Alive and unreaped | Returns `Ok(None)`; no group scan, no reap |
| Exited (zombie, unreaped) | Runs `refresh_owned`, anchored because the unreaped zombie still holds the PID and so the process-group ID, then reaps as today |
| Native identity unknown | Skips the leader check, runs `refresh_owned` and reaps as today |

`try_wait` delegates to `try_wait_with(&mut impl ProcessObservations)`, so tests drive it with the existing fake observations. The 2 ms tick loop, `tick_sleep` and every deadline, deferred retry, health reply, input, observation and shutdown path are unchanged; profiled with the fix, the main thread drops from 80.3 % to 2.4 % (tick 46 µs, `observe_exit` 8.3 ms → 12 µs), the remainder being the 2 ms-sleep wakes.

**Preserved invariants.**

- Every live group member is still pinned before the leader is reaped, and `signal_verified` and `survivors` call `refresh_owned` themselves before any signal, so shutdown TERM/KILL, `survivors`, `unpinned_group_member` and the cleanup diagnostics see every process still in the group at that point.
- `owned process group lost its pinned generation` cannot fire while the leader is unreaped, so dropping the per-tick refresh removes no reachable mid-run check.

**Observable differences.** Cleanup covers exactly the group's members at leader exit and at each shutdown refresh, instead of every process the per-tick refresh ever saw in the group:

- A short-lived descendant that exits while the leader still runs is no longer pinned, so a later reuse of its PID inside the group no longer fails the run mid-run with `owned PID was replaced by another generation`, a spurious runner failure today.
- A descendant that leaves the native process group (`setsid`/`setpgid`) while the leader runs is no longer pinned and therefore no longer terminated at shutdown.

`SPEC.md` specifies termination of the owned process group at shutdown and no per-tick scan, so it needs no change.

**Rejected alternative.** Blocking the serve loop in `poll(2)` on stdin, the PTY master, stdout, the control listener, the clients and a signal self-pipe would cut the remaining ~2.4 % to an expected ~0.3 %. It needs a timeout derived from every in-flight deadline and touches the loop, the transport and the signal guards, while the leader check alone meets the Success Criteria.

### Workstream B — bounded refactor

Each item is behavior-preserving unless its row names a contract change. Line counts are implementation-region estimates from the research.

B17–B22 are the larger refactors that the research ranked behind B1–B16. By the user's decision of 2026-11-07 they also land in this change, after B1–B16, so that the line count reaches the Success Criteria target. B1–B16 measured −176 lines net (Step 7), and B17–B22 are estimated at about −820 to −1,060. Each keeps every `SPEC.md` contract; where an item names an error-text change, the contract fixes the rejection class, not the wording.

#### B1. Bundled catalog: one static instance, fixture verification as a build guard

Before: every `ProfileCatalog::bundled()` re-parses `bundled.json` and the four embedded `native/*.json` files (~600 KB), recompiles every regex and recomputes 191 SHA-256 digests (~40–50 ms in the test build); production embeds the fixture files only to re-verify them at each `member create` and runtime start.

After:

```rust
static BUNDLED: LazyLock<ProfileCatalog> = LazyLock::new(|| {
    ProfileCatalog::from_definitions(&serde_json::from_str(BUNDLED_DEFINITIONS)
        .expect("bundled.json is valid JSON"))
        .expect("the bundled catalog passes its build guard")
});

impl ProfileCatalog {
    pub(crate) fn bundled() -> &'static ProfileCatalog { &BUNDLED }
}
```

- `CLAUDE_FIXTURES` … `HERMES_FIXTURES`, `BUNDLED_FIXTURES`, `verify_bundled_fixtures`, `verify_native_fixture_file`, `verify_native_screen`, `SCREEN_FIELDS` and `verify_cell_backgrounds` move under `#[cfg(test)]` in `catalog::test_support`; `from_bundled_definitions` becomes test-only. One test asserts `from_bundled_definitions(&bundled_definitions(), BUNDLED_FIXTURES)` is `Ok`.
- The production callers (`cli/member.rs` `create`, `runtime/service.rs` runner start) borrow the `&'static` value; `RunnerDependencies` stores `&'static ProfileCatalog`. Tests that need a synthetic catalog keep building owned values.
- `catalog::test_support::bundled_file` / `bundled_fixture` parse each embedded file once through `LazyLock<Value>`.
- Contract: `SPEC.md` § Bundled profiles and native screen fixtures, the paragraph "The bundled catalog loads only when each source reference resolves…", becomes the bundled catalog's build guard: "A CAFleet build whose bundled backend files fail these checks does not pass its test suite", keeping the error table.
- Effect: −190 production lines, −598 KB of release `.rodata`, no fixture verification on `member create`/runtime start, and one catalog load per test process instead of ~4,700.

#### B2. Launch policy without native-version gating

Before: `policy_choices(native_version)` gates Claude on `Some("2.1.302" | "2.1.312")` and Codex on `Some("0.163.0")` with `_ => &[]`, but every production caller passes `None`; `build_spawn_argv_with_policy` re-runs `resolve_policy` to prove its input came from `resolve_policy`.

After:

```rust
pub(crate) fn policy_choices(self) -> &'static [&'static str] { match self { … } }
pub(crate) fn resolve_policy(self, requested: Option<&str>) -> Result<LaunchPolicySelection, CafleetError>;

pub(crate) struct LaunchPolicySelection {     // private fields, getters
    requested_mode: Option<String>,
    resolved_mode: &'static str,
    mode_source: ModeSource,
    permission_argv: Vec<String>,
}
```

Delete the `backend`/`native_version` fields, the self-revalidation block and the "launch policy does not match its backend/catalog" error, together with every test that asserts a version-gated choice list (a choice set that differs by native version, or an empty set for an unknown version) or the rejection of a selection resolved for another backend or catalog; those tests have no § C5 ledger row. The `native_capture` harness's "capture recorded permission policy mismatch" check (`src/runtime/observation.rs`) re-resolves with `backend.resolve_policy(run.requested_mode.as_deref())`, compares `resolved_mode`, `mode_source` and `permission_argv` with the recorded run as today, then calls `validate_spawn_policy_argv`; `run.native_version` stays checked by its executable-identity assertion just above. Contract: `SPEC.md` § Launch policy and capability registry drops "native-version eligibility" from the catalog's list and states the Claude and Codex choices unversioned, keeping the sentence that records which versions were verified.

#### B3. `NativeCapabilities` becomes one query

Before: a production struct with nine fields, eight of them `#[cfg(test)]`, built by a 60-line `ProfileCatalog::capabilities`; production reads only `.detection`.

After: `pub(crate) fn detection_coverage(&self, backend: CodingAgent, version: Option<&str>) -> Coverage` (~8 lines). The composer/submission/answer projections tests need move to a `#[cfg(test)] impl ProfileCatalog` in `catalog::test_support`; the echo fields and their assertions are deleted (−69 production lines).

#### B4. Composer and answer plans carry no echoed provenance

- `ComposerOperation::prefix(self) -> &'static [u8]` replaces `ComposerSubmission.prefix`, the `encode` assertion, the `b""`/`b"! "` literals in `runtime/input.rs` and `test_support::operation_prefix`; the `prefix_hex` JSON field is still validated against it.
- `ComposerPlan` drops `backend`, `native_version`, `rule_ids`, `fixture_ids` and their getters, and `runtime/input.rs` drops the `"composer plan provenance changed"` ensure that compared them with the arguments the plan was just built from.
- `AnswerStage` drops `required_rule_id`/`required_choice_id`; `runtime/input.rs` compares `selection.plan.capability().request_rule_id()` with the observed rule. The JSON keys stay validated at load.

#### B5. Detector: one observation pass per frame

Before: for a request frame with N answer plans, `match_answer` re-runs `observe(screen)` and rebuilds `ScreenLines` per plan (1 + 2N detector passes), rebuilds the plan to prove it genuine, and `extract` clones the whole `Rule` into every `Matched`.

After: `fn match_answers(&self, screen, observed: &ScreenObservation, plans: &[AnswerPlan]) -> Result<Vec<AnswerMatch>>` builds `ScreenLines` and the matched request rule once; `Matched<'a> { rule: &'a Rule, … }`; one `fn evidence(base, rule, operation, excerpt, sensitive) -> Value` shared by `annotation_blocks` and `observe`. `AnswerMatch` output is byte-identical; the existing detector tests pin it.

#### B6. One bounded-subprocess engine

`src/runtime/system.rs` keeps the `poll(2)` engine and gains environment support and a deadline entry point that returns status, stdout and stderr. `coding_agent::run_bounded_output` and `run_bounded` become ~10-line adapters (A6). Inside `system.rs`: `CommandRunner::run(timeout_secs)` stays, because `run` with `None` is the untimed run of `fleet create`, capture and the herdr layout calls, and `SystemRunner::run` and `run_for` share one `run_process`; every `ErrorKind::Interrupted` loop calls `retry_interrupted`; `binary_exists` uses `find_executable_in(PATH)`; the dead `#[cfg(not(unix))]` branch of `is_executable_file` is deleted; `cleanup_absolute` sleeps through `nap_until`. ~70–90 lines.

#### B7. One Hermes module

`src/coding_agent/hermes.rs` absorbs `hermes_settlement.rs`, `HermesCli` (A1) and the Hermes config helpers from `src/assets.rs` (`hermes_config_get`, `plugin_list`, `hermes_config_set`, `hermes_plugin_dir`, `read_plugin_state` and the plugin write planner), so every Hermes CLI interaction lives in one module. Settlement keeps one `fn poll<T>(host, until, done) -> Result<Option<T>>` for its three polling loops, returns `CafleetError` internally instead of round-tripping through `String` (16 `.map_err(message)`), and narrows every item without an outside caller to private.

#### B8. Test-only code leaves production regions; registries become globs

- `persist_event` and `persist_event_with_effective_policy` are deleted (callers use `storage::persist_transition(..).unwrap().0`); `reserve_input`, `settle_input`, `release_permission_claim` and `create_run` move into `broker::runtime::test_support`; the 13 `#[cfg(test)] pub(crate) use` lines go.
- `src/broker/runtime/mod.rs`'s 35-line re-export registry becomes `pub(crate) use events::*; pub(crate) use inputs::*;` plus the two renamed items; `src/output/mod.rs`'s 22 `format_*` lines become `pub(crate) use crate::output::text::*;`. `mise //:lint` must stay clean of `ambiguous_glob_reexports`.
- `CodingAgent::validate_spawn_policy_argv` (85-line `#[cfg(test)]` method inside a production `impl`) moves to a `#[cfg(test)] impl CodingAgent` block.

#### B9. `Multiplexer` trait

Before: `AnyMultiplexer { Tmux, Herdr }` forwards ten methods through two-arm matches (130 lines), `PostCloseLayout` needs an `unreachable!` for a tmux value meeting a herdr layout, and `resolve_multiplexer_name` returns a string that is matched again behind another `unreachable!`.

After: `pub(crate) trait Multiplexer` with the shared methods (`name`, `ensure_available`, `bounded`, `validate_key_target`, `send_keys`, `context_discovery`, `split_window`, `capture_pane`, `kill_pane`) and provided `close_pane`/`restore_after_close` defaults (tmux behavior), overridden by herdr, which keeps its tab id privately. `resolve_multiplexer` returns `Box<dyn Multiplexer>`. The `validate_key_target` deadline/availability preamble and pane-id lookup move into one provided method; the per-backend `name`/`ensure_available`/`bounded` delegates and herdr's `run` wrapper go. `tmux_argv`, `herdr_argv` and `hermes_args` become one `argv(&[&str]) -> Vec<String>`. Error strings are unchanged.

#### B10. Pane-lock retry helper

`fn try_lock(file, path) -> Result<bool>` and `fn retry_until<T>(deadline, attempt: impl FnMut() -> Result<Option<T>>) -> Result<Option<T>>` replace the three 50 ms retry loops and two `TryLockError` mappings in `src/pane_lock.rs` (`reacquire_within`, `acquire_within`, the `lock_deletion` inner loop). `lock_deletion` now makes one last attempt after its final sleep before reporting busy.

#### B11. Runtime invariants get one name each

| Item | Change |
|---|---|
| Database identity check | `fn ensure_same_database(conn, expected: &Value) -> Result<()>` replaces nine identical `validate_database(&storage::database_identity(conn)?, …, || error("runtime database identity changed"))` triples; `validate_database` is deleted. |
| Runtime wire enums | `SubmissionStatus`, `SubmissionStage`, `MissingEvidence` (its index-based `VALUES` table goes), `Visibility`, `RecoveryOutcome` and `ObservationFailure` use the existing `stored_enum!` macro, so each wire string is written once. |
| `RunRecord` derivations | `owner_process()`, `child_process() -> Result<Option<ProcessIdentity>>` and `is_settled()` replace the hand-built `ProcessIdentity` literals and the `status.terminal() && cleanup_state == Complete` predicate at their call sites (including `cli/setup.rs` `classify_registry`). |
| Doctor status | `CheckStatus` (A7). |

#### B12. Input client: one error type at the host seam

- `InputCommandHost::{lock, health, exchange}` return `crate::runtime::Result<T>`; `host_failure` and the 15 `.map_err(CafleetError::from)` round-trips go.
- `InputFailure { source, context: InputContext }` with `#[derive(Default)] InputContext` replaces the duplicated four context fields and the mostly-`None` struct literals.
- `validate_write_now`, `acquire_write_guard` and the reconcile settle path return `Result<T>` instead of building an `InputFailure` that their single caller strips and re-wraps.
- Helpers `control_request(run, expiration, operation)`, `lock_pane(host, …, busy)`, `checked_health(host, run, deadline)` and `bracket_health(conn, host, run, deadline, validate)` replace the copy-pasted control-exchange skeleton of submit/respond/inspect/reconcile; `reconcile_ordinary_with_host` is deleted; `pane: Option<(&Placement, &PaneLock)>` replaces the tuple of options and its pairing `ensure`.

#### B13. `InputCoordinator::advance` split

The 455-line `advance` becomes a ~40-line dispatcher over `pending.phase` calling `reserve_pending`, `arm_pending` and `write_pending_stage`, plus `interrupt_composer` for the duplicated composer-interruption block. `terminal_bookkeeping` stays set before every early return the epilogue inspects. The 30 redundant `use crate::runtime::X;` lines above `use crate::runtime::*;` in `input.rs` are deleted (compile-verified).

#### B14. Event writer: worker struct and one connection

- `start_worker`'s 265-line closure becomes `struct Worker { conn, spool, run, lock, mailbox, queue, committed, compact, … }` with `receive`, `sync_pending`, `persist_refresh`, `persist_front`, `compact_if_idle` and `refill_sequences`; `#[derive(Default)] struct Mailbox`; one `fn send(&self, work)` for both producer paths.
- `Outbox` drops its second read-only connection; every validating method takes the worker's `&mut Connection`, collapsing `validate`/`validate_on`, `scan`/`scan_recovery` and `append_synced_with_policy`/`append_recovery` into one of each. `Outbox::open_recovery` returns `(Self, OutboxScan)`, so recovery reads and hashes the outbox once instead of twice. `const FRAME_HEADER: u64 = 4 + 32;`.
- The writer's outbox open reuses `artifacts::open_private` (which also sets `O_CLOEXEC`).

#### B15. Fail-fast fixes

| Site | Before | After |
|---|---|---|
| `config.rs` `home_dir` and `resolve`, `cli/setup.rs` env closure, `runtime/system.rs` `SystemProbe::env_var` | `std::env::var(name).ok()` treats a non-UTF-8 `HOME`, `HERMES_HOME`, `CODEX_HOME`, … as unset and silently selects the default | `config::process_env(name) -> Result<Option<String>, CafleetError>`: `NotPresent` → `None`, `NotUnicode` → `App("{name} is not valid UTF-8")`. Contract: add the error to `SPEC.md` § Config-dir resolution. |
| `broker/members.rs` `idle_seconds` | `parse_lenient(latest).ok()?` turns a corrupt stored timestamp into "idle unknown" | `?` with `InvalidStoredValue` |
| `cli/message.rs` argv0 parse | `split(..).next().unwrap_or_default()` | `.expect("split yields a first segment")` |

#### B16. CLI output plumbing

`cli::emit` returns `Result<(), CafleetError>` and writes through `writeln!(std::io::stdout().lock(), …)` instead of `println!`, so the 21 `emit(..); Ok(())` tails become `emit(..)`. This is a contract change: today a failed stdout write panics (exit 101). After it:

| Write outcome | Behavior |
|---|---|
| Success | Unchanged |
| `ErrorKind::BrokenPipe` (the reader closed, e.g. `cafleet member list 1 \| head -1`) | The command stops writing and exits with its own result, as if the output had been consumed; nothing goes to stderr |
| Any other write error | `Error: failed to write output: <cause>`, exit 1 |

`SPEC.md` § Error Messages gains both rows. `impl From<MultiplexerError> for CafleetError` replaces the six hand-written conversions in `cli/`.

#### B17. Serde-typed input-control codec

Before: `src/runtime/control.rs` decodes and encodes the input-control protocol by hand. Each `kind` string is matched in four places (`InputRoute`, `request_fields`, `decode_request_value`, `encode_input_request`). Each field name is listed twice: once in an exact-field list (`request_fields`, `RESPONSE_SUBMISSION_FIELDS`, `SUBMISSION_FIELDS`, `RESPONSE_FIELDS`, `INSPECTION_FIELDS`) and once in a `json!` encoder (`PermissionRespondInput::wire`, `encode_result`). `Object` re-implements unknown-field and required-field rejection, and `encode_input_request`/`encode_input_reply` decode their own output as a self-check.

After: the domain types themselves are the wire. `InputControlRequest`, `InputControlOperation`, `InputControlReply`, `ReceiptReply` and their payload structs derive `Serialize`/`Deserialize`: `#[serde(tag = "kind", rename_all = "snake_case")]` for the operation and reply enums and `#[serde(tag = "status")]` for receipts. The request carries its operation through `#[serde(flatten)]`, so the frame stays flat, as `SPEC.md` § Input control frames and deadlines fixes it: `protocol_version`, `kind`, `identity` and `expires_at_mono_ns` sit at the top level beside the kind's own fields. `ReceiptAction` uses `#[serde(tag = "action")]`, `PermissionRespondInput` and `PermissionResponseResult` flatten their answer key, a private `Frame<T>` carries `protocol_version`, and private `ReplyBody`/`ResultWire`/`ReceiptWire` shape the replies. No wire-to-domain conversion layer exists.

Exactness is enforced by two checks instead of per-type `deny_unknown_fields`, which `flatten` cannot combine with:

- **Duplicate-key rejection:** `decode_strict_json`/`UniqueValue` reject a duplicate key at any depth, including inside `Value` fields such as `database_identity_json`.
- **Decode-then-re-encode equality:** `decode_exact` decodes, then requires the re-encoding to equal the received `Value`: `serde_json::to_value`, or the public `output::json` projection for an inspection and a permission response. An unknown field, a missing required-but-nullable field such as `message_id`, a field that would decode to a default (`delivery`, `choice`, a receipt's `null` units) and a float-rendered integer all re-encode differently and are rejected.

`ControlIdentity` validates its positive IDs and nonce on decode. The domain checks (positive IDs, `validate_operation_key`, the single-line input rule, kind/message pairing, `validate_respond`, `validate_order`, choice uniqueness, `validate_binding`) become `InputControlRequest::validate` and `validate_reply`, which run before encoding and after decoding.

Delete `Object`, `request_fields`, the five field-list consts, `PermissionRespondInput::wire`, the `json!` encoders and the encoders' self-decode round trips. `InputRoute` stays and keeps routing every frame that carries a `kind` to the input decoder.

Files: `src/runtime/control.rs`, plus the callers in `src/runtime/input_client.rs` and `src/runtime/service.rs`. Expected: −430 to −530 lines; the landed change measures −137 in `control.rs`. Behavior: the accepted and rejected frame sets and the reply bytes are unchanged — unknown, duplicate, missing and wrongly typed fields still fail before any effect. The request key order becomes `protocol_version, identity, expires_at_mono_ns, kind, …`; `SPEC.md` fixes no request key order. Only the structural error descriptions change, under the existing error prefix. A test that pins a hand-codec description asserts the rejection instead.

#### B18. Serde-typed catalog loader

Before: `src/coding_agent/catalog.rs` validates the bundled profile JSON with hand-written helpers (`invalid`, `require`, `object`, `text`, `array`, `texts`, `boolean`, `backend`, `positive`, `one_of`, `expression`, `unique`, `semver`, `lower_hex`) and per-compiler field lists that change with the observation kind, `panel_separator`, `focus_style`, `decision`, the stage gate, the composer `submission` and `cell_backgrounds`. `compile_rule` (~175 lines) and `ProfileCatalog::from_definitions` (~240 lines) are the longest functions in the module. `ChoiceScope` already derives `Deserialize` but is still parsed through `one_of`.

After: the loader reads typed structs that derive `Deserialize` with `deny_unknown_fields`. Private `Raw*` mirrors (`RawCatalog`, `RawFixture`, `RawProfile`, `RawRule`, `RawExtract`, `RawOperation`, `RawCapabilities`, `RawCapability`, `RawSubmissions`, `RawSubmission`, `RawAnswer`, `RawStage`) cover the shapes that compile into another type. `Capture`, `Choice`, `Matcher`, `Region` and `Frame` already have the loaded shape and deserialize directly.

- **Required-but-nullable keys** (`operation`, `native_request_id`, `detection`, `composer`, `annotations`, `prompt`, `exec`) use `deserialize_with = "Option::deserialize"`, so a missing key still fails.
- **Optional keys** use `#[serde(default)]` with the `present` deserializer, so an absent key is `None` and a present `null` stays an error, as in the hand loader.
- **Newtypes and field deserializers** carry the nonblank, regex, positive-bound, native-version and lowercase-hex rules. Every string field that the hand loader required to be nonblank uses the nonblank deserializer.
- **Enums** (`ScreenSelector`, `CursorRule`, `MatcherScope`, `FocusStyle`, `ChoiceScope`, `MarginRows`) derive `Deserialize` with `rename_all = "snake_case"`. `CodingAgent` derives it with `rename_all = "lowercase"`, and `NativeVersion` and `ObservationKind` deserialize through `TryFrom<String>` over their existing parsers.
- **Semantic checks keep their hand-written messages:** kind-dependent `frame`/continuation presence, gate exclusivity, provenance intervals, and the answer and stage relations.
- **`from_definitions` splits** into `compile_profile` and `compile_answer`.

Files: `src/coding_agent/catalog.rs`, with the derives in `src/coding_agent/mod.rs` and `src/coding_agent/detector.rs`. Expected: −175 to −275 lines; the landed change measures −88 in `catalog.rs` and +15 in the derive files. The fixture-screen verification row of the research estimate does not apply, because B1 already made it a build guard. Behavior: the same catalogs load, and every catalog that fails today still fails. Structural error texts become serde's under the `invalid native profile: ` prefix, which `SPEC.md` does not quote; the semantic messages the catalog tests assert stay unchanged.

#### B19. `run_service` decomposition and typed lifecycle frames

Before: `src/runtime/service.rs` `run_service` is a ~700-line function whose loop nests up to 12 levels.

- `InputRequest { .. }` is built twice verbatim.
- `if client.reply…(…).is_err() { client.closed = true; … }` appears six times.
- The stopped-delivery publication repeats `publish_stopped_results`.
- The `#[cfg(test)]` capture branch re-queries the placement that the `start` branch has just validated.
- Lifecycle frames are untyped: the runner matches `frame.get("kind")` strings and extracts `remaining_ms`/`pane_id` by hand, while `src/runtime/lifecycle.rs`, `src/runtime/control.rs` and `src/runtime/recovery.rs` build the same frames with `json!` and pass `kind: &str` to `control::request`.
- The reply envelope `{protocol_version, result | error}` is built in two places and parsed in two.

After:

1. `#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)] enum LifecycleRequest { Prepare { identity, remaining_ms }, Start { identity, pane_id }, Health { identity }, Shutdown { identity } }`, carrying `protocol_version` in each variant with no `expires_at_mono_ns`, as `SPEC.md` fixes. `control::request(run, LifecycleRequest, deadline)` replaces the `kind: &str` form.
2. A `struct RunnerLoop` owns the loop state: the clients and the next client ID, the creator connection, the `started` flag, the coordinator, the pending input client, the deferred retry, the shutdown client, the handshake deadline, the run, the owner, the lock, the artifacts, the descriptor and the process. Its methods are `accept`, `serve`, `prepare`, `start`, `health`, `tick_running`, `complete_pending_input`, `retry_deferred` and `finish`. The five locals that `NativeIo` borrows mutably move into the struct, so the borrow structure is kept. `run_service` becomes setup, `catch_unwind(loop.run())` and `finish`.
3. `Client::reply`/`reply_input` mark the client closed on failure themselves, and one `reply_envelope`/envelope decoder in `src/runtime/control.rs` serves both encoders and both parsers.

Landed shape:

- **Runner loop:** `RunnerLoop` holds the loop state, the borrowed setup values and `NativeIo`. `run_service` builds it, runs `catch_unwind(|| runner.serve())`, then takes back the run, the `started` flag, the coordinator, the clients, the shutdown client and the native exit for the unchanged finish. The input request is folded into `RunnerLoop::inspect`/`submit`, and `Client::answer`/`answer_input` close the client on failure and return whether the reply was queued.
- **Lifecycle frames:** `LifecycleRequest::Start` holds `pane_id: Option<String>`, so a missing pane still reads `start requires concrete pane placement`; `LifecycleRequest::health(run)`/`shutdown(run)` build the probe frames, and `ControlClient::exchange` takes a `&LifecycleRequest`.
- **Reply envelope:** one `control::encode_reply<T>` (`ReplyBody<T>`) serves `encode_input_reply` and the runner's lifecycle replies, and one `decode_reply<T>` (strict JSON, version check, `decode_exact`) serves `decode_input_reply` and `ControlClient::exchange`.

Files: `src/runtime/service.rs`, `src/runtime/control.rs`, `src/runtime/lifecycle.rs`, `src/runtime/recovery.rs`. Expected: −100 to −140 lines. Behavior: the lifecycle frames keep their specified fields, and every state transition, reply and shutdown path is unchanged; the existing service tests cover those paths. A lifecycle frame that fails to decode reads `invalid lifecycle control fields`, a lifecycle frame with an unknown field is refused as `SPEC.md` § Input control frames and deadlines requires, and a malformed lifecycle reply fails through `decode_exact`.

#### B20. `CancellationScope` merged into `DatabaseBudget`

Before: two thread-local deadline stacks repeat the same push, nested-restore and pop logic: `CANCELLATION_DEADLINES` with `CancellationScope` in `src/runtime/lifecycle.rs`, and `DATABASE_WAITS` with `DatabaseBudget` in `src/db.rs`. `with_database_deadline` always nests one inside the other. `CancellationScope::new` spawns a named OS thread per scope whose only job is `interrupt()` at the deadline. The same scope already installs `progress_handler(1000, …)`, which aborts a running statement at VM-op granularity, the only points where `sqlite3_interrupt` is observed anyway.

After: `DatabaseBudget` keeps one frame stack that holds both the busy deadline and the cancellation deadline, and installs the progress handler. Delete `CancellationScope`, `CANCELLATION_DEADLINES`, `database_cancelled` and the per-scope watchdog thread. `db::with_database_deadline`, re-exported from `runtime::lifecycle`, runs a `ScopeKind::Cancellable` budget frame whose cancellation instant drives the progress handler, then checks the remaining time; no thread is spawned.

Files: `src/runtime/lifecycle.rs`, `src/db.rs`. Expected: about −55 lines, plus one thread spawn and join fewer per database scope. That matters for doctor's per-member diagnosis and the recovery replay. Behavior: a statement still stops at the scope deadline, through the progress handler, and a lock wait through the busy handler. If a test shows a statement blocked outside the VM loop that only the interrupt stops, a single process-wide watchdog thread fed by a channel replaces the per-scope threads instead.

#### B21. Typed `PendingInput` state

Before: `src/runtime/input.rs` `PendingInput` carries a payload-free `PendingReservation` beside `ready: Option<PermissionReceipt>` and `input_id: Option<i64>`, and `installed_input_id` exists only to `assert!` their cross-field invariants. `permission: Option<Arc<PermissionSelection>>` and `composer: Option<ComposerDelivery>` are mutually exclusive. They are checked at runtime through `ordinary input has no composer delivery` and `nonfinal stage requires a staged plan` errors, and the `pending.composer.as_ref()/as_mut().and_then(|c| c.deferred…)` accessor chain is spelled out at about seven sites.

After:

- `enum PendingReservation { Unreserved, HistoricalReady(Box<PermissionReceipt>), Installed(i64) }` replaces `ready` and `input_id`. `input_id()` derives from the variant, and a historical-ready receipt keeps its known ID from `receipt.input.input_id`. `installed_input_id` becomes a three-arm `match` with no asserts.
- `enum PendingDelivery { Permission(Arc<PermissionSelection>), Composer(Box<ComposerDelivery>) }` replaces the two options. `PendingInput` exposes `input_id()`, `installed_input_id()`, `ready()`, `permission()`, `composer()`/`composer_mut()` and `deferred()`/`deferred_mut()`.

Files: `src/runtime/input.rs`. Expected: about −40 lines. Behavior: purely internal; the impossible states become unrepresentable, and no contract changes.

#### B22. `get_event` validates by rebuilding the payload

Before: `src/broker/runtime/events.rs` `get_event` (~170 lines) validates a stored event by re-deriving its public payload field by field: the per-kind key list, the schema version, the identity loop, the interaction, classification, kind, observation and decision fields, `native_request_id` and `director_action`. `payload()` produces that same schema. `payload()` does not own the delivery-result fields, so `persist_transition_in_transaction` passes an anonymous four-tuple, then removes `director_action`, inserts `cause`/`input_id`/`message_id`/`delivery`/`stage` and re-inserts `director_action` to keep the key order. `get_event` also calls `payload.as_object().expect(..)` twice.

After:

1. `payload(run, transition, event_id, interaction_id, delivery: Option<&DeliveryFields>)` with `struct DeliveryFields { input_id, message_id, delivery, stage, cause }` inserts the delivery fields before `director_action`. That removes the remove/insert sequence and the tuple.
2. `get_event` rebuilds the expected payload from the stored view through `original(..)` and `payload(..)`, and requires it to equal `payload_json`. The checks that `payload()` cannot express stay: `evidence::validate`, `event_seq > 0`, the payload size limit, the notification block and `published_delivery`, which checks the input receipt and supplies the published delivery fields.

Files: `src/broker/runtime/events.rs`. Expected: about −68 lines; the landed change measures −81. Behavior: the persisted key order is unchanged, and every event that validates today still validates. A corrupt stored payload is still refused, under one description, `stored payload does not match its event`, instead of about six. The `InputDeliveryResult` cause rule is preserved and gets its own review.

#### Companion compactions

Alongside B17–B22, the research findings below are applied in the files those items touch. Each is behavior-preserving; where one changes an error description, its bullet names the new text, and `SPEC.md` quotes none of the old ones. Findings that measured as adding lines after `rustfmt` are not applied, nor is F09 (`check_deadline`), which would change an error variant and exit code.

- **Doctor (`src/cli/setup.rs`):**
  - `render` shares one line formatter for the verbose and summary modes, and `Check`, `Summary`, `RuntimeFields` and `MemberGroup` derive `Serialize` in place of `check_json`/`summary_json`.
  - The multiplexer and database rows build their check and summary pair once.
  - One `unavailable(name, paths, available)` builds every configuration-failed and executable-absent `info` row; the Hermes reading is the plain `(config_path, plugins)` pair.
  - `probe_runs` gives each of at most 16 workers a contiguous share of the rows. Results stay in registry order, and the budget is still checked at each probe start.
  - `MemberRun` borrows its `MemberRunRegistry` row. Doctor output is byte-identical in summary, `--verbose` and `--json`.
- **CLI:**
  - The Director-only guard carries its subject-to-fleet query instead of a subject enum matched twice.
  - `member create` reads its arguments directly, `member exec` takes the resolved command, and `require_pane(member, action)` reads the member ID itself.
- **Coding agents:**
  - `lower_hex_bytes(text, max)` validates and decodes every catalog hex field.
  - `HermesOutput` is an `(exit, stdout, stderr)` tuple, the plugin enablement has one post-write check, and the settlement host converts multiplexer errors through `From<MultiplexerError>`.
  - In the detector, `SemanticPart` and `AnnotationFingerprint` drop the constant backend name.
- **Broker input rows (`src/broker/runtime/inputs.rs`):**
  - One `update_input` replaces the six compare-and-set blocks.
  - `settle_any_input_in_transaction` validates the owner once.
  - `reserve_permission_response` leaves the ownership, epoch and open-event checks to `validate_current_capability`, so those refusals read `permission_response: current open runner permission association required`.
- **Broker events and evidence:**
  - `deferred_delivery` serves both delivery lookups, and one `match` on the archive reason yields the notification state.
  - `runtime::rfc3339` replaces four timestamp checks; the effective-policy description becomes `RFC 3339 timestamp required`.
  - A private `StoredEffectivePolicy` decodes the stored effective-policy record and requires its re-encoding to equal the stored value. Its failures name the field `effective_policy.*`, and the stored key order is unchanged.
  - The evidence module keeps one field registry and one shrink step.
- **Broker members, messages and fleets:**
  - `MEMBER_COLUMNS`/`MEMBER_FROM` hold the shared member join, and `get_message` absorbs `message_row`.
  - `require_live_fleet` reads the Director in one join.
  - `delete_fleet_rows` stamps `deleted_at` only when unset, and `create_fleet` returns the committed Director read back through `get_member`.
- **Runtime observation and transport:**
  - Named settle durations and one `settled` helper replace the seven debounce blocks.
  - `ObservationController::apply`/`quiet` replace the repeated observe-and-route sequence, and `Candidate::since` installs both candidate kinds.
  - The transport gains `flush_nonblocking` and `TerminalState::advance_sequence`.
- **Runtime recovery, lifecycle and processes:**
  - `reserve_recovery_range` replaces the two ceiling-and-reserve blocks, `recover` checks the borrowed run itself, and the host-injecting `recover_member_with_held_pane_lock` becomes test-only.
  - `lifecycle::note`, `ShutdownOutcome::fail` and `lifecycle::guarded` share the cleanup diagnostics.
  - `signal_pid`, `running`, `std_child` and `pty_size` fold the process-ownership plumbing.
  - `LaunchDescriptor` embeds `ControlIdentity`, so the launch descriptor file nests the identity, and `ControlIdentity::from_descriptor` is gone.
- **Runtime input and artifacts:**
  - `present(path)` and a capped `read_manifest(path)` serve the artifact checks.
  - `press` uses `broker::active_member` and drops a post-lock placement re-check that the lock already covers.
  - `CafleetError::with_diagnostic` replaces two local helpers.
  - `InputCoordinator::note_head_obstacle` sets the queued-head obstacle.
  - The input client's interaction-existence check fails through `ok_or_else`.

### Workstream C — tests

#### C1. Speed levers (production seams and fixtures)

| ID | Change | Expected effect |
|---|---|---|
| C1.1 | Replace `catalog::tests::bundled_guard_rejects_every_unreviewed_change_to_each_bundled_fixture` and the three per-backend real-fixture guard tests (ledger rows) with two synthetic tables, `bundled_backend_files::a_screen_outside_the_reviewed_shape_is_rejected` (every `SCREEN_CHANGES` mutation, the `alternate_screen` flip, the masked-row and pending-wrap-cursor cases, and the new `kind: "synthetic"` rejection) and `bundled_backend_files::cell_backgrounds_outside_the_reviewed_shape_are_rejected` (one case per `verify_cell_backgrounds` branch), plus one positive load of the real bundled corpus and the file-level cases (`schema_version`, `provenance`, `backend`, missing screen, missing embedded source) once per embedded source through `from_bundled_definitions`. `guarded_fixture`, `guarded_fixture_file`, `load_bundled_file`, `bundled_entry` and `bundled_embedded` leave `catalog::test_support`. | 181 s → ≤ 2 s; cost no longer grows with the fixture count |
| C1.2 | B1's static catalog; every test-side `ProfileCatalog::bundled()`, `detector::test_support::bundled_observe`/`bundled_detector` and `catalog::test_support::bundled_fixture` call reuses it. | `coding_agent` tests 99.5 s → ~21 s serial |
| C1.3 | Delete `runtime::input::tests::ordinary_prompt::deferred_queue::queue_capacity_bounds_queued_plus_active_work_at_4096_with_a_zero_byte_error` and add its deferred-Ready case (CTE prefill with `state='ready'` and `deferred_at` set, then one refused reservation) to `broker::runtime::inputs::tests::runtime_permission_shared_unresolved_limit_preserves_retained_receipts`; `settled_receipt_history_can_exceed_the_unresolved_cache_bound` prefills its 4,096 settled rows with one recursive-CTE insert, then reserves and settles one more through `storage::settle_input`. Both land in Step 2; their § C5 rows record the same change. | −20 s serial, critical path 26 s → ~10 s |
| C1.4 | `cli/message.rs`: `codex_queue_budget()` and `full_queue_retry_interval()` read `CAFLEET_TEST_CODEX_QUEUE_BUDGET_MS` / `CAFLEET_TEST_CODEX_FULL_QUEUE_RETRY_MS` under `#[cfg(test)]` (the existing `full_queue_limit()` pattern) and return the 10 s / 1 s constants under `#[cfg(not(test))]`; the `did not finish within {} s` diagnostic and the `>= 900 ms` gap assertion derive from the same function. The `codex_forwarding` children run with 1 s / 100 ms. | `codex_forwarding` 46.9 s → ~8 s serial |
| C1.5 | `primed_pipe` (`cli/message.rs` tests) creates its pipe with `pipe2(O_CLOEXEC)` and duplicates with `OwnedFd::try_clone`; test PTYs from `openpty` (`runtime/process.rs`, `runtime/service.rs` tests) set `FD_CLOEXEC` on both ends. | `a_drained_native_pipe_emits_full_jsonl_and_restores_its_shared_flags` 10.1 s → 0.6 s under parallel load; removes a cross-test coupling |
| C1.6 | `InputServices` gains `stage_settle: Duration` (production `STAGE_SETTLE` = 500 ms); the test `Host` uses 50 ms; `staged_delivery::SETTLE` is deleted and `assert_settled_gaps` reads the injected value; `ordinary_prompt::prompt_reservation_and_each_hook_require_a_fresh_drain_before_one_real_write` (which absorbs the 500 ms settle test per § C5) runs its host with the production `STAGE_SETTLE`. `STAGE_WRITE_MARGIN` and attempt deadlines stay. | `runtime::input` 93 s → ~68 s serial |
| C1.7 | `static LIBTEST_DIGEST: OnceLock<String>` in `observation::native_capture::test_support` replaces the ~120 per-fixture SHA-256 reads of the 200 MB test binary (`CaptureFixture::new`, `service::test_support`, `read_native_capture_producer`); the 7 arm/reader fixtures that need no producer use a plain `Fixture`. | ~17–150 s CPU, plus a 200 MB allocation per read |
| C1.8 | `runtime/system.rs` `ProcessFixture` uses `tempfile::tempdir()` instead of a directory in the repository; its four 1 s-deadline cases use 2 s and the `PROCESS_FIXTURE_LOCK` mutex is deleted (each still asserts `elapsed < 5 s`). | No NFS writes from the suite; no serialized lock queue |
| C1.9 | `doctor_summary` scenario tests call `gather`/`render` (A4) in-process with fake `SpawnProbe`/`HermesConfig`; one `--json` child re-exec per binary-level smoke scenario remains (healthy, Hermes absent). | 58 child re-execs → ≤ 4 |
| C1.10 | `pane_lock::test_support::LockFixture::hold_for(None)` holds the guard on the test thread instead of a 3 s sleeper thread; `db.rs`'s inline 1 s completion grace becomes a named `const COMPLETION_GRACE` read by its test. | ~3 s |
| C1.11 | The `//:test` task caps libtest threads at 32 unless `RUST_TEST_THREADS` is already set (command below). | Removes the 5–7 deadline flakes at 64 threads and ~470 s of system CPU; wall unchanged |

C1.11's task definition; positional arguments still append to `cargo test`:

```toml
[tasks.test]
run = "RUST_TEST_THREADS=${RUST_TEST_THREADS:-$(( $(nproc) < 32 ? $(nproc) : 32 ))} cargo test --bin cafleet"
```

#### C2. Shortened real-time waits (test kept, budget shrunk)

| Test | Today | Change |
|---|---|---|
| `runtime::observation::native_capture::exporter::tests::runner_collector_encode_and_prepermit_manifest_stalls_return_unfinished_and_revoke_late_publication` | 9.45 s | Scale its 3 s / 5 s / 2 s budgets to 300 / 500 / 200 ms |
| `delivery::tests::wrapped_send_is_submitted_and_stamped_when_another_writer_takes_the_released_lock` | 5.1 s | 5 s `until` → 1 s |
| `runtime::input::tests::ordinary_prompt::deferred_queue::queue_head_under_an_observer_outage_settles_within_the_observer_budget` | ≥ 5 s | Backdate `attempt_deadline` as `queue_head_acquiring_the_pane_lock_too_late_for_its_settles_writes_zero_bytes` does |
| `runtime::input_client::tests::external_input_accepted_reply_succeeds_when_another_writer_takes_the_released_lock` | 4.1 s | 4 s `until` → 1 s |
| `runtime::writer::tests::outbox_total_capacity_includes_frame_overhead_and_rejects_before_append` | 3.9 s | Fill through one bulk append |
| `pane_lock::tests::pane_lock_wait_tests::a_lock_still_busy_at_the_reserved_deadline_returns_none` | 3.0 s | C1.10 |
| `runtime::process::tests::runner_real_shutdown_kills_and_reaps_a_term_ignoring_owned_child_within_the_shared_deadline` and `…closes_a_slave_retaining_descendant…` | 2 s TERM grace each | Pass `ShutdownClock::new(now).clipped(now + 300ms)` |
| `runtime::press::tests::press_entry_rechecks_full_placement_and_original_expiry_after_actual_metadata` (`ExpireMetadata`) | ~2 s | 300 ms deadline for that iteration |
| `runtime::input::tests::ordinary_prompt::checkpoint_hook_error_precedes_expiry_and_blocks_before_write` | ≥ 2 s | 150 ms deadline |
| `runtime::input::tests::ordinary_prompt::deferred_queue::queue_timer_retries_a_busy_pane_lock_without_child_output_and_delivers_once_free` | 1 s hold | 300 ms hold |
| `cli::fleet::tests::recovery::*` outer-scope `LONG_SQL` cases | ~3 s | Keep the unwind case only; reuse `runtime::lifecycle::test_support::SLOW_SUM` |

#### C3. Accepted long tests

Exactly one test keeps a real wait longer than 3 s, because the wait is its asserted behavior: `runtime::input::tests::ordinary_prompt::deferred_queue::queue_attempt_deadline_with_a_busy_pane_lock_settles_not_submitted_and_publishes_once` (≥ 5 s, the one real `DEFERRED_ATTEMPT_BUDGET` assertion). Every other real-time wait the change keeps — `deferred_queue::accepted_preview_stamp_failure_is_retried_and_published_once_without_retyping` (outlasts the 1 s `DEFERRED_FINALIZATION`), the `GATE_FINALIZATION_RESERVE` tests `gated::gated_stage_waits_while_the_previous_focus_persists_until_the_deadline`, `gated::gated_continuation_with_a_reason_never_satisfies_the_screen_gate`, `gated::gated_continuation_binds_only_by_succession_in_the_same_episode`, `gated::gated_inspection_of_a_continuation_offers_no_token_and_keeps_the_episode` and `gated::gated_first_stage_is_never_written_while_its_focus_gate_is_unmet`, the production-value settle test (C1.6), the `coding_agent::tests` kill-and-reap probe of the bounded runner, and `cli::message::tests::message_wait_repeats_missing_run_causes_for_each_pending_attempt` (`--timeout 2`) — stays under 3 s.

#### C4. Pruning rules

A test is deleted or merged when one of these holds; every case is recorded in § C5.

| Rule | Action |
|---|---|
| Its assertions are a subset of another test's, or it drives the same production branch with a different literal | Delete; the ledger names the covering test |
| It re-tests a decision owned by another file (storage verdicts re-checked through runtime layers, validators re-checked through the CLI) | Keep one wiring test per call site; delete the rest |
| A loop's extra cases hit the same branch | Trim to one case per branch |
| It asserts embedded data text, keys, counts, ids or layout (`bundled.json` answer bytes, schema table lists, skill ids), the harness's own code, kernel behavior, or a private SQL access pattern | Rewrite against the production API or delete (`.claude/rules/code-quality.md` § Test placement) |
| It guards a removed flag by enumeration | Keep one parser-default unknown-option case per subcommand family with a neutral name (`--unknown`); delete the rest |
| It lives in the wrong file | Move to the file that owns the asserted behavior |

Coverage that pruning adds: `detector::observe`'s `parts.dedup()` path and the non-interaction conflict → `Unknown`; `substitute_spawn_placeholders`' nested-`{` branch; a `permission_preflight` CLI test; the two cross-field rules of the `events::validate` effective-policy reader; `config::require_absolute` in its own file; a direct `digest::canonical_json` test; the synthetic `kind: "synthetic"` provenance rejection; `PaneSettlementHost::registered_process_running`'s `/proc` start-time comparison; a deferred-Ready case in `inputs.rs` `runtime_permission_shared_unresolved_limit_preserves_retained_receipts`.

The `native_capture` harness stays in the default test build (design 0000198); only its lowest-value self-tests go (the four driver prompt-text tests, `capture_ignored_entry_requires_explicit_configuration_before_launching` and the duplicate placeholder-error test).

#### C5. Pruning ledger

Each row names the action for one test; the file sections follow source path order, and Kept tests, Totals, Helper consolidation and Rule-violation fixes follow them.

- **Columns.** The Test column is the module path below the file's `mod tests` (tests in `runtime/observation.rs` `native_capture` sit outside `mod tests` and are given from the file root). Covering and destination tests are written `file::module::test_name`, with the file relative to `src/`; "new" marks a destination test this change creates.
- **Actions.** `delete`: removed, its behavior covered by the named surviving test. `merge`: folded into the named destination, with the moved assertion or case in Note. `move`: relocated unchanged to the named file. `trim`: kept, with the cases, assertions or waits cut as Note states. `speed`: kept, only its wait, budget or fixture shrinks.
- **Removed behavior.** Tests of behavior that Workstream D or B2 removes are deleted or rewritten with that behavior and have no ledger row: § D6 Removed tests defines the Workstream D set and § B2 the version-gating set. The default-URL and verbatim-URL facts of the deleted `Settings` tests move to D5's `--database-url` tests. A test rewritten with its behavior under a new name is listed in § Renamed rewrites.
- **Scope.** Each row is a decided action from the high- and medium-confidence test-audit findings; lower-confidence findings are out of this change.

##### src/assets.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `installation_failure_records_only_completed_installations` | delete | `assets.rs::opencode_preset_failure_fails_installation_and_records_nothing` | Same preset-loop failure with zero recorded rows. |
| `opencode_extension_failure_fails_installation_and_records_nothing` | trim | — | Derive the expected path from `embedded_extension(OPENCODE_EXTENSION_PREFIX)[0].0` instead of the literal `plugins/cafleet.ts`. |
| `opencode_default_installation_uses_the_native_config_directory_for_every_asset` | trim | — | Keep the `.config/opencode` identity and the presets under it. Drop the SKILLS loop, the output-line block and the embedded extension-list assertion. |
| `opencode_rerun_replaces_a_tampered_extension_and_record` | delete | `assets.rs::installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths` | A tampered file and a symlink take the same `remove_existing_target` arm. |
| `hermes::hermes_paths_use_the_home_skills_dir_without_presets` | delete | `assets.rs::installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths` | Asserted private fields. The relative-path error lives in the new `config.rs::require_absolute_rejects_non_absolute_values`. |
| `hermes::hermes_setup_sets_gateway_injection_unless_it_is_exactly_true` | trim | — | 4 → 2 cases: keep `false` and `"true"`. |
| `hermes::hermes_enablement_is_idempotent` | delete | `assets.rs::hermes::hermes_setup_with_the_plugin_already_enabled_only_reads` | First-run writes stay in `hermes_setup_writes_serially_then_rereads_plugins_once_each_with_its_own_budget`. |
| `coding_agent/hermes.rs::cli::get_reads_json_and_treats_only_the_exact_not_set_line_under_exit_one_as_absent` | trim | — | 9 → 3 cases: positive with surrounding noise, mismatched line under exit 1, exact line under exit 2. Was `hermes::hermes_setup_treats_only_an_exact_not_set_line_as_an_absent_key`; the not-set rule moved to `coding_agent/hermes.rs` with A1. |
| `hermes::hermes_setup_refuses_a_plugins_disabled_entry_for_cafleet` | trim | — | 2 → 1 case: keep `None`. |
| `coding_agent/hermes.rs::plugin_lists_must_be_arrays_of_strings_and_plugins_an_object` | trim | — | 8 → 2 cases: `(DISABLED_KEY, "cafleet")` and `(ENABLED_KEY, ["cafleet", null])`. Was `hermes::hermes_setup_fails_for_plugin_lists_that_are_not_arrays_of_strings`; plugin-shape validation moved to `coding_agent/hermes.rs` with A1. |
| `hermes::hermes_setup_fails_with_the_cause_of_a_failed_plugin_command` | trim | — | Drop the `get(DISABLED)` exit-3 scenario. |
| `hermes::hermes_setup_fails_into_the_resolved_base_when_the_config_path_is_not_absolute` | trim | — | 2 → 1 printed path: keep `"config.yaml"`. |

##### src/broker/fleets.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `registration_is_atomic_at_every_stage_and_preserves_identity` | trim | — | 4 → 1 trigger: keep `BEFORE UPDATE OF director_member_id ON fleets`. |
| `cli/fleet.rs::deletion_rows::fleet_deletion_is_atomic_soft_idempotent_and_preserves_messages` | trim | — | 3 → 1 trigger: keep `BEFORE DELETE ON member_placements`. Move the idempotency, message-retention and `999` asserts out of the loop. Was `deletion_is_atomic_soft_idempotent_and_preserves_messages`; moved to `cli/fleet.rs` by a § Rule-violation fixes entry. |

##### src/broker/members.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `registration_and_placement_updates_preserve_required_fields_and_valid_absence` | trim | — | Drop the asserts on the `#[cfg(test)]` helper `update_placement_pane_id`. |
| `live_fleet_and_director_checks_guard_member_registration` | trim | — | Keep "unknown fleet" and "root Director cannot deregister". The four damage cases move to a new direct `require_live_fleet` test in `broker/fleets.rs`. |
| `idle_clamps_future_activity_to_zero` | trim | — | One case per column → one case. |
| `idle_is_absent_without_activity_and_refuses_a_corrupt_latest_timestamp` | trim | — | 2 invalid cases → 1, which B15 turns into the `InvalidStoredValue` refusal. Was `idle_is_absent_when_the_latest_timestamp_is_absent_or_invalid`. |

##### src/broker/messaging.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `listen_snapshot::broker_snapshots_exclude_acks_committed_before_the_snapshot` | merge | `broker/messaging.rs::listen_snapshot::broker_snapshots_are_read_only_oldest_first_and_ignore_preview_metadata` | Add one acked message to the destination fixture and assert it is absent. |
| `an_ordinary_subject_retries_ordinary_members_and_preserves_the_directors_inbox` | delete | `broker/messaging.rs::retry_eligibility_follows_the_fleets_director_back_reference` | Subject exclusion stays in `the_subjects_own_previews_are_not_listed`. |

##### src/broker/runtime/events.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `effective_policy_writes::redacted_run_records_the_post_policy_displayed_mode_or_null` | delete | `broker/runtime/events.rs::effective_policy_writes::review_state_changes_alternate_between_event_carried_and_available_writes` | The `None` path stays in `available_update_writes_a_null_event_id_only_when_review_or_mode_changes`. |
| `effective_policy_writes::available_update_commits_only_for_a_running_or_degraded_run` | trim | — | 9 statuses → 5: `Starting`, `Stopping`, `Exited`, `Running`, `Degraded`. |
| `interaction_refresh::runtime_refresh_barrier_must_resolve_the_current_committed_maximum` | trim | — | `[-1, 0, 2, 256]` → `[0, 2]` on one hoisted fixture. |
| `interaction_refresh::runtime_refresh_closed_or_superseded_episode_preserves_its_closure_evidence` | trim | — | 3 close reasons → 1. |
| `interaction_refresh::runtime_refresh_targets_the_matching_open_run_and_episode_only` | trim | — | Keep `run` and `absent` on one hoisted fixture. |
| `interaction_refresh::runtime_refresh_validates_evidence_sequence_digest_policy_backend_and_native_version` | trim | — | 7 → 5 cases: drop `shape` and `raw`; hoist the fixture. |
| `interaction_refresh::runtime_refresh_validates_every_current_owner_identity_component` | delete | `broker/runtime/mod.rs::stale_owner_kind_epoch_or_database_identity_fences_every_mutation` | Refresh's use of the fence stays in `runtime_refresh_takeover_fences_the_previous_owner`. |
| `interaction_refresh::runtime_refresh_database_contention_obeys_the_absolute_deadline_and_restores_reuse` | delete | `runtime/lifecycle.rs::runner_database_deadline_bounds_a_locked_write_and_restores_the_busy_timeout` | Saves ~80 ms. `runtime_refresh_expired_deadline_has_zero_effect_and_no_fresh_budget` keeps the refresh-deadline wiring. |
| `interaction_refresh::runtime_refresh_post_policy_redacted_and_sensitive_updates_remain_secret_free` | trim | — | Keep `sensitive=false` only. |
| `versioned_events::runtime_payload_v2_other_interaction_classes_have_explicit_null_action` | trim | — | 3 classes → 1. |
| `versioned_events::runtime_payload_v2_reader_rejects_a_forged_question_action` | trim | — | 9 → 1 case: keep `permission-instruction`. |
| `versioned_events::runtime_payload_v2_lifecycle_events_have_null_action_and_preserve_typed_causes` | trim | — | Keep `RunnerDegraded` and `RunnerLost`. |
| `versioned_events::runtime_payload_v2_reader_accepts_an_independent_valid_action` | delete | `broker/runtime/events.rs::versioned_events::runtime_payload_v2_reader_rejects_forged_action_and_matching_message` | Its `independent_v2` helper already expects readability. |
| `versioned_events::runtime_payload_v2_reader_rejects_forged_action_and_matching_message` | trim | — | 9 → 2 cases: `instruction` and `missing` on one hoisted fixture. |
| `versioned_events::runtime_payload_v2_final_payload_updates_and_commit_are_atomic` | delete | `broker/runtime/events.rs::failure_at_each_transaction_stage_rolls_back_opening_and_inbox` | Its retry-after-removal step moves to the covering test. |
| `versioned_events::runtime_payload_v2_reader_rejects_an_instruction_that_disagrees_with_the_row_count` | trim | — | 2 → 1 case: rows=1 with the multi-row sentence. |
| `versioned_events::runtime_payload_v2_ack_consumes_only_the_notification` | delete | `broker/runtime/events.rs::ack_consumes_the_message_without_closing_the_observed_interaction` | Same ack and the same open-interaction assertions. |
| `automatic_review_events::automatic_refusal_metadata_evidence_carries_no_screen_text` | delete | `broker/runtime/evidence.rs::metadata_removes_all_screen_fields_and_hashes_only_post_policy_evidence` | The instruction stays in `automatic_refusal_event_routes_a_review_action_to_the_director_inbox`. |
| `automatic_review_events::automatic_review_unavailable_keeps_the_constant_instruction_under_redaction` | delete | `broker/runtime/events.rs::automatic_review_events::automatic_review_unavailable_event_routes_its_supervision_action` | Redaction cannot change the unavailable instruction. |
| `automatic_review_events::automatic_review_event_reader_rejects_a_forged_action` | trim | — | 5 forges + null per kind → `other_kind` + null. |
| `automatic_review_events::automatic_review_events_reject_an_interaction_or_lifecycle_cause` | trim | — | Both kinds → 1 kind. |
| `automatic_review_events::automatic_review_event_replay_is_idempotent_and_keeps_its_message` | delete | `broker/runtime/events.rs::effective_policy_writes::unavailable_event_write_replays_idempotently_with_its_event` | Add `assert!(!inserted)` to `identical_observed_transition_is_idempotent_without_generated_ids_or_commit_time` by switching its second call to `persist_transition`. |
| `automatic_review_events::automatic_review_event_recovery_for_an_inactive_member_archives_with_its_action` | trim | — | Both kinds → 1 kind. |
| `each_conflicting_original_transition_field_rejects_sequence_reuse` | trim | — | Keep `time`, `episode`, `kind`, `classification` on one hoisted fixture. |
| `failure_at_each_transaction_stage_rolls_back_opening_and_inbox` | trim | — | Keep the `member_interactions` insert, `messages.text` update and `commit` targets. Receives the retry step of the deleted v2 atomicity test. |
| `refusal_without_native_correlation_and_run_exit_keep_nullable_episode_links` | delete | `broker/runtime/events.rs::versioned_events::runtime_payload_v2_obscurity_closure_and_refusal_have_null_action` | RunnerExited stays in `runtime_payload_v2_lifecycle_events_have_null_action_and_preserve_typed_causes`. |
| `transition_outside_reserved_sequences_or_with_invalid_payload_is_rejected_atomically` | trim | — | Keep `unreserved` and `missing-evidence` on one hoisted fixture. |
| `terminal_live_write_regression_keeps_committed_retries_and_rejects_new_observations` | trim | — | Keep `exited` and drop the status `UPDATE`. |
| `event_shape_regression_requires_a_typed_lifecycle_cause_before_commit` | trim | — | 16 → 3 cases: `CoverageUnknown` × causes `None`, `{}`, invented kind on one hoisted fixture. |
| `event_shape_regression_rejects_lifecycle_causes_on_screen_transitions` | trim | — | Keep `PermissionDeniedObserved` only. |
| `event_shape_regression_rejects_missing_or_malformed_persisted_exit_causes` | trim | — | Keep `None` and string `exit_code`; hoist the fixture. |
| `event_shape_regression_binds_public_native_request_id_to_post_policy_evidence` | trim | — | Keep the redacted (non-null id) case. |

##### src/broker/runtime/evidence.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `evidence_policy_rejects_a_row_count_that_disagrees_with_the_operation` | trim | — | 6 → 3 cases: `(op, null)`, `(null, 1)`, `(op, 0)`. |
| `evidence_validation_binds_operation_rows_to_the_operation` | trim | — | 6 → missing, zero and count-without-operation, plus the forged metadata case. |
| `metadata_and_redacted_transitions_remain_secret_free_through_storage_and_updates` | delete | `broker/runtime/events.rs::versioned_events::runtime_payload_v2_metadata_and_redacted_actions_preserve_secret_free_evidence` | The policy half stays in `matching_redaction_replaces_each_normalized_field_in_full`. |
| `evidence_utf8_and_complete_payload_bounds_record_truncation_after_policy` | trim | — | Drop the persist-and-16-KiB storage half (covered by `runtime_payload_v2_large_utf8_evidence_and_action_fit_the_complete_payload_bound`). Assert no ESC on `transformed`. |

##### src/broker/runtime/inputs.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `operation_key_rejects_a_changed_digest_kind_or_message` | speed | — | Hoist one fixture and drop the unused per-case message. |
| `preview_receipt_requires_a_message_addressed_to_its_run_member` | trim | — | Drop `foreign` (same conjunct as `director`); hoist the fixture. |
| `settled_receipt_history_can_exceed_the_unresolved_cache_bound` | speed | — | Bulk-insert 4096 settled rows with the recursive CTE, then one production reservation. Removes 8194 owned transactions. |
| `uncertain_cause::record_input_uncertain_stores_every_cause_value` | speed | — | One fixture holding 8 receipts instead of 8 fixtures. |
| `uncertain_cause::uncertain_cause_survives_each_reconcile_disposition` | delete | `broker/runtime/inputs.rs::uncertain_cause::unpublished_delivery_results_return_the_receipt_cause` | Cited cover `..._reconciled_out_of_uncertainty` merges into this destination. |
| `uncertain_cause::receipts_that_never_became_uncertain_keep_a_null_cause` | delete | `broker/runtime/inputs.rs::uncertain_cause::record_input_uncertain_persists_its_cause_across_reconnection` | Accept and settle never write the column. |
| `uncertain_cause::stored_cause_on_a_state_without_uncertainty_is_rejected_on_read` | trim | — | Keep `in_progress` only. |
| `uncertain_cause::unknown_stored_cause_is_rejected_on_read` | delete | `broker/runtime/inputs.rs::failed_input_reservation_and_corrupt_receipt_reads_surface_the_original_state` | Same `stored_enum` decode failure. |
| `uncertain_cause::unpublished_delivery_results_report_no_cause_for_a_receipt_reconciled_out_of_uncertainty` | merge | `broker/runtime/inputs.rs::uncertain_cause::unpublished_delivery_results_return_the_receipt_cause` | Its reconciled receipts join the destination's single fixture. |
| `uncertain_cause::unpublished_delivery_results_report_no_cause_for_a_causeless_uncertain_receipt` | merge | `broker/runtime/inputs.rs::uncertain_cause::unpublished_delivery_results_return_the_receipt_cause` | Its causeless in-progress receipt joins the same fixture. |
| `runtime_choice_response_reserves_against_an_open_question_and_stores_only_its_choice` | trim | — | 3 choices → 1, plus the decision tail. |
| `runtime_permission_digest_covers_the_choice_id_with_null_for_the_absent_answer` | merge | `broker/runtime/inputs.rs::runtime_permission_digest_matches_independent_sorted_metadata_and_changes_per_field` | `Choice("decline")` and `Choice("not-now")` become per-field cases. |
| `runtime_permission_metadata_refuses_a_choice_outside_the_choice_id_shape` | trim | — | Check through `permission_response_digest` without a DB; choice table → `"Decline"` only. The shape table is owned by `coding_agent/catalog.rs::question_answers::a_choice_id_follows_the_lowercase_hyphenated_shape_up_to_64_bytes`. |
| `runtime_permission_answer_kind_must_match_the_opened_interaction_class` | trim | — | 8 → 2 cases: `decision_on_question`, `choice_on_permission`. |
| `runtime_choice_capability_binding_requires_the_requested_choice` | delete | `broker/runtime/inputs.rs::runtime_permission_capability_binding_is_required_only_for_a_fresh_attempt` | All three cases fail the `decision` case's key check. |
| `runtime_permission_response_rejects_an_automatic_review_event_before_any_write` | trim | — | 2 kinds → 1. |
| `runtime_permission_historical_associations_are_checked_with_a_recomputed_valid_digest` | trim | — | Drop `event`; build `another_run` only for the `run` case. |
| `runtime_permission_expired_reservation_has_zero_effect` | trim | — | Keep `expired`; drop the 120 ms `blocked` wait (owned by `runtime/lifecycle.rs`). Was `runtime_permission_expired_and_blocked_reservations_share_the_caller_deadline`. |
| `runtime_permission_user_decisions_roundtrip_each_attributed_mode_and_decision` | trim | — | 6 → 2 cases: (Manual, Deny) and (Unknown, Allow). |
| `runtime_permission_invalid_identity_token_mode_and_required_metadata_roll_back` | trim | — | Check pure cases through `permission_response_digest` on one fixture; tokens → short and uppercase. Keep one `assert_rejected_unchanged`. |
| `runtime_permission_reference_validation_uses_utf8_bytes_and_rejects_controls` | trim | — | 4 references on one field + 1 on the other, through `permission_response_digest`. Keep one reservation-boundary check. |
| `runtime_permission_new_reservation_requires_matching_owner_and_live_relationships` | trim | — | Drop `kind`, `director`, `fleet`; hoist the fixture for the in-memory cases. |
| `runtime_permission_new_reservation_requires_the_open_inbox_permission_event` | trim | — | Drop `message` and `wrong-class`. |
| `runtime_permission_capability_binding_is_required_only_for_a_fresh_attempt` | speed | — | Hoist one fixture (6 → 1 DB). |
| `runtime_permission_episode_claim_blocks_a_different_key_in_every_possible_write_state` | trim | — | 7 → 2 states: `in-progress`, `not-submitted`. |
| `runtime_permission_possible_write_states_keep_claim_even_with_matching_proof` | trim | — | 5 → 2 states: `in-progress`, `submitted`. |
| `runtime_permission_release_rejects_foreign_and_stale_borrowed_proofs` | speed | — | Hoist one fixture. |
| `runtime_permission_readers_reject_missing_foreign_or_corrupted_audit_claim_data` | trim | — | Drop `token`, `director` and `reference`. |

##### src/broker/runtime/mod.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `run_creation_validates_member_fleet_and_pinned_director_before_allocating` | speed | — | Hoist the fixture and `register("new-worker")` out of the loop. |
| `typed_runtime_readers_fail_on_corrupt_required_values` | trim | — | 6 → 3 cases: `status`, `database_identity_json`, `owner_start_identity`. |
| `stale_owner_kind_epoch_or_database_identity_fences_every_mutation` | speed | — | Hoist the fixture out of the loop. |

##### src/cli/fleet.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `fleet_create_accepts_every_registered_backend_including_hermes` | trim | — | Drop the explicit `hermes` parse; the `CodingAgent::all()` loop already parses it. |
| `fleet::runner_review_fleet_changed_owner_on_the_same_pane_uses_only_the_fresh_snapshot` | delete | `cli/fleet.rs::fleet::runner_review_fleet_changed_run_on_the_same_pane_preserves_the_new_run_identity` | One relock branch: `stable_run_identity` covers both epoch and run id. |
| `fleet::runner_review_fleet_mutable_shutdown_evidence_keeps_one_deadline_and_run_less_teardown` | trim | — | Keep `already_complete = true` only. |
| `recovery::runner_recovery_deletion_unwind_restores_the_still_live_outer_scope` | trim | — | Keep the `unwind` exit; replace `LONG_SQL` with `runtime::lifecycle::test_support::SLOW_SUM` (~2 s). Success/error restoration stays in lifecycle `runner_database_deadline_review_inner_success_restores_the_live_outer_scope`. Was `recovery::runner_recovery_deletion_success_error_and_unwind_restore_the_still_live_outer_scope`. |
| `pane_locks::deletion_deduplicates_physical_panes_and_revalidates_after_all_locks` | move | `pane_lock.rs` `mod tests` | Calls `pane_lock::lock_deletion` directly; build targets with `LockFixture`. |
| `pane_locks::deletion_acquires_physical_keys_in_sorted_order` | move | `pane_lock.rs` `mod tests` | Same owner as above. |

##### src/cli/member.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `recovery_deletion::runner_recovery_member_deletion_foreign_placement_after_close_preserves_registration` | delete | `cli/fleet.rs::fleet::runner_review_fleet_final_identity_change_preserves_registry_without_repeating_effects` | Same after-close `revalidate_member` branch as the `epoch` variant; drop the `"placement"` arm. |
| `member_prompt_run_less_member_fails_before_pane_io` | delete | `cli/member.rs::member_exec_run_less_member_fails_before_pane_io` | Same `require_runtime` error; −1 subprocess. |
| `external_input_member_prompt_exec_route_raw_commands_and_print_actual_durable_ids` | trim | — | 4 → 2 invocations: `(prompt, json)` and `(exec, text)`. |
| `launch_policy::runtime_policy_parser_accepts_the_backend_qualified_launch_selection` | delete | `coding_agent/mod.rs::runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode` | Cannot fail: `--permission-mode` is a free string at parse time. |
| `launch_policy::runtime_policy_parser_accepts_explicit_metadata_evidence` | merge | `cli/member.rs::launch_policy::runtime_policy_parser_keeps_omitted_selection_and_model_effort_boundaries` | The destination asserts the parsed `event_evidence`. |
| `launch_policy::runtime_policy_parser_accepts_redacted_evidence_with_its_absolute_file` | merge | `cli/member.rs::launch_policy::runtime_policy_parser_keeps_omitted_selection_and_model_effort_boundaries` | `--event-evidence redacted --event-redaction-file /abs` and both parsed fields move. |
| `launch_policy::runtime_policy_invalid_launch_selection_has_usage_diagnostics_before_creation_effects` | trim | — | `CodingAgent::all()` loop → one backend. |
| `launch_policy::hermes_invalid_effort_and_policy_fail_with_usage_errors_before_creation_effects` | delete | `coding_agent/mod.rs::model_and_effort_validation_preserves_backend_rules` | The policy message stays pinned by `runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error` |
| `launch_policy::runtime_policy_invalid_redaction_configuration_fails_before_creation_effects` | trim | — | Keep `not json`, `[0xff]` and one policy case. |
| `launch_policy::runtime_policy_redaction_options_form_a_validated_launch_only_group` | trim | — | 3 → 2 cases. |
| `press::press_cli_invalid_tail_is_usage_before_missing_database_validation` | move | `cli/mod.rs` `mod tests` | Asserts `dispatch` ordering. |
| `press::press_presentation_is_canonical_acknowledgment_without_effect_counts` | move | `output/json.rs` and `output/text.rs` `mod tests` | Split: JSON half to json.rs, text half to text.rs. |
| `recovery_entry::runner_recovery_entry_ordinary_parser_accepts_positive_member_ids_and_json_selection` | delete | `cli/member.rs::recovery_entry::runner_recovery_entry_full_attestation_preserves_snapshot_reference_and_json_values` | Ids 7 and `i64::MAX` are parsed with and without `--json` there. |
| `recovery_entry::runner_recovery_entry_member_ids_require_positive_assigned_integers` | trim | — | Keep `0` plus one non-integer. |
| `recovery_entry::runner_recovery_entry_full_attestation_preserves_snapshot_reference_and_json_values` | trim | — | 8 → 2 iterations; destructure `MemberCommand::Recover` instead of raw `ArgMatches` and drop `argument_id`. |
| `recovery_entry::runner_recovery_entry_attestation_snapshot_ids_require_positive_integers` | trim | — | Keep `0` plus one non-integer per argument. |
| `recovery_entry::runner_recovery_entry_attestation_rejects_blank_references_and_missing_option_values` | trim | — | 3 blanks → 1. |
| `substitutes_all_four_placeholders` | merge | new `cli/member.rs::spawn_placeholders_substitute_and_unescape` | Becomes the four-placeholder row. |
| `substitutes_repeated_placeholders` | delete | new `cli/member.rs::spawn_placeholders_substitute_and_unescape` | Adds nothing over the four-placeholder and compose rows. |
| `body_without_placeholders_is_returned_verbatim` | merge | new `cli/member.rs::spawn_placeholders_substitute_and_unescape` | Becomes the plain-text row. |
| `doubled_braces_render_literal_braces` | merge | new `cli/member.rs::spawn_placeholders_substitute_and_unescape` | First case becomes the `{{`/`}}` row; the second case is dropped. |
| `doubled_braces_compose_with_substitution` | merge | new `cli/member.rs::spawn_placeholders_substitute_and_unescape` | Becomes the escape-with-substitution row. |
| `unknown_placeholder_is_a_usage_error_with_the_pinned_message` | merge | new `cli/member.rs::spawn_placeholder_errors_are_usage_errors` | Pinned `{foo}` row. The table also adds the untested nested `{a{b}` row. |
| `stray_open_brace_is_a_malformed_usage_error` | delete | new `cli/member.rs::spawn_placeholder_errors_are_usage_errors` | Same `expected '}'` branch as the unclosed row. |
| `stray_close_brace_is_a_malformed_usage_error` | merge | new `cli/member.rs::spawn_placeholder_errors_are_usage_errors` | Stray `}` row. |
| `unclosed_placeholder_is_a_malformed_usage_error` | merge | new `cli/member.rs::spawn_placeholder_errors_are_usage_errors` | Unclosed `{fleet_id` row. |
| `snapshot_treats_crlf_row_terminators_as_line_endings` | delete | `cli/member.rs::snapshot_strips_csi_and_cr_rewrites_before_hashing_utf8` | CRLF handling is owned by `output::strip_ansi` tests. |

##### src/cli/message.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `message_wait_repeats_missing_run_causes_for_each_pending_attempt` | trim | — | Two `isolated_cli` runs → one; keep `--timeout 2` (~2 s). |
| `send_delivery_reports_a_wrapped_self_send_and_the_queued_preview_receipt` | trim | — | Drop the self-send half (covered by `send_delivery_reports_self_sends_the_root_director_and_run_less_recipients`). Receives the held-recipient case. |
| `a_send_whose_recipient_is_deregistered_after_the_commit_reports_it_held` | merge | `cli/message.rs::send_delivery_reports_a_wrapped_self_send_and_the_queued_preview_receipt` | Its `notification_held` case joins the destination's in-process table. |
| `a_snapshot_budget_failure_terminates_the_stream_instead_of_becoming_a_timeout` | merge | `cli/message.rs::startup_validation_errors_precede_ready_and_midstream_errors_preserve_complete_records` | The `fail_at = 2` midstream error becomes `DeadlineExceeded`; assert the match and no `closed` record. |
| `output_failure_stops_before_another_snapshot_and_retains_only_flushed_records` | trim | — | `fail_at` cases → `[0, 1]`. |
| `an_ack_output_race_preserves_the_snapshot_record` | trim | — | Drop the restart half. Was `an_ack_output_race_preserves_the_snapshot_record_and_a_restart_replays_only_pending_ids`. |
| `real_pipe_backpressure_and_broken_consumers_restore_flags_and_preserve_pending_rows` | trim | — | Hoist one fixture and one ~3 MB message; 4 → 2 iterations: `(false, true)`, `(true, false)`. |
| `codex_forwarding::a_failed_codex_queue_exits_one_after_forwarding_the_closed_notice` | merge | `cli/message.rs::codex_forwarding::a_non_full_queue_failure_after_a_refusal_fails_without_another_wait` | `assert_stdout_has_no_forwarded_records` and the `thread … not found` notice check move. |
| `codex_forwarding::a_codex_queue_past_its_deadline_is_killed_and_the_closed_notice_names_the_deadline` | speed | — | Add a `#[cfg(test)]` override for `CODEX_QUEUE_BUDGET` (like `full_queue_limit()`) and run with 1 s instead of 10 s. |
| `codex_forwarding::full_queue_refusals_retry_the_same_record_each_second_until_accepted_and_later_records_keep_their_order` | speed | — | 3 → 2 refusals at C1.4's 100 ms retry interval and `--timeout 8` → `1` (< 1 s). |
| `codex_forwarding::a_codex_queue_past_its_deadline_after_the_pinned_session_ended_closes_the_listener_with_session_exited` | delete | `cli/message.rs::codex_forwarding::a_codex_queue_failing_after_the_pinned_session_ended_closes_the_listener_with_session_exited` | Same `forward_record` match arm; saves ≥10 s. |
| `codex_forwarding::a_listener_keeps_running_while_its_codex_session_lives` | delete | `cli/message.rs::codex_forwarding::forwarding_queues_each_message_then_the_closing_record_and_stdout_keeps_only_ready_and_closed` | Removes a fixed 3 s sleep and a subprocess. |

##### src/cli/mod.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes` | trim | — | 36 dispatches → `PRAGMA application_id=42` × one command per `invocation_connection` call site. The sites are `fleet list --json`, `member press 1 Enter` and `permission inspect …`. |
| `command_options_are_scoped_to_their_subcommands` | trim | — | Keep the `--json fleet list` case and the positive parses; delete the 24 undefined-option sentinel cases; keep at most one neutral parser-default unknown-option case per subcommand family (§ C4). |
| `member_exec_help_describes_runner_delivery_and_orders_member_before_command` | delete | `cli/mod.rs::member_and_message_body_sources_are_exclusive` | Asserts doc-comment words and the literal `override_usage`. |
| `help_describes_command_groups_and_derives_choices_from_their_registries` | trim | — | Compare against the enums' `VALUES`, render help once, drop the disposition/event_evidence literals and the invalid-usage block. Keep the `CodingAgent::all()` check. |
| `member_runner_internal_command_requires_its_descriptor_argument` | merge | `cli/mod.rs::member_runner_internal_command_accepts_an_absolute_descriptor_and_stays_hidden` | One missing-argument line folds in. |
| `native_command_files_strip_only_one_terminal_line_ending` | trim | — | Invalid list → `""`, `"echo x\n\n"`, `"echo x\r"`. |

##### src/cli/runtime.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `permission_entry::permission_commands_accept_positive_ids_with_json_and_reject_every_other_id` | trim | — | Becomes the wiring test: reject `0` for each of the 8 permission id fields and for `event show` / `input show`. |
| `permission_entry::permission_respond_accepts_each_semantic_decision_and_valid_source_mode_pair` | trim | — | Iterate the `VALUES` constants once instead of 8 hand-listed pairs. |
| `permission_entry::permission_respond_rejects_a_choice_outside_the_choice_id_shape` | merge | new `cli/runtime.rs::permission_entry::permission_respond_validates_each_checked_flag` | One rejected `--choice` value stays as wiring proof. |
| `permission_entry::permission_respond_requires_each_snapshot_decision_and_provenance_field` | merge | `cli/runtime.rs::permission_entry::permission_inspect_requires_its_interaction_event_and_director` | Required-field cases join; the `--decision` removal is dropped as a duplicate of the "neither" case. |
| `permission_entry::permission_response_accepts_exact_lowercase_hex_tokens` | delete | `runtime/control.rs::request_kind_payload_permission_binding_and_reference_validation` | The valid path is also proven by the `respond()` fixtures. |
| `permission_entry::permission_response_rejects_uppercase_nonhex_and_wrong_length_tokens` | merge | new `cli/runtime.rs::permission_entry::permission_respond_validates_each_checked_flag` | One invalid `--request-token` value moves. |
| `permission_entry::permission_response_references_accept_nonblank_utf8_at_exact_byte_boundaries` | delete | `runtime/control.rs::request_kind_payload_permission_binding_and_reference_validation` | Byte boundaries live with the validator. |
| `permission_entry::permission_response_references_reject_blank_oversize_and_every_control` | merge | new `cli/runtime.rs::permission_entry::permission_respond_validates_each_checked_flag` | One invalid value each for `--mode-evidence-ref` and `--decision-evidence-ref` moves. |
| `permission_entry::permission_response_operation_key_accepts_literal_nonblank_values_and_rejects_blank` | merge | new `cli/runtime.rs::permission_entry::permission_respond_validates_each_checked_flag` | The blank `--operation-key` case moves. |
| `runtime_read_commands_accept_positive_assigned_ids_and_json_selection` | merge | `cli/runtime.rs::permission_entry::permission_commands_accept_positive_ids_with_json_and_reject_every_other_id` | Its `event show` / `input show` ids become wiring rows. |
| `positive_id_accepts_only_positive_i64_values` | trim | — | Becomes the direct `positive_id` unit test: accept `1`, `i64::MAX`; reject `0`, `-1`, `unknown`, `1.0`, `i64::MAX+1`. Was `runtime_read_ids_reject_zero_negative_malformed_and_overflowing_values`. |
| `input_reconcile_requires_valid_disposition_and_nonempty_evidence` | trim | — | 8 → 5 invalid cases. |

##### src/cli/setup.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `parser_uses_enum_choices_and_supports_repeated_multi_value_selection` | trim | — | Drop the `unknown` refusal and the separate `hermes` parse. |
| `refused_wal_databases_and_sidecars_remain_byte_identical_with_a_held_reader` | trim | — | 5 → 2 mutations: `application_id=42`, `cafleet_version='0.0.0'`. |
| `setup_rejects_unknown_options_positionals_and_agent_values` | trim | — | 6 → 4 cases. Replace the removed `--skip` example with a neutral `--unknown`. |
| `healthy_database_distinguishes_absent_stale_current_and_failed_install_queries` | trim | — | Drop the opencode preset status asserts (owned by the preset-check table). |
| `opencode_preset_checks_follow_catalog_order_and_report_each_preset_state` | trim | — | 6 → 2 cases: `[true, false]`, `[false, true]`. |
| `opencode_extension_check_reports_each_plugin_and_record_state` | trim | — | 9 → 7 cases: drop "unreadable plugin" and "record names a directory". |
| `opencode_extension_check_is_unavailable_without_the_executable_or_configuration` | trim | — | Drop the final loop that recomputes `agent_checks`. |
| `hermes::plain_setup_without_hermes_on_path_installs_hermes_skills_and_skips_the_plugin` | trim | — | Drop the embedded skill-id path assertion; assert `installed_agents` and the absent plugin dir. Was `hermes::plain_setup_without_hermes_on_path_installs_hermes_skills_and_skips_the_merge`. |
| `hermes::setup_with_hermes_on_path_runs_it_against_the_resolved_home_and_enables_the_plugin` | trim | — | Keep `calls[0] == path_call()` and the `hermes` row; drop the enabled/injection/record value asserts. Was `hermes::plain_setup_with_hermes_on_path_installs_the_plugin_and_merges_the_configuration`. |
| `hermes::explicit_hermes_setup_uses_the_hermes_home_override` | delete | `assets.rs::installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths` | Env forwarding stays in `setup_records_successes_before_the_first_asset_failure`. |
| `doctor_summary::id_ranges_collapse_runs_of_three_or_more_consecutive_ids` | trim | — | 9 → 5 cases: `[]`, `[7,8]`, `[1,2,3]`, `[3,3,4,5,5]`, `[9,1,2,3,7]`. |
| `doctor_summary::doctor_groups_members_by_state_and_joins_backend_problems` | speed | — | Split into two in-process tests over `member_checks` and `backend_summary`; −4 child re-execs. |
| `doctor_summary::doctor_reports_a_failed_registry_read_and_still_prints_every_other_check` | speed | — | In-process `member_checks(&conn, failing_registry)` instead of 4 child re-execs. |
| `doctor_summary::doctor_reports_a_run_whose_diagnosis_fails_as_could_not_be_checked` | speed | — | In-process `member_checks(&conn, phantom_registry)` instead of 4 child re-execs. |
| `doctor_summary::doctor_reports_healthy_extension_rows_after_their_backend_rows` | merge | `cli/setup.rs::doctor_summary::doctor_with_only_warnings_prints_ready_backends_and_exits_zero` | The three `named_row` ok-detail asserts move; the row-order asserts are dropped (pinned in-process). |
| `doctor_summary::doctor_reports_an_unreadable_hermes_config_path_on_its_row_and_summary` | trim | — | Keep only the `config path` failure scenario (−6 re-execs). Was `doctor_summary::doctor_reports_each_hermes_extension_error_on_its_row_and_summary`. |
| `doctor_summary::doctor_reports_a_relative_hermes_config_path_with_the_member_create_cause` | delete | `cli/setup.rs::doctor_summary::doctor_reports_an_unreadable_hermes_config_path_on_its_row_and_summary` | Same `Err` arm; the cause text stays pinned by `assets.rs::hermes::hermes_setup_fails_into_the_resolved_base_when_the_config_path_is_not_absolute`. |
| `doctor_summary::doctor_reports_each_hermes_plugin_enablement_state_on_its_row_and_summary` | trim | — | 10 → 5 cases (one per doctor branch); −10 re-execs. |

##### src/coding_agent/catalog.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `panel_separator::a_region_loads_with_one_non_whitespace_panel_separator_glyph` | trim | — | One multibyte glyph `"│"` on one rule. |
| `panel_separator::a_panel_separator_that_is_not_one_non_whitespace_glyph_is_refused` | trim | — | 9 → 4 values: `1`, `""`, `" "`, `"││"`. |
| `question_answers::a_decision_answer_on_a_question_rule_is_refused` | trim | — | 2 → 1 case. |
| `question_answers::a_duplicate_question_rule_and_choice_pair_is_refused` | trim | — | 2 → 1 case. |
| `question_answers::a_choice_id_follows_the_lowercase_hyphenated_shape_up_to_64_bytes` | trim | — | Valid 3 → `"a"` and 64×`a`; invalid 7 → `"Later"`, `"-later"`, `"ask--later"`, 65×`a`. Owner test of the choice-id shape. |
| `question_answers::a_choice_answer_validates_its_stages_like_a_decision_answer` | delete | `coding_agent/catalog.rs::runtime_profile_answer_scope_focus_stages_and_fixture_relationships_are_checked` | Same answer loop as decision answers. |
| `question_answers::marker_focus_loads_and_gates_a_choice_plan` | trim | — | Drop the invalid `focus_style` loop (one value stays in the gated test). |
| `question_answers::an_interaction_frame_requires_a_reject_pattern` | trim | — | Rule loop → 1; drop the 4 frame-shape cases. |
| `question_answers::an_annotation_frame_still_requires_a_live_screen_pattern` | merge | `coding_agent/catalog.rs::annotation::annotation_catalog_enforces_annotation_rule_fields_and_the_frame_guard` | The destination's "empty frame require patterns" case asserts the exact message. |
| `bundled_backend_files::loads_multiple_screens_including_a_catalogued_regression_without_profile_citations` | delete | `coding_agent/catalog.rs::bundled_backend_files::canonical_screen_digests_accept_compact_and_pretty_backend_files` | Same compact load asserted `Ok`. |
| `bundled_backend_files::an_uncited_screen_in_an_unreferenced_file_is_rejected_after_referenced_files` | merge | `coding_agent/catalog.rs::bundled_backend_files::an_uncited_screen_in_an_unreferenced_file_is_rejected_before_referenced_files` | One test asserts both orders. |
| `bundled_guard_rejects_every_unreviewed_change_to_each_bundled_fixture` | merge | new `coding_agent/catalog.rs::bundled_backend_files::a_screen_outside_the_reviewed_shape_is_rejected` | `SCREEN_CHANGES` apply to a synthetic `NativeFixtureCatalog` (~3,440 catalog loads removed). The new test adds the `kind: "synthetic"` rejection. |
| `permission_inspection_catalog_retains_exact_executable_plans_and_provenance` | trim | — | Drop the bundled tail; add `assert!(stage.gate().is_none())` to the stage loop. |
| `runtime_profile_composer_provenance_supplies_only_its_declared_component` | delete | `coding_agent/catalog.rs::composer_observation_only_definition_preserves_observation_without_executable_plans` | Same definition, superset of the assertions. |
| `runtime_profile_both_answers_can_be_declared_without_composer_coverage` | delete | `coding_agent/catalog.rs::composer_operation_provenance_and_exact_selection_do_not_authorize_other_profiles` | Composer Unknown stays in `runtime_profile_capabilities_are_independent_and_version_selected`. |
| `runtime_profile_operation_row_margin_must_be_a_nonempty_single_row_string` | trim | — | 6 → 3 invalid values: `1`, `""`, `"\n"`. |
| `runtime_profile_operation_margin_rows_select_all_or_continuation_rows_of_a_margin` | trim | — | 6 → 2 invalid cases: `"first"` and a missing `row_margin`. |
| `runtime_profile_operation_row_trailer_is_a_trimmed_single_row_literal` | trim | — | 9 → 4 invalid values: `1`, `""`, `" █"`, `"█\n█"`. |
| `runtime_profile_bundled_loader_validates_its_packaged_declarations` | delete | `coding_agent/catalog.rs::runtime_profile_capabilities_are_independent_and_version_selected` | The bundled load runs in every bundled test. |
| `runtime_profile_private_fingerprint_supports_comparison_without_debug_or_serialization` | move | `coding_agent/detector.rs` `mod tests` | `SemanticFingerprint` lives in detector.rs; place beside `assert_no_debug_or_serialize!(AnswerMatch)`. |
| `gated_catalog::gated_catalog_keeps_existing_ungated_plans_without_gates` | delete | `coding_agent/catalog.rs::question_answers::a_choice_answer_resolves_by_its_question_rule_and_choice_key` | Bundled-plan echo; the `gate().is_none()` check moves to `permission_inspection_catalog_retains_exact_executable_plans_and_provenance`. |
| `gated_catalog::gated_catalog_requires_a_background_focus_matcher_over_line_scoped_labels` | trim | — | Invalid `focus_style` values 4 → 1. |
| `gated_catalog::every_backend_rejects_an_escape_byte_in_any_answer_stage` | trim | — | → one ungated `"0d1b"` and one gated `"1b"`. |
| `composer_submission::composer_prefix_shape_hex_and_operation_identity_are_validated` | trim | — | 22 → 3 rejects: Prompt `"2120"`, Exec `""`, non-string `17`. |
| `composer_submission::composer_submit_hex_requires_nonempty_even_lowercase_hex_and_string_type` | trim | — | 22 → 4 on one operation: `""`, `"0"`, `"ABCDEF"`, `null`. |
| `composer_submission::composer_optional_submission_and_operation_objects_have_exact_complete_fields` | trim | — | ~30 → ~10 rejects on the generic `object()` branch. |
| `composer_submission::composer_each_operation_requires_its_own_valid_fixture_reference_set` | trim | — | 22 → `[]`, `[" "]`, unknown, duplicate, `null` and the backend/version mismatches on one operation. Keep one case for the other operation. |
| `composer_submission::composer_rule_references_keep_component_identity_and_declared_order` | trim | — | Run the inner reject loop once instead of per operation. |
| `bundled_claude::bundled_guard_rejects_a_restored_masked_status_row_of_the_idle_composer` | merge | new `coding_agent/catalog.rs::bundled_backend_files::a_screen_outside_the_reviewed_shape_is_rejected` | Becomes the `reviewed_privacy` masked-row variant; the hard-coded row 33 text goes. |
| `bundled_claude::bundled_claude_resize_between_wrapped_and_unwrapped_renderings_replaces_the_episode` | delete | `runtime/observation.rs::runtime_episode_class_and_stable_choice_changes_supersede_after_stability` | `runtime/observation.rs::runtime_episode_stable_replacement_closes_old_episode_before_opening_the_next` merges into this cover (its row below). Rewrap identity stays in detector `bundled_claude_wrapped_identity_follows_the_displayed_rows_so_a_rewrap_replaces_it`. |
| `bundled_claude_auto::bundled_claude_auto_annotations_leave_every_primary_observation_unchanged` | delete | `coding_agent/detector.rs::annotation::annotation_detector_leaves_the_primary_observation_unchanged` | Also removes `without_annotation_rules` and its direct `include_str!`. |
| `bundled_claude_auto::bundled_claude_auto_ask_form_answers_allow_with_1_and_deny_with_2_ungated` | trim | — | Drop the `bundled.json` echo lines and the trailing `PERMISSION_RULE` observe. |
| `bundled_claude_auto::bundled_claude_auto_tip_form_answers_allow_with_1_and_deny_with_4_ungated` | trim | — | Drop the `bundled.json` echo lines and the trailing `PERMISSION_RULE` observe. |
| `bundled_claude_auto::bundled_claude_read_outside_dialogs_are_complete_permission_requests_with_their_operation` | trim | — | Versions `[2.1.302, 2.1.309]` → `2.1.302`. |
| `bundled_claude_auto::bundled_claude_read_outside_dialog_answers_allow_with_4_and_deny_with_3_ungated` | trim | — | Drop the `bundled.json` echo lines. |
| `bundled_claude_auto::tracker_opens_separate_question_episodes_for_the_startup_offer_and_its_wizard` | delete | `coding_agent/catalog.rs::bundled_claude_auto::bundled_claude_auto_startup_offer_and_wizard_are_separate_questions` | Episode mechanics stay in observation.rs. |
| `bundled_claude_auto::tracker_opens_one_permission_episode_for_the_tip_form_and_closes_it_once_the_command_runs` | move | `runtime/observation.rs` `mod tests` | Tracker behavior; moves unchanged. |
| `bundled_claude_auto::tracker_emits_a_refusal_notice_after_two_compatible_frames_and_only_once_while_present` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_emits_two_events_for_two_different_refusals_visible_together` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_resets_a_refusal_only_after_250_ms_of_absence_on_verified_frames` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_keeps_a_refusal_hidden_by_a_form_or_overlay_and_emits_nothing_when_it_returns` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_keeps_a_rewrapped_refusal_as_the_same_fingerprint` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_ignores_quotes_viewers_and_primary_screen_copies` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_emits_unavailable_once_until_automatic_review_is_observed_available` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_records_the_displayed_mode_only_under_redacted_evidence` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_records_a_re_shown_indicator_after_a_reset_as_a_second_policy_observation` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`. |
| `bundled_claude_auto::tracker_annotations_leave_visibility_eligibility_and_episodes_unchanged` | move | `runtime/observation.rs` `mod tests` | Only coverage of `EpisodeTracker::observe_annotations`; replays 43 frames today. |
| `bundled_claude_2_1_312::primary_screen_fixtures_match_their_alternate_screen_rendering` | trim | — | Drop the `alternate_screen == false` fixture-key precondition. |
| `bundled_claude_2_1_312::sidebar_fixtures_match_their_full_width_rendering` | trim | — | Drop the `cell_backgrounds`-present fixture-key precondition. |
| `bundled_claude_2_1_312::existing_claude_rules_keep_classifying_their_2_1_302_forms` | delete | `coding_agent/catalog.rs::bundled_claude_2_1_312::a_heading_counter_changes_no_identity_on_every_heading_anchored_form` | Claude idle stays in detector `IDLE_COMPOSERS`. |
| `bundled_claude_2_1_312::a_heading_counter_changes_no_identity_on_every_heading_anchored_form` | trim | — | 14 forms × 4 → one form per rule (7) × counter `"10 of 12"`; keep the `rules ==` check. |
| `bundled_claude_2_1_312::dialog_a_decline_resolves_to_two_marker_gated_stages` | trim | — | Drop the `bundled.json` echo lines and the `format!`-built id layout. |
| `bundled_claude_2_1_312::dialog_b_offer_choices_resolve_to_their_single_digits` | trim | — | Drop the `bundled.json` echo lines and the `format!`-built id layout. |
| `bundled_claude_2_1_312::no_plan_selects_a_yes_choice_of_either_nudge_dialog` | trim | — | Drop the all-rules loop (no-Escape is a load-time invariant). |
| `bundled_claude_2_1_312::file_and_no_tip_decisions_resolve_to_their_single_request_digits` | trim | — | Drop the `bundled.json` echo lines and the `format!`-built id layout. |
| `bundled_claude_2_1_312::tracker_replaces_the_pending_notice_with_dialog_a_after_the_draft_is_cleared` | move | `runtime/observation.rs` `mod tests` | Tracker behavior. |
| `bundled_claude_2_1_312::tracker_closes_the_pending_notice_without_dialog_a_after_the_draft_is_sent` | move | `runtime/observation.rs` `mod tests` | Tracker behavior. |
| `bundled_claude_2_1_312::tracker_closes_dialog_a_once_its_decline_returns_the_composer` | move | `runtime/observation.rs` `mod tests` | Tracker behavior. |
| `bundled_claude_2_1_312::tracker_supersedes_the_first_batched_read_with_the_second_after_its_allow` | move | `runtime/observation.rs` `mod tests` | Tracker behavior. |
| `bundled_codex_auto::codex_tracker_emits_one_refusal_after_two_frames_and_none_while_present` | delete | `runtime/observation.rs::tracker_emits_a_refusal_notice_after_two_compatible_frames_and_only_once_while_present` | The cover moves to `runtime/observation.rs` (its move row above). |
| `bundled_codex_auto::codex_tracker_emits_one_event_per_distinct_visible_refusal` | delete | `runtime/observation.rs::tracker_emits_two_events_for_two_different_refusals_visible_together` | The cover moves to `runtime/observation.rs` (its move row above). |
| `bundled_codex_auto::codex_tracker_does_not_re_emit_after_a_resize_rewrap` | delete | `runtime/observation.rs::tracker_keeps_a_rewrapped_refusal_as_the_same_fingerprint` | The cover moves to `runtime/observation.rs` (its move row above); codex rewrap stays in `bundled_codex_auto_rewrap_inside_a_token_keeps_each_refusal_fingerprint`. |
| `bundled_codex_auto::codex_tracker_resets_only_after_250_ms_of_verified_absence` | delete | `runtime/observation.rs::tracker_resets_a_refusal_only_after_250_ms_of_absence_on_verified_frames` | The cover moves to `runtime/observation.rs` (its move row above). |
| `bundled_codex_auto::codex_tracker_keeps_a_refusal_hidden_by_the_warnings_overlay` | delete | `runtime/observation.rs::tracker_keeps_a_refusal_hidden_by_a_form_or_overlay_and_emits_nothing_when_it_returns` | The cover moves to `runtime/observation.rs` (its move row above). |
| `bundled_codex_auto::codex_tracker_ignores_quoted_refusal_text` | delete | `runtime/observation.rs::tracker_ignores_quotes_viewers_and_primary_screen_copies` | The cover moves to `runtime/observation.rs` (its move row above). |
| `bundled_codex_auto::codex_tracker_records_the_status_indicator_once_with_the_mode_only_when_redacted` | delete | `runtime/observation.rs::tracker_records_the_displayed_mode_only_under_redacted_evidence` | The cover moves to `runtime/observation.rs` (its move row above). |
| `bundled_codex::bundled_guard_accepts_a_pending_wrap_cursor_at_cols_and_rejects_one_past_it` | merge | new `coding_agent/catalog.rs::bundled_backend_files::a_screen_outside_the_reviewed_shape_is_rejected` | The accepted `cursor_col == cols` boundary moves; its rejections are already `SCREEN_CHANGES` rows. |
| `bundled_backends::each_bundled_profile_declares_detection_idle_composer_both_operations_and_both_answers` | delete | `coding_agent/catalog.rs::bundled_backends::bundled_submission_fixtures_replay_their_composer_stages_through_the_production_catalog` | Restates bundled `min_version` and capability declarations. |
| `bundled_backends::bundled_submission_fixtures_replay_their_composer_stages_through_the_production_catalog` | trim | — | Drop the `submission_fixture_ids` subset assertion and the id-suffix branches; keep the composer-stage non-interaction check. |
| `bundled_backends::single_byte_answer_plans_are_the_measured_stages` | delete | `coding_agent/detector.rs::bundled_answer_plans_match_their_measured_pre_stage_forms_and_bind_focus` | Also deletes `SINGLE_BYTE_ANSWERS`; no Hermes rows move in. |
| `bundled_backends::episodes_close_not_visible_once_the_idle_composer_holds_250_ms` | delete | `runtime/observation.rs::runtime_episode_verified_composer_remains_held_until_the_250ms_episode_closure` | Idle classification stays in the kept detector `bundled_idle_composers_are_verified_non_interactions_distinct_from_the_form` |
| `bundled_opencode::bundled_opencode_compact_answers_repeat_the_measured_stages_on_the_compact_rules` | delete | `coding_agent/catalog.rs::bundled_opencode::bundled_opencode_narrow_stage_gates_hold_on_their_pre_stage_fixtures` | Restates bundled stage bytes and rule ids. |
| `bundled_opencode::bundled_opencode_answer_plans_are_the_measured_gated_stages` | delete | `coding_agent/catalog.rs::bundled_opencode::bundled_opencode_stage_gates_hold_on_their_pre_stage_fixtures` | Restates bundled stage bytes and rule ids. |
| `bundled_opencode::bundled_opencode_replay_paints_each_captured_background_run` | delete | `coding_agent/catalog.rs::bundled_opencode::bundled_opencode_focus_resolves_from_the_captured_cell_backgrounds` | Asserted fixture RGB values over a 4,320-cell loop. |
| `bundled_opencode::bundled_guard_accepts_the_opencode_fixtures_and_rejects_altered_backgrounds` | merge | new `coding_agent/catalog.rs::bundled_backend_files::cell_backgrounds_outside_the_reviewed_shape_are_rejected` | 11 synthetic cases, one per `verify_cell_backgrounds` branch. |
| `bundled_opencode::bundled_opencode_episode_survives_the_dialog_and_closes_not_visible_on_the_idle_composer` | delete | `runtime/observation.rs::continuation_episodes::continuation_keeps_the_active_episode_open_and_visible_without_events` | The 250 ms closure stays in `runtime/observation.rs::runtime_episode_verified_composer_remains_held_until_the_250ms_episode_closure`. |
| `profile_thresholds::bundled_later_versions_reuse_the_complete_boundary_profile_including_claude_2_1_308` | delete | `coding_agent/catalog.rs::profile_thresholds::capability_reuse_resolves_every_capability_through_the_selected_profile` | Last non-mod.rs user of `BUNDLED_BOUNDARIES`. |
| `composer_exec_plan_retains_exact_selected_metadata_and_independent_coverage` | merge | `coding_agent/catalog.rs::composer_prompt_plan_retains_exact_selected_metadata_and_independent_coverage` | One test loops over `ComposerOperation`. |
| `composer_operation_provenance_and_exact_selection_do_not_authorize_other_profiles` | trim | — | Drop the bundled-catalog tail. |

##### src/coding_agent/detector.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `bundled_claude_read_outside_dialog_outside_a_live_region_or_on_the_idle_composer_is_no_request` | trim | — | Drop the primary/alternate comparison block and the `AUTO_IDLE` replay. |
| `bundled_claude_detector_replays_the_native_permission_fixture_from_unmasked_rows` | trim | — | Drop the `(23, 1)` geometry assert and the `reused`/`below`/`unprobed` version blocks. |
| `bundled_claude_detects_the_wrapped_native_form_and_reports_its_displayed_operation_rows` | trim | — | Drop the geometry asserts, the wrapped pre-stage loop and the `unprobed`/`fallback` blocks. |
| `bundled_claude_unwrapped_native_forms_stay_detected_under_the_wrapped_profile` | delete | `coding_agent/detector.rs::bundled_claude_detector_replays_the_native_permission_fixture_from_unmasked_rows` | Pre-stage fixtures stay matched in `bundled_answer_plans_match_their_measured_pre_stage_forms_and_bind_focus`. |
| `bundled_claude_wrapped_identity_follows_the_displayed_rows_so_a_rewrap_replaces_it` | trim | — | Drop the 52/120-column same-request check. |
| `bundled_claude_operation_is_only_the_rows_between_the_dashed_rules` | trim | — | Drop the fixture row/text and `lines[20] == base` asserts. |
| `bundled_claude_detects_the_native_boxed_form_and_reports_its_displayed_gutter_rows` | trim | — | Drop the `(36, 89)`/`(27, 1)` geometry asserts. Receives the cleared-row-33 case. |
| `bundled_claude_boxed_form_tolerates_the_written_spaces_before_its_closing_line` | merge | `coding_agent/detector.rs::bundled_claude_detects_the_native_boxed_form_and_reports_its_displayed_gutter_rows` | The cleared-row-33 request/fingerprint check moves; the fixture-text asserts go. |
| `bundled_claude_detects_the_native_single_row_box_and_keeps_its_gutter` | trim | — | Drop the `(36, 120)`/`(23, 1)` geometry asserts. |
| `bundled_claude_wrapped_form_keeps_the_false_positive_guards` | trim | — | Drop the primary/alternate comparison block. |
| `bundled_codex_detector_replays_the_native_approval_form_from_unmasked_rows` | trim | — | Drop the `below`/`unprobed` blocks; keep `reused` 0.164.0. |
| `bundled_answer_plans_match_their_measured_pre_stage_forms_and_bind_focus` | trim | — | Drop the cursor == focus asserts, the `stages().len()`/bytes asserts and the primary/alternate block. |
| `bundled_idle_rules_reject_drafts_primary_copies_and_a_cursor_off_the_prefix_end` | trim | — | Drop the primary/alternate comparison block. |
| `claude_2_1_312_guards::trailing_padding_keeps_each_new_rule_identity` | trim | — | Paddings `[1, 4]` → `[4]`; drop 3 duplicate-rule fixtures (33 → 16 replays). |
| `panel_clipping::a_profile_without_any_panel_separator_never_clips` | delete | `coding_agent/detector.rs::panel_clipping::rules_without_a_panel_separator_see_unclipped_rows` | The paneled-form check stays in `a_clipped_row_is_never_soft_joined_to_the_next_row`. |
| `focus_resolution::focus_requires_every_cell_of_the_label_to_differ_from_the_row_background` | delete | `coding_agent/detector.rs::focus_resolution::focus_style_background_resolves_no_choice_for_none_several_or_an_unlisted_option` | Same partly-styled-label branch. |
| `runtime_detector_decoration_and_geometry_keep_stable_semantics` | trim | — | Drop the `min_version` block. |
| `runtime_detector_semantic_fingerprint_is_backend_class_operation_and_choices` | delete | `coding_agent/detector.rs::runtime_detector_fingerprints_carry_no_profile_element_across_profile_boundaries` | Operation-change identity stays in `runtime_detector_decoration_and_geometry_keep_stable_semantics`. |
| `runtime_detector_fragmented_terminal_sequences_reconstruct_the_same_observation` | delete | `runtime/transport.rs::runner_terminal_fragmented_utf8_and_ansi_reconstruct_the_same_live_screen` | ~170 synthetic catalog rebuilds removed. |
| `runtime_detector_equal_priority_compatible_rules_require_equal_extraction` | merge | `coding_agent/detector.rs::runtime_detector_strongest_incompatible_rules_preserve_conflicting_interaction` | Becomes a third loop case. Add the untested `parts.dedup()` case (equal extraction stays `Complete`) there. |
| `annotation::annotation_kinds_parse_from_their_exact_profile_strings` | delete | `coding_agent/detector.rs::annotation::annotation_detector_yields_every_qualifying_block_in_row_order_beside_the_primary` | The error arm stays in catalog `runtime_profile_enums_geometry_and_real_boolean_types_are_validated`. |
| `annotation::annotation_detector_yields_nothing_without_a_selected_profile` | delete | `coding_agent/detector.rs::runtime_detector_unsupported_version_reports_absent_evidence_without_nearby_profile` | Same no-profile early return. |
| `opencode_layouts::opencode_clipped_commands_yield_no_request_for_either_request_rule` | trim | — | Drop the positive unclipped asserts. |
| `opencode_layouts::opencode_exactly_one_request_rule_matches_each_supported_layout` | delete | `coding_agent/detector.rs::opencode_layouts::opencode_wide_layout_keeps_its_single_row_request` | The split and compact layout tests assert the other cases. |
| `hermes_profile::hermes_detects_each_captured_form_at_every_width_on_both_multiplexers` | trim | — | Drop the version-selection block. |
| `hermes_profile::hermes_focus_follows_the_marker_row_and_a_form_without_it_matches_no_plan` | trim | — | 20 forms → 4: default and deny-focus marker per rule. |
| `hermes_profile::hermes_answer_plans_are_the_measured_single_digit_stages_of_their_own_form` | delete | `coding_agent/detector.rs::question_answers::match_answer_refuses_a_choice_plan_on_a_permission_form_and_a_decision_plan_on_a_question` | Deleted as an embedded-data echo; no rows move to catalog.rs. |
| `hermes_profile::hermes_forms_past_their_layout_limit_or_clipped_yield_no_request` | trim | — | "Bottomless" loop 20 forms → `escalate-tmux-120` and `override-tmux-120`. |
| `hermes_profile::hermes_idle_composer_is_a_verified_empty_composer_and_a_draft_is_not` | trim | — | Drop the 20-form `!verified_noninteraction` loop. |
| `hermes_profile::hermes_composer_plans_submit_a_prompt_and_a_native_exec_in_separate_stages` | move | `coding_agent/catalog.rs` `mod tests::bundled_backends` | Asserts only `ProfileCatalog::composer_plan` output. |
| `hermes_profile::hermes_fragmented_ansi_and_utf8_reads_reconstruct_the_same_observation` | delete | `runtime/transport.rs::runner_terminal_fragmented_utf8_and_ansi_reconstruct_the_same_live_screen` | Medium; the alternative keeps one styled whole-feed stage. |
| `hermes_profile::hermes_redraws_stay_one_episode_the_timeout_closes_not_visible_and_a_repeat_reopens` | delete | `runtime/observation.rs::runtime_episode_equivalent_redraw_schedules_no_refresh_and_no_opening` | Also covered by `runtime/observation.rs::runtime_episode_verified_composer_remains_held_until_the_250ms_episode_closure` and `runtime/observation.rs::runtime_episode_identical_requests_are_distinct_only_after_a_verified_boundary`; drop the imports only it uses. |

##### src/coding_agent/hermes_settlement.rs

B7 merges this file into `coding_agent/hermes.rs`; its tests live there in `mod tests::settlement`.

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `settlement_fails_without_typing_a_command_when_clearing_leaves_text_in_the_composer` | trim | — | Use `assert_unverified(..)` instead of the inlined message format. |
| `settlement_counts_a_live_pid_with_another_start_time_as_stopped` | delete | `coding_agent/hermes.rs::settlement::settlement_interrupts_a_running_turn_stops_and_exits_then_verifies_its_own_processes` | Tested the fake host's comparison, not production. |
| `effective_home_is_the_directory_of_the_config_path_hermes_reports_for_the_base` | speed | — | Assert `home_of_config_path("config.yaml")` directly; keep one failing spawn (−1 shell stub). |

##### src/coding_agent/mod.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `selected_preflight_checks_paths_then_binary_and_relocated_required_preset` | trim | — | Opencode arm → one "write preset, then ok" step. |
| `forwarded_config_ignores_an_invalid_value_of_another_backend` | delete | `coding_agent/mod.rs::forwarded_config_returns_only_the_selected_variable_with_the_creator_exact_value` | `forwarded_config` reads only the agent's own variable. |
| `forwarded_config_rejects_an_invalid_selected_value_with_the_config_dir_error` | trim | — | Every agent with one value, showing `config_dir` and `forwarded_config` surface the `require_absolute` error. |
| `selected_preflight_reads_the_native_opencode_config_directory_by_default` | delete | `coding_agent/mod.rs::unset_variable_resolves_the_default` | Sentinel for the former `.opencode/agents` base. The preset read stays in `selected_preflight_reads_only_the_selected_opencode_preset_with_the_shipped_error`. |
| `required_preset_is_the_selected_opencode_agent_file_and_absent_for_other_backends` | trim | — | Drop the literal `cafleet-ask` re-check. |
| `empty_value_fails_with_the_pinned_error` | merge | new `config.rs::require_absolute_rejects_non_absolute_values` | Becomes its `""` case. |
| `relative_or_tilde_value_fails_with_the_pinned_error` | merge | new `config.rs::require_absolute_rejects_non_absolute_values` | Becomes its `rel` and `~/x` cases. |
| `opencode_config_dir_defaults_to_the_skills_base_the_native_cli_reads` | delete | `coding_agent/mod.rs::unset_variable_resolves_the_default` | Asserted the production expression against itself. |
| `version_token_counts_a_v_prefixed_token_toward_the_exactly_one_token_rule` | merge | `coding_agent/mod.rs::version_token_records_the_documented_first_lines_without_a_leading_v` | The destination becomes one table with one row per `version_token` branch; this adds the several-tokens row. |
| `version_token_accepts_semver_prerelease_and_build_metadata_together` | delete | `coding_agent/mod.rs::version_token_records_the_documented_first_lines_without_a_leading_v` | Tests the `semver` crate, not this code. |
| `version_token_requires_one_complete_canonical_semver_token` | merge | `coding_agent/mod.rs::version_token_records_the_documented_first_lines_without_a_leading_v` | Adds the one-valid-among-invalid and no-token rows; drops `01.2.3`/`1.2.3-01`. |
| `version_token_accepts_only_a_single_optional_leading_v` | merge | `coding_agent/mod.rs::version_token_records_the_documented_first_lines_without_a_leading_v` | Adds the `vv` single-strip row; drops `V2.1.4`. |
| `native_version_probe::native_version_probe_records_the_single_version_token_of_a_version_only_invocation` | merge | `coding_agent/mod.rs::native_launch_probes_and_launches_the_same_resolved_executable` | Add a stdin read and a stderr line to the shared stub (−1 spawn). |
| `native_version_probe::native_launch_identity_selects_the_boundary_profile_from_its_minimum_onwards` | trim | — | Replace the `BUNDLED_BOUNDARIES` loop with a synthetic `thresholds` catalog; derive coverage from `catalog.detection_coverage(backend, version)` (B3). |
| `native_version_probe::native_launch_identity_rejects_a_known_version_below_the_lowest_boundary` | trim | — | Delete the `BUNDLED_BOUNDARIES` loop; keep the synthetic `several` part. |
| `runtime_policy_opencode_unknown_choice_fails_with_the_exact_usage_error` | delete | `coding_agent/mod.rs::runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error` | One `format!` in `resolve_policy` serves every backend. |
| `runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error` | trim | — | One requested value. |
| `hermes::hermes_registry_entry_names_its_binary_config_variable_and_default_home` | delete | `coding_agent/mod.rs::unset_variable_resolves_the_default` | Custom value and errors stay in `absolute_value_is_used_verbatim` and the pinned-error tests. |
| `hermes::hermes_argv_is_chat_cli_with_tool_source_and_cwd_then_model_reasoning_and_flag_prompt` | delete | `coding_agent/mod.rs::literal_argv_preserves_permission_and_optional_flag_boundaries` | The cover asserts every backend's exact argv. |
| `hermes::hermes_argv_keeps_a_flag_like_prompt_and_spaced_cwd_as_single_values` | delete | `coding_agent/mod.rs::literal_argv_preserves_permission_and_optional_flag_boundaries` | The cover uses a flag-like prompt and a spaced `CWD`. |
| `hermes::hermes_argv_ignores_the_display_name` | delete | `coding_agent/mod.rs::literal_argv_preserves_permission_and_optional_flag_boundaries` | The cover passes display name `"display name"`. |
| `hermes::only_hermes_argv_carries_the_member_cwd` | delete | `coding_agent/mod.rs::literal_argv_preserves_permission_and_optional_flag_boundaries` | Non-Hermes argv carries no cwd there. |
| `hermes::hermes_launch_argv_replaces_only_the_binary_with_the_resolved_executable` | delete | `coding_agent/mod.rs::native_launch_probes_and_launches_the_same_resolved_executable` | Backend-agnostic argv[0] replacement. |
| `hermes::hermes_effort_accepts_its_reasoning_levels_and_rejects_others_with_the_usage_error` | merge | `coding_agent/mod.rs::model_and_effort_validation_preserves_backend_rules` | The Hermes valid list joins the `(agent, valid, invalid)` table. Keep one exact-message assert and one `build_spawn_argv_with_policy` refusal. |
| `hermes::hermes_model_passes_through_verbatim` | delete | `coding_agent/mod.rs::runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode` | Hermes model + reasoning are covered for both modes. |
| `hermes::hermes_version_rule_rejects_a_line_that_does_not_match` | trim | — | 14 → 5 lines: `""`, `"Hermes Agent v0.22.4"`, trailing space, leading space, double date. |
| `hermes::hermes_version_rule_requires_a_canonical_semver_release_date` | trim | — | 7 → 2: `(2026.09.24)` and `(unknown)`. |
| `hermes::other_backends_keep_the_single_token_version_rule` | delete | `coding_agent/mod.rs::native_version_probe::native_version_probe_rejects_failed_ambiguous_invalid_or_missing_versions` | Restated the dispatch match arm. |
| `hermes::hermes_native_version_probe_applies_the_hermes_rule_to_the_first_line` | delete | `coding_agent/mod.rs::native_version_probe::native_version_probe_rejects_failed_ambiguous_invalid_or_missing_versions` | Hermes dispatch stays in `hermes_version_rule_records_the_parenthesized_release_date`; −1 spawn. |
| `hermes::hermes_native_version_probe_fails_when_only_the_package_semver_is_printed` | trim | — | Drop the `native_launch_identity` tail (covered by `native_launch_identity_launches_a_failed_probe_with_unknown_coverage_and_its_warning`). |
| `hermes::hermes_native_version_probe_rejects_a_non_semver_release_date` | delete | `coding_agent/mod.rs::hermes::hermes_version_rule_requires_a_canonical_semver_release_date` | In-process rule check; −1 spawn. |
| `hermes::hermes_launch_policy_is_smart_or_manual_at_every_version_with_no_permission_argv` | delete | `coding_agent/mod.rs::runtime_policy_omission_selects_each_backends_automatic_default_argv` | Explicit modes stay in `runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode`. |
| `hermes::hermes_launch_policy_rejects_every_other_mode_with_the_exact_usage_error` | delete | `coding_agent/mod.rs::runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error` | Per-backend choices stay in `runtime_policy_invalid_explicit_selection_reports_backend_and_catalog_choices` |
| `hermes::hermes_forwarded_config_returns_hermes_home` | delete | `coding_agent/mod.rs::forwarded_config_returns_only_the_selected_variable_with_the_creator_exact_value` | The cover iterates every agent. |

##### src/db.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `baseline_initialization_records_the_identity_ledger_version_and_shape` | trim | — | Drop the 14-name table list, the two column lists, `column_names` and the identity count. Keep identity, ledger and recorded version. |
| `another_versions_file_is_incompatible_and_left_byte_identical` | merge | `db.rs::a_v8_family_file_is_incompatible_and_left_byte_identical` | `cafleet_version='0.0.0'` becomes the second case. |
| `required_schema_components_and_identity_are_refused` | trim | — | 8 → 4 damages: drop index, recreate index, `DROP TABLE asset_installs`, `DELETE FROM cafleet_database`. |
| `connection_pragmas_and_foreign_keys_protect_real_writes` | trim | — | Drop the `foreign_keys`/`busy_timeout`/`application_id` duplicates; keep the FK-orphan and non-sqlite-URL parts. |
| `runtime_tests::runtime_schema_exposes_the_complete_record_shapes` | delete | `db.rs::required_schema_components_and_identity_are_refused` | Embedded-data echo of baseline column names; the guard keeps its structure test. |
| `runtime_tests::current_schema_guard_requires_each_runtime_table` | delete | `db.rs::required_schema_components_and_identity_are_refused` | Same column-mismatch branch as its `DROP TABLE asset_installs` case; −5 file DBs. |
| `runtime_tests::every_nonterminal_status_enforces_one_live_run_per_member` | speed | — | Reuse one fixture across the 6 cases, marking each run `exited` before the next. |
| `runtime_tests::terminal_run_history_allows_a_new_run_without_losing_the_old_one` | speed | — | Reuse one fixture across the 3 cases. |
| `runtime_tests::run_backend_check_accepts_every_supported_backend_only` | trim | — | Drive the accept loop from `CodingAgent::all()`; rejects 4 → `"gemini"`, `"Hermes"`. |
| `runtime_tests::native_session_column_accepts_only_null_or_a_json_object` | trim | — | 7 rejects → `"[]"` and `"not json"`. |
| `runtime_tests::interaction_classes_preserve_distinct_native_forms` | speed | — | Reuse one fixture across the 10 cases, closing each interaction first. |
| `database_scope_tests::nested_exit_restores_the_earliest_outer_deadline_on_every_exit` | trim | — | Drop exit 1 (same `restore()` path as exit 0); ~0.16 s. |

##### src/delivery.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `wrapped_delivery_reports_the_runner_held_reason` | merge | `delivery.rs::wrapped_send_is_submitted_and_stamped_when_another_writer_takes_the_released_lock` | Runs as the destination's first phase on the same service (−1 child spawn). |
| `wrapped_delivery_history_and_conflicts_precede_the_real_held_pane_lock` | speed | — | Use the DB-only `runtime::test_support::Fixture` instead of a live service; drop the records assertion (−1 child spawn). |
| `wrapped_delivery_submits_real_preview_and_atomically_settles_the_actual_message` | trim | — | Replace the re-implemented digest with the existing `preview_digest(..)` helper. |
| `a_director_snapshot_preserves_its_pending_inbox_without_a_preview` | delete | `delivery.rs::director_delivery_ends_before_resolving_or_locking_a_pane` | `eligible_member` filters directors before any preview. |
| `director_delivery_ends_before_resolving_or_locking_a_pane` | trim | — | 3 backends → 1 (−2 file DBs). |
| `a_folded_body_of_exactly_4000_characters_is_inline` | delete | `delivery.rs::the_limit_counts_unicode_scalar_values_not_bytes` | Same 4000 boundary with a multibyte char. |
| `a_folded_body_of_4001_characters_becomes_the_show_pointer` | delete | `delivery.rs::the_limit_counts_unicode_scalar_values_not_bytes` | Same 4001 boundary with a multibyte char. |
| `deferred_preview::deferred_preview_during_a_detected_interaction_is_queued_then_drained_once_after_closure` | delete | `delivery.rs::deferred_preview::deferred_previews_sent_during_one_interaction_queue_separately_and_drain_in_order` | Same setup and per-preview asserts; −1 detecting service child. |

##### src/multiplexer/herdr.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `capture_pane_over_fetch_survives_a_blank_tail_deeper_than_n` | delete | `multiplexer/mod.rs::capture_window_drops_only_the_visually_blank_tail_then_keeps_the_last_lines` | Over-fetch depth stays in `capture_pane_keeps_crlf_rows_and_drops_the_crlf_blank_tail`. |

##### src/multiplexer/mod.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `auto_detect_picks_the_single_present_backend` | merge | new `multiplexer/mod.rs::auto_detection_resolves_the_single_present_backend_or_fails` | Single-present rows (tmux, herdr) of one `(env pairs, expected)` table. |
| `empty_presence_values_count_as_unset` | merge | new `multiplexer/mod.rs::auto_detection_resolves_the_single_present_backend_or_fails` | Empty-value row. |
| `both_present_is_a_hard_error` | merge | new `multiplexer/mod.rs::auto_detection_resolves_the_single_present_backend_or_fails` | Both-present row asserting the reworded D4 ambiguity error. |
| `neither_present_is_a_hard_error` | merge | new `multiplexer/mod.rs::auto_detection_resolves_the_single_present_backend_or_fails` | Neither-present row asserting the reworded D4 no-multiplexer error. |

##### src/output/json.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `runtime::runtime_presentation_renders_a_profiled_run_on_member_and_event_surfaces` | merge | `output/json.rs::runtime::runtime_presentation_exposes_recorded_owner_snapshot_and_unresolved_inputs` | The `coverage='tested'` update and its JSON and text asserts move. |
| `runtime::effective_policy_record_is_rendered_last_in_every_runtime_snapshot_surface` | trim | — | Replace the two last-key checks with `assert_key_order(.., &RUNTIME_KEYS)`. |
| `runtime::input_presentation_projects_delivery_from_the_stored_receipt_state` | move | `broker/runtime/inputs.rs` `mod tests` | Only test of `delivery_projection`. |
| `runner_registry::runner_registry_show_returns_historical_health_without_contacting_a_live_endpoint` | move | `broker/runtime/mod.rs` `mod tests` | Owner of `member_runtime`. |

##### src/output/text.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `permission_inspection_json_places_supported_choices_after_supported_decisions` | merge | `output/json.rs::runtime::permission_inspection_request_orders_its_identity_and_capability_keys` | The `supported_choices` value assertion (`question_inspection()`) moves. |

##### src/pane_lock.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `pane_guard::runner_recovery_pane_guard_validates_the_actual_database_placement_and_named_inode` | trim | — | Backend/session/pane loop → one field; build settings and placement with `LockFixture`. |
| `pane_lock_wait_tests::a_lock_released_just_before_the_reserved_deadline_is_acquired` | delete | `pane_lock.rs::pane_lock_wait_tests::a_lock_released_within_the_wait_is_acquired` | Same retry branch; ~1.3 s. |
| `pane_lock_wait_tests::a_lock_still_busy_at_the_reserved_deadline_returns_none` | speed | — | Hold the guard on the test thread instead of `hold_for(None)` and its join (~1.5 s). |

##### src/runtime/artifacts.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `lock_transfer::runner_recovery_lock_transfer_rejects_every_changed_stable_expected_identity` | trim | — | 11 → 1 case; field coverage moves to the new direct `stable_run_identity` test. |
| `lock_transfer::runner_recovery_lock_transfer_ignores_mutable_health_diagnostics_and_sequence_snapshots` | merge | new direct `stable_run_identity` unit test in `runtime/recovery.rs` | Its mutable-field insensitivity cases move. |

##### src/runtime/control.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `control_codec_tests::runner_review_codec_encoding_pins_big_endian_length_and_required_version` | trim | — | Drop the two constant pins (`CONTROL_PROTOCOL_VERSION`, `MAX_CONTROL_FRAME`). |
| `request_lifecycle_then_input_duplicate_kind_cannot_escape_strict_routing` | merge | `runtime/control.rs::request_input_then_lifecycle_duplicate_kind_cannot_escape_strict_routing` | The mirror assertion joins the destination. |
| `reply_inspection_supported_and_unknown_preserve_complete_public_object` | trim | — | Replace the 20 per-field asserts with one `serde_json::to_value` equality. |
| `identity::identity_decode_requires_a_32_character_ascii_hex_instance_id` | merge | `runtime/control.rs::identity::identity_decode_requires_an_absolute_string_database_path` | Mirror the encoder loop over both fields. |
| `identity::identity_decode_requires_an_unsigned_64_bit_inode` | merge | `runtime/control.rs::identity::identity_decode_requires_an_unsigned_64_bit_device` | Mirror the encoder loop over both fields. |
| `runtime_diagnosis::invalid_run_paths_are_an_interrupted_launch` | merge | `runtime/control.rs::runtime_diagnosis::a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome` | The destination becomes one `(descriptor, lock, health) → (state, phrases)` table; this adds the InvalidPaths row. |
| `runtime_diagnosis::a_health_reply_is_running_whatever_the_lock_state` | merge | `runtime/control.rs::runtime_diagnosis::a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome` | Table row. |
| `runtime_diagnosis::a_held_lock_without_a_health_reply_is_not_responding` | merge | `runtime/control.rs::runtime_diagnosis::a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome` | Table row. |
| `runtime_diagnosis::a_free_lock_without_a_health_reply_is_a_lost_runtime` | merge | `runtime/control.rs::runtime_diagnosis::a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome` | Table row. |
| `runtime_diagnosis::a_health_probe_skipped_by_the_deadline_is_not_checked` | merge | `runtime/control.rs::runtime_diagnosis::a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome` | Table row. |
| `runtime_diagnosis::an_owner_lock_probe_failure_is_check_failed_with_only_its_cause` | merge | `runtime/control.rs::runtime_diagnosis::a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome` | Table row. |

##### src/runtime/input.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `ordinary_prompt::checkpoint_hook_error_precedes_expiry_and_blocks_before_write` | speed | — | Hook deadline 1 s → 150 ms (~1.7 s). |
| `ordinary_prompt::preview_exec::operations_exec_selects_its_native_shell_plan_and_requires_its_own_coverage` | trim | — | Drop the 3 catalog capability asserts (catalog.rs behavior). |
| `ordinary_prompt::staged_delivery::staged_profile_delivery_writes_the_literal_then_its_submit_key_after_a_500ms_settle` | merge | `runtime/input.rs::ordinary_prompt::prompt_reservation_and_each_hook_require_a_fresh_drain_before_one_real_write` | Its settled-gap, no-ESC and delivery-selection asserts move. |
| `ordinary_prompt::staged_delivery::staged_fallback_for_unknown_coverage_uses_the_missing_plan_reason` | delete | `runtime/input.rs::ordinary_prompt::staged_delivery::staged_fallback_without_an_operation_plan_escapes_settles_types_settles_and_enters` | Same `None` arm of `select_delivery` (~1.1 s). |
| `ordinary_prompt::staged_delivery::staged_delivery_writes_zero_bytes_including_escape_while_an_interaction_is_open` | delete | `runtime/input.rs::ordinary_prompt::deferred_queue::queue_admission_commits_a_durable_ready_receipt_and_keeps_the_literal_in_memory` | Strict subset plus a real 1.2 s hold. |
| `ordinary_prompt::staged_delivery::staged_new_interaction_during_the_settle_stops_the_submit_stage_with_uncertainty` | delete | `runtime/input.rs::ordinary_prompt::staged_delivery::uncertain_cause::staged_candidate_during_the_settle_records_interaction_candidate` | The open variant stays in `staged_open_interaction_during_the_settle_records_interaction_open`. |
| `ordinary_prompt::staged_delivery::staged_new_interaction_after_the_settle_before_the_submit_write_is_uncertain` | merge | `runtime/input.rs::ordinary_prompt::staged_delivery::uncertain_cause::staged_open_interaction_after_the_settle_records_interaction_open` | Append its tail: `show(SCREEN)`, `hold_turns`, `stage_bytes == [LITERAL]`. |
| `ordinary_prompt::deferred_queue::queue_capacity_bounds_queued_plus_active_work_at_4096_with_a_zero_byte_error` | delete | `broker/runtime/inputs.rs::runtime_permission_shared_unresolved_limit_preserves_retained_receipts` | Add a deferred-Ready CTE-prefill case to the cover first. 4,096 admissions under a 120 s deadline. |
| `ordinary_prompt::deferred_queue::queue_timer_retries_a_busy_pane_lock_without_child_output_and_delivers_once_free` | speed | — | Lock hold 1 s → 300 ms (~0.7 s). |
| `ordinary_prompt::deferred_queue::preview_receipts_use_the_canonical_digest_and_reject_another_literal` | trim | — | Compare with `storage::input_payload_digest(..)` instead of re-deriving the private formula. |
| `ordinary_prompt::deferred_queue::queue_head_interrupted_between_stages_holds_the_next_input_until_reconciliation` | merge | `runtime/input.rs::ordinary_prompt::deferred_queue::queue_head_failure_after_a_possible_write_is_uncertain_published_once_and_reconcilable` | Becomes a third loop case, `"human_input_during_the_settle"`. |
| `ordinary_prompt::deferred_queue::queue_head_under_an_observer_outage_settles_within_the_observer_budget` | speed | — | Backdate `attempt_deadline` as `queue_head_acquiring_the_pane_lock_too_late_for_its_settles_writes_zero_bytes` does (~5 s). |
| `ordinary_prompt::composer_busy::composer_busy_queues_with_zero_bytes_and_drains_by_profile_when_the_composer_empties` | trim | — | Drop the unclassifiable-screen case (same `composer_busy` branch). |
| `ordinary_prompt::sql::prompt_final_sql_is_nonwaiting_on_the_supplied_connection_and_fresh_drain_progresses` | trim | — | Drop the `&mut` pointer-address assert and the `db.rs` assertions (`begins`, read authorizer, `busy_timeout`). Keep the elapsed and `DeadlineExceeded` outcomes. |
| `ordinary_prompt::sql::prompt_inherited_expiry_refuses_before_sql_and_after_uncontended_sql_on_the_actual_connection` | trim | — | Same cuts as the sibling `sql` test. |
| `permission_response_contracts::permission_screen_seq_only_redraw_schedules_no_refresh_and_keeps_the_settled_token` | trim | — | Drop the observation.rs/writer.rs assertions (~20 lines); keep token survival and the locked-DB respond. |
| `permission_response_contracts::a_preview_during_the_claude_read_outside_dialog_queues_with_zero_bytes_and_drains_once_on_the_idle_composer` | merge | `runtime/input.rs::permission_response_contracts::a_preview_prompt_and_exec_during_dialog_a_queue_with_zero_bytes_and_each_drains_once` | The destination loops over `[("2.1.309", read-outside), ("2.1.312", nudge)]`. |
| `permission_response_contracts::permission_claude_blink_cursor_transit_keeps_generation_and_token_and_admits_respond` | trim | — | Drop the 6 lines asserting its own generated input (write count, byte count, digest). |
| `permission_response_contracts::permission_receipt_cacheless_ready_proof_is_borrowed_and_not_an_attributed_disposition` | trim | — | Replace the hand-rolled `AmbiguousIf*` checks with a macro. Move the `storage::ZeroByteRetirement` half to `broker/runtime/inputs.rs`. |
| `permission_response_contracts::permission_response_changed_focus_after_positive_stage_stops_remaining_bytes` | merge | `runtime/input.rs::permission_response_contracts::permission_response_failure_after_its_first_stage_records_write_failed` | Its checkpoint `(Partial, 1)` assertion moves. |
| `permission_response_contracts::question::a_gated_choice_writes_nothing_while_its_first_focus_gate_is_unmet` | delete | `runtime/input.rs::permission_response_contracts::gated::gated_first_stage_is_never_written_while_its_focus_gate_is_unmet` | Same first-stage gate `ensure` (~1 s). |
| `permission_response_contracts::gated::gated_unexpected_change_between_stages_fails_uncertain_without_another_byte` | trim | — | Drop one of the `"always"`/`"none"` cases. |
| `permission_response_contracts::gated::gated_human_input_between_stages_fails_uncertain_without_another_stage` | delete | `runtime/input.rs::permission_response_contracts::gated::gated_continuation_binds_only_by_succession_in_the_same_episode` | Same `manual_generation` conjunct as its "human input" case. |
| `retirement_contracts::retirement_requires_settlement_and_no_active_writer_then_removes_only_its_cache` | trim | — | Replace the `member_input_operations` read-authorizer count with a behavioral check. Add `total_changes` equality around the `again` retire. |
| `retirement_contracts::live_receipt_retirement_preserves_settled_ordinary_history_without_a_cached_attempt` | delete | `runtime/input.rs::retirement_contracts::retirement_requires_settlement_and_no_active_writer_then_removes_only_its_cache` | Three rows on one cache-absent branch. |
| `clock_capture_monotonic_origin_yields_positive_expiration` | merge | `runtime/input.rs::clock_capture_equal_bracket_preserves_one_nanosecond` | Same 1 ns success path; becomes a second case. |
| `clock_receive_rejects_already_expired_original_admission_cap` | merge | `runtime/input.rs::clock_receive_rejects_bracket_delay_equal_to_or_past_projected_deadline` | Becomes a row of the destination's case table. |
| `clock_receive_rejects_expired_shutdown_despite_positive_expiration` | merge | `runtime/input.rs::clock_receive_rejects_bracket_delay_equal_to_or_past_projected_deadline` | Becomes a row of the destination's case table. |

##### src/runtime/input_client.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `permission_response_actual_service_lost_reply_retains_audit_and_same_key_never_retypes` | trim | — | End after the lost-reply audit (~line 3040); the replay tail is covered elsewhere. |
| `permission_receipt_postcommit_expiry_preserves_claim_context_and_consumed_history` | trim | — | End after the settled-row asserts (~line 3200). Start the reconcile with a 300 ms deadline instead of 2 s. |
| `exclusive_permission_receipt_client_preserves_uncertain_claim_and_continuous_guard` | trim | — | Drop the `absent = true` iteration. |
| `exclusive_receipt_client_keeps_one_guard_original_row_and_error_across_live_fallback` | trim | — | Drop the `"settlement_failure"` mode. |
| `live_receipt_client_postcommit_expiry_retains_settlement_context_and_no_live_retry` | delete | `runtime/input_client.rs::permission_receipt_postcommit_expiry_preserves_claim_context_and_consumed_history` | Same reconcile branch; −1 runner spawn and up to 2 s sleep. |
| `external_input_retained_history_precedes_busy_pane_and_inactive_owner` | trim | — | 5 → 3 states: drop `InProgress` and `Settled/Submitted`. |
| `prepared_submission::clock_prepared_submission_captures_one_paired_sample` | delete | `runtime/input.rs::clock_capture_uses_later_instant_remaining_without_bracket_extension` | Same `ScriptClock` values through a thin wrapper. |
| `prepared_submission::clock_prepared_submission_source_failure_preserves_unused_later_sample` | delete | `runtime/input.rs::clock_capture_source_failure_consumes_one_sample_without_fallback` | Identical script. |
| `prepared_submission::clock_prepared_submission_expiry_rejects_without_resampling_new_budget` | delete | `runtime/input.rs::clock_capture_rejects_equal_or_expired_deadline_without_resampling` | Same as its `after_ms=100` case. |
| `permission_history::entry::retained_and_consumed_history_projects_each_state_without_live_calls` | trim | — | Drop `Abandoned` (same `projection` arm as `NotSubmitted`). |
| `permission_history::entry::history_preserves_both_decisions_and_every_valid_attributed_source_mode` | delete | `broker/runtime/inputs.rs::runtime_permission_user_decisions_roundtrip_each_attributed_mode_and_decision` | The history path never branches on decision, mode or source. |
| `permission_history::entry::history_survives_each_later_lifecycle_change_without_live_preflight` | trim | — | Keep only the `OwnerFenced` case, inlined; drop the enum and four cases. |
| `permission_history::entry::retained_lookup_precedes_actual_unresolved_capacity_refusal` | delete | `broker/runtime/inputs.rs::runtime_permission_shared_unresolved_limit_preserves_retained_receipts` | The history path makes no reservation call. |
| `permission_history::entry::each_conflicting_caller_binding_fails_before_live_calls_with_exact_rows` | trim | — | 11 → 3 fields: `run`, `director`, one digest field. Receives the Ready-state assertions of the merged staged test. |
| `permission_history::entry::retained_ordinary_input_kind_cannot_impersonate_a_permission_response` | delete | `broker/runtime/inputs.rs::runtime_permission_absent_key_is_distinct_from_an_existing_other_input_kind` | Storage verdict. |
| `permission_history::entry::history_rejects_missing_audit_or_possibly_transmitted_claim_without_live_fallback` | delete | `broker/runtime/inputs.rs::runtime_permission_readers_reject_missing_foreign_or_corrupted_audit_claim_data` | Error propagation stays in `historical_message_association_is_checked_despite_matching_canonical_digest`. |
| `permission_history::entry::semantic_source_mode_preflight_preserves_usage_category_before_history_or_live_effects` | trim | — | `[Manual, Unknown]` → 1. |
| `permission_history::entry::input_failure_without_context_preserves_the_complete_source_category` | trim | — | 7 causes → 2 (one plain, one `WithDiagnostics`). Move it to the top-level `mod tests`. |
| `permission_history::entry::input_failure_context_preserves_cause_and_escapes_ordered_key_metadata` | trim | — | 7 causes → 2 (one plain, one `WithDiagnostics`). Move it to the top-level `mod tests`. |
| `permission_history::entry::input_failure_context_omits_unknown_optional_identities` | move | `runtime/input_client.rs` top-level `mod tests` | Tests `InputFailure`, not history. |
| `permission_history::entry::claim_and_association_history::supplied_deferred_snapshot_survives_committed_association_change` | trim | — | Keep the mid-read commit outcome; drop the `ObservedSql` bookkeeping and the 7 table-name asserts. |
| `permission_history::entry::staged::permission_ready_conflict_precedes_live_lock_and_preserves_binding` | merge | `runtime/input_client.rs::permission_history::entry::each_conflicting_caller_binding_fails_before_live_calls_with_exact_rows` | Its `InvalidStoredValue` match and `response_context(..)` asserts join one iteration. |
| `uncertain_reason::durable_outcome_reports_the_recorded_cause_as_the_uncertain_reason` | trim | — | 4 causes → 1. |
| `uncertain_reason::live_and_repeated_permission_replies_report_the_same_recorded_cause` | delete | `runtime/input_client.rs::permission_history::entry::retained_and_consumed_history_projects_each_state_without_live_calls` | Same reason-string projection. |
| `uncertain_reason::repeated_permission_reply_for_a_causeless_receipt_keeps_the_fixed_reason` | delete | `runtime/input_client.rs::permission_history::entry::retained_and_consumed_history_projects_each_state_without_live_calls` | Same reason-string projection. |
| `exclusive_receipt_revalidates_original_input_placement_and_budget_after_real_sql_entry` | trim | — | 5 → 3 cases: keep `digest`, `placement`, `deadline` (−2 writer threads and 40 ms sleeps). |

##### src/runtime/lifecycle.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `launch_readiness_tests::runner_readiness_nonprivate_socket_expires_without_a_prepare_connection` | delete | `runtime/lifecycle.rs::launch_readiness_tests::runner_readiness_bound_unready_socket_expires_at_the_supplied_deadline` | Same `check_deadline` loop head (120 ms wait). |
| `launch_readiness_tests::runner_readiness_regular_file_is_an_identity_failure` | delete | `runtime/lifecycle.rs::launch_readiness_tests::runner_readiness_symlink_to_a_private_socket_is_an_identity_failure` | Same `is_socket` ensure. |
| `launch_readiness_tests::runner_readiness_dangling_symlink_is_an_identity_failure` | delete | `runtime/lifecycle.rs::launch_readiness_tests::runner_readiness_symlink_to_a_private_socket_is_an_identity_failure` | Same `is_socket` ensure. |
| `launch_readiness_tests::runner_readiness_malformed_reply_after_written_prepare_is_not_reconnected` | merge | `runtime/lifecycle.rs::launch_readiness_tests::runner_readiness_lost_reply_after_written_prepare_is_not_reconnected` | One loop over `[None, protocol-v2, {"identity":null}]`. |
| `runner_creation_rejects_each_stale_prepared_identity_before_starting_a_child` | trim | — | 4 corrupt fields → 1 (one derived equality). |
| `hermes_creation_settlement_failure_is_a_cleanup_diagnostic_and_compensation_continues` | merge | `runtime/lifecycle.rs::hermes_creation_compensation_settles_a_started_client_before_runner_shutdown` | One loop over `hermes_failure: [None, Some(..)]`. |
| `creation_compensation_settles_only_a_hermes_run_whose_child_started` | trim | — | Drop one of `("hermes","start")`/`("hermes","prepared")`. |
| `runner_database_deadline_review_inner_error_restores_the_live_outer_scope` | delete | `runtime/lifecycle.rs::runner_database_deadline_review_inner_success_restores_the_live_outer_scope` | Same explicit `restore()`; drop the `Error` arm (~460 ms). |

##### src/runtime/observation.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `native_capture::tests::collection::capture_invalid_request_leaves_the_finalized_channel_unused` | trim | — | Drop `path`, `control`, `long`; move `unicode` into `capture_arm_validates_schema_identity_duration_and_safe_capture_id` (−4 heavy iterations). |
| `native_capture::exporter::tests::runner_collector_encode_and_prepermit_manifest_stalls_return_unfinished_and_revoke_late_publication` | trim | — | Encode-only; move the 3 run-status asserts into `runner_collector_cancelled_manifest_worker_preserves_foreign_identities_and_cleans_its_matching_temporaries` (−1 service run). |
| `native_capture::exporter::tests::runner_collector_confirmed_pair_remains_accepted_while_native_work_continues_and_after_exit` | merge | `runtime/observation.rs::native_capture::tests::service_capture::service_private_capture_publishes_manifest_last_and_keeps_native_work_running` | The after-`finish()` re-read and `OutputFile::assert_unchanged` move (−1 service spawn). |
| `native_capture::files::tests::capture_arm_validates_schema_identity_duration_and_safe_capture_id` | speed | — | Plain `runtime::test_support::Fixture` (with chmod) instead of `CaptureFixture`; no libtest hash. |
| `native_capture::files::tests::capture_arm_reader_consumes_once_and_detects_replacement_after_claim` | speed | — | Plain `Fixture` instead of `CaptureFixture`. |
| `native_capture::files::tests::capture_arm_reader_claims_malformed_or_oversized_input_without_accepting_repair` | speed | — | Plain `Fixture` instead of `CaptureFixture` (×2). |
| `native_capture::files::tests::capture_arm_reader_rejects_symlinks_public_modes_and_replaced_parent` | speed | — | Plain `Fixture` instead of `CaptureFixture` (×3). |
| `native_capture::files::tests::duplicate_fields::runner_collector_nested_identical_duplicate_fields_fail_before_normalization` | merge | `runtime/observation.rs::native_capture::files::tests::duplicate_fields::runner_collector_top_level_identical_duplicate_fields_fail_before_normalization` | Becomes the `nested = true` loop case. |
| `native_capture::files::tests::duplicate_fields::runner_collector_top_level_conflicting_duplicates_reject_both_field_orders` | delete | `runtime/observation.rs::native_capture::files::tests::duplicate_fields::runner_collector_top_level_identical_duplicate_fields_fail_before_normalization` | Same serde duplicate-field branch; drop the `conflicting` helper parameter. |
| `native_capture::files::tests::duplicate_fields::runner_collector_nested_conflicting_duplicates_reject_both_field_orders` | delete | `runtime/observation.rs::native_capture::files::tests::duplicate_fields::runner_collector_top_level_identical_duplicate_fields_fail_before_normalization` | Same serde duplicate-field branch. |
| `native_capture::verification::tests::driver::an_outside_placeholder_without_a_file_probe_refuses_the_case` | merge | `runtime/observation.rs::native_capture::verification::tests::driver::overlay_placeholders_expand_to_the_absolute_case_directories` | Same single error site. |
| `runtime_episode_quiet_timer_recheck_uses_the_same_output_sequence` | delete | `runtime/observation.rs::controller_tests::controller_quiet_poll_opens_one_episode_with_the_unchanged_parser_sequence` | `test_support::open` already performs the quiet recheck. |
| `runtime_episode_stable_replacement_closes_old_episode_before_opening_the_next` | merge | `runtime/observation.rs::runtime_episode_class_and_stable_choice_changes_supersede_after_stability` | Add `"operation"` to the loop; keep the 299 ms and `latest_evidence.is_none()` checks once. |
| `runtime_episode_gap_reset_preserves_open_identity_and_discards_pending_timers` | delete | `runtime/observation.rs::runtime_episode_gap_reset_restarts_closure_debounce_and_keeps_the_active_episode_held` | Cannot fail on its claim. Add both `reset_observation` reasons to `runtime_episode_reset_and_finish_validate_their_lifecycle_scope`. |
| `runtime_episode_sequence_pool_rejects_invalid_and_overlapping_reservations` | merge | `runtime/observation.rs::runtime_episode_sequence_pool_uses_only_committed_grants_and_retains_borrowed_drafts` | Its invalid-range and overlap checks join the shared body. |
| `observation_contract::service_transient_form_and_clear_in_one_native_write_produce_no_episode` | delete | `runtime/observation.rs::controller_tests::controller_actual_adapter_observes_transient_form_before_clear_without_opening` | −1 service spawn and 180 ms of polling. |
| `observation_contract::service_later_version_profile_emits_no_coverage_unknown_while_no_profile_still_does` | trim | — | Drop the bundled `min_version == "0.163.0"` and `detection == Tested` asserts (embedded data). |
| `inspection_tests::permission_inspection_request_generation_exhaustion_invalidates_without_wrap` | speed | — | Deterministic `writer::test_support::Clock` instead of `RealObservationClock` (~210 ms of real waits). |
| `inspection_tests::opencode_resize_across_w_split_starts_a_new_generation_and_a_rewrap_a_new_episode` | speed | — | Deterministic `writer::test_support::Clock` instead of `RealObservationClock` (~300 ms). |
| `controller_tests::failure_handling::runner_failure_after_terminal_enqueue_never_appends_a_live_post_terminal_gap` | trim | — | Drop the asserts implied by `kinds == [Opened, Closed, Exited]`. |

##### src/runtime/press.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `press_entry_rechecks_full_placement_and_original_expiry_after_actual_metadata` | speed | — | `ExpireMetadata` iteration: `until = now + 300ms` instead of 2 s. |

##### src/runtime/process.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `mechanisms_tests::runner_real_shutdown_kills_and_reaps_a_term_ignoring_owned_child_within_the_shared_deadline` | speed | — | Let `OwnedPty::stop` take a clock and pass a clipped `ShutdownClock` (300 ms) instead of the 2 s TERM grace. |
| `mechanisms_tests::runner_real_shutdown_closes_a_slave_retaining_descendant_and_preserves_the_native_zero_exit` | speed | — | Same clipped `ShutdownClock` (~1.7 s). |
| `ownership_refresh_tests::refresh_fresh_remaining_members_preserve_exact_refusal_without_adoption` | trim | — | `[B]`, `[A]`, `[B,C]` → 1 case. |
| `ownership_refresh_tests::reaped_stat::runner_process_stat_read_of_a_task_reaped_after_open_fails_with_esrch` | delete | `runtime/process.rs::ownership_refresh_tests::reaped_stat::runner_process_esrch_stat_read_is_a_vanished_process` | Asserted kernel behavior; the cover unwraps the same precondition. |

##### src/runtime/recovery.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `unpublished_history::runner_recovery_machine_missing_paths_preserve_original_exited_and_failed_history` | trim | — | Shared scenario loop `Exited`/`Failed` → one status. |
| `unpublished_history::runner_recovery_operator_missing_paths_preserve_original_exited_and_failed_history` | trim | — | Shared scenario loop `Exited`/`Failed` → one status; the one kept operator scenario. |
| `unpublished_history::runner_recovery_machine_missing_paths_keep_committed_native_exit_code_signal_and_message` | trim | — | Shared scenario loop code/signal → one. |
| `unpublished_history::runner_recovery_operator_missing_paths_keep_committed_native_exit_code_signal_and_message` | delete | `runtime/recovery.rs::unpublished_history::runner_recovery_machine_missing_paths_keep_committed_native_exit_code_signal_and_message` | The attestation diagnostic stays in `runner_recovery_attestation_records_attributed_original_provenance_and_current_terminal_retries`. |
| `unpublished_history::runner_recovery_operator_artifact_loss_after_committed_runner_lost_creates_no_duplicate` | delete | `runtime/recovery.rs::unpublished_history::runner_recovery_machine_artifact_loss_after_committed_runner_lost_creates_no_duplicate` | Same as above. |
| `unpublished_history::runner_recovery_terminal_bookkeeping_faults_rollback_atomically_then_retry_without_events` | trim | — | Drop one of the `"diagnostic"`/`"gap"` triggers (one statement). |
| `unpublished_relationships::runner_recovery_exceptional_zero_event_takeover_refuses_actual_director_fleet_change_atomically` | delete | `runtime/recovery.rs::unpublished_relationships::runner_recovery_exceptional_zero_event_takeover_refuses_actual_member_fleet_change_atomically` | One relationship per stage stays. The conjunct matrix moves to a new direct `historical_ownership`/`validate_owner` test in `broker/runtime/mod.rs`. |
| `unpublished_relationships::runner_recovery_exceptional_zero_event_takeover_refuses_actual_fleet_director_change_atomically` | delete | `runtime/recovery.rs::unpublished_relationships::runner_recovery_exceptional_zero_event_takeover_refuses_actual_member_fleet_change_atomically` | Same predicate; conjuncts move to the new broker test. |
| `replay::runner_recovery_validates_all_complete_frames_before_any_replay_and_preserves_corruption` | trim | — | 4 damages → 1-2 (scan rejections are writer.rs's). |
| `replay::runner_recovery_preserves_recorded_terminal_outcomes_through_first_reconciliation` | delete | `runtime/recovery.rs::terminal_tail::runner_recovery_terminal_empty_incomplete_suffix_is_retired_without_new_events` | Add `vec![]` to the cover's suffix list. |
| `replay::runner_recovery_sequence_allocation_uses_current_reserved_ceiling_and_complete_spool_maximum` | delete | `runtime/recovery.rs::replay::runner_recovery_replays_opening_and_obscurity_before_one_monitor_closure_and_loss` | Its spool-maximum half never ran (seeded seq below the ceiling). |
| `terminal_tail::runner_recovery_terminal_empty_incomplete_suffix_is_retired_without_new_events` | trim | — | 3 statuses × 2 suffixes → 2 cases, plus the empty-suffix case from the deleted replay test. |
| `terminal_tail::runner_recovery_terminal_committed_originals_before_tail_keep_zero_new_counts_and_truthful_exit` | trim | — | Code/signal loop → 1. |
| `descriptor::runner_recovery_absent_descriptor_and_completed_retry_are_idempotent` | merge | `runtime/recovery.rs::identity::runner_recovery_creator_positive_complete_startup_termination_allows_one_bookkeeping_loss` | Append the descriptor-absent check and the idempotent-retry `rows` check. |
| `descriptor::runner_recovery_replaced_descriptor_after_pin_is_preserved_with_incomplete_cleanup` | move | `runtime/artifacts.rs` `mod tests` | Rewrite as a direct `RecoveryDescriptor::pin`/`remove` test. |
| `descriptor::runner_recovery_symlink_after_pin_preserves_link_and_target_with_incomplete_cleanup` | move | `runtime/artifacts.rs` `mod tests` | Rewrite as a direct `pin`/`remove` test. |
| `descriptor::runner_recovery_in_place_descriptor_change_after_pin_is_preserved` | move | `runtime/artifacts.rs` `mod tests` | Rewrite as a direct `pin`/`remove` test. |
| `descriptor::runner_recovery_descriptor_appearing_after_absence_pin_is_preserved` | move | `runtime/artifacts.rs` `mod tests` | Rewrite as a direct `pin`/`remove` test. |
| `descriptor::runner_recovery_descriptor_disappearing_after_pin_is_safe_absence` | move | `runtime/artifacts.rs` `mod tests` | Rewrite as a direct `pin`/`remove` test. |
| `descriptor::runner_recovery_invalid_descriptor_identity_epoch_shape_or_permissions_fails_privately` | trim | — | 14 → 2 cases here. A new direct `pin` test in artifacts.rs takes one case per `ensure` (6). |
| `descriptor::runner_recovery_original_deadline_after_before_finish_keeps_descriptor_retryable` | delete | `runtime/recovery.rs::descriptor_cleanup::runner_recovery_expired_original_budget_prevents_actual_unlink_dispatch` | Identical body minus the unlink count (500 ms). |
| `replay_relationships::runner_recovery_finalization_after_replay_refuses_actual_director_fleet_change_atomically` | delete | `runtime/recovery.rs::replay_relationships::runner_recovery_finalization_after_replay_refuses_actual_member_fleet_change_atomically` | Same predicate; conjuncts move to the new broker test. |
| `replay_relationships::runner_recovery_finalization_after_replay_refuses_actual_fleet_director_change_atomically` | delete | `runtime/recovery.rs::replay_relationships::runner_recovery_finalization_after_replay_refuses_actual_member_fleet_change_atomically` | Same predicate; conjuncts move to the new broker test. |
| `replay_relationships::runner_recovery_valid_historical_replay_selects_current_archive_disposition` | delete | `runtime/recovery.rs::replay_history::runner_recovery_archives_each_original_and_synthesized_event_by_current_lifecycle` | Same per-lifecycle archive assertions. |
| `delivery_results::runner_recovery_replay_publishes_each_uncertain_result_with_its_receipt_cause` | trim | — | Drop the tautological `events(..).len() == report_events(..)` assert. |
| `delivery_results::runner_recovery_replay_publishes_a_causeless_result_for_a_receipt_reconciled_before_publication` | trim | — | Drop the same tautological assert and the `report_events` helper. |
| `identity::runner_recovery_borrowed_expectations_reject_stable_run_and_concrete_placement_mismatches` | trim | — | 19 → 3 cases (one run field, `member`, one placement field); add the direct `stable_run_identity` unit test. |
| `clock::runner_recovery_short_outer_cap_clips_every_actual_cleanup_boundary` | delete | `runtime/lifecycle.rs::runner_recovery_clock_clips_each_original_boundary_and_never_renews_time` | ~260 ms of sleeps; the borrowed 3.3 s case clips end to end. |
| `completion::runner_recovery_same_owner_completion_before_ordinary_takeover_has_zero_effects` | trim | — | Shared `immediate_completion` helper: 3 statuses → 1. |
| `completion::runner_recovery_same_owner_completion_before_exceptional_takeover_has_zero_effects` | trim | — | Shared `immediate_completion` helper: 3 statuses → 1. |
| `completion::runner_recovery_completed_foreign_owner_still_rejects_fixed_snapshot_and_attestation` | trim | — | Shared `immediate_completion` helper: 3 statuses → 1. |
| `completion::runner_recovery_ordinary_completion_is_rechecked_after_actual_write_lock_wait` | trim | — | Shared `completion_behind_write_lock` helper: 3 statuses → 1. |
| `completion::runner_recovery_exceptional_completion_is_rechecked_after_actual_write_lock_wait` | trim | — | Shared `completion_behind_write_lock` helper: 3 statuses → 1. |
| `relationships::runner_recovery_ordinary_takeover_refuses_actual_director_fleet_change_atomically` | delete | `runtime/recovery.rs::relationships::runner_recovery_ordinary_takeover_refuses_actual_member_fleet_change_atomically` | Same predicate; conjuncts move to the new broker test. |
| `relationships::runner_recovery_ordinary_takeover_refuses_actual_fleet_director_change_atomically` | delete | `runtime/recovery.rs::relationships::runner_recovery_ordinary_takeover_refuses_actual_member_fleet_change_atomically` | Same predicate; conjuncts move to the new broker test. |

##### src/runtime/service.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `control_service::control_stale_start_preserves_running_child_health_and_manual_io` | merge | `runtime/service.rs::control_service::control_invalid_protocol_versions_leave_the_running_child_available` | Its rejection runs against the destination's fixture (−2 child+PTY spawns). |
| `control_service::control_repeated_creator_start_preserves_child_health_and_manual_io` | merge | `runtime/service.rs::control_service::control_invalid_protocol_versions_leave_the_running_child_available` | Its rejection runs against the same fixture (−1 spawn). |
| `observation_contract::service_started_health_reports_observation_without_composer_authority` | delete | `runtime/service.rs::observation_contract::service_sqlite_and_writer_failures_keep_health_manual_display_and_resize_live` | Key absence is enforced by every typed `health()` (`deny_unknown_fields`); −1 child spawn. |
| `input_admission_tests::client_poll_proves_first_byte_before_later_completion` | delete | `runtime/service.rs::input_admission_tests::client_empty_poll_supplies_no_admission_time` | Identical sequence after an extra empty poll. |

##### src/runtime/system.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `timed_runner_kills_and_reaps_a_sleeping_direct_child_within_five_seconds` | speed | — | C1.8: 1 s → 2 s deadline with `PROCESS_FIXTURE_LOCK` removed, so the four cases run in parallel. |
| `timed_runner_checks_deadline_while_both_streams_keep_producing` | speed | — | Same as the first C1.8 case. |
| `timed_runner_applies_deadline_after_child_exit_with_inherited_pipes_open` | speed | — | Same as the first C1.8 case. |
| `timed_runner_waits_for_child_exit_even_after_both_pipes_close` | speed | — | Same as the first C1.8 case. |
| `runner_preserves_binary_not_found_classification` | trim | — | `[None, Some(1)]` → 1 (both fail inside `spawn_piped`). |

##### src/runtime/transport.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `drain_eof_is_not_a_quiet_read_and_adds_no_manual_generation` | merge | `runtime/transport.rs::drain_interrupted_reads_require_a_later_actual_same_round_wouldblock_pair` | Loop over `[(Read::Interrupted, ..), (Read::Eof, ..)]`; keep the seal and `assert_bounds` for both. |
| `drain_opaque_proof_is_single_use_and_diagnostic_copies_supply_no_authority` | delete | `runtime/transport.rs::drain_actual_quiet_round_seals_health_without_granting_composer_eligibility` | The copy half tests Rust `Copy`; single-use is asserted in the cover. |
| `drain_pending_reply_prefix_blocks_the_proof_with_terminal_reply_pending` | merge | `runtime/transport.rs::drain_completed_reply_prefix_seals_a_proof_with_unchanged_generation` | Its blocker, `pty.bytes`, `pending_input_bytes` and generation asserts go after the first drain. |
| `manual::reply_classifier_overlong_csi_in_one_read_counts_every_byte` | delete | `runtime/transport.rs::manual::reply_classifier_overlong_csi_is_manual_at_the_byte_exceeding_the_limit` | Same parameter-limit arm; one read equals many. |
| `manual::reply_classifier_bracketed_paste_split_across_reads_counts_every_byte` | delete | `runtime/transport.rs::manual::reply_classifier_bracketed_paste_counts_every_byte_including_reply_forms` | `feed` is a per-byte sum; chunking has no effect. |
| `invalid_continuation::reply_classifier_bel_ends_an_osc_reply` | delete | `runtime/transport.rs::allowlist::reply_classifier_osc_replies_terminate_with_bel_or_st` | Same OSC-BEL branch. |
| `pending::reply_classifier_reply_fed_one_byte_per_read_counts_zero` | delete | `runtime/transport.rs::pending::reply_classifier_reply_split_at_every_boundary_counts_zero` | The cover checks every prefix of the same replies. |
| `pending::reply_classifier_backdate_ages_a_pending_prefix_to_the_bound` | delete | `runtime/transport.rs::pending::reply_classifier_lone_escape_resolves_as_manual_at_the_bound` | Tested the `#[cfg(test)]` `backdate`; it stays exercised end to end. |
| `runner_transport_keystrokes_still_advance_manual_generation` | delete | `runtime/transport.rs::runner_transport_one_tick_drains_a_split_outer_reply_and_writes_it_in_one_offer` | Generation equals the manual byte count there and in `drain_fragmented_reads..`. |
| `runner_transport_outer_eof_after_a_complete_reply_adds_no_manual_generation` | delete | `runtime/transport.rs::runner_transport_outer_eof_counts_a_pending_reply_prefix_as_manual` | Finish-with-nothing-pending stays in `reply_classifier_finish_without_a_pending_prefix_counts_zero`. |
| `runner_transport_lone_escape_counts_as_manual_once_its_prefix_bound_passes` | merge | `runtime/transport.rs::runner_transport_passes_escape_sequences_unframed` | Add `progress.human_bytes == 0` to the destination's second tick. |

##### src/runtime/writer.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `outbox_truncate_failure_preserves_committed_originals_for_retry` | delete | `runtime/writer.rs::terminal_tail::runner_recovery_terminal_tail_truncate_failure_retains_bytes_and_retries_the_same_owner` | `compact_committed` has no tail-specific branch. |
| `refresh::writer_refresh_newer_revision_survives_an_inflight_failed_attempt` | merge | `runtime/writer.rs::refresh::writer_refresh_newer_revision_survives_an_inflight_successful_attempt` | `for fail in [false, true]`; byte-identical bodies. |
| `writer_starts_with_the_exact_private_recorded_outbox` | delete | `runtime/writer.rs::writer_spool_sync_is_pending_until_the_actual_sqlite_commit` | Starts the same outbox and asserts `finish == (0, 0)` after real work. |

##### src/time.rs

| Test (module path from `mod tests`) | Action | Covering / destination test | Note |
|---|---|---|---|
| `format_emits_zero_microseconds_as_000000` | merge | `time.rs::format_emits_six_digit_microseconds_and_offset` | Becomes a row of one `%.6f` table. |
| `format_zero_pads_short_microsecond_values` | merge | `time.rs::format_emits_six_digit_microseconds_and_offset` | Becomes a row of the same table. |
| `nested_spans_restore_the_outer_mask` | delete | `time.rs::a_span_restores_signals_blocked_before_it_began` | An outer span is a pre-blocked mask on the same `SIG_SETMASK` restore. |

##### Kept tests

| Test | Area | Reason |
|---|---|---|
| `runtime/recovery.rs` `crash::runner_recovery_hard_kill_before_writer_append_records_loss_without_inventing_an_observation` | Recovery crash tier | No surviving test covers it; it stays. |
| `coding_agent/detector.rs` `bundled_idle_composers_are_verified_non_interactions_distinct_from_the_form` | Detector idle composers | It stays as the owner test of idle classification; `coding_agent/catalog.rs::bundled_backends::episodes_close_not_visible_once_the_idle_composer_holds_250_ms` is deleted instead. |

##### Renamed rewrites

Each test below is rewritten with the item that changes its behavior and keeps its scenario under a new name; it is neither deleted nor a ledger row.

| Test at the branch point | Test at the branch head | Rewritten by |
|---|---|---|
| `multiplexer/mod.rs::press_target_validation_rejects_a_wrong_tmux_tuple_and_ambiguous_herdr_target` | `multiplexer/mod.rs::press_target_validation_reads_only_the_recorded_target_on_the_recorded_server` | D4 |
| `assets.rs::hermes::hermes_setup_issues_the_complete_config_call_sequence` | `assets.rs::hermes::hermes_setup_writes_serially_then_rereads_plugins_once_each_with_its_own_budget` | A2, A3 |
| `assets.rs::hermes::hermes_setup_without_a_runner_installs_skills_records_the_row_and_reports_the_skip` | `assets.rs::hermes::plain_setup_without_hermes_installs_skills_records_the_row_and_reports_the_skip` | A1 |
| `assets.rs::installation_prints_skills_then_presets_then_extensions_then_hermes_merges` | `assets.rs::installation_prints_skills_then_presets_then_extensions_then_hermes_enablement` | D6 |
| `runtime/input_client.rs::exclusive_receipt_absent_placement_requires_real_inactive_history_and_a_matched_pair` | `runtime/input_client.rs::exclusive_receipt_absent_placement_requires_real_inactive_history` | B12, which drops the pairing loop |

##### Totals

| File | delete | merge | move | trim | speed |
|---|---|---|---|---|---|
| `src/assets.rs` | 4 | 0 | 0 | 8 | 0 |
| `src/broker/fleets.rs` | 0 | 0 | 0 | 2 | 0 |
| `src/broker/members.rs` | 0 | 0 | 0 | 4 | 0 |
| `src/broker/messaging.rs` | 1 | 1 | 0 | 0 | 0 |
| `src/broker/runtime/events.rs` | 10 | 0 | 0 | 22 | 0 |
| `src/broker/runtime/evidence.rs` | 1 | 0 | 0 | 3 | 0 |
| `src/broker/runtime/inputs.rs` | 4 | 3 | 0 | 16 | 5 |
| `src/broker/runtime/mod.rs` | 0 | 0 | 0 | 1 | 2 |
| `src/cli/fleet.rs` | 1 | 0 | 2 | 3 | 0 |
| `src/cli/member.rs` | 8 | 9 | 2 | 8 | 0 |
| `src/cli/message.rs` | 2 | 3 | 0 | 5 | 2 |
| `src/cli/mod.rs` | 1 | 1 | 0 | 4 | 0 |
| `src/cli/runtime.rs` | 2 | 6 | 0 | 4 | 0 |
| `src/cli/setup.rs` | 2 | 1 | 0 | 12 | 3 |
| `src/coding_agent/catalog.rs` | 25 | 7 | 16 | 31 | 0 |
| `src/coding_agent/detector.rs` | 11 | 2 | 1 | 18 | 0 |
| `src/coding_agent/hermes_settlement.rs` | 1 | 0 | 0 | 1 | 1 |
| `src/coding_agent/mod.rs` | 18 | 7 | 0 | 9 | 0 |
| `src/db.rs` | 2 | 1 | 0 | 6 | 3 |
| `src/delivery.rs` | 4 | 1 | 0 | 2 | 1 |
| `src/multiplexer/herdr.rs` | 1 | 0 | 0 | 0 | 0 |
| `src/multiplexer/mod.rs` | 0 | 4 | 0 | 0 | 0 |
| `src/output/json.rs` | 0 | 1 | 2 | 1 | 0 |
| `src/output/text.rs` | 0 | 1 | 0 | 0 | 0 |
| `src/pane_lock.rs` | 1 | 0 | 0 | 1 | 1 |
| `src/runtime/artifacts.rs` | 0 | 1 | 0 | 1 | 0 |
| `src/runtime/control.rs` | 0 | 9 | 0 | 2 | 0 |
| `src/runtime/input.rs` | 7 | 8 | 0 | 10 | 3 |
| `src/runtime/input_client.rs` | 10 | 1 | 1 | 14 | 0 |
| `src/runtime/lifecycle.rs` | 4 | 2 | 0 | 2 | 0 |
| `src/runtime/observation.rs` | 5 | 5 | 0 | 4 | 6 |
| `src/runtime/press.rs` | 0 | 0 | 0 | 0 | 1 |
| `src/runtime/process.rs` | 1 | 0 | 0 | 1 | 2 |
| `src/runtime/recovery.rs` | 13 | 1 | 5 | 16 | 0 |
| `src/runtime/service.rs` | 2 | 2 | 0 | 0 | 0 |
| `src/runtime/system.rs` | 0 | 0 | 0 | 1 | 4 |
| `src/runtime/transport.rs` | 8 | 3 | 0 | 0 | 0 |
| `src/runtime/writer.rs` | 2 | 1 | 0 | 0 | 0 |
| `src/time.rs` | 1 | 2 | 0 | 0 | 0 |
| **Total** | **152** | **83** | **29** | **212** | **34** |

##### Helper consolidation

- Create one configurable `FakePty`/`FakeDisplay` in `transport::test_support` replacing transport.rs `Bytes`/`Pty`/`Output` and `Reader`/`DrainPty`/`DrainOutput`, input.rs `manual_progress::{Human, Pty, Output}`, the `ordinary_prompt::support` fakes and `StageIo`.
- Add `Endpoints { human, pty, output }`, `Harness::drain_with` and `Harness::tick_with` in transport.rs `mod tests` for the 33 drain and 7 tick call sites.
- Drop the redundant `h.control()` seal from the ~20 transport drain tests that later seal or reach `InputDrain::Quiescent`.
- Rewrite transport `drain_real_pending_writer_commit_blocks_health_until_release_and_persistence` over `h.hooks.wait`, `writer::test_support::wait_until` and `committed`.
- Move `input_client::test_support::{TypedHost, Termination, BoundaryAction}` into input_client.rs `mod tests`, their only user.
- Put one recording `NoLiveHost` in `input_client::test_support` replacing `tests::NoLiveHost`, `permission_history::entry::NoLive` and `delivery.rs` `NoDeliveryHost`.
- Collapse input_client.rs duplicate helpers: `recovery_notified_at` → `notified_at`, and reuse `member_placement`, `assert_owner_only` and `opened_interaction` at their retyped sites.
- Add `db::test_support::record_read_budgets(conn, table)` with an authorizer-clearing drop guard for the 5 input_client.rs sites and the delivery.rs site.
- Add `runtime::test_support::Fixture::running_owner()` for the running-owner setup copied in input_client.rs and 11 other runtime sites, plus `mark_exited(conn, run_id)`.
- Share one PTY `read_until(&[&str])` helper in process.rs `mod tests`.
- Build input.rs `support::RunnerFixture` on `writer::test_support::WriterFixture::new`/`start`.
- Use `coding_agent::catalog::test_support::VERSION` instead of input.rs `support::VERSION`.
- Extract input.rs `support::coordinator(fixture, lock, catalog, hooks)` for the 3 `InputCoordinator::new` copies, and reuse `current_ids` for `question::opened_ids`.
- Extract the input.rs copy-paste table into `support` helpers (`Host::message`, `Host::notified_at`, `Host::reply_until`, `Host::quiesce_once`, `turns_until`, `fail_head_write`, `with_composer`, `Host::placement`, `fail_manual_accounting`, `BLOCKERS`, `delivered()`, `Host::submit`, `busy_for`) and reuse `until_head_literal`, `hold_turns`, `counts` and one `offers`.
- Replace input.rs `method_rendering::preview_digest` with `storage::input_payload_digest`.
- Add an `assert_not_impl!` macro beside `assert_no_debug_or_serialize!` at the top of `main.rs` for the cacheless-proof trait checks.
- Extract observation.rs helpers: `quiet(..)` in `tests`, `opened()` in `test_support`, a path helper on `ServiceFixture`, `stalled(stall)`, a generalized `CaptureFixture::pair`, and `assert_detector_gap`.
- Reuse `writer::test_support::WriterFixture`, `start` and `running` for the 3 inline observation.rs controller setups, and drop most `catch_unwind` cleanup while keeping `writer.finish() == (0, 0)`.
- Replace `observation::native_capture::test_support::Clock` with `writer::test_support::Clock`.
- Keep one copy of observation.rs `inspection_tests::opencode_form` and input.rs `opencode_command` in `coding_agent::detector::test_support`.
- Move `observation::test_support::synthetic` to `service::test_support::observation_services`.
- Move the observation helpers in `writer::test_support` (`Clock`, the form builders, `redacted_policy`, `configure`, `running`, `VERSION`/`NOW`) to `observation::test_support`.
- Reduce the `LIFECYCLES` loop in recovery.rs `test_support::relationships::historical_control` to one lifecycle.
- Use `writer::test_support::frame_bodies` instead of the hand-parsed frames in recovery.rs `replay::runner_recovery_fsynced_synthesis_from_an_interrupted_pass_counts_as_next_pass_input`.
- Extract recovery.rs `seed_unavailable(case)`, `fail_next(conn, Option<&str>)` and `Case::with_owner_lock`, and reuse `test_support::descriptor::identity` for `unpublished_history::metadata`.
- Replace recovery.rs's four `member_input_operations` inserts and three `input_delivery_result` queries with one builder and one query helper in `test_support`.
- Apply the recovery.rs/artifacts.rs/lifecycle.rs copy-paste table: `missing_lock`, `persist_opening`, `claim(run)`, `bump_epoch(conn, run_id)`, `Case::pane_lock()`, an artifacts `lock_transfer` fixture fn and lifecycle `launch_case()`.
- Reuse writer.rs `test_support::committed`/`wait_until` at the inline sites and add `compact`, `try_open` and `persist` beside `open()`.
- Make writer.rs `independent_frames` call `frame_bodies` (one frame walker).
- Give control.rs `test_support::activate` a column parameter for its two inline variants.
- Move `SCREEN_CHANGES`, `first_masked_or_empty_row` and `push_mask` into catalog.rs `bundled_backend_files` (C1.1 deletes the guard helpers), and fold `bundled_codex` into `bundled_codex_auto`.
- Delete catalog.rs `test_support::submission_fixture_ids` and the codex-tracker-only helpers `redacted`, `replay_run`, `operations`, `sorted`.
- Collapse detector.rs's five `*_observation` helpers into `detector::test_support::observe_bundled(backend, version, &screen)`.
- Keep one `assert_primary_matches_alternate` in `detector::test_support`, replacing catalog.rs `assert_same_evidence`.
- Add `screen_of(bytes)`, `fixture_lines(id)`, `fixture_size(id)` and `replay_as_captured(id, change, cursor)` to `detector::test_support`.
- Call the `assets` install helpers (made `pub(crate)` or wrapped in `assets::test_support`) from setup.rs instead of its copies, deriving file lists from `embedded_extension`.
- Add one `coding_agent::test_support::FakeProbe` replacing setup.rs `Probe`/`Resolved` and `coding_agent` `Probe`/`hermes::HermesProbe`.
- Drop the SKILLS read-back from assets.rs `hermes::install_with`.
- Replace assets.rs `extension_targets`'s prefix filter with `embedded_extension(prefix)`.
- Re-export the production config-key consts instead of assets.rs `test_support` re-declarations.
- Add `Fixture::redacted()` and `redacted_policy(patterns)` to `broker::runtime::test_support`, reused by events.rs and evidence.rs.
- Add `Fixture::running_sibling(name)` to `broker::runtime::test_support` for the three hand-built second runs.
- Move `broker::runtime::test_support::automatic_review` into `events::tests`.
- Build cli.rs `NewRun` literals from `broker::runtime::test_support::Fixture::spec`, unify the usage-error helpers as `cli::test_support::assert_usage_rejected`, reuse `fleet_deleted()`, and share one child-spawn helper for `isolated_cli_with`/`run_guarded_child`.
- Hoist `RUNTIME_KEYS` and `INPUT_KEYS` consts to output/json.rs `mod runtime` scope.
- Use delivery.rs `settings_for` in `deferred_preview`.
- Rewrite messaging.rs `test_support::broker_fixture` over `migrated_conn`, `create_fleet_fixture` and `register_member`, and extract `install_million_row_messages_view` in `listen_snapshot`.

##### Rule-violation fixes

- Delete `runtime/observation.rs` `native_capture::verification::tests::driver::no_tip_access_prompt_runs_touch_on_the_outside_directory_with_bash`: it asserts the harness's own prompt text.
- Delete `runtime/observation.rs` `native_capture::verification::tests::driver::batched_read_prompt_names_two_outside_files_in_one_turn`: harness prompt text.
- Delete `runtime/observation.rs` `native_capture::verification::tests::driver::batched_bash_prompt_runs_the_probe_helper_twice_in_one_turn`: harness prompt text.
- Delete `runtime/observation.rs` `native_capture::verification::tests::driver::sidebar_prompt_edits_the_untracked_probe_before_the_bash_probe`: harness prompt text.
- Delete `runtime/observation.rs` `native_capture::verification::tests::capture_ignored_entry_requires_explicit_configuration_before_launching`: it re-runs the test binary to check an ignored test's own `expect` message.
- Delete `runtime/observation.rs` `controller_tests::failure_handling::observation_failures::runner_observation_hooks_install_once_before_the_matching_native_start`: it tests the `#[cfg(test)]` hook installer's guard.
- Delete `runtime/writer.rs` `worker::writer_hook_timeout_releases_the_mutex_before_unwinding_through_cleanup`: only test `Hooks`/`Release` code runs.
- Delete `runtime/writer.rs` `worker::writer_hook_cleanup_releases_known_state_after_a_worker_poison_without_panicking`: only test harness code runs.
- Delete `runtime/system.rs` `test_child::owned_child_guard_cleans_up_when_identity_setup_unwinds`: it drives only `TestChild`.
- Replace input.rs `method_rendering::DISMISSAL` with bytes built by `crate::delivery::fallback_preview_line` (C1.6 removes `staged_delivery::SETTLE`).
- Drop the `'█'`-at-`trailer_col` fixture-layout assertion from observation.rs `inspection_tests::opencode_form` and input.rs `opencode_command`.
- Drop the fixture `cursor_col == 120` and row-width preconditions from detector.rs `replay_with_pending_wrap`, keeping the live pending-wrap assertion.
- Delete catalog.rs `test_support::BUNDLED_BOUNDARIES` once the ledger's `coding_agent/mod.rs` `native_version_probe::native_launch_identity_*` trims and catalog `profile_thresholds` rows no longer read it.
- Stop asserting embedded skill ids and copying `install_agent`'s root derivation in assets.rs `hermes::skills_line`.
- Drive db.rs `runtime_tests` domain loops (interaction classes, close reasons, dispositions, `archive_reasons_accept_the_specified_domain`) from `stored_enum!` `VALUES` instead of the hand-copied baseline `IN (...)` lists.
- Move `ZeroByteRetirement::fixture` into its own `#[cfg(test)] impl` block beside the type.
- Move the atomicity half of broker/fleets.rs and broker/members.rs tests that drive `cli::guarded_delete_fleet` to cli/fleet.rs and call `delete_fleet_rows` for the rest, removing the `cli/mod.rs` `#[cfg(test)]` re-export.
- Delete the historical-residue comment "The descendant is deliberately bounded even against the old blocking runner." in system.rs `mod tests`; its `sleep 6` stays.

### Workstream D — No CAFleet environment variables; Hermes plugin kept

#### D1. Decisions

`terminal.client_env_passthrough` exists only because Hermes runs every session's terminal tools in one shared gateway, so the process inheritance that delivers `CAFLEET_MEMBER_ID`, `CAFLEET_DATABASE_URL` and the multiplexer variables to Claude, Codex and OpenCode tool commands does not reach a Hermes tool. Instead of re-creating that inheritance for Hermes, CAFleet stops depending on those variables on every backend, and the user extended the removal to every user-facing `CAFLEET_*` setting; test-only `CAFLEET_TEST_*`, `CAFLEET_REVIEW_*` and `CAFLEET_NATIVE_CAPTURE_CONFIG` names stay inside `#[cfg(test)]`:

| Global Hermes edit | Decision | Reason |
|---|---|---|
| `terminal.client_env_passthrough` (11 names) | Removed: setup no longer reads or writes it, and no preflight or doctor row checks it | D2–D5 remove every reader of the names on every backend |
| `<hermes home>/plugins/cafleet/` | Kept | The gateway loads only user, bundled or entry-point plugins; it is the only route to the Hermes Director's tier-2 `cafleet_listen` reception |
| `plugins.enabled` += `cafleet` | Kept | Non-bundled plugins load only when listed here |
| `plugins.entries.cafleet.allow_gateway_injection: true` | Kept | Hermes's consent for plugin-started turns |

Entries that earlier setups appended to an operator's `terminal.client_env_passthrough` stay where they are: setup no longer touches the key, and a listed name that no client exports is simply unset in each session.

| Former variable use | Replacement | Section |
|---|---|---|
| `CAFLEET_MEMBER_ID` marks a member process for the Director-only guard | Required `--from-member-id` checked against `fleets.director_member_id` | D2 |
| `CAFLEET_MEMBER_ID` makes the Hermes plugin and the OpenCode extension refuse member sessions | The tools take the literal Director ID and verify it is a root Director | D3 |
| `TMUX`, `TMUX_PANE`, `HERDR_*` on every Director command | The fleet Director's recorded placement plus a recorded multiplexer server | D4 |
| `CAFLEET_DATABASE_URL` selects the database for every process | Global `--database-url` option; omitted means the default database | D5 |
| `CAFLEET_MULTIPLEXER` overrides multiplexer detection | Nothing: `fleet create` and doctor auto-detect from `TMUX`/`HERDR_ENV` only | D4 |
| `CAFLEET_MAX_TEXT_LEN` sets the text-output body limit | Nothing: the limit is fixed at 200 codepoints; `--json` carries full bodies | D9 |

#### D2. Director-only commands carry the caller's literal ID

- `member prompt`, `member exec`, `member press`, `member recover` and `input reconcile` gain a required `--from-member-id <director-member-id>` (clap `value_parser!(i64).range(1..)`); `permission respond` already requires it as its pinned Director.
- `refuse_director_only` and its environment read are deleted. After argument parsing and the invocation connection's schema-version guard, and before any multiplexer, lock, socket, file access or write, `require_director(conn, command)` resolves the subject's fleet (member → its fleet; input → its run's fleet; interaction → its run's fleet) and requires that fleet to be live with `director_member_id == --from-member-id`. Otherwise the command exits 1 with nothing on stdout (`permission respond` included, as today) and:

```
Error: 'cafleet <command>' is a Director-only command and member <id> is not the root Director of fleet <fleet_id>; send the request to your Director.
```

- An unknown subject keeps its existing not-found error, which precedes this check.

- Doctor's next-command hints for `member recover` name the caller: `cafleet member recover <member-id> --from-member-id <director-id>`, with the member's fleet Director ID filled in literally; a summary line keeps `ID` literal for the members and fills in the Director ID when every listed member shares one fleet Director, keeping `<director-id>` literal otherwise.
- `SPEC.md` § Director-only command guard states the guard as cooperative: it stops a member that passes its own `YOUR MEMBER ID:` (or calls with no ID, a parse error); a member that copies `DIRECTOR MEMBER ID:` passes it. Member authority remains defined by the member role and the Director's assignment. A Director-only command the Director dispatches into a member pane with `member exec` is checked the same way, by the ID it carries.

#### D3. Plugin and extension tools verify a literal Director ID

- A Director-only tool verifies its `director_id` argument by running the recorded executable as `<cafleet> member show <director_id> --json` (adding `--database-url <url>` when the tool received one, D5) and requiring exit 0 with `"kind": "director"`; any other outcome returns the tool's existing member-refusal error with the CLI's stderr appended. `message listen` keeps accepting any active member, so the check lives in the tools.
- Hermes plugin `cafleet_listen` (`extensions/hermes/cafleet/__init__.py` `_listen`): delete the client-environment reads — the `CAFLEET_MEMBER_ID` check, the `CAFLEET_DATABASE_URL` copy, both client-environment refusals (`bound_client_env()` is `None`; an empty `tool_env()`) and the `agent.session_client_env` seam, because without the passthrough names `tool_env()` is empty on a home lacking CAFleet's names and the literal `director_id` verification replaces them; keep the session-key, authority (attachment) seam, injection-seam and consent checks; the listen child runs with the gateway environment minus `CAFLEET_*` names, plus `--database-url` when given. The tool schema gains an optional `database_url` string.
- OpenCode extension (`extensions/opencode/cafleet.ts`): delete `isMember` and the early `return () => undefined`; `cafleet_listen` verifies its `director_id`; `cafleet_goal_status`, `cafleet_goal_progress` and `cafleet_goal_complete` gain a required `director_id` and verify it the same way; every tool gains an optional `database_url`. The `context` hook registers in every session; it and the event loop act only on a session holding a goal stored by the operator-typed `/cafleet-goal` command or by a goal tool whose `director_id` verified.
- `skills/cafleet/reference/coding-agents.md` § hermes and § opencode reception recipes pass the literal Director ID (already) and, for a non-default database, `database_url`.

#### D4. Multiplexer context from the recorded Director placement

- Schema: `member_placements` gains `mux_server TEXT NOT NULL`, the server every CAFleet invocation for that fleet targets explicitly. `migrations/V1__baseline.sql` is edited in place.

| Backend | `mux_server` holds | Every CAFleet invocation after `fleet create` |
|---|---|---|
| tmux | The absolute server socket path | `tmux -S <mux_server> …` |
| herdr | The herdr session name (`default` for the default session) | `herdr --session <mux_server> …` |

  herdr evidence (herdr 0.9.5, checked 2026-11-06 on the reference host): with every `HERDR_*` variable unset, `herdr --session default pane list` and `herdr --session default pane current --pane w2:p8R` returned the live session's panes, and `herdr session list` prints each session's name, status, directory and socket. herdr also honored `HERDR_SOCKET_PATH`, but `herdr --help` documents only `--session`, so CAFleet uses `--session`.

- `fleet create` is the only command (with `doctor`'s `multiplexer` row) that reads the caller's multiplexer environment. Synopsis: `cafleet fleet create --name NAME --coding-agent AGENT [--pane PANE_ID [--mux-server SERVER]] [--json]`.

| Input | Source, first match wins |
|---|---|
| Backend | `TMUX`/`HERDR_ENV` auto-detection only (`resolve_multiplexer_name` loses its override argument) |
| `mux_server` (tmux) | `--mux-server`; else everything before the last two commas of a non-empty `TMUX` (`rsplitn(3, ',')`, so a socket path containing `,` survives); else `tmux display-message -p '#{socket_path}'` (the default server) |
| `mux_server` (herdr) | `--mux-server`; else a non-empty `HERDR_SESSION`; else the `herdr session list` row whose socket equals `HERDR_SOCKET_PATH`; else `default` |
| Director pane | `--pane`; else `TMUX_PANE` (tmux); else, for herdr, `herdr --session <server> pane current` run with the caller's environment, only when `HERDR_PANE_ID` is set — with it unset, `pane current` returns the focused pane, so `fleet create` refuses and asks for `--pane` instead of recording it |

- The pane is then read on the chosen server — `tmux -S <server> display-message -p -t <pane> '#{session_name}|#{window_id}|#{pane_id}'` or `herdr --session <server> pane current --pane <pane>` — and its session, window and pane are recorded. tmux prints `||` and exits 0 for a missing pane, so the missing-pane refusal follows from an empty field or a returned `pane_id` that differs from the requested pane, not from the exit status. Failures exit 1 before any write:

```
Error: no tmux server found; pass --mux-server <socket path>
Error: pane <pane-id> was not found on the <backend> server <server>; pass --pane and --mux-server for the Director's pane
Error: cafleet fleet create cannot identify its herdr pane because HERDR_PANE_ID is not set; pass --pane <pane-id>
Error: cafleet fleet create cannot identify its pane inside a Hermes session; pass --pane <pane-id>
```

  The last refusal applies when `HERMES_SESSION_ID` is set and `--pane` is absent, because a Hermes terminal tool runs in the shared gateway and may carry another pane's `TMUX_PANE`. A Hermes Director finds its pane ID with Hermes's leading-`!` local shell, which runs in the client process with the pane's own environment (`! echo $TMUX_PANE` or `! echo $HERDR_PANE_ID`), then runs `cafleet fleet create … --pane <id>`.
- Every other command that drives the multiplexer (`member create`, `member delete`, `fleet delete`, `member prompt/exec/press/capture`, runner recovery and Hermes settlement) builds it from the fleet Director's placement row (backend, `mux_server`, session, window, pane) with the per-backend targeting above.
- `member create` takes the split reference (session, window, Director pane) from the Director placement instead of `context_discovery`; each new member placement copies the Director's `mux_server`.
- `validate_key_target` drops the caller-session cross-check (`caller_context`, `herdr pane current`); it keeps the target-pane check against the recorded session and window. `ensure_available` for a recorded placement requires only the binary on `PATH`.
- `available_mux()` (no settings argument) remains for `fleet create` and `doctor`; every other caller switches to `mux_for_fleet(conn, fleet_id, deadline)`. The detection errors drop their `CAFLEET_MULTIPLEXER` advice:

```
ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; unset the one that does not belong to this session
no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session
```

#### D5. Database selection by explicit option

- `CAFLEET_DATABASE_URL` is removed. A global option `--database-url <url>` (clap `global = true`, accepted after the subcommand so prefix allow rules such as `Bash(cafleet message *)` keep matching) selects the database; omitted, the default `sqlite:///<HOME>/.local/share/cafleet/cafleet_<VERSION>.db` applies. URL form, relative-path resolution and the no-tilde-expansion rule move unchanged from `SPEC.md` § Environment variables, which is deleted, to a new § Database selection.
- The runner launches as `<cafleet> --database-url sqlite:///<canonical path> __member-run --descriptor <path>` (no environment entry); `CodingAgent::spawn_environment` keeps only the forwarded config-location variable.
- Spawn prompts gain the placeholder `{database_url}`, rendered by `member create` as `default` when the canonical database path equals the default path and as `sqlite:///<canonical path>` otherwise. A member whose `DATABASE URL:` line names a URL passes `--database-url <url>` on every `cafleet` command.
- Fail fast: `member create` against a non-default database refuses before registration when the prompt lacks `{database_url}`:

```
Error: the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url
```

- A member that omits the option on a custom-database fleet reaches the default database; when that file is absent it fails with `no cafleet database; run 'cafleet setup'`. When it exists, literal IDs could match another fleet; this is the same cooperative limit as D2, stated in `SPEC.md` § Database selection.
- `skills/cafleet/SKILL.md` § IDs adds `{database_url}` to the CLI's spawn placeholders, and every spawn-prompt template that renders the identity placeholders adds a `DATABASE URL: {database_url}` line.

#### D6. Removed surfaces

Delete each of these and every mention of it (code, tests, `SPEC.md`, `docs/`, `skills/`, README):

- `HERMES_CLIENT_ENV_PASSTHROUGH`, `merge_client_env_passthrough`, `read_passthrough`, `missing_member_names`, `passthrough_failure`, `hermes_client_env_names`, `hermes_missing_client_env_names`, `hermes_missing_names_phrase`, `SpawnProbe::hermes_client_env_passthrough` and the Hermes branch of `ensure_available`.
- The doctor row `hermes.client_env_passthrough`, its two summary phrases and the setup line `hermes: … terminal.client_env_passthrough …`.
- `SPEC.md` § Hermes environment passthrough, § Hermes availability preflight's passthrough rows, § Member environment rows 1–2, the whole § Environment variables section, the `CAFLEET_MULTIPLEXER` override rule and table column of the multiplexer resolution section, and the inheritance paragraph of § Director-only command guard (including the Codex `shell_environment_policy` dependency); `skills/cafleet/reference/cli.md` § Environment variables and the `skills/cafleet/SKILL.md` link to it.
- The in-session branch of `ensure_available` on a recorded server (`cafleet member commands must be run inside a <backend> session`), unreachable once detection requires a non-empty `TMUX`/`HERDR_ENV`; the binary-on-`PATH` check stays.

**Removed tests.** Every test whose subject is a removed surface is deleted with it, and the absence is the test. That set is exactly the tests that assert:

- the Hermes client-environment passthrough: its setup merge, re-read and skip lines, the availability preflight's passthrough rows, the doctor row and its summary phrases, and the `member create` refusal for a passthrough that lacks a CAFleet name;
- the `CAFLEET_MEMBER_ID` identity: the inherited-variable Director-only guard, the runner and run arguments that carried it, and its handoff to the native child (D2);
- the spawn environment that carried `CAFLEET_DATABASE_URL` and `HERMES_HOME` (`CodingAgent::spawn_environment`, D5);
- `Settings` read from `CAFLEET_*` variables, including `CAFLEET_MAX_TEXT_LEN` parsing, and the `CAFLEET_MULTIPLEXER` override of backend detection (D9);
- the in-session branch of a recorded server's `ensure_available` above, per backend.

#### D7. Resulting Hermes sequences

Setup for `hermes`, after the skills:

1. One concurrent batch: `hermes config path` and `hermes config get --json plugins` (A2). A `config path` failure fails setup with `failed to install extension into <resolved Hermes base>: <cause>`.
2. Install the extension into `<hermes home>/plugins/cafleet/` and print the extension line.
3. Evaluate `plugins` (A3): a `disabled` list naming `cafleet` fails with today's cause; append `cafleet` to `enabled` when missing (`config set plugins.enabled '<compact JSON>'`); set `plugins.entries.cafleet.allow_gateway_injection true` when not `true`; writes run serially.
4. When step 3 wrote, re-read `plugins` once and require both values.
5. Print `hermes: plugin cafleet already enabled -> <config path>` or `hermes: enabled plugin cafleet -> <config path>`.

Plain `cafleet setup` with `hermes` absent installs the skills, records the row and prints `hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes`.

Doctor's Hermes rows are `hermes.configuration`, `hermes.executable`, `hermes.extension`, `hermes.plugin_enabled` and `hermes.installation`; the extension and plugin rows share one batch (A2) and follow the A8 no-home rule.

#### D8. Documentation of the kept edits

`docs/docs/quickstart.md` § Configure states, in its backend table row and the `### Hermes` subsection, that `cafleet setup --coding-agent hermes` installs the plugin into `<hermes home>/plugins/cafleet/`, adds `cafleet` to `plugins.enabled` and sets `plugins.entries.cafleet.allow_gateway_injection` to `true` in the Hermes configuration, keeping today's `plugins.disabled` and plugin-grandfathering notes. Every passthrough sentence goes. No other docs page describes these edits, and no page describes a dedicated `HERMES_HOME` profile.

#### D9. Settings without environment variables

- `Settings` keeps only `database_url`, built from `--database-url` (D5); `Settings::from_env`, `from_lookup`, the `multiplexer` and `max_text_len` fields and their parse error `CAFLEET_MAX_TEXT_LEN must be a non-negative integer (got '<value>')` are deleted.
- Text output truncates message bodies at `const TEXT_BODY_LIMIT: usize = 200` codepoints in `src/output/text.rs`; every `format_*` caller drops its `max_text_len` argument. `SPEC.md` §6.4 Text-mode projection and § Message Body Truncation state the fixed 200 and point to `--json` for full bodies.
- Test children that set `CAFLEET_DATABASE_URL`, `CAFLEET_MULTIPLEXER` or `CAFLEET_MAX_TEXT_LEN` pass `--database-url` instead or drop the setting; the remaining `CAFLEET_TEST_*`, `CAFLEET_REVIEW_*` and `CAFLEET_NATIVE_CAPTURE_CONFIG` reads sit only in `#[cfg(test)]` code.

### Documentation updates

Implemented first, in this order, per `.claude/rules/documentation-maintenance.md`:

| Target | Sections |
|---|---|
| `SPEC.md` | § Environment variables deleted and a new § Database selection (D5, D9); every mention of `CAFLEET_MULTIPLEXER` or `CAFLEET_MAX_TEXT_LEN` — today the multiplexer resolution section (its override rule, table column and "set `CAFLEET_MULTIPLEXER` to disambiguate" advice, replaced by the D4 error texts), § Message Body Truncation, §6.4 Text-mode projection, §6.5's pane-notification limit paragraph and §7.3 Output / JSON / truncation — restated for auto-detection only and the fixed 200-codepoint limit (D4, D9); § Config-dir resolution (B15); § Positional subject ids and § Director-only command guard (D2); § `cafleet setup` (Asset installation, Hermes plugin enablement per D7; § Hermes environment passthrough deleted); § `fleet create` (synopsis with `--pane`/`--mux-server`, the D4 source table, its three errors); § `member create`, § Member environment and § Hermes availability preflight (D4–D6); § `cafleet doctor` (Check rows, Summary lines phrase table, Extension checks, Member runtime checks; § Hermes passthrough check deleted); every Director-only command synopsis (`--from-member-id`); § Hermes work settlement and the plugin consent paragraph (D3); § Launch policy and capability registry (B2); § Bundled profiles and native screen fixtures (B1); §8 Database schema (`mux_server`) |
| `docs/` | `docs/docs/quickstart.md` § Configure per D8, and its `### Hermes` subsection also gains the Hermes Director's `fleet create --pane` step (find the pane with `! echo $TMUX_PANE` / `! echo $HERDR_PANE_ID`); every page that walks through `fleet create` (today `docs/docs/mixed-backend-team.md`, `concepts/members.md`, `concepts/coding-agents.md`) gains `--pane`/`--mux-server` where it covers a Hermes Director; every page that names `CAFLEET_MEMBER_ID` or `CAFLEET_DATABASE_URL` (today `docs/docs/concepts/coding-agents.md`, `concepts/members.md`, `concepts/storage.md`, `contributing.md`) is rewritten to `--from-member-id`, `--database-url` and the recorded placement |
| `README.md` | Only where it shows a Director-only command, a custom database or the Hermes setup lines |
| `skills/` | `skills/cafleet/reference/cli.md` (Director-only synopses, `--database-url`, `fleet create --pane`, setup/doctor/`member create` facts, § Environment variables deleted); `skills/cafleet/SKILL.md` § IDs (`{database_url}`, the environment-variables link replaced by the `--database-url` fact) and § Send (fixed 200-codepoint text truncation); `skills/cafleet/roles/director.md` and `roles/member.md` wherever they show a Director-only command, a database or `fleet create` (the Hermes Director's `--pane` step); `skills/cafleet/reference/coding-agents.md` § hermes and § opencode rows and reception recipes that describe the passthrough, the member marker or the tool arguments, plus the Hermes Director's `fleet create --pane` step beside the Hermes reception recipe; every spawn-prompt template that renders the identity placeholders (D5) |
| `.claude/rules/commands.md` | The `//:test` thread cap (C1.11) only if it changes an invocation convention |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `SPEC.md` per § Documentation updates <!-- completed: 2026-11-06T23:56 -->
- [x] Update `docs/` per § Documentation updates and D8 <!-- completed: 2026-11-06T23:58 -->
- [x] Update `README.md` where it shows a Director-only command, a custom database or the Hermes setup lines <!-- completed: 2026-11-06T23:58 -->
- [x] Update `skills/` per § Documentation updates, including every spawn-prompt template <!-- completed: 2026-11-07T00:03 -->
- [x] Run `mise //docs:build` and fix broken links or tables <!-- completed: 2026-11-07T00:03 -->

### Step 2: Test-suite speed levers

- [x] Record the baseline: `time mise //:test`, and per-test timings with `RUSTC_BOOTSTRAP=1 <test binary> -Z unstable-options --report-time --format json` <!-- completed: 2026-11-07T00:20 -->
- [x] C1.1 rewrite the bundled guard test <!-- completed: 2026-11-07T00:39 -->
- [x] B1 static bundled catalog and test-only fixture verification, with C1.2 call-site migration <!-- completed: 2026-11-06T23:51 -->
- [x] C1.3 CTE prefill for the 4,096-row tests <!-- completed: 2026-11-07T00:39 -->
- [x] C1.4 test overrides for the Codex queue budget and retry interval <!-- completed: 2026-11-06T23:51 -->
- [x] C1.5 close-on-exec test pipes and PTYs <!-- completed: 2026-11-07T00:39 -->
- [x] C1.6 inject `stage_settle` through `InputServices` <!-- completed: 2026-11-06T23:51 -->
- [x] C1.7 memoized libtest digest <!-- completed: 2026-11-07T00:39 -->
- [x] C1.8 `ProcessFixture` temp dir and lock removal <!-- completed: 2026-11-07T00:39 -->
- [x] C1.10 pane-lock fixture and named completion grace <!-- completed: 2026-11-06T23:51 -->
- [x] C1.11 thread cap in the `//:test` task <!-- completed: 2026-11-06T23:51 -->
- [x] C2 shorten each listed real-time wait <!-- completed: 2026-11-07T01:14 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T00:42 -->

### Step 3: No CAFleet environment variables

- [x] D6 delete the passthrough merge, preflight, doctor row and their tests <!-- completed: 2026-11-07T00:01 -->
- [x] D7 Hermes setup sequence and skip line <!-- completed: 2026-11-07T00:01 -->
- [x] D2 `--from-member-id` on the Director-only commands and `require_director` <!-- completed: 2026-11-07T00:01 -->
- [x] D5 global `--database-url`, runner argv, `{database_url}` placeholder and the non-default-database refusal; migrate test children from the environment variable to the option <!-- completed: 2026-11-07T00:01 -->
- [x] D9 `Settings` without environment variables, fixed `TEXT_BODY_LIMIT`, `CAFLEET_MULTIPLEXER` override removed from detection <!-- completed: 2026-11-07T00:01 -->
- [x] D4 precondition: confirm live on tmux that `tmux -S <socket>` and `tmux display-message -p '#{socket_path}'` reach the Director's server from a shell with `TMUX`/`TMUX_PANE` unset, and recheck the recorded herdr `--session` evidence on the installed herdr <!-- completed: 2026-11-06T23:56 -->
- [x] D4 `mux_server` column, `fleet create --pane`/`--mux-server` and the Hermes refusal, `mux_for_fleet` for every other multiplexer caller <!-- completed: 2026-11-07T00:08 -->
- [x] D3 Hermes plugin and OpenCode extension Director verification and `database_url` arguments <!-- completed: 2026-11-07T00:01 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T01:38 -->

### Step 4: Doctor and setup speed

- [x] B6 one bounded-subprocess engine with environment support; A6 adapter <!-- completed: 2026-11-07T00:20 -->
- [x] A1 `HermesCli` / `HermesConfig` seam and `HermesSetup` selection <!-- completed: 2026-11-07T00:20 -->
- [x] A2 concurrent read batches for setup, doctor and `member create` <!-- completed: 2026-11-07T00:20 -->
- [x] A3 `plugins` subtree read and write-only verification <!-- completed: 2026-11-07T00:20 -->
- [x] A4 doctor `gather`/`render` split with the Hermes batch overlapping the other checks <!-- completed: 2026-11-07T00:20 -->
- [x] A5 concurrent member probe pass <!-- completed: 2026-11-07T00:20 -->
- [x] A7 doctor per-backend resolution, `Check` builders, `CheckStatus`, `format_id_ranges` <!-- completed: 2026-11-07T00:20 -->
- [x] A8 no Hermes home → `info` rows without spawning `hermes` <!-- completed: 2026-11-07T00:20 -->
- [x] C1.9 in-process doctor scenario tests <!-- completed: 2026-11-07T01:38 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T01:38 -->

### Step 5: Bounded refactor

- [x] B2 launch policy without version gating <!-- completed: 2026-11-07T00:57 -->
- [x] B3 `detection_coverage` <!-- completed: 2026-11-07T00:57 -->
- [x] B4 composer and answer plans without echoed provenance <!-- completed: 2026-11-07T00:57 -->
- [x] B5 detector single observation pass <!-- completed: 2026-11-07T00:57 -->
- [x] B7 `coding_agent/hermes.rs` <!-- completed: 2026-11-07T00:57 -->
- [x] B8 test-only wrappers to `test_support`; glob re-exports <!-- completed: 2026-11-07T01:07 -->
- [x] B9 `Multiplexer` trait and shared `argv` helper <!-- completed: 2026-11-07T00:53 -->
- [x] B10 pane-lock retry helper <!-- completed: 2026-11-07T00:53 -->
- [x] B11 runtime invariant helpers and `stored_enum!` enums <!-- completed: 2026-11-07T00:48 -->
- [x] B12 input-client host seam <!-- completed: 2026-11-07T00:53 -->
- [x] B13 `advance` split and import cleanup <!-- completed: 2026-11-07T00:56 -->
- [x] B14 writer worker and single-connection outbox <!-- completed: 2026-11-07T01:00 -->
- [x] B15 fail-fast fixes <!-- completed: 2026-11-07T00:53 -->
- [x] B16 `emit` returns `Result`; `From<MultiplexerError>` <!-- completed: 2026-11-07T00:53 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T01:38 -->

### Step 5b: Deferred refactors

- [x] B17 serde-typed input-control codec (Programmer-3) <!-- completed: 2026-11-07T02:11 -->
- [x] B18 serde-typed catalog loader (Programmer) <!-- completed: 2026-11-07T02:11 -->
- [x] B19 `run_service` decomposition and typed lifecycle frames (Programmer-2) <!-- completed: 2026-11-07T02:16 -->
- [x] B20 `CancellationScope` merged into `DatabaseBudget` (Programmer-2) <!-- completed: 2026-11-07T02:09 -->
- [x] B21 typed `PendingInput` state (Programmer-2) <!-- completed: 2026-11-07T02:09 -->
- [x] B22 `get_event` payload rebuild with `DeliveryFields` (Programmer-3) <!-- completed: 2026-11-07T02:11 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T02:48 -->

### Step 6: Test pruning

- [x] Apply the § C5 ledger rows for `src/runtime/` <!-- completed: 2026-11-07T01:38 -->
- [x] Apply the § C5 ledger rows outside `src/runtime/` <!-- completed: 2026-11-07T01:38 -->
- [x] Apply the C5 helper consolidation and rule-violation fixes, and add the C4 coverage <!-- completed: 2026-11-07T01:38 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T01:38 -->

### Step 7: Verification

- [x] Measure doctor and setup on the reference host against the Success Criteria (three runs each: doctor with and without Hermes, setup steady state, first Hermes install in a sandbox `HOME`) <!-- completed: 2026-11-07T04:03 -->
- [x] Measure `mise //:test` wall, serial CPU and the slowest tests against the Success Criteria <!-- completed: 2026-11-07T03:04 -->
- [x] Count implementation-region lines at the branch point and the branch head with the § Line-count method script and record both totals in the PR description <!-- completed: 2026-11-07T01:39 -->
- [x] Run a live fleet on tmux and on herdr with a Claude member and a Hermes member on a home whose `terminal.client_env_passthrough` lacks CAFleet's names: send, poll, `member exec`, a refused Director-only command from the member, and `member delete`; repeat once against a custom `--database-url` <!-- completed: 2026-11-07T03:30 -->

The real-home check (reinstall the CLI with `mise //:install`, then run `cafleet doctor` and `cafleet setup` on the operator's home) runs with the release that bumps the version: this change edits the schema baseline under the unchanged version 0.32.0, so the existing `cafleet_0.32.0.db` is incompatible with a branch build, and the version bump gives the installed binary a fresh default database.

### Step 8: Idle runner CPU

- [x] Profile an idle runner's serve loop and record the A11 mechanism, approved by the Director <!-- completed: 2026-11-07T04:47 -->
- [x] Implement the A11 leader check in `PtySession::try_wait` <!-- completed: 2026-11-07T05:00 -->
- [x] Add tests that no group scan runs while the leader is alive and that one runs before the reap <!-- completed: 2026-11-07T04:47 -->
- [x] Measure idle runner main-thread CPU over 60 s for a Claude member and a Hermes member against the Success Criteria <!-- completed: 2026-11-07T04:54 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-07T05:00 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-06 | Initial draft |
| 2026-11-06 | Review rounds 1–2: Workstream D adopts the no-environment-variable redesign (all four user-facing `CAFLEET_*` settings removed, Hermes plugin kept), herdr `--session` targeting, ledger reconciled; approved |
| 2026-11-07 | User decision: the deferred refactors land in this change as B17–B23 (Step 5b), because B1–B16 measured −176 lines net against the 900-line target; Success Criteria cover B1–B23 |
| 2026-11-07 | B23 (per-backend `BackendSpec` table) dropped: as implemented it added 38 lines instead of removing about 30; § B18 records the landed loader shape; Success Criteria cover B1–B22 |
| 2026-11-07 | User decisions: the idle runner busy-loop is fixed in this change (§ A11, Step 8, a new idle-CPU Success Criterion); the real-home reinstall check moves to the version-bump release, because the same-version schema change makes the existing `cafleet_0.32.0.db` incompatible |
| 2026-11-07 | Idle runner CPU fix landed (§ A11 leader check, idle main thread 1.8–2.0 %); final verification ticks every Success Criterion, the 12-member doctor and probe-pass criteria by their implementation bounds; line count −922 at the branch head; complete |
