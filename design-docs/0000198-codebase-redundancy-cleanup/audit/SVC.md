# Audit: SVC — runtime runner process/service plumbing

## Summary
The biggest win is `process/ownership_diagnostics.rs`. It is a test-only refresh tracer of about 381 lines, wired into `PtySession::refresh_owned_with` through about 15 `#[cfg(test)]` hooks and exercised mostly by roughly 280 lines of tests that check the tracer itself. Deleting it removes about 740 lines. The other large items are all test-side. The same IO fakes, length-prefix framers, socket read/write loops and "baseline re-run" preambles are copied across `codec.rs`, `control.rs`, `service.rs`, `transport.rs`, `drain.rs` and `outbox.rs`. Several overlapping test pairs can also be merged: codec decoder tests, lifecycle clock tests, outbox compaction/truncate tests, and the control-client `Peer` scaffolding, which includes a test of its own cleanup. On the production side, the wins are smaller. `service.rs` keeps a parallel `RunnerTestServices` type and a long `run_service` with repeated quiesce/exit/reply blocks. `ShutdownOutcome` is built with an eight-field zero literal at 8 sites. `outbox.rs` has two copies of the open/validate paths, and `system.rs` repeats the spawn and output decoding code. Estimated total removable: about 1,820 lines. A further ~150 lines of `#[cfg(test)]` instrumentation inside production paths could go, but only if the observation slice retires the test-only native-capture suite (SVC-20).

## Test placement inventory
| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `cafleet/src/runtime/runner_tests/mod.rs` | 168 | `cafleet/src/runtime/runner/mod.rs` (`#[cfg(test)] pub(crate) mod test_support`) | `Fixture` (run + artifacts + `SpawnSpec`) is used by nearly every runner test module and by `press.rs`; `read_fixture_bytes`/`write_fixture_bytes` → `codec.rs` test support (control-socket framing owner); `sha256_hex` duplicates `coding_agent/profiles/catalog.rs:1458` (see SVC-23) | `Fixture::new` hard-codes the `/tmp/.p194-` prefix; keep it as is |
| `cafleet/src/runtime/runner_tests/creation.rs` | 989 | `cafleet/src/runtime/runner/creation.rs` | Fake `Host: LaunchHost`, `inputs`, `launch`, `pending_run`, `assert_compensated` | `Host::identity` (75-85) re-implements `ControlIdentity::from_run` (SVC-18) |
| `cafleet/src/runtime/runner_tests/owned_child.rs` | 104 | `cafleet/src/runtime/system.rs` (`#[cfg(test)] pub(crate)` child guard next to the production `OwnedChild`) | Used by `cli/member.rs`, `input.rs`, `observation.rs`, `writer.rs`, `control_service.rs`, `native_capture/*`, `recovery/crash.rs`; re-exported via `runtime/mod.rs:18` | The name collides with the private production `system::OwnedChild` (161). Rename the test guard (for example `TestChild`) when moving it (SVC-21) |
| `cafleet/src/runtime/runner_tests/fleet_deletion.rs` | 533 | `cafleet/src/input.rs` (`delete_fleet_with_host`, `DeletionHost`) | Fake `DeletionHost`; zero-field `ShutdownOutcome` literal (SVC-08) | Out of SVC slice by target; listed because it lives in the shared runner test tree |
| `cafleet/src/runtime/runner_tests/control_service.rs` | 786 | `cafleet/src/runtime/runner/service.rs` | `ServiceFixture` (re-exec harness), `Wire`, `wire_frame` (SVC-05) | The `CHILD_TEST` constant (24) hard-codes the module path `runtime::runner_tests::control_service::runner_review_service_child`; it must be updated to the new `service::tests::…` path. That `#[test]` is a re-exec entrypoint that returns at once when the env var is absent |
| `cafleet/src/runtime/runner_tests/control_service/input_service.rs` + `input_service/native.rs` | 764 + 109 | `cafleet/src/runtime/runner/service.rs` (`#[cfg(test)] pub(crate) mod test_support` for the harness; the 5 `step5_service_input_*` tests in `service.rs` `mod tests`) | Crate-wide harness consumed by `delivery.rs`, `cli/member.rs`, `cli/runtime.rs`, `input_client.rs` through five renamed re-exports in `runtime/mod.rs:9-16` (SVC-17); loads `synthetic-profile.json` (133) | Uses the test-only `ControlDecoder::feed` (217) (SVC-03) |
| `cafleet/src/runtime/runner_tests/control_service/observation_contract.rs`, `observation_services.rs` | 362 + 194 | `cafleet/src/runtime/runner/service.rs` (tests drive `run_service` observation wiring); `observation_services::run_configured` goes with the harness | Loads `synthetic-profile.json` (96) | Overlaps the OBSERVE slice (observation controller semantics) |
| `cafleet/src/runtime/runner_tests/control_service/review_collector.rs`, `review_export.rs`, `review_failures.rs` | 62 + 747 + 562 | `cafleet/src/runtime/runner/service.rs` / `native_capture.rs` (OBSERVE slice decides) | Depend on the `#[cfg(test)]` capture port (SVC-20) | Out of SVC slice by subject |
| `cafleet/src/runtime/runner_tests/fixtures/synthetic-profile.json` | — | `cafleet/src/coding_agent/profiles/` (owner of `ProfileCatalog::from_definitions`), one `#[cfg(test)]` loader | Loaded by 7 `include_str!` sites, all test-only: `catalog.rs:21,83`, `input.rs:8361`, `writer.rs:851`, `observation_services.rs:96`, `input_service.rs:133`, `native_capture/support.rs:28` | Not runtime data; not bundled (SVC-15) |
| `cafleet/src/runtime/runner_tests/fixtures/claude-2.1.302-blocked-form-blink-1.raw` | — | Next to `cafleet/src/runtime/runner/input.rs` (single consumer `include_bytes!` at 1046, test-only) | — | INPUT slice owns the move |
| In-file but cross-module: `writer.rs` `contract_tests::{support, worker, effective_policy}` (840-1440) | ~600 | `support::Clock` → `observation.rs` (owns `ObservationClock`); `form`/`detailed_form`/`detail_rewrite` → `coding_agent/profiles/catalog.rs` `inspection_fixture_form`; `catalog`/`definitions`/`VERSION`/`PROFILE` → profile fixture loader; `ChildGuard` → with the test child guard in `system.rs`; `worker::Hooks`/`Release`/`wait_until` stay in `writer.rs` (they implement `WriterHooks`) | Imported by `observation.rs:1230-1234`, `transport/drain.rs:245-247`, `outbox.rs:455,901`, `input.rs` | Test-only helpers unrelated to the writer are parked in the writer's test module (SVC-15) |

