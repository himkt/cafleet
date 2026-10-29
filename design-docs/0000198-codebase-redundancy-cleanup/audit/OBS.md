# Audit: OBS — runtime runner observation and evidence

## Summary

The largest item is structural. `runner/native_capture` (`native_capture.rs`, `exporter.rs`, `files.rs`, `verification.rs`, `review_service_tests.rs`; about 3,660 lines) is compiled only under `#[cfg(test)]` (`runner/mod.rs:11-14`). It is the maintainer's private native-capture harness. Production types carry `#[cfg(test)]` hook fields for it (`ObservationController.capture`, `RunnerDependencies.capture`). Another ~2,600 lines of `runner_tests/` exercise it. Whether to keep this harness, or how to fold it, is a user decision (OBS-01).

Within production code, the clear wins are these:
- `ScreenSnapshot` builds test-only `contents`/`title` fields on every live snapshot, and `#[allow(dead_code)]` hides this (OBS-02).
- A production clock-kind check is tautological (OBS-03).
- Recovery, descriptor, episode and terminal logic contain several duplicated helpers (OBS-12, OBS-14 to OBS-19).
- The lifecycle-evidence JSON literal and SHA-256 hex encoding are each copied across the repository (OBS-06, OBS-07).

Test-side wins:
- Three near-identical deterministic test clocks (OBS-04).
- Oversized, repeated controller test setup (OBS-22).
- Misplaced and overlapping health tests (OBS-23).
- 113 test names that carry review-round or batch provenance (OBS-21).

Every `runner_tests/recovery/**` and `runner_tests/native_capture/**` file has a concrete target implementation file. Self-exec libtest path literals must be updated in the same change (inventory notes).

Estimated removable code:
- Keeping the harness: ~-660 lines (~-510 from OBS-02 to OBS-25 plus ~-150 harness consolidation).
- Removing the harness (OBS-01 option b): ~-6,800 lines. OBS-08 to OBS-11 and OBS-13 are then subsumed.

