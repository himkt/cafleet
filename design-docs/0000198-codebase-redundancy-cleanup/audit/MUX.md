# Audit: delivery and multiplexer (MUX)

Slice files: `cafleet/src/delivery.rs` (2347), `listen.rs` (1287), `receive.rs` (594), `capture.rs` (117), `signals.rs` (128), `multiplexer/mod.rs` (1293), `multiplexer/tmux.rs` (810), `multiplexer/herdr.rs` (1286), `multiplexer/keys.rs` (115), `presentation.rs` (150), `diagnosis.rs` (143). Total 8270 lines.

## Summary

The biggest win is the tmux/herdr backend pair: the two adapters re-implement the same runner plumbing (`run`, `press_run`, `env_var`, `bounded`, `ensure_available`, `send_keys`, `send_prompt`, `map_run_error`, argv helper), and their test modules re-test the shared `submit_prompt` and `capture_window` code once per backend, on top of the direct `submission_tests` in `multiplexer/mod.rs`. A shared backend core plus one test home for shared logic removes roughly 415 lines (MUX-01..04). The second cluster is test scaffolding: `delivery.rs` tests copy `broker::tests::{migrated_conn, create_fleet, register}` verbatim and re-type the same placement/stamp queries many times, and `listen.rs`/`receive.rs`/`delivery.rs` each define their own `TestClock`/`MessageRecord` fixtures (MUX-12, MUX-14). One placement fix is needed: about 295 lines of `listen.rs` tests exercise `broker::messaging::listen_snapshot` and belong in `broker/messaging.rs` (MUX-05). Production code has a three-level `deliver_pending` forwarding chain, one practically dead deadline check, and an `AttemptEnd` enum whose four non-failure variants no production code reads (MUX-06..09). `signals.rs`, `capture.rs` and `presentation.rs` are lean; every `presentation` function has a caller. The estimated total is about 700 removable lines, with no change to the shipped contract.

## Test placement inventory

No file in the slice is test-only. Every slice file keeps its tests in an in-file `#[cfg(test)]` module. The rows below are misplaced in-file tests and test-only dependencies that touch the slice.

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `cafleet/src/listen.rs` tests `broker_snapshots_*`, `broker_snapshot_requires_*`, `a_long_running_select_*`, `row_collection_discards_*`, `success_and_query_failure_*`, `a_failed_snapshot_commit_*`, `an_unrelated_sqlite_interrupt_*` (lines 993-1287) | ~295 | `cafleet/src/broker/messaging.rs` (the owner of `listen_snapshot` / `listen_snapshot_with_clock`, lines 196-205) | `snapshot_messages`, `assert_handler_cleared`, and the `broker_fixture` part used by these tests move with them. Build `broker_fixture` from `broker::tests::{migrated_conn, create_fleet}` (`broker/mod.rs:6245-6266`). | The tests are in an implementation file, but not the one they exercise. `run_on_descriptor` pipe tests (lines 900-991) stay in `listen.rs`. |
| `cafleet/src/runtime/runner_tests/control_service.rs` (`ServiceFixture`, line 123) and `runner_tests/control_service/input_service.rs` (`with_service`, `records`, `wait_record`, `barrier`, `composer`, lines 279-342), re-exported via `runtime/mod.rs:6-18` | 786 + 764 | Owned by the runtime slice. `delivery.rs` tests consume them as `crate::runtime::{with_input_service, InputServiceFixture, input_service_barrier, input_service_records, wait_input_service_record}`. | They become `#[cfg(test)]` helpers inside the runtime service/input-client implementation module that owns the control service. | `delivery.rs` tests need only import-path updates when the runtime slice relocates them. The re-export block in `runtime/mod.rs:6-18` disappears with that relocation. |
| `cafleet/src/multiplexer/mod.rs` `#[cfg(test)] pub(crate) mod recording` (lines 1172-1293) | 122 | Stays in `multiplexer/mod.rs`, which owns `CommandRunner`. | `FakeRunner`, `Event`, `argv`, `env`, `run_event`, `sleep_event`, `herdr_envelope`, `herdr_error_stderr`; also used by `delivery.rs`, `broker/mod.rs:735` and `runtime/runner/control.rs`. | This already matches the required shape. MUX-04 merges `recording::argv` with the production argv helper. |
| `cafleet/src/multiplexer/mod.rs` `mod press_tests` test `step5_press_keys_validate_the_complete_common_grammar_and_byte_count_bounds` (lines 13-109) | ~97 | `cafleet/src/multiplexer/keys.rs`, which owns `PressKeys::parse` / `canonical` and has no tests | `parse`, `usage` helpers | The adapter-level press tests (lines 111-323) stay in `multiplexer/mod.rs` (see MUX-04). |