## Findings

### SVC-01: Delete the test-only ownership-refresh tracer
- Category: redundant-test
- Location: `cafleet/src/runtime/runner/process/ownership_diagnostics.rs:1-381`; hooks in `cafleet/src/runtime/runner/process.rs:11-14, 329-332, 365-368, 400-403, 413-416, 444-526, 558-561, 586-589`; trace tests `process.rs:1385-1413, 1660-1936`; use in `process.rs:1154, 1185-1196`
- Evidence: the module and every hook are `#[cfg(test)]` (process.rs:11, 329, 331, 444, 455, 475, 488, 495, 507, 522). `enable_ownership_diagnostics`/`take_ownership_diagnostics`/`refresh_owned_traced_with` have no callers outside `process.rs` tests (grep: 21 hits, all in `process.rs` test modules). Of the 10 `step5_refresh_trace*`/`traced_*` tests, 9 assert the tracer's own records (`first_unanchored`, ring eviction, UTF-8 truncation). The 10th, `step5_refresh_traced_wrapper_without_diagnostics_uses_only_the_scripted_core`, only checks that the tracer forwards when disabled. The only other consumer is the real-PTY test at 1154/1185, which prints the trace and asserts `!records.is_empty()`. Each hook site also carries a `#[cfg(not(test))] self.refresh_owned()?` twin, so the compiled test code path differs from production. The six hand-written `impl fmt::Debug` blocks (50-188) reproduce `#[derive(Debug)]` output field for field.
- Proposal: delete `ownership_diagnostics.rs` and the `ownership_diagnostics`/`pending_ownership_refresh` fields. Have the 4 call sites invoke `self.refresh_owned()` unconditionally. Delete the trace tests together with `traced`, `assert_false_record`, the `traced` parameter of `inert`, and the enable/take/`records` assertions in `runner_real_shutdown_closes_a_slave_retaining_descendant…`. Keep every `step5_refresh_*` behavior test that calls `refresh_owned_with(&mut Script)` directly. The `Script` fake on `ProcessObservations` already verifies each decision. If the tracer is kept, the minimal step is to replace the six manual `Debug` impls with `#[derive(Debug)]` (−~90 lines).
- Spec impact: none. Everything removed is `#[cfg(test)]`, so the release binary does not change.
- Risk: low. The scripted `refresh_owned_with` tests still pin every refresh outcome. Only the extra eprintln evidence on a flaky real-PTY failure is lost.
- Estimated delta: −740 lines