## Test placement inventory

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `cafleet/src/runtime/runner/native_capture/review_service_tests.rs` | 107 | `runner/native_capture.rs` (`#[cfg(test)] mod tests`; the module itself is already cfg(test)) | Uses `run_with_test_services` from `service.rs` | Ignored self-exec child `runner_collector_resize_child`; its libtest path literal lives at `runner_tests/control_service.rs:282` and must be updated in the same change. |
| `cafleet/src/runtime/runner_tests/native_capture.rs` | 70 | `runner/native_capture/verification.rs` | `sha256_hex` from `runner_tests/mod.rs:77` | Ignored entry `native_capture_service_child`; path literal hard-coded at `verification.rs:944`, `runner_tests/control_service.rs:305`, `runner_tests/native_capture/files.rs:429`. |
| `cafleet/src/runtime/runner_tests/native_capture/collection.rs` | 590 | `runner/native_capture.rs` (port/receiver/projection/sample limits) | `support.rs` `CaptureFixture`, `Clock` | — |
| `cafleet/src/runtime/runner_tests/native_capture/files.rs` | 456 | `runner/native_capture/files.rs` (arm reader, config, producer, publication); the `capture_bind_checks_actual_native_digest` test → `native_capture.rs` (`bind_prepared`) | `support.rs` | Contains a self-exec path literal (line 429). |
| `cafleet/src/runtime/runner_tests/native_capture/review_existing.rs` | 313 | `native_capture.rs` (`bind_prepared` permission/catalog-origin checks, `f1` tests); `native_capture/files.rs` (`read_native_capture_producer` duplicate-field tests, `f4`) | `support.rs` | — |
| `cafleet/src/runtime/runner_tests/native_capture/review_policy.rs` | 150 | `runner/native_capture.rs` (`NativeCapturePolicy`) | Test `Compiler` impl of `NativeCapturePatternCompiler` | See OBS-11 (trait exists only for this file). |
| `cafleet/src/runtime/runner_tests/native_capture/support.rs` | 276 | `#[cfg(test)] mod test_support` inside `runner/native_capture.rs` | `CaptureFixture`, deterministic `Clock` (OBS-04), loads `fixtures/synthetic-profile.json` | Shared helper for the four files above. |
| `cafleet/src/runtime/runner_tests/control_service/review_export.rs` | 747 | `runner/native_capture/exporter.rs` (export lifecycle, publication revocation) | `ServiceFixture` from `runner_tests/control_service.rs` (service slice) | Ignored child `runner_collector_export_child`; path literal at `runner_tests/control_service.rs:285`. |
| `cafleet/src/runtime/runner_tests/recovery.rs` | 422 | `#[cfg(test)] mod test_support` inside `runner/recovery.rs` | `Case`, `Host` (`RecoveryHost` impl), evidence helper at line 318 (OBS-06) | Shared helper for every `recovery/*` file below. |
| `cafleet/src/runtime/runner_tests/recovery/foundations.rs` | 330 | `runner/artifacts.rs` (lock transfer: `transfer_to_reconciler_*` tests); `runner/recovery.rs` (stable-identity tests); presentation test `runner_recovery_presentation_pins_ordered_json_outcomes…` → the module owning recovery JSON/text rendering; pane-guard tests → `crate::input` (`PaneLock` owner) | `recovery.rs` `Case` | needs-verification: pane-guard test targets; confirm via the `PaneLock` methods they call. |
| `cafleet/src/runtime/runner_tests/recovery/replay.rs` | 491 | `runner/recovery/replay.rs` | `Case`, `Host`; local `unavailable()` evidence builder (OBS-06) | — |
| `cafleet/src/runtime/runner_tests/recovery/history.rs` | 288 | `runner/recovery/replay.rs` (archive disposition, V1/V2 bytes preservation) | `Case` | — |
| `cafleet/src/runtime/runner_tests/recovery/terminal_tail.rs` | 158 | `runner/recovery/replay.rs` (`incomplete_tail` diagnostic path) | `Case`, `Host` | — |
| `cafleet/src/runtime/runner_tests/recovery/missing_paths.rs` | 522 | `runner/recovery/exceptional.rs` (database-only reconciliation, attestation) | `Case`, `Host` | — |
| `cafleet/src/runtime/runner_tests/recovery/review_history.rs` | 354 | `runner/recovery/exceptional.rs` | `Case`; local `TableSnapshot` (duplicate of `health.rs:466` `TableSnapshot` and `registry_snapshot`) | Merge the two table-snapshot helpers into one `#[cfg(test)]` helper. |
| `cafleet/src/runtime/runner_tests/recovery/identity.rs` | 397 | `runner/recovery.rs` (coordinator `recover`, borrowed pane variant) | `Case`, `Host` | — |
| `cafleet/src/runtime/runner_tests/recovery/review_completion.rs` | 238 | `runner/recovery.rs` (same-owner completion rechecks) | `Case` | — |
| `cafleet/src/runtime/runner_tests/recovery/review_relationships.rs` | 443 | Split by branch: ordinary takeover → `runner/recovery.rs`; exceptional zero-event → `recovery/exceptional.rs`; finalization after replay → `recovery/replay.rs` | `Case` | The nine near-identical `…member_fleet/director_fleet/fleet_director` tests are already table-driven via helpers; keep them as one parameterized test per branch. |
| `cafleet/src/runtime/runner_tests/recovery/review_clock.rs` | 166 | `runner/recovery.rs` (`ShutdownClock` entry clock) | `ClockHost` (`RecoveryHost` impl) | — |
| `cafleet/src/runtime/runner_tests/recovery/review_descriptor.rs` | 386 | `runner/recovery/replay.rs` (coordinator descriptor removal) | `Case` | — |
| `cafleet/src/runtime/runner_tests/recovery/review_descriptor/io.rs` | 318 | `runner/artifacts/recovery_descriptor.rs` (direct `pin`/`remove`) | `UnlinkHost` (`RecoveryHost` impl) | — |
| `cafleet/src/runtime/runner_tests/recovery/crash.rs` | 626 | `runner/recovery/replay.rs` (hard kill after/before fsync); `runner/recovery/exceptional.rs` (exceptional transaction kill/concurrency) | `Case`, writer support | Ignored self-exec child `runner_recovery_owned_process_child`; path literal at `crash.rs:46`. |
| `cafleet/src/runtime/runner_tests/recovery/deletion.rs` | 642 | `runner/recovery.rs` (`recover_member_with_held_pane_lock` under deletion budget) | `Case`, `Host` | needs-verification: some tests may belong with member/fleet deletion (`cli/member.rs` or `runner_tests/fleet_deletion.rs` owner, outside this slice). |
| `cafleet/src/runtime/runner_tests/mod.rs` (`Fixture`, `sha256_hex`, fixture byte IO) | 168 | `Fixture` → `#[cfg(test)] pub(crate) mod test_support` in `runner/artifacts.rs` (it builds `RuntimePaths`/`RunArtifacts`/`SpawnSpec`). `sha256_hex` → shared digest helper (OBS-07). | Used by `artifacts.rs`, `health.rs`, `observation.rs` tests in this slice | The `.p194-` temp prefix carries design provenance (OBS-21). |
| `cafleet/src/runtime/runner_tests/owned_child.rs` | 104 | Outside slice. Merge with the private production `OwnedChild` in `runtime/system.rs:161`, or move into `runner/process.rs` as `#[cfg(test)]` | Used by the `observation.rs:18` inspection test and `cli/member.rs` | Two same-named child-guard types exist. |
| `cafleet/src/runtime/runner_tests/fixtures/synthetic-profile.json` | — | Test-only fixture (only `include_str!` under `#[cfg(test)]`; not bundled). Move next to its owner `coding_agent/profiles/` | Parsed separately at 7 sites (OBS-25) | — |
| `cafleet/src/runtime/runner_tests/fixtures/claude-2.1.302-blocked-form-blink-1.raw` | — | Test-only fixture (`include_bytes!` at `runner/input.rs:1046`). Move next to `input.rs` / profiles fixtures | — | Outside slice; listed because the fixture directory must leave `runner_tests/`. |