## Findings

### MUX-01: tmux and herdr adapters duplicate the whole runner/press/submission core
- Category: duplication
- Location: `cafleet/src/multiplexer/tmux.rs:8-129,196-211,290-326`; `cafleet/src/multiplexer/herdr.rs:13-49,108-195,257-272,455-492`
- Evidence: Line-for-line twins that differ only in the backend name, argv prefix or pane-gone predicate:
  - `map_run_error`: tmux 19-41 / herdr 27-49, same four arms with "tmux" or "herdr" in each message.
  - `bounded`: 53-58 / 108-113.
  - `env_var`: 64-69 / 119-124.
  - `run`: 71-75 / 126-130.
  - `run_tolerating_pane_gone` / `run_tolerating_missing`: 77-90 / 141-156.
  - `ensure_available`: 92-102 / 158-168.
  - `press_run`: 104-129 / 170-195, identical 25 lines.
  - `send_keys`: 196-211 / 257-272.
  - `send_prompt` (valid_input check, `bounded(Deadline::new())`, `submit_prompt`): 290-313 / 455-478, including the identical error string `send_prompt: text must be one non-empty line without control characters`.
  - The `capture_pane` positivity guard: 322-326 / 488-492.
  - The `deadline.remaining().map_err(|error| MultiplexerError::new(error.message()))` prologue: 3× in each backend (tmux 110-112, 136-138, 140-142; herdr 176-178, 202-204, 206-208).
  - `"press target context does not match the recorded placement"`: 2× in tmux, 3× in herdr.
- Proposal: Introduce one private `BackendCore { runner: Rc<dyn CommandRunner>, env: HashMap<String,String>, name: &'static str, env_key: &'static str, pane_gone: fn(&str) -> bool }` in `multiplexer/mod.rs`. Give it the shared `bounded`, `env_var`, `run`, `run_tolerating_missing`, `ensure_available`, `press_run`, `check_deadline`, `send_keys(prefix, args)` and `send_prompt(PaneInput)` methods, plus `map_run_error(name, …)`. Both adapters embed it and keep only backend-specific argv construction and parsing: tmux `display-message` parsing, herdr envelopes and layout. `submit_prompt` takes the core (or a `&dyn Fn` mapper) instead of a bare `fn` pointer. Hoist the mismatch string into one constant.
- Spec impact: none. Every message is reconstructed byte-identically from `name` ("tmux binary not found on PATH", "cafleet member commands must be run inside a tmux session", "tmux command failed: …", "tmux key acknowledgment was unreadable"), and the argv sent to each binary is unchanged.
- Risk: low. The existing per-backend `ensure_available`, `send_keys` and `split_window` argv tests pin the observable strings.
- Estimated delta: -120 lines