### SVC-02: Consolidate the duplicated `ByteInput`/`PtyIo`/`Display` test fakes
- Category: duplication
- Location: `cafleet/src/runtime/runner/transport.rs:471-610`; `transport/drain.rs:254-463`; `input.rs:2583-2600`, `input.rs:7122-7190`, `input.rs:8493-8560`
- Evidence: 5 independent fake sets implement the three traits owned by `transport.rs:41-52` (`grep "impl ByteInput for|impl PtyIo for|impl Display for"`: 13 test impls). Each repeats the same pieces: chunked read with WouldBlock/EOF, max-write capping, a blocked flag and an output byte sink. The `drain.rs` `Reader`/`Pty`/`Output` set is a superset (trace, scripted steps, error kinds, short reads).
- Proposal: add one `#[cfg(test)] pub(in crate::runtime::runner) mod fakes` in `transport.rs`, based on the `drain.rs` set, with builder setters (`blocked`, `max_write`, `eof`, `trace`). Replace the other four sets with it.
- Spec impact: none — test-only.
- Risk: low.
- Estimated delta: −150 lines

### SVC-03: Merge the two overlapping control-decoder test modules and drop the test-only `ControlDecoder::feed`
- Category: redundant-test
- Location: `cafleet/src/runtime/runner/codec.rs:4-197` (`input_frames_tests`), `codec.rs:199-356` (`control_codec_tests`), `codec.rs:516-528` (`feed`)
- Evidence: the two modules overlap pair by pair: f1 and `decodes_every_header_and_body_split` (every split plus single-byte feed); f3/f5 and `accepts_the_inclusive_body_cap_without_an_aggregate_frame_cap`; f5 and `rejects_oversized_lengths_at_the_fourth_header_byte`; f6 and `eof_rejects_partial_headers_and_bodies`. The two `frame`/`independently_framed` helpers (9-13, 209-213) are identical. Every `input_frames_tests` test first calls `legacy()` (17-22, 7 calls), which repeats one fixed health-frame check. `ControlDecoder::feed` carries `expect(dead_code)` outside tests (516-522). Its only callers are these tests and `runner_tests/control_service/input_service.rs:217`.
- Proposal: keep one module. Assert both `admitted_at` and the decoded `Value` in the same split/cap/oversize/EOF cases. Delete `legacy()`. Replace `feed` with a `#[cfg(test)]` helper `feed_values(decoder, bytes)` that runs `feed_admitted` + `decode_legacy_control_body`, and remove the `cfg_attr(expect(dead_code))` method from the production impl.
- Spec impact: none — framing behavior and error strings are unchanged.
- Risk: low.
- Estimated delta: −130 lines

### SVC-04: Shrink the control-client `Peer` scaffolding in `control.rs`
- Category: over-abstraction
- Location: `cafleet/src/runtime/runner/control.rs:703-1299` (`input_client_tests`), especially 771-932 and 1202-1271
- Evidence: `Peer` adds a global `UNCONFIRMED_PEERS` registry (846), a `Drop` that retains leaked handles, and `finish_until`/`finish_exchange` formatting. The test `step5_wire_peer_cleanup_timeout_retains_ownership_until_release_and_confirmed_join` (1202-1271, 70 lines) checks only this scaffold. `read_peer`/`write_peer` (780-821) repeat `runner_tests::read_fixture_bytes`/`write_fixture_bytes` with a stop flag added. `pause_until` (398-402) repeats production `control::pause`. Every test x2-x6 first calls `baseline(&fixture)` (6 calls), so x1 runs 6 extra times. `raw_frame` (447) and `frame` (722) are two more copies of the framer.
- Proposal: run peers in `std::thread::scope` (as `launch_readiness_tests` already does at 479 and 621). Scoped threads join on exit, which removes `Peer`, `UNCONFIRMED_PEERS`, the Drop logic and the scaffold self-test. Reuse the shared socket byte helpers. Drop the `baseline` preambles, since x1 is the baseline test.
- Spec impact: none — test-only.
- Risk: low. Scoped threads already give the bounded-join guarantee the scaffold tried to provide.
- Estimated delta: −170 lines