The in-slice inline test modules already follow the required placement: `artifacts.rs:524`, `health.rs:154`, `terminal.rs:187`, `observation.rs:11`, `observation.rs:1226`. Their layout problems are covered by OBS-22 and OBS-23.

## Findings

### OBS-01: Native-capture harness is a cfg(test)-only implementation tree wired through production types
- Category: test-placement
- Location: `cafleet/src/runtime/runner/mod.rs:11-14`; `runner/native_capture.rs:1-1108`; `runner/native_capture/{exporter.rs,files.rs,verification.rs,review_service_tests.rs}`; hooks at `runner/observation.rs:344-347,497-500,912-931,954-959,1001-1004,1114-1117,1211-1222`; `runner/service.rs:396-408`
- Evidence: `mod native_capture;` and its re-export are both `#[cfg(test)]`. Every entry point (`run_native_capture_verification`, `native_capture_channel`, `NativeCapturePolicy::from_launch`) has callers only in test code. The runs are `#[ignore]` entries driven by `CAFLEET_NATIVE_CAPTURE_CONFIG` (`runner_tests/native_capture.rs:11`, `verification.rs:1585`). Git history labels it "the private harness" (e.g. `3424fc04 chore: add an OpenCode herdr-environment capture case to the private harness`). The docs (`docs/docs/spec/member-runtime.md:73`, `SPEC.md:2367`) say bundled fixtures "rest on accepted native captures", but no doc names the harness itself. Production structs carry `#[cfg(test)]` fields and branches for it.
- Proposal (needs user decision):
  - (a) Keep the harness. Collapse it into `runner/native_capture.rs` plus its submodules, with all its tests inline (inventory rows above). Apply OBS-08 to OBS-11 and OBS-13.
  - (b) Delete the harness and its tests. Remove every `#[cfg(test)]` capture hook from `ObservationController`, `RunnerDependencies` and `RunnerTestServices`, and the `CaptureTrigger` calls. Bundled fixtures stay as committed data; regeneration would then be done out of tree.
- Spec impact: none. The tree is not compiled into the shipped binary.
- Risk: medium. Option (b) removes the maintainer's only in-tree way to re-capture native evidence for new profile versions.
- Estimated delta: (a) about -150 lines; (b) about -6,300 lines (slice ~3,660 + `runner_tests/native_capture/**` 1,855 + `review_export.rs` 747 + hooks ~40).

### OBS-02: ScreenSnapshot builds test-only `contents` and `title` on every live snapshot
- Category: dead-code
- Location: `cafleet/src/runtime/runner/terminal.rs:5-13,15-27,128-143`
- Evidence: `#[allow(dead_code)]` sits on `ScreenSnapshot`. Every read of the `.contents` field is in test code (`terminal.rs:203-261`, `transport.rs:601`, `transport/drain.rs` test modules, `profiles.rs:87`). The detector uses only `screen`, `rows`, `cols`, `cursor_*`, `alternate_screen` and `screen_seq` (`detector.rs:1574-1609`). `.title` is read only by `native_capture.rs` (cfg(test)) and terminal tests. `snapshot()` runs per control byte in `feed_observed_classified`, so each call allocates a full-screen `String` in production.
- Proposal: Delete the `contents` field; tests read `snapshot.screen.contents()`. Make the `title` field, the `Callbacks` title capture and `new_with_callbacks` `#[cfg(test)]`, or delete them with OBS-01(b). Delete `#[allow(dead_code)]`.
- Spec impact: none. Neither field reaches events, evidence or output.
- Risk: low. About 30 test call sites need a mechanical edit.
- Estimated delta: -12 lines (production), ±0 tests