### MUX-02: send_prompt failure-stage tests are triplicated across tmux, herdr and mod.rs
- Category: redundant-test
- Location: `cafleet/src/multiplexer/tmux.rs:510-642`; `cafleet/src/multiplexer/herdr.rs:881-1022`; `cafleet/src/multiplexer/mod.rs:948-985,988-1170`
- Evidence: `submit_prompt` (`mod.rs:531-575`) is the single implementation of escape/settle/literal/enter staging and uncertainty classification. `submission_tests` already tests it directly: `reserved_budgets_cap_escape_*` and `only_started_literal_or_enter_failures_*`, a 7-row stage×error matrix. On top of that:
  - Each backend repeats `send_prompt_preserves_literal_text_and_settles_before_enter`, `escape_lead_sends_the_escape_key_*`, `escape_failure_propagates_cause_*`, `first_write_failure_propagates_cause_*`, `lost_enter_propagates_cause_*` and `prompt_line_validation_precedes_transport`. These are 6 tests per backend with the same structure; only the argv literals differ.
  - `any_multiplexer_send_prompt_tests::dispatch_forwards_the_lead_to_each_backend` (`mod.rs:952-984`) re-asserts the same tmux Escape and herdr Direct argv sequences a fourth time.
  - `prompt_line_validation_precedes_transport` re-tests `crate::input::valid_input`, which has its own owner, twice.