### SVC-05: One independent length-prefix framer for tests
- Category: duplication
- Location: `codec.rs:9, 209`; `control.rs:447, 722`; `service.rs:1315`; `writer.rs:1666`; `outbox.rs:905` (framed with digest); `runner_tests/recovery.rs:359`; `runner_tests/control_service.rs:68`
- Evidence: `grep "to_be_bytes().to_vec()"` finds 9 identical `u32 BE length + body` test helpers.
- Proposal: add one `#[cfg(test)] pub(crate) fn independent_frame(body: &[u8]) -> Vec<u8>` in `codec.rs` (the owner of the wire format). The outbox and writer variants can call it after inserting the checksum.
- Spec impact: none.
- Risk: low.
- Estimated delta: −40 lines

### SVC-06: Collapse `RunnerTestServices` into `RunnerDependencies` and the writer start branch
- Category: duplication
- Location: `cafleet/src/runtime/runner/service.rs:392-441, 544-559`; `writer.rs:125-132`; `runner/mod.rs:46-47`
- Evidence: `RunnerTestServices` (402-408) has the same five fields as `RunnerDependencies` (392-400), except that `writer_hooks` is not an `Option`. `run_with_test_services` (410-426) only copies one struct into the other. `run` passes `writer_hooks: None`, and `run_service` then branches between `EventWriter::start_with_hooks` and `EventWriter::start` (544-559). `start` itself is just `start_with_hooks(…, Arc::new(InertWriterHooks))`. The callers are `native_capture/verification.rs:58`, `review_service_tests.rs:91` and `observation_services.rs:57`.
- Proposal: make `RunnerDependencies.writer_hooks: Arc<dyn WriterHooks>` and have `run` supply `Arc::new(InertWriterHooks)`. Expose `RunnerDependencies` plus `run_service` as `pub(crate)` under `#[cfg(test)]` for those three callers, and delete `RunnerTestServices`, `run_with_test_services` and the start branch. Once `EventWriter::start` has no production caller, make it `#[cfg(test)]`.
- Spec impact: none — the production writer behaves identically with inert hooks.
- Risk: low.
- Estimated delta: −35 lines

### SVC-07: Factor the repeated blocks inside `run_service`
- Category: duplication
- Location: `cafleet/src/runtime/runner/service.rs:442-1222` (one 780-line function)
- Evidence: the closure `coordinator.progress_with_manual(&mut transport, |t| t.quiesce_for_input(&mut input, &mut pty, &mut output, &mut controller.observer(&writer, clock), deadline))` appears 3 times (641-651, 709-718, 1046-1054). The block "`pty.try_wait()` → `native_exit = Some` → `controller.child_exited(…)` → `return Ok(ShutdownClock::new(now))`" appears twice (1033-1042, 1062-1071). `match &result { Ok(r) => client.reply_input(Ok(r)), Err(c) => client.reply_input(Err(c)) }` appears 3 times (672-675, 762-766, 1097-1101); `reply_input` already takes `Result<&_, &_>`, which is what `result.as_ref()` yields. The `#[cfg(test)]` capture path reads `broker::get_member(...).and_then(|m| m.placement)` twice (869-877, 905-910).
- Proposal: add three small helpers: `quiesce(...)`, `observe_exit(...) -> Option<ShutdownClock>` and `client.reply_input(result.as_ref())`. Extract the per-request dispatch (`handle_input_request`, `handle_legacy_request`) into functions that take a context struct, so the loop body reads as a sequence.
- Spec impact: none — same control flow, replies and error strings.
- Risk: medium. This is the runner's main loop. The service harness tests (control_service/*) cover it.
- Estimated delta: −60 lines