### OBS-03: Tautological production clock-kind check
- Category: dead-code
- Location: `cafleet/src/runtime/runner/observation.rs:229-238,261-272,867-871`
- Evidence: `ObservationClockKind::Deterministic` is `#[cfg(test)]`, so in non-test builds the enum has the single variant `Real`. The `#[cfg(not(test))] ensure(clock.kind() == ObservationClockKind::Real, …)` therefore cannot fail. `kind()` is consumed only by `native_capture.rs:471,621,624` (cfg(test)) and test clocks.
- Proposal: Make `ObservationClockKind` and `ObservationClock::kind` `#[cfg(test)]`, and delete the `#[cfg(not(test))]` ensure.
- Spec impact: none. The error `live observation requires a real clock` is unreachable.
- Risk: low.
- Estimated delta: -10 lines

### OBS-04: Three near-identical deterministic test clocks plus a copy of the real clock
- Category: duplication
- Location: `cafleet/src/runtime/runner/writer.rs:885-902` (`Clock`); `runner/observation.rs:1518-1552` (`Timeline`); `runner_tests/native_capture/support.rs:57-75` (`Clock`); `runner_tests/control_service/observation_services.rs:7-18` (`Clock`)
- Evidence: the three deterministic clocks are each `origin: Instant` plus `AtomicU64` millis with `set(ms)`; they differ only in how `observed_at` is derived (fixed `NOW`, `NOW`+ms, or a fixed literal). `observation_services.rs` `Clock` is byte-for-byte `RealObservationClock` (`observation.rs:261-272`).
- Proposal: Add one `#[cfg(test)] pub(crate) struct ManualObservationClock` in `observation.rs`, the owner of the trait, with `set(ms)` and an advancing `observed_at`. Replace the other three, and use `RealObservationClock` in `observation_services.rs`.
- Spec impact: none. Test-only.
- Risk: low. Confirm that no test asserts a fixed `observed_at` from the writer clock; `NOW`+0 equals `NOW`.
- Estimated delta: -40 lines

### OBS-05: `ObservationSite` is a test-only alias for `ObservationPhase`
- Category: over-abstraction
- Location: `cafleet/src/runtime/runner/observation.rs:246-247` (≈15 uses in `observation.rs`, `review_service_tests.rs`, `transport/drain.rs:1805`, `runner_tests/control_service/review_failures.rs`)
- Evidence: `pub(crate) type ObservationSite = ObservationPhase;` adds a second name for one enum.
- Proposal: Delete the alias and use `ObservationPhase`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -2 lines (one concept)

### OBS-06: The run-scoped metadata evidence JSON literal is copied across production and tests
- Category: duplication
- Location: `cafleet/src/runtime/runner/observation.rs:682`; `runner/recovery/replay.rs:114-118`; tests: `writer.rs:1382,1650`, `input_client.rs:4766,5046,5311`, `input_protocol/wire.rs:371`, `runner_tests/recovery.rs:318`, `runner_tests/recovery/replay.rs:11`, `runner_tests/creation.rs:199`, `runner_tests/control_service/observation_contract.rs:250`
- Evidence: every copy builds the same 11-key `{"backend","native_version","profile_version","rule_id","screen_seq","operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}` object and passes it through `EvidencePolicy::metadata().apply`. Copies differ only in `rule_id` and `screen_seq`.
- Proposal: Add one helper, e.g. `evidence::run_metadata(run: &RunRecord, rule_id, screen_seq) -> Result<Value>`, owned by `broker::runtime::evidence`. Use it from `ObservationController::lifecycle`, `replay::loss_transitions` and the tests.
- Spec impact: none. Identical key set and values, so `evidence_digest` is unchanged.
- Risk: low. The helper lands outside this slice (`broker::runtime::evidence`); coordinate with that auditor.
- Estimated delta: -30 lines