- Proposal: Keep the stage/uncertainty matrix in `submission_tests`. Per backend, keep exactly one Escape-lead argv/timeout/sleep test (it pins the backend argv) and the pane-gone classification case (`each_backend_classifies_confirmed_missing_panes_*`, plus herdr's `pane_not_found` escape row). Delete the per-backend failure-stage tests, `dispatch_forwards_the_lead_to_each_backend`, and one of the two `prompt_line_validation_precedes_transport` copies. After MUX-01 the validation is shared, so keep a single test of it.
- Spec impact: none. Tests only; every behavior stays covered once at its owning function.
- Risk: low.
- Estimated delta: -200 lines

### MUX-03: capture windowing is re-tested through each backend
- Category: redundant-test
- Location: `cafleet/src/multiplexer/tmux.rs:644-754`; `cafleet/src/multiplexer/herdr.rs:1024-1087`
- Evidence: Both backends delegate to `super::capture_window` (`mod.rs:656-667`), and `capture_window` has no direct test. The blank-tail, interior-blank, ANSI-aware blank detection, all-blank buffer and over-fetch-deeper-than-N behaviors are tested through tmux (`capture_pane_drops_trailing_whitespace_only_lines_*`, `capture_pane_keeps_interior_blank_lines`, `capture_pane_blank_detection_is_ansi_aware`, `capture_pane_of_an_all_blank_buffer_is_empty`, `capture_pane_over_fetch_survives_*`, `capture_pane_keeps_the_last_lines_*`). They are tested again through herdr (`capture_pane_blank_detection_is_ansi_aware`, `capture_pane_enforces_the_last_n_window_client_side`, `capture_pane_over_fetch_survives_*`, `capture_pane_drops_the_blank_tail_*`). The `lines must be positive` guard is also asserted in both.
- Proposal: Add one table-driven `capture_window` unit test in `multiplexer/mod.rs` covering blank tail, interior blanks, CSI-only rows, CR survival, all-blank input and last-N slicing. Per backend, keep one test that asserts the fetch argv (`-S -1002` / `--lines 1002`) and returns the windowed result. Keep one positivity-guard test; after MUX-01 the guard is shared.
- Spec impact: none. Tests only.
- Risk: low.
- Estimated delta: -95 lines

### MUX-04: multiplexer/mod.rs scatters four test modules and triplicates the argv helper
- Category: test-placement
- Location: `cafleet/src/multiplexer/mod.rs:3-324` (`press_tests`, placed between `pub mod herdr;` and `pub mod tmux;`), `859-986` (`tests` with nested `resolver_tests` and `any_multiplexer_send_prompt_tests`), `988-1170` (`submission_tests`), `1262-1264` (`recording::argv`); `tmux.rs:8-10` (`tmux_argv`); `herdr.rs:13-15` (`herdr_argv`)
- Evidence: Production items are split around a 320-line test module at the top of the file. The PressKeys grammar test tests `keys.rs`, which has no test module of its own (see inventory). `tmux_argv`, `herdr_argv` and `recording::argv` have identical bodies (`parts.iter().map(|part| part.to_string()).collect()`). Four test names carry the `step5_` design-step prefix (`mod.rs:33,126,237,304`); the same prefix appears on about 200 tests crate-wide outside this slice.
- Proposal: Move the grammar test into a `#[cfg(test)] mod tests` in `keys.rs`. Merge `press_tests` (adapter part), `tests`, `resolver_tests` and `submission_tests` into one `mod tests` at the bottom of `mod.rs`, with no nested modules. Declare `pub mod herdr; mod keys; pub mod tmux;` together at the top. Replace the three argv helpers with one `pub(super) fn argv(&[&str]) -> Vec<String>` in `mod.rs` that `recording` re-exports. Drop the `step5_` prefix from test names in this slice; the crate-wide rename belongs to the slices that own the other tests.
- Spec impact: none. Test layout and a private helper only.
- Risk: low.
- Estimated delta: -40 lines

### MUX-05: listen.rs carries the broker snapshot tests and a duplicated million-row view fixture
- Category: test-placement
- Location: `cafleet/src/listen.rs:993-1287`; `cafleet/src/listen.rs:1105-1113` and `1247-1255`; `cafleet/src/broker/messaging.rs:196-205`
- Evidence: Ten tests call `listen_snapshot` / `listen_snapshot_with_clock` directly and never call `listen()`. `listen_snapshot_with_clock` is `pub(crate)` only because `listen.rs:1138` imports it; grep shows no other caller outside `messaging.rs`. The `ALTER TABLE messages RENAME TO stored_messages; CREATE VIEW messages AS WITH RECURSIVE …` batch is pasted twice, identically.
- Proposal: Move these tests and their helpers (`snapshot_messages`, `assert_handler_cleared`, the fleet fixture) into `broker/messaging.rs`'s test module, and make `listen_snapshot_with_clock` private. Extract the view batch into one `fn expensive_messages_view(conn)` helper.
- Spec impact: none. Tests and visibility only.
- Risk: low.
- Estimated delta: -9 lines (plus ~295 lines relocated)

### MUX-06: deliver_pending is a three-level forwarding chain
- Category: over-abstraction
- Location: `cafleet/src/runtime/mod.rs:39-56`; `cafleet/src/delivery.rs:98-117,169-182`
- Evidence: `runtime::deliver_pending` only adds `|| resolve_mux(settings)`, and its two callers are `cli/message.rs:111` and `receive.rs:245`. `delivery::deliver_pending` only adds `&mut NativeInputHost`, and outside its own tests it is called only from `runtime::deliver_pending`. The core `deliver_pending_with_host` needs `#[expect(clippy::too_many_arguments, reason = "the approved delivery core retains its public arguments and shared input host")]`. That reason text addresses a reviewer rather than stating a constraint.
- Proposal: Make `delivery::deliver_pending(conn, settings, member_id, expected_fleet, deadline, attempted)` the single production entry, resolving through `crate::runtime::resolve_mux(settings)` and `NativeInputHost`. Keep one test seam, `deliver_pending_with(…, resolve, host)`. Delete `runtime::deliver_pending`. Delivery tests that passed a `resolve` closure to `deliver_pending` call the seam with `&mut NativeInputHost`. Rewrite or drop the `#[expect]` reason accordingly.
- Spec impact: none. The same function body runs with the same resolver and host.
- Risk: low.
- Estimated delta: -20 lines

### MUX-07: Practically unreachable BudgetExhausted check after a `?` on the same deadline
- Category: dead-code
- Location: `cafleet/src/delivery.rs:450-453`
- Evidence: `deadline.remaining()?;` is immediately followed by `if deadline.remaining().is_err() { break AttemptEnd::BudgetExhausted; }`. The `if` can only fire if the deadline expires in the nanoseconds between two consecutive calls. In every realistic case the preceding `?` has already turned expiry into an error, which the closure maps to the Retirement-stage warning.
- Proposal: Delete lines 451-453 and keep the `?`, which preserves the observed behavior. Needs-verification with the Director: if the design intended `BudgetExhausted` at this point, delete the `?` instead. That choice changes which (non-contract) `AttemptEnd` is produced, so it needs a test.
- Spec impact: none if the `if` is removed. `AttemptEnd` is internal (see MUX-09), and the CLI diagnostic for this path is unchanged.
- Risk: low
- Estimated delta: -3 lines

### MUX-08: Duplicate placement filter and a nested `unreachable!` stage match in delivery
- Category: duplication
- Location: `cafleet/src/delivery.rs:196-212` vs `337-343`; `492-502`
- Evidence: Both the direct path and `deliver_wrapped_pending` independently compute `member.placement.as_ref().filter(|p| p.mux_pane_id.is_some())` and return `Held` when it is absent. The wrapped path is entered only from line 197, right after the same snapshot. The warning builder matches `stage` twice: the inner `match stage { … Stage::Submission => unreachable!() }` repeats the outer arm.
- Proposal: Compute the concrete placement once, before the `member.runtime.is_some()` branch, and pass it into `deliver_wrapped_pending`. Map the stage with a single `match` that yields `Option<&str>` for the phase (`None` for Submission), with no `unreachable!`.
- Spec impact: none. Both paths still end with `Held` before any lock or effect when the pane is missing, and the warning strings are unchanged.
- Risk: low
- Estimated delta: -10 lines

### MUX-09: AttemptEnd's four non-failure variants are consumed only by tests
- Category: over-abstraction
- Location: `cafleet/src/receive.rs:49-71,159-173`; producers throughout `cafleet/src/delivery.rs:187-518`
- Evidence: Grep for `AttemptEnd::(NoPending|Drained|BudgetExhausted|Held)` outside `#[cfg(test)]` finds only producers in `delivery.rs` and one no-op arm in `receive.rs:160-161`. Both production consumers, `cli/message.rs:120` and `receive.rs:162`, branch only on `AttemptEnd::Failed`. `NoPending`, `Drained`, `BudgetExhausted` and `Held` are distinguished only by assertions in `delivery.rs` tests and `runtime/runner/input_client.rs:1541,1696-1698`.
- Proposal (needs-decision): Replace `AttemptOutcome { sent, end: AttemptEnd }` with `AttemptOutcome { sent, failure: Option<NotificationError> }`. Tests that distinguished `Held` from `NoPending` (placement-change detection) assert the observable effects they already check: no runner events, no `notified_at` stamp, pending row retained. If the Director prefers to keep the enum as a test oracle, at least collapse `receive.rs:159-173` to `if let AttemptEnd::Failed(error) = outcome.end { … }` (folded into MUX-10).
- Spec impact: none. The enum never reaches output, and the stderr diagnostics come from `NotificationError` alone.
- Risk: medium. Many delivery tests use the variants as an oracle, so their assertions must be rewritten against side effects.
- Estimated delta: -10 lines (production), test assertions rewritten in place

### MUX-10: receive::wait has split no-op match arms and a single-field result wrapper
- Category: other
- Location: `cafleet/src/receive.rs:73-76,159-173,179`; callers `cafleet/src/cli/message.rs:158-163`, `runtime/runner/input_client.rs:1684`
- Evidence: `AttemptEnd::Held | AttemptEnd::Drained | AttemptEnd::BudgetExhausted => {}` and `AttemptEnd::NoPending => {}` are two separate empty arms. `WaitOutcome { messages }` wraps exactly one `Vec<MessageRecord>`, and both callers immediately read `.messages`.
- Proposal: Use `if let AttemptEnd::Failed(error) = outcome.end { … }`. Return `Vec<MessageRecord>` from `wait` and delete `WaitOutcome`.
- Spec impact: none. Internal types only.
- Risk: low
- Estimated delta: -8 lines

### MUX-11: listen's EmittedMessages emulates a set with HashMap<i64, bool>
- Category: other
- Location: `cafleet/src/listen.rs:46-71`
- Evidence: `reconcile` resets every value to `false`, marks present IDs `true`, then retains the `true` entries. `flushed` inserts `true`, and `contains` checks key presence. That is exactly set intersection.
- Proposal: Use `struct EmittedMessages(HashSet<i64>)` with `reconcile` = `let live: HashSet<_> = messages.iter().map(|m| m.message_id).collect(); self.0.retain(|id| live.contains(id));`, `flushed` = `insert`, and `contains` = `contains`.
- Spec impact: none. The same IDs are suppressed. `an_ack_output_race_*` and `an_unacked_a_allows_*` pin the behavior.
- Risk: low
- Estimated delta: -8 lines

### MUX-12: delivery.rs test scaffolding copies broker fixtures and repeats lookups
- Category: duplication
- Location: `cafleet/src/delivery.rs:537-564`, `571-574` vs `1495-1498`, `1604-1609` vs `1522-1528`, placement fetch chains at `588-596, 791-795, 1016-1020, 1124-1128, 1239-1243, 1728-1732, 1914-1918`, inline `notified_at` queries at `1459-1465, 1709-1715, 2101-2109, 2182-2190`
- Evidence:
  - `migrated_conn`, `create_fleet` and `register` are copies of `broker::tests::{migrated_conn, create_fleet, register}` (`broker/mod.rs:6245-6272`). They differ only in the temp-db filename and member description, which no assertion reads.
  - `settings_for` exists but one test (1495-1498) rebuilds it inline.
  - `fake_mux` exists but one test (1522-1528) rebuilds it inline.
  - `broker::get_member(..).unwrap().unwrap().placement.unwrap()` appears 7 times.
  - `preview_stamp` exists, yet four tests re-type its `SELECT notified_at` query.
- Proposal: Import `broker::tests::{migrated_conn, create_fleet, register}` and delete the local copies. Use `settings_for` and `fake_mux` everywhere. Add `fn placement_of(conn, member, fleet) -> Placement` and use `preview_stamp` for every `notified_at` read.
- Spec impact: none. Tests only.
- Risk: low
- Estimated delta: -80 lines

### MUX-13: notification_line tests assert the same facts several times
- Category: redundant-test
- Location: `cafleet/src/delivery.rs:2242-2346`
- Evidence: `line_body` already asserts the exact prefix and the two-clause suffix on every call. As a result:
  - `inline_and_pointer_lines_both_carry_both_clauses` (2333-2339) repeats what the pointer tests already check through `line_body`.
  - `a_single_line_body_is_inline_unchanged` (2287-2289) is subsumed by `the_line_matches_the_specified_example` (2277-2284) and the 4000-character test.
  - `the_line_ignores_the_cli_text_length_limit` (2342-2345) asserts that a 3999-character body is inline. `a_folded_body_of_exactly_4000_characters_is_inline` already covers that, and `notification_line` takes no `Settings`, so no CLI limit can reach it.
  - The tests also sit in a nested `mod notification_line_tests` inside `mod tests`.
- Proposal: Delete the three subsumed tests and flatten the nested module into `mod tests`.
- Spec impact: none. The specified example, the inline/pointer boundary, Unicode counting, folding and folded-vs-stored counting stay covered.
- Risk: low
- Estimated delta: -25 lines

### MUX-14: TestClock and MessageRecord fixtures are re-implemented in three test modules
- Category: duplication
- Location: `cafleet/src/listen.rs:317-343,396-406`; `cafleet/src/receive.rs:262-289,328-338`; `cafleet/src/delivery.rs:2249-2259`
- Evidence: `listen` and `receive` each define a `TestClock { origin, elapsed: Cell<Duration> }` implementing `receive::Clock`. They differ only in that listen records sleeps and offsets its origin. `listen`, `receive` and `delivery` each construct a literal `MessageRecord { … status: MessageStatus::InputRequired, … }`.
- Proposal: Put one `#[cfg(test)] pub(crate) struct TestClock` (with `advance` and a `sleeps` log) in `receive.rs`, next to the `Clock` trait it implements, and use it from `listen` tests. Add a `#[cfg(test)] impl MessageRecord { pub(crate) fn fixture(id, from, to, text) }` in `broker/records.rs`, which owns the type.
- Spec impact: none. Tests only.
- Risk: low
- Estimated delta: -35 lines

### MUX-15: herdr re-parses `pane list` three times and bypasses its own run_json helper
- Category: duplication
- Location: `cafleet/src/multiplexer/herdr.rs:227-234, 300-303, 568-571` (pane list), `300, 374-377` (`parse_envelope(&self.run(..)?)` beside `run_json` at 132-139), `416-421` vs `629-634` (y-sort comparator)
- Evidence: `result["panes"].as_array().ok_or_else(|| MultiplexerError::new("herdr pane list missing 'panes' field"))` appears three times. Two call sites spell out `parse_envelope(&self.run(...)?)` even though `run_json` exists for that purpose; `validate_key_target` uses `press_run`, so it legitimately differs. The rect-`y` `sort_by` closure is duplicated.
- Proposal: Add `fn list_panes(&self) -> Result<Vec<Value>, MultiplexerError>`, used by `split_window` and `surviving_pane_in_tab`, and a parse-only `fn panes_of(result)` for the press path. Use `run_json` at lines 300 and 374. Add `fn sort_by_y(&mut [Value])`.
- Spec impact: none. Same commands and the same error strings.
- Risk: low
- Estimated delta: -20 lines

### MUX-16: tmux validate_key_target duplicates context_discovery's caller query
- Category: duplication
- Location: `cafleet/src/multiplexer/tmux.rs:143-164` vs `213-241`
- Evidence: Both read `TMUX_PANE` with the identical `"TMUX_PANE is not set; not running inside a tmux pane"` error. Both run `display-message -p -t <caller> "#{session_name}|#{window_id}|#{pane_id}"` and `splitn(3, '|')` the result; only the runner path differs (`press_run` with the deadline vs `run`).
- Proposal: Extract `fn caller_context(&self, run: impl Fn(&[String]) -> Result<String, MultiplexerError>) -> Result<MultiplexerContext, MultiplexerError>`. `validate_key_target` compares the returned `session` with `placement.mux_session`.
- Spec impact: none. The same argv is sent. Malformed output in the press path still ends in a `MultiplexerError`; keep its existing message by mapping the parse failure to the placement-mismatch string.
- Risk: low
- Estimated delta: -12 lines

### MUX-17: Small duplicated constants and arithmetic in multiplexer/mod.rs
- Category: duplication
- Location: `cafleet/src/multiplexer/mod.rs:340-344` vs `356-362`; `498-503` vs `527-529`; `489`, `637`, `cafleet/src/runtime/system.rs:78-79`
- Evidence:
  - `INPUT_RESERVATION` re-derives `SUBMIT_DELAY + 1s + ESCAPE_SETTLE_DELAY`, which `Lead::Escape.reserved()` also computes.
  - `CommandRunner::run_for`'s default body inlines exactly `whole_seconds(timeout)`.
  - The literal `"command deadline expired before launch"` is built three times, although `TimedRunError::before_launch()` exists for this.
- Proposal: Express the default `run_for` via `whole_seconds`. Introduce `const DEADLINE_EXPIRED_BEFORE_LAUNCH: &str` (or `RunError::deadline_expired()`) and use it at all three sites. Define `INPUT_RESERVATION` as `REVALIDATION_MARGIN + MIN_LITERAL_BUDGET + <escape reservation>`, sharing one `const` escape reservation with `Lead::reserved`. Needs-verification: whether `Duration::from_secs_f64` is usable in `const` on the pinned toolchain; otherwise express the delays as `Duration` consts with millisecond literals.
- Spec impact: none. Same values and same strings.
- Risk: low
- Estimated delta: -6 lines

### MUX-18: SHA-256 hex encoding is hand-rolled at about 27 call sites crate-wide
- Category: duplication
- Location: in slice: `cafleet/src/capture.rs:21-24`, `cafleet/src/delivery.rs:1214-1217` (test). Crate-wide: `input.rs:543`, `broker/mod.rs:620,5147`, `broker/runtime/permissions.rs:144`, `broker/runtime/evidence.rs:261`, `cli/member.rs:461,1457`, `coding_agent/profiles.rs` (7 sites), `coding_agent/profiles/catalog.rs:1458` (an existing `sha256_hex`), `runtime/runner/input_client.rs:200,3316`, `runtime/runner/input.rs:1193,5394,7666`, `runtime/runner/native_capture/files.rs:22,567`, `runtime/runner_tests/mod.rs:77` (a second `sha256_hex`)
- Evidence: `grep 'format!("{byte:02x}")'` returns 27 hits. Two independent `sha256_hex` functions already exist.
- Proposal: Promote one `pub(crate) fn sha256_hex(bytes: &[u8]) -> String` (for example in `time.rs`'s sibling utility module, or next to `catalog.rs`'s existing one, made crate-visible) and replace every inline copy. This is cross-slice; the Director assigns the crate-wide change.
- Spec impact: none. The digest bytes are identical.
- Risk: low
- Estimated delta: -2 lines in slice (~-60 crate-wide)

### MUX-19: Over-broad visibility on module-internal helpers
- Category: dead-code
- Location: `cafleet/src/multiplexer/mod.rs:830` (`pub fn resolve_multiplexer_name`); `cafleet/src/diagnosis.rs:37` (`pub(crate) fn recorded_version`); `cafleet/src/broker/messaging.rs:205` (`pub(crate) fn listen_snapshot_with_clock`, after MUX-05)
- Evidence: Grep across `cafleet/src` finds `resolve_multiplexer_name` called only inside `multiplexer/mod.rs` (the resolver and its tests), and `recorded_version` only inside `diagnosis.rs`. The crate has no external consumer: `cafleet/` tracks no `tests/` directory, and `main.rs` is the only other target.
- Proposal: Make the first two private, and make the third private once its test moves.
- Spec impact: none. There is no change to the binary or any skill-visible surface.
- Risk: low
- Estimated delta: 0 lines

### MUX-20: delivery.rs wrapped-delivery tests depend on fixtures in the runtime_tests folder
- Category: test-placement
- Location: `cafleet/src/delivery.rs:583-621,781-1405` (via `crate::runtime::{with_input_service, InputServiceFixture, input_service_barrier, input_service_records, wait_input_service_record}`); `cafleet/src/runtime/mod.rs:6-18`
- Evidence: Six wrapped-delivery tests run on `runtime/runner_tests/control_service{,/input_service}.rs` fixtures, re-exported through `runtime/mod.rs`. That folder is test-only and must be dissolved under requirement 1.
- Proposal: Coordinate with the runtime-slice auditor. When `ServiceFixture` and `with_service` become `#[cfg(test)]` helpers in the runtime control-service implementation module, update only the import paths in `delivery.rs` and delete the re-export block in `runtime/mod.rs:6-18`. The delivery tests themselves stay in `delivery.rs`, which owns `deliver_wrapped_pending`.
- Spec impact: none. Tests only.
- Risk: low. Ordering with the runtime relocation is the only coupling.
- Estimated delta: -13 lines (the re-export block)

## Checked and found lean

- `signals.rs`: a single RAII type, and its five tests cover distinct mask behaviors.
- `capture.rs`: aside from MUX-18 there is no waste. `write_member_capture` and `CaptureSnapshot` each have one caller.
- `presentation.rs`: every function has at least one caller (verified by grep for `presentation::<fn>`).
- `diagnosis.rs`: `SchemaState` and its classifiers are used by `doctor`, `setup`, `cli/mod.rs`, `runtime/runner/service.rs` and `db` tests. Its single test does not duplicate the `db/mod.rs` per-version checks.
- `ListenIo`, `ListenOutput` and `ReceiveIo` each have one production implementor, but they are the deterministic-clock and output-fault injection seams the listen/receive tests depend on. Keep them.