### SVC-08: Give `ShutdownOutcome` a constructor
- Category: duplication
- Location: `lifecycle.rs:249-256`; `process.rs:212-219, 686-697`; `control.rs:276-283`; `runner_tests/creation.rs:255-262`; `runner_tests/fleet_deletion.rs:86-93`; `runner_tests/recovery.rs:286`; `runner_tests/recovery/deletion.rs:79`
- Evidence: `grep "unsaved_observation_gap: 0"` finds 8 literals. Each sets three counters to zero and only varies `native_exit`/`cleanup_state`/`diagnostics`. `process.rs:688-692` and `lifecycle.rs:333-335` both derive `cleanup_state` from `diagnostics.is_empty()`.
- Proposal: add `ShutdownOutcome::new(native_exit, diagnostics)` (state derived from the diagnostics) and `ShutdownOutcome::complete(native_exit)`. Use them at every site.
- Spec impact: none — serialized fields and values are unchanged (`control.rs` keeps `Incomplete` through its non-empty diagnostics).
- Risk: low.
- Estimated delta: −30 lines

### SVC-09: Remove the duplicate open/validate paths and test forwarders in `Outbox`
- Category: duplication
- Location: `cafleet/src/runtime/runner/outbox.rs:94-146, 179-188, 192-214, 304-311`
- Evidence: `open_recovery` (94-120) and `open_owned` (121-146) share the same validate → `validate_private_file` → `O_NOFOLLOW` open → dev/ino comparison block (~20 lines), with the comparison written two ways (113 vs 140). `validate` (192-206) and `validate_on` (207-214) differ only in which connection they use. `from_io` (179-188) is a `#[cfg(test)]` forwarder to the private `initialize`; its only callers are `outbox.rs`'s own child `mod tests`, which can call `initialize` directly. `append_synced` (304-311) is a `#[cfg(test)]` forwarder adding `None`. The effective-policy kind check runs three times: `writer.rs:409-413`, `outbox.rs:346-350` and `outbox.rs:70-74`. The writer is the only caller that passes `Some`.
- Proposal: extract `fn open_private(conn, run, lock, until) -> Result<(File, (u64, u64))>`. Implement `validate` as `validate_with(self.conn…)`. Delete `from_io` (tests call `initialize`). Keep the read-side check in `parse_frame_body`. Removing the append-side check in `append_validated` is an optional extra, and it requires adjusting the first assertion of `runner_outbox_rejects_an_effective_policy_on_any_other_frame`.
- Spec impact: none — outbox bytes, checks and error strings are unchanged (the optional append-check removal keeps the writer-side error).
- Risk: low.
- Estimated delta: −35 lines

### SVC-10: Outbox tests — drop duplicates and factor the commit/compact boilerplate
- Category: redundant-test
- Location: `cafleet/src/runtime/runner/outbox.rs:1316-1345, 1347-1381, 466-481`; boilerplate throughout 483-1411
- Evidence: `runner_batch2_outbox_compaction_checks_event_message_association_before_removal` (1316-1345) is the `"error"` case of `runner_review_r2_compaction_restores_supplied_connection_after_success_error_and_unwind` (593-598, 613, 627): the same `UPDATE messages SET text=…`, the same `compact_committed` error and the same byte preservation. `runner_batch2_outbox_truncate_failure_preserves_committed_originals_for_retry` (1347-1381) is covered by the `committed=true` case of `runner_recovery_terminal_tail_truncate_failure_retains_bytes_and_retries_the_same_owner` (718-752). `storage::persist_transition(&mut fixture.fixture.conn, &owner, &v, PersistenceMode::Live).unwrap()` appears 8 times and `outbox.compact_committed(&mut fixture.fixture.conn, &fixture.fixture.run.owner(), deadline())` 12 times. `ordinary_connection_work` (466-481) repeats `lifecycle.rs` `progress_checked_sum` (SVC-11).
- Proposal: delete the two subsumed tests, or add a no-tail case to the terminal-tail test. Add `commit(&mut fixture, &value)` and `compact(&mut outbox, &mut fixture)` helpers. Share one recursive-CTE probe (SVC-11).
- Spec impact: none — test-only.
- Risk: low.
- Estimated delta: −120 lines