### OBS-07: SHA-256 lowercase-hex encoding is re-implemented in many places
- Category: duplication
- Location: in slice: `native_capture/files.rs:8-24,565-568`, `native_capture/verification.rs:627-632`; touching: `runner_tests/mod.rs:77-85`, `coding_agent/profiles/catalog.rs:1458`; repo-wide, about 25 inline `.map(|byte| format!("{byte:02x}"))` sites (`input.rs:543`, `capture.rs:23`, `delivery.rs:1216`, `broker/mod.rs:620,5147`, `broker/runtime/evidence.rs:261`, `broker/runtime/permissions.rs:144`, `cli/member.rs:461,1457`, `runner/input_client.rs:200,3316`, `runner/input.rs:1193,5394,7666`, `coding_agent/profiles.rs` ×7)
- Evidence: there are three named helpers (`sha256_hex` ×2, `driver_digest`), one streaming file digest, and many inline encoders that all produce the same lowercase hex string.
- Proposal: Add one `crate::digest::{sha256_hex(&[u8]), sha256_file_hex(&Path)}` and replace every copy.
- Spec impact: none. Output is byte-identical lowercase hex.
- Risk: low. Cross-slice; this slice owns 4 sites.
- Estimated delta: -15 lines in slice, about -60 lines repo-wide

### OBS-08: Four atomic private-file publishers inside the capture harness
- Category: duplication
- Location: `native_capture/verification.rs:638-669` (`driver_publish`); `native_capture/files.rs:388-506` (`PublicationTemporary`/`publish_file`); `runner_tests/native_capture.rs:52-64`; `native_capture/review_service_tests.rs:34-46` (`write_pair`)
- Evidence: each does create_new mode 0600 (O_NOFOLLOW), write, fsync, then hard_link or rename, then cleanup.
- Proposal: Route the three ad-hoc writers through `files.rs` `publish_file(…, control=None, &InertCaptureExportHooks, …)`, or a thin `publish_private(path, bytes, until)` wrapper over it.
- Spec impact: none. Test-only harness.
- Risk: low. Applies only under OBS-01(a).
- Estimated delta: -45 lines

### OBS-09: Duplicate shutdown-deadline clipping and permit acquisition in the exporter
- Category: duplication
- Location: `runner/native_capture.rs:520-526` (`NativeCaptureReceiver::publication_deadline`); `native_capture/exporter.rs:56-62` (`CaptureExportControl::deadline`); `native_capture/files.rs:441-452` (`operation_permit`) vs `exporter.rs:233-239`
- Evidence: both deadline functions compute `state.shutdown_until.map_or(until, |end| end.min(until))` over the same `Arc<Mutex<CaptureState>>`. The `match control { Some → acquire, None → check_deadline }` tail of `publication_permit` is `operation_permit` verbatim.
- Proposal: Keep one `CaptureState::clip(until)`. Implement `publication_permit` as `hooks.before(…)?; operation_permit(control, until)`.
- Spec impact: none. Test-only harness.
- Risk: low.
- Estimated delta: -20 lines

### OBS-10: Two bounded JSON writers in the capture harness
- Category: duplication
- Location: `runner/native_capture.rs:1084-1108` (`SizeWriter`, `encoded_size`); `native_capture/files.rs:367-387` (`BoundedBytes`)
- Evidence: both implement `io::Write` with a checked-add byte limit. One counts bytes; the other also buffers them.
- Proposal: Keep one bounded writer with an optional buffer, and implement `encoded_size` on it.
- Spec impact: none.
- Risk: low.
- Estimated delta: -15 lines

### OBS-11: NativeCapturePolicy validates and compiles patterns twice; a one-implementor trait serves a test seam
- Category: over-abstraction
- Location: `runner/native_capture.rs:127-143,153-172,180-263,409-410`
- Evidence:
  - `from_launch_with_compiler` checks pattern count and bytes against `NativeCaptureLimits::default()` and compiles with the injected compiler.
  - `apply_limits` repeats the count and bytes checks against the selected limits, which are already ≤ defaults by `validate`. It then recompiles every pattern with the hard-coded `BoundedCapturePatternCompiler`, ignoring the injected compiler.
  - `native_capture_channel` (the only consumer) always calls `apply_limits`.
  - `NativeCapturePatternCompiler` has one production implementor; the other implementor is the test `Compiler` in `runner_tests/native_capture/review_policy.rs:9`.
- Proposal: Store raw pattern strings in the policy and keep the structural checks in `from_launch`. Compile exactly once in `apply_limits` with the selected `max_compiled_regex_bytes`. Delete the trait and `from_launch_with_compiler` if the `review_policy.rs` cases can be expressed through compiled-size limits.
- Spec impact: none. Test-only harness.
- Risk: medium. needs-verification: confirm that each `review_policy.rs` case (lines 60-150) asserts behavior reachable through limits alone.
- Estimated delta: -35 lines

### OBS-12: Launch-descriptor parsing and size limit duplicated between loader and recovery pin
- Category: duplication
- Location: `cafleet/src/runtime/runner/artifacts.rs:291-295,355-373`; `runner/artifacts/recovery_descriptor.rs:4,42-60,88-94`
- Evidence: `1024 * 1024` appears at `artifacts.rs:293`, `artifacts.rs:362` and `recovery_descriptor.rs:4`. Both `load_descriptor` and `RecoveryDescriptor::pin` run `serde_json::from_slice::<LaunchDescriptor>` with the same `"malformed launch descriptor"` mapping, then the same `schema_version == 1` / `"unsupported launch descriptor schema"` check. Only the identity predicates differ legitimately: epoch equality vs `0 < epoch ≤ current`.
- Proposal: One `const DESCRIPTOR_LIMIT` in `artifacts.rs`, and one `LaunchDescriptor::parse(bytes) -> Result<Self>` used by both. Keep the two identity predicates separate.
- Spec impact: none. The error strings are preserved verbatim.
- Risk: low.
- Estimated delta: -15 lines

### OBS-13: Private-directory check duplicated between the capture harness and runtime artifacts
- Category: duplication
- Location: `runner/native_capture/files.rs:25-36` (`private_directory`) vs `runner/artifacts.rs:117-138` (`private_metadata(…, directory=true)`)
- Evidence: both reject symlinks, require a directory with euid owner and mode 0o700, and return the device/inode identity.
- Proposal: Expose `private_metadata` as `pub(super)` and have `files.rs` call it.
- Spec impact: none. Harness-only error text changes.
- Risk: low. Applies only under OBS-01(a).
- Estimated delta: -10 lines

### OBS-14: Recovery commit counting implemented twice
- Category: duplication
- Location: `cafleet/src/runtime/runner/recovery/replay.rs:35-53` (`count`) and `:234-246` (`count_exceptional`), used at `recovery/exceptional.rs:74`
- Evidence: both increment `inbox_count` / `archived_count` by `notification_state` with the same `"recovery commit count overflow"` error. `count_exceptional` equals `count` with `original=false` on a bare `RecoveryReport`.
- Proposal: One `count_commit(report: &mut RecoveryReport, event, original: bool)`, used by both branches.
- Spec impact: none. Same counters and error.
- Risk: low.
- Estimated delta: -12 lines

### OBS-15: EpisodeTracker repeats its six-field boundary reset and re-inlines `earliest()`
- Category: duplication
- Location: `cafleet/src/runtime/runner/episodes.rs:447-451,502-507,640-645` (reset block); `:592-593,605-606` vs helper `:71-73`
- Evidence: the identical `boundary/refusal_boundary/obscurity = None; obscured = false; refusal_candidate = None` block appears three times. `update.next_deadline = Some(update.next_deadline.map_or(until, |old| old.min(until)))` duplicates the existing `earliest(&mut update.next_deadline, until)` twice.
- Proposal: Extract `fn clear_boundaries(&mut self)` and call `earliest()` at both inline sites.
- Spec impact: none.
- Risk: low.
- Estimated delta: -14 lines

### OBS-16: `EpisodeUpdate` default construction is spelled out at every site
- Category: duplication
- Location: `cafleet/src/runtime/runner/episodes.rs:438-445,647-654,669-676`; `runner/observation.rs:725-734`; test `observation.rs:1256-1265`
- Evidence: each site writes all six fields with `transitions: Vec::new(), latest_evidence: None, next_deadline: None, effective_policy: None`, varying only `visibility` and `eligibility` (and `transitions`).
- Proposal: Add `EpisodeUpdate::new(visibility, eligibility)`, plus a `with_transitions` builder or struct-update syntax.
- Spec impact: none.
- Risk: low.
- Estimated delta: -18 lines