### SVC-11: Lifecycle tests — merge clock modules, share the SQL probe, parametrize nested scopes
- Category: redundant-test
- Location: `cafleet/src/runtime/runner/lifecycle.rs:339-391, 564-588, 646-872`
- Evidence: the file has two test modules (`shutdown_tests`, `lifecycle_tests`). `runner_shutdown_clock_anchors_all_work_to_one_original_instant` (573-588) is mostly covered by `runner_recovery_clock_clips_each_original_boundary…` (349-374): its offset-9 case equals the unclipped schedule. The long recursive CTE literal appears 6 times (654, 659, 690, 788, 811, 849) and once more in `outbox.rs:474, 504`. `runner_database_deadline_review_inner_unwind_restores_the_live_outer_cancellation_scope` (764-801) and `assert_retired_inner_scope_preserves_outer` (824-862) are the same scenario with different inner outcomes (panic vs Ok/Err).
- Proposal: use one `mod tests`. Fold the remaining-time assertions into the clip test. Add `const SLOW_SUM`/`FAST_SUM` plus helpers (shared with `outbox.rs` from one `#[cfg(test)]` location in `lifecycle.rs`, which owns `with_database_deadline`). Run one nested-scope helper over `{success, error, panic}`.
- Spec impact: none.
- Risk: low.
- Estimated delta: −90 lines

### SVC-12: Deduplicate exit reconciliation in `lifecycle::shutdown`
- Category: duplication
- Location: `cafleet/src/runtime/runner/lifecycle.rs:268-296`
- Evidence: the drain-exit block (268-283) and the final-exit block (286-296) are the same "record the first exit, or flag a conflict" logic and differ only in the message. `ShutdownHost`'s two default methods (`final_native_exit`, `native_was_spawned`, 233-238) are overridden by the only production implementor `LiveShutdown` (`service.rs:344-349`); the defaults serve just the test `Host`.
- Proposal: add `fn reconcile_exit(outcome, exit, conflict: &str)`. Make both trait methods required and implement them in the test fake.
- Spec impact: none — diagnostic strings are kept verbatim.
- Risk: low.
- Estimated delta: −12 lines

### SVC-13: One bounded-sleep helper and one deadline check
- Category: duplication
- Location: `transport/drain.rs:28-34`; `creation.rs:444-450`; `control.rs:16-22`; bounded sleeps at `service.rs:235, 302, 317, 338`, `process.rs:294, 665`, `writer.rs:199, 580`, `runner_tests/owned_child.rs:25`
- Evidence: `drain::before_deadline` is `check_deadline(Instant::now(), deadline)` in full (both return `CafleetError::DeadlineExceeded`). `check_deadline` is defined in `creation.rs`, although `control`, `codec`, `writer`, `outbox`, `service` and `drain` use it. The idiom `std::thread::sleep(Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())))` appears 9 times in production code; `control::pause` is the same with a deadline check in front. `service.rs:362` sleeps a fixed 2 ms without the deadline bound, unlike its siblings.
- Proposal: move `check_deadline` next to `ensure` in `runner/mod.rs` and delete `before_deadline`. Add `fn nap_until(deadline)` there and use it at each site (`pause` becomes `check_deadline` + `nap_until`).
- Spec impact: none. `service.rs:362` gains the same deadline bound its sibling loops use; the loop already exits at `deadline`.
- Risk: low.
- Estimated delta: −30 lines

### SVC-14: Remove the "re-run the baseline first" preambles in tests
- Category: redundant-test
- Location: `codec.rs` (`legacy()`, 7 calls); `control.rs` (`baseline(&fixture)`, 6 calls); `process.rs` (`baseline()`, 6 calls at 1417-1551)
- Evidence: in each module, one test is the baseline itself (`step5_wire_x1_…` = `baseline`; `step5_refresh_matching_anchor…` = `baseline`). Every sibling then re-runs it before its own scenario, adding 19 executions of an already-standalone test.
- Proposal: delete the preamble calls (the codec and control counts overlap with SVC-03/SVC-04; the process count is new).
- Spec impact: none.
- Risk: low.
- Estimated delta: −19 lines (−6 net beyond SVC-03/04)