### OBS-17: `reset_observation` returns an update its only production caller discards
- Category: dead-code
- Location: `cafleet/src/runtime/runner/episodes.rs:634-655`; caller `runner/observation.rs:762`
- Evidence: `self.tracker.reset_observation(GateReason::Gap)?;` discards the `EpisodeUpdate`. That update is fully determined by the input (`Unknown`, `Hold(reason)`, no transitions). Only the tracker tests at `coding_agent/profiles.rs:2352-2480` read it.
- Proposal: Return `Result<()>`, and have those tests assert the controller's held status instead.
- Spec impact: none.
- Risk: low. Out-of-slice test edits in `profiles.rs`.
- Estimated delta: -8 lines

### OBS-18: `feed_observed_classified` repeats its feed/observe/failure policy three times
- Category: duplication
- Location: `cafleet/src/runtime/runner/terminal.rs:71-115`
- Evidence: the sequence `self.feed(slice).map_err(Parser)?; if let Err(cause) = observer.observe(&self.snapshot()) { if fatal { return Err(Observer(cause)) } failure.get_or_insert(cause); }` appears three times: the pre-control slice, the control byte and the tail.
- Proposal: Extract a local `step(&mut self, slice, observer, &mut failure)` helper.
- Spec impact: none. Same ordering and failure semantics.
- Risk: low.
- Estimated delta: -15 lines

### OBS-19: `EventSequencePool::supply` re-implements `exhausted()`
- Category: duplication
- Location: `cafleet/src/runtime/runner/episodes.rs:684-697`
- Evidence: `ensure(self.range.as_ref().is_none_or(|range| range.is_empty()), …)` is the body of `exhausted()`.
- Proposal: `ensure(self.exhausted(), "consume the current sequence grant first")`. Replace the hand-written `new()` with `#[derive(Default)]` (a `None` range and high-water 0 are the derived defaults).
- Spec impact: none.
- Risk: low.
- Estimated delta: -6 lines

### OBS-20: Health diagnostic allowlist hand-copies observation failure codes and accepts a never-produced code
- Category: duplication
- Location: `cafleet/src/runtime/runner/health.rs:125-139`; codes owned by `runner/observation.rs:280-289`
- Evidence: the allowlist lists the four `ObservationFailure::code()` strings plus `"observation_gap"`. `"observation_gap"` is produced nowhere; the only hits are `health.rs:134` and the test at `health.rs:357`. `ObservationController::health` emits only `ObservationFailure` codes and `"sink_unavailable"` (`observation.rs:1192-1196`).
- Proposal: Validate against `ObservationFailure::ALL.map(code)` and drop `"observation_gap"`.
- Spec impact: none if confirmed. The diagnostic is internal to the runner↔CLI control frame.
- Risk: medium. needs-verification: `SPEC.md:3004` keeps "health/prepare/start/shutdown frame compatibility". Confirm with `git log -S observation_gap -- cafleet/src` that no shipped runner binary ever emitted it.
- Estimated delta: -8 lines

### OBS-21: 113 slice-related tests carry review-round or batch provenance in their names
- Category: stale-doc
- Location:
  - `runner/observation.rs` (`runner_batch2_*`, `runner_review_r1_*`, `runner_review_r3_*`, `mod review`)
  - `runner/health.rs:338-435` (`runner_batch2_*`)
  - `runner_tests/recovery/*` (`runner_recovery_review_r1..r5_*`, `review_*.rs` file names)
  - `runner_tests/native_capture/*` (`runner_batch2_capture_*`, `runner_collector_f1_*`, `runner_collector_f4_*`)
  - `runner_tests/control_service/review_export.rs` (`runner_collector_t1_*`)
  - canary `"review-canary-194"` (`observation.rs:1368`); temp prefix `".p194-"` (`runner_tests/mod.rs:96`)
- Evidence: count from `grep -rhoE "fn runner_[a-z_]*(batch[0-9]|review_r[0-9]|collector_[ft][0-9])"` over the slice and its runner_tests = 113. One test name, `…validates_legacy_identity…` (`health.rs:338`), describes the current optional `observation` field as "legacy".
- Proposal: During relocation, rename each test, module and file to the behavior it asserts, and drop round, batch and design-number tokens (CLAUDE.md § Removal). Update the self-exec libtest path literals in the same change (`verification.rs:944`, `runner_tests/control_service.rs:24,282,285,305`, `runner_tests/native_capture/files.rs:429`, `runner_tests/recovery/crash.rs:46`). Keep `DRIVER_NEUTRAL_ROOT`/`DRIVER_PRIVATE_ROOT` (`/tmp/cafleet-194*`, `verification.rs:82-86`) unchanged: committed native fixtures cite those paths.
- Spec impact: none.
- Risk: medium. A stale `--exact` path silently runs zero tests, which makes the self-exec child a no-op.
- Estimated delta: 0 lines (concept cleanup)