### SVC-15: Move cross-module test helpers out of `writer.rs` and reuse the existing fixture form
- Category: test-placement
- Location: `cafleet/src/runtime/runner/writer.rs:840-952` (`contract_tests::support`); consumers `observation.rs:1230-1234`, `transport/drain.rs:245-247`, `outbox.rs:455, 901`, `input.rs`
- Evidence: `support::form(op)` (935-937) is byte-for-byte `coding_agent::profiles::catalog::inspection_fixture_form(op, "synthetic-only")` (`catalog.rs:72-74`). `support::Clock` implements `ObservationClock` (observation-owned). `support::catalog`/`definitions` is one of 7 test loaders of `synthetic-profile.json` (inventory row). `ChildGuard` wraps the test `OwnedChild` (SVC-21). None of these helpers relate to `EventWriter`.
- Proposal: replace `form` with `inspection_fixture_form`. Move `Clock` to `observation.rs` `#[cfg(test)]`. Add a single `#[cfg(test)] pub(crate) fn synthetic_profile()` in `coding_agent/profiles/catalog.rs`, move the JSON next to it, and have all 7 sites call it. Keep `worker::{Hooks, Release, wait_until, committed}` and `WriterFixture` in `writer.rs`.
- Spec impact: none — the fixture is not bundled (every `include_str!` is under `#[cfg(test)]`).
- Risk: low.
- Estimated delta: −40 lines

### SVC-16: `system.rs` — share spawn/decoding and narrow visibility
- Category: duplication
- Location: `cafleet/src/runtime/system.rs:19-33, 74-159, 255-273, 461-518`
- Evidence: the mapping of spawn errors to `BinaryNotFound`/`NotStarted` is written twice (88-96, 127-134). The decoding "status success → lossy stdout, else `Failed{lossy stderr}`" is written 3 times (103-109, 146-152, 265-272). `find_on_path` and `find_executable_in` are `pub`, but their only callers are inside `system.rs` (grep: 0 external callers). The test module nests a `mod press_tests` (461-518) whose single test covers `SystemRunner::run_until`, not press.
- Proposal: add `fn spawn_piped(argv) -> Result<Child, RunError>` and `fn decode(status, stdout, stderr) -> Result<String, RunError>`. Make the two path functions private. Inline the misnamed nested module.
- Spec impact: none — error variants and strings are unchanged.
- Risk: low.
- Estimated delta: −30 lines

### SVC-17: `runtime/mod.rs` — drop the alias layer and one-use helpers
- Category: over-abstraction
- Location: `cafleet/src/runtime/mod.rs:9-18, 27-29, 39-56`
- Evidence: five test re-exports rename items at the crate root (`ServiceFixture as InputServiceFixture`, `barrier as input_service_barrier`, `composer as input_service_composer`, `records as input_service_records`, `wait_record as wait_input_service_record`, `with_service as with_input_service`). That gives every harness function two names. `env_snapshot` (27-29) is a one-line helper used once. `deliver_pending` (39-56) only forwards to `crate::delivery::deliver_pending` with `|| resolve_mux(settings)` and has 2 callers (`receive.rs:245`, `cli/message.rs:111`).
- Proposal: when the harness moves into `service.rs` (inventory), give its functions their descriptive names at the definition and drop the aliases. Inline `env_snapshot`. Optionally, have the two callers pass `|| crate::runtime::resolve_mux(settings)` directly and delete the wrapper.
- Spec impact: none.
- Risk: low.
- Estimated delta: −20 lines

### SVC-18: `creation.rs` — fold `launch_registered`, reuse `ControlIdentity` constructors
- Category: over-abstraction
- Location: `cafleet/src/runtime/runner/creation.rs:36, 238-260, 262-274`; `control.rs:224-232`; `runner_tests/creation.rs:75-85`
- Evidence: `launch_registered` is `pub(crate)`, but its only caller is `create_registered` (grep: 1 caller). It also builds a second `PendingGuard` right after `create_registered` disarms the first (258-274). `register_pending_run` is `pub(crate)`, with only internal callers plus tests. `NativeLaunchHost::wait_prepared` builds `ControlIdentity` field by field from a `LaunchDescriptor` (224-232). The test `Host::identity` re-implements `ControlIdentity::from_run` field by field.
- Proposal: make `launch_registered` private and pass the existing guard (`&mut PendingGuard`) into it instead of re-creating one. Narrow `register_pending_run` to `pub(super)`. Add `ControlIdentity::from_descriptor` (or `LaunchDescriptor::identity()`) and use `from_run` in the test host.
- Spec impact: none.
- Risk: low.
- Estimated delta: −25 lines

### SVC-19: `service.rs` small redundancies — forwarding re-export and subsumed tests
- Category: redundant-test
- Location: `service.rs:2`; `service.rs:1294-1303`; `service.rs:1479-1496`
- Evidence: `pub(super) use super::control::request;` exists only so that `health.rs:50` and `recovery.rs:494` can write `super::service::request`. `pacing_keeps_the_idle_sleep_when_a_blocked_master_accepted_no_queued_input` (1294-1303) asserts `progress(0,0)` cases already asserted in `pacing_keeps_the_idle_sleep_without_outer_input` (1270-1292). `step5_wire_a4_actual_client_rejects_partial_eof_and_oversized_header` (1479-1496) repeats codec oversize/EOF tests through a socket pair; the socket-specific value (EOF via `Client::poll`) is one assertion.
- Proposal: point both callers at `super::control::request` and drop the re-export. Delete the subsumed pacing test. Keep only the partial-EOF assertion of a4.
- Spec impact: none.
- Risk: low.
- Estimated delta: −25 lines

### SVC-20: `#[cfg(test)]` instrumentation inside production paths (needs-verification)
- Category: other
- Location: `transport.rs:94-99, 181-186, 211-219, 263-266, 276-292, 417-441, 450-454`; `transport/drain.rs:193-201, 214-218`; `service.rs:219-240, 259-261, 267-271, 396-399, 435-438, 539-543, 862-887, 903-913`; `runner/mod.rs:11-14` (`native_capture` compiled only in tests); `writer.rs:38-39, 60-61, 94-97, 161-162, 554-555`
- Evidence: 76 `#[cfg(test)]`/`#[cfg(not(test))]` gates across the slice's production files (grep counts: process 25, service 15, transport 14, writer 11, drain 3, outbox 3, mod 3, codec 2). Most of them serve the test-only `native_capture` verification suite: the observation hooks, observation phase, deferred child bytes/prefetch, capture port, `await_native_exit_before_parsing` and `policy_diagnostic`. The compiled test binary therefore runs different branches from the release binary.
- Proposal: decide together with the OBSERVE slice. If the native-capture verification suite is retired or reworked into an injected dependency, delete these gates and their fields (~150 lines in this slice). If it stays, inject one `RunnerInstrumentation` value with inert production implementations so the code path is the same in both builds.
- Spec impact: none — release behavior is identical either way.
- Risk: medium. It depends on what the native-capture tests are kept for.
- Estimated delta: −150 lines (conditional; excluded from the total)

### SVC-21: Name and place the test child guard beside the production one
- Category: test-placement
- Location: `cafleet/src/runtime/runner_tests/owned_child.rs:1-104`; `system.rs:161-244`; `writer.rs:903-926` (`ChildGuard`)
- Evidence: two different `OwnedChild` types exist: production `system::OwnedChild` (private, absolute-deadline cleanup) and test `runner_tests::owned_child::OwnedChild` (Deref plus kill/reap on drop), used in 9 files. `writer::support::ChildGuard` adds a `ProcessIdentity` on top of the test guard.
- Proposal: move the test guard into `system.rs` as `#[cfg(test)] pub(crate) struct TestChild` (with its unwind test). Give it an optional `identity()` accessor that replaces `ChildGuard`.
- Spec impact: none.
- Risk: low.
- Estimated delta: −20 lines

### SVC-22: Misplaced transport test
- Category: test-placement
- Location: `cafleet/src/runtime/runner/transport.rs:842-868`
- Evidence: `runner_transport_visits_a_transient_screen_before_a_later_clear_in_one_fragment` drives `TerminalState::feed_observed` directly and never builds a `Transport`.
- Proposal: move it to `terminal.rs` `mod tests`.
- Spec impact: none.
- Risk: low.
- Estimated delta: 0 lines

### SVC-23: Duplicate `sha256_hex`
- Category: duplication
- Location: `cafleet/src/runtime/runner_tests/mod.rs:77-85`; `cafleet/src/coding_agent/profiles/catalog.rs:1458-1464`
- Evidence: two implementations of the same lowercase SHA-256 hex encoding. The test copy has 9 test call sites.
- Proposal: make the catalog function `pub(crate)` and delete the test copy.
- Spec impact: none.
- Risk: low.
- Estimated delta: −9 lines