### OBS-22: Observation controller tests: split test modules, a bespoke child guard, and repeated live setup
- Category: redundant-test
- Location: `cafleet/src/runtime/runner/observation.rs:10-223` (`inspection_tests` above the implementation), `:1225-2624` (`controller_tests` with nested `review::observation_failures`); setup repeated at `:1601,1967,2025,2093,2128,2199,2359,2493`
- Evidence:
  - Two test modules sit at opposite ends of the file.
  - The inspection test hand-rolls a `Native` drop guard (`:23-31,62-72`), its lock transfer and its writer start, although `writer::contract_tests::support::{ChildGuard, running, start}` already provide them.
  - Eight tests repeat the same ~15-line sequence: `WriterFixture::new; configure; Hooks; start; Clock; ObservationController::new(…, metadata, 1..=256); ChildGuard::spawn("/bin/cat"); running; TerminalState::new(24×80); started`. `LiveCase::build` (`:1585-1625`) already encapsulates it.
- Proposal: Use one bottom `#[cfg(test)] mod tests`. Generalize `LiveCase` with a `started()` constructor that omits the opening form, and use it in the eight tests and the inspection test. Delete the `Native` guard.
- Spec impact: none.
- Risk: low.
- Estimated delta: -120 lines

### OBS-23: Health tests include a misplaced registry test and an overlapping weaker diagnosis test
- Category: redundant-test
- Location: `cafleet/src/runtime/runner/health.rs:247-296,298-328`
- Evidence: `runner_registry_show_returns_historical_health_without_contacting_a_live_endpoint` exercises `crate::broker::get_member` and `crate::presentation::member`, not health code. `runner_doctor_runtime_diagnosis_reports_leftovers_and_preserves_registry_and_artifact_bytes` asserts a subset of the substring checks ("descriptor", "recover") and of the file-preservation checks. The `recovery_entry` tests at `:587-625` assert the same things with full table and artifact snapshots.
- Proposal: Move the registry-show test to the broker/presentation owner. Delete the weaker diagnosis test, keeping the `recovery_entry` pair. Also merge `recovery_entry::registry_snapshot`/`TableSnapshot` with the identical helper in `runner_tests/recovery/review_history.rs` (inventory).
- Spec impact: none.
- Risk: low.
- Estimated delta: -50 lines

### OBS-24: `recover_member_with_host` is a pure forward
- Category: over-abstraction
- Location: `cafleet/src/runtime/runner/recovery.rs:232-241`
- Evidence: the body is `recover(conn, settings, member_id, attestation, deadline, None, host).map(|v| v.report)`. Production calls `recover_member` (`cli/member.rs:275`), which forwards again to it. `_with_host` has only test callers (`runner_tests/recovery/*`, ~12 sites).
- Proposal: Have `recover_member` call `recover(…, &mut NativeRecoveryHost)` directly, and make the host-injecting entry `#[cfg(test)]` beside the relocated recovery tests.
- Spec impact: none.
- Risk: low.
- Estimated delta: -8 lines

### OBS-25: Synthetic profile fixture parsed separately at seven sites, from a directory scheduled for deletion
- Category: duplication
- Location: `coding_agent/profiles/catalog.rs:20-23,82-85`; `runner/input.rs:8361`; `runner/writer.rs:851`; `runner_tests/control_service/input_service.rs:133`; `runner_tests/control_service/observation_services.rs:96`; `runner_tests/native_capture/support.rs:28`
- Evidence: each site runs `serde_json::from_str(include_str!(…/runner_tests/fixtures/synthetic-profile.json))`. The file is test-only; no non-test `include_str!` references it.
- Proposal: Add one `#[cfg(test)] pub(crate) fn synthetic_profile_definition() -> Value` in `coding_agent/profiles/catalog.rs` (owner of `ProfileCatalog::from_definitions`). Move `runner_tests/fixtures/` next to that owner when `runner_tests/` is dissolved.
- Spec impact: none.
- Risk: low. Mostly out-of-slice call sites.
- Estimated delta: -15 lines
