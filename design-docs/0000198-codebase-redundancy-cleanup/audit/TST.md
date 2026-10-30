# Audit: TST — test placement across the whole crate

## Summary

The crate holds three test-only `.rs` trees and two mis-housed in-file test modules. The trees are `cafleet/src/runtime/runner_tests/` (36 files, 13,016 lines, 212 `#[test]`), the `#[cfg(test)]`-gated `cafleet/src/runtime/runner/native_capture/` module tree (5 files, 3,662 lines), and `cafleet/src/runtime/runner/process/ownership_diagnostics.rs` (381 lines, test-only). The mis-housed modules are `coding_agent/profiles.rs` `contract_tests`, which takes up lines 21–8531 of an 8,601-line file and tests `catalog.rs`/`detector.rs`/`composer.rs`/`runtime/runner/episodes.rs`, and `broker/mod.rs` `runtime_contract_tests` (lines 14–6236, 142 tests), which tests `broker/runtime/*.rs`. Three test-only data files exist: `runner_tests/fixtures/synthetic-profile.json`, `runner_tests/fixtures/claude-2.1.302-blocked-form-blink-1.raw` and `coding_agent/profiles/contract_tests/synthetic-permission-screen.json`. All 77 `coding_agent/profiles/native/*.json` files are runtime data embedded by `catalog.rs`, so they stay. Moving the files removes no behavior. The line wins come from consolidating duplicated test scaffolding: about 940 lines verified and about 400 more marked needs-verification. There is also one silent-breakage hazard: six string literals name test-module paths used to re-exec the test binary. Spec impact is none throughout: every item is `#[cfg(test)]` code or a test-only data file.

## Test placement inventory

Paths are relative to `cafleet/src/`. "Tests" is the `#[test]` count.

### `runtime/runner_tests/` (top level)

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `runtime/runner_tests/mod.rs` | 168 | `runtime/runner/mod.rs` → new `#[cfg(test)] pub(crate) mod test_support` | `Fixture` (64 users in `runner/*`, 35 in `runner_tests/*`), `read_fixture_bytes`/`write_fixture_bytes` (users only in `runner/control.rs`, `runner/codec.rs`), `sha256_hex` | `Fixture::new/artifacts/spec` are `pub(super)`; make them `pub(crate)` so `input.rs` tests can use them. Replace `sha256_hex` with the existing production `catalog.rs:1458` `sha256_hex` made `pub(crate)` (TST-09). |
| `runtime/runner_tests/owned_child.rs` | 104 | `runtime/runner/process.rs` (`#[cfg(test)] pub(crate) struct OwnedChild`); its 1 test → `process.rs` `mechanisms_tests` | `OwnedChild` used by `cli/member.rs`, `runner/input.rs`, `observation.rs`, `writer.rs`, `runner_tests/*` | Reachable as `crate::runtime::runner::OwnedChild` via `runner/mod.rs:45` `pub(crate) use process::*`. Keep the `runtime/mod.rs:18` re-export for `cli/member.rs`, or point it at the new path. |
| `runtime/runner_tests/creation.rs` | 989 | `runtime/runner/creation.rs` (450 lines, no inline tests today) | All helpers file-private | 18 tests on `register_pending_run`/`create_registered`. No split needed. |
| `runtime/runner_tests/fleet_deletion.rs` | 533 | `input.rs` (`delete_fleet_with_host` :1618, `DeletionHost` :1338) | File-private `Host`, `Mutation`, `while_locks_are_acquired` | 8 tests. Needs `Fixture` to be `pub(crate)`. |
| `runtime/runner_tests/native_capture.rs` | 70 | `runtime/runner/native_capture/verification.rs` (top level, beside `native_capture_prepare_start_driver`) | Uses `sha256_hex` → use `verification.rs:627` `driver_digest` | 1 ignored re-exec child `native_capture_service_child` (path literal, TST-04). |

### `runtime/runner_tests/control_service/`

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `runtime/runner_tests/control_service.rs` | 786 | `runtime/runner/service.rs` (`run` :427, dispatch :617/:795/:824/:938/:955) | `ServiceFixture` → `service.rs` `#[cfg(test)] pub(crate) mod test_support`; used crate-wide via `runtime/mod.rs:9-16` (`delivery.rs`, `cli/member.rs`, `cli/runtime.rs`, `runner/input_client.rs`) | 7 tests and 1 child. `CHILD_TEST` literal :24; selectors :282/:285/:305. Re-point the `runtime/mod.rs:9-16` re-export to `runner::service::test_support` through a `#[cfg(test)] pub(crate) use` in `runner/mod.rs`, like `runner/mod.rs:46-47`. |
| `runtime/runner_tests/control_service/input_service.rs` | 764 | `runtime/runner/service.rs` (socket input dispatch :617) | `with_service`, `records`, `composer`, `barrier`, `wait_record` (crate-wide via `runtime/mod.rs:11-14`) → `service.rs` `test_support` | 5 tests. `include_str!("../fixtures/synthetic-profile.json")` at :133 goes away (TST-08). |
| `runtime/runner_tests/control_service/input_service/native.rs` | 109 | `runtime/runner/service.rs` `test_support` (pseudo-native child body) | Only caller `input_service.rs:9` | 0 tests. |
| `runtime/runner_tests/control_service/observation_contract.rs` | 362 | split | — | 10 tests. `service.rs`: `existing_reservation` :212, `each_durable_history` :244, `started_health` :290, `sqlite_and_writer_failures` :130 (writer hooks sit at the service boundary). `runtime/runner/observation.rs` `controller_tests`: `quiet_timer` :83, `transient` :101, `shutdown_uses_active_controller` :179, `unknown_coverage` :312, `fallback_profile` :328. `native_capture` (TST-02): `private_capture` :5. |
| `runtime/runner_tests/control_service/observation_services.rs` | 194 | `runtime/runner/service.rs` `test_support` | `run_configured`, `private_write`, `synthetic` (also used by `input_client.rs:1797/1943/2308`), `capture`, `wait_until`, `count`, `health` | 0 tests. `Clock` duplicates production code (TST-07). |
| `runtime/runner_tests/control_service/review_collector.rs` | 62 | `runtime/runner/native_capture/verification.rs`, beside its child `runner_collector_resize_child` | `capture`, `health`, `private_write`, `wait_until` | 1 test. |
| `runtime/runner_tests/control_service/review_export.rs` | 747 | `runtime/runner/native_capture/exporter.rs` (`NativeCaptureExportHooks` :10, unfinished paths :68/:95/:170) | All helpers file-private | 5 tests and 1 ignored child `runner_collector_export_child` (path literal at `control_service.rs:285`). |
| `runtime/runner_tests/control_service/review_failures.rs` | 562 | split | `hooks` is called by `run_configured`, so it moves with it to `service.rs` `test_support` | 6 tests. `observation.rs` `controller_tests`: :246, :261, :367. `service.rs`: :297, :387, :417. |

### `runtime/runner_tests/native_capture/`

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `runtime/runner_tests/native_capture/collection.rs` | 590 | `runtime/runner/native_capture.rs` | `CaptureFixture`, `redacted`, `mask`, `value`, `objects`, `assert_keys` | 16 tests. |
| `runtime/runner_tests/native_capture/files.rs` | 456 | split | `arm`, `config` → `native_capture/files.rs` tests | 11 tests. `native_capture/files.rs`: arm :36/:85/:104/:123, config :156, producer :205, publication :280/:342/:376. `native_capture.rs`: `bind_checks_actual_native_digest` :247. `verification.rs`: `ignored_entry_requires_explicit_configuration` :413 (path literal :429). |
| `runtime/runner_tests/native_capture/review_existing.rs` | 313 | split | `prepared`, `producer`, `channel`, `select_catalog` (duplicates, TST-12) | 9 tests. `native_capture.rs`: f1 tests :59/:107/:129/:164. `native_capture/files.rs`: f4 tests :279–:296. |
| `runtime/runner_tests/native_capture/review_policy.rs` | 150 | `runtime/runner/native_capture.rs` (`NativeCapturePolicy::from_launch_with_compiler` :180) | `Compiler` file-private | 5 tests. |
| `runtime/runner_tests/native_capture/support.rs` | 276 | `runtime/runner/native_capture.rs` tests (`CaptureFixture` as `pub(super)` for `native_capture/files.rs` tests) | `private_write`, `digest`, `catalog`, `Clock` are duplicates (TST-07, TST-08) | 0 tests. |

### `runtime/runner_tests/recovery/`

The private recovery fixture belongs in `runtime/runner/recovery.rs` as `#[cfg(test)] mod test_support`. The descendant `replay::tests` and `exceptional::tests` reach it directly; `input.rs` and `artifacts/recovery_descriptor.rs` need `pub(crate)` / `pub(in crate::runtime::runner)`.

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `runtime/runner_tests/recovery.rs` | 422 | `runtime/runner/recovery.rs` `#[cfg(test)] mod test_support` | `Case`, `Host` (`impl RecoveryHost`), `Evidence`, `opening`/`obscured`/`exited`, `events`/`counts`, `TestResult`, `NOW`; `TableSnapshot`/`rows` → `crate::broker::tests` (TST-10); `frame` → reuse `writer.rs:1664` | 0 tests. |
| `runtime/runner_tests/recovery/crash.rs` | 626 | `runtime/runner/recovery.rs` | `Task`, `PersistGate`, `write_json` | 6 tests including 1 ignored child (path literal :46, TST-04). Uses `sha256_hex` (TST-09). |
| `runtime/runner_tests/recovery/deletion.rs` | 642 | `input.rs` (`delete_fleet_with_host`), beside the `fleet_deletion.rs` tests | `DeletionHost` duplicates the `fleet_deletion.rs` `Host` (TST-11) | 7 tests. |
| `runtime/runner_tests/recovery/foundations.rs` | 330 | split | — | 8 tests. `artifacts.rs` `mod tests`: `lock_transfer_*` :5/:44/:105/:136/:159. `input.rs` `pane_lock_wait_tests`: `pane_guard_*` :184/:234. `presentation.rs`: `presentation_pins_*` :265. |
| `runtime/runner_tests/recovery/history.rs` | 288 | `runtime/runner/recovery/replay.rs` | — | 4 tests. |
| `runtime/runner_tests/recovery/identity.rs` | 397 | `runtime/runner/recovery.rs` | — | 10 tests. |
| `runtime/runner_tests/recovery/missing_paths.rs` | 522 | split | `persist_opening`, `missing_lock`, `claim`, `recover_claim`, `assert_loss` | 10 tests. `recovery.rs`: :79, :111, :138, :373. `recovery/exceptional.rs`: :173, :192, :236, :283, :414, :469. |
| `runtime/runner_tests/recovery/replay.rs` | 491 | `runtime/runner/recovery/replay.rs` | `unavailable` | 13 tests. |
| `runtime/runner_tests/recovery/review_clock.rs` | 166 | `runtime/runner/recovery.rs` | `ClockHost` is a forwarding wrapper (TST-11) | 3 tests. |
| `runtime/runner_tests/recovery/review_completion.rs` | 238 | `runtime/runner/recovery.rs` | — | 5 tests. |
| `runtime/runner_tests/recovery/review_descriptor.rs` | 386 | `runtime/runner/recovery/replay.rs` | `described_creator`, `identity`, `owned_bytes`, `secret_free`, `incomplete` (shared with `io.rs`) | 12 tests. |
| `runtime/runner_tests/recovery/review_descriptor/io.rs` | 318 | split | `UnlinkHost` is a forwarding wrapper (TST-11) | 6 tests. `runtime/runner/artifacts/recovery_descriptor.rs`: :4, :50, :69, :117. `recovery/replay.rs`: :251, :296. |
| `runtime/runner_tests/recovery/review_history.rs` | 354 | `runtime/runner/recovery/exceptional.rs` | — | 7 tests. |
| `runtime/runner_tests/recovery/review_relationships.rs` | 443 | `runtime/runner/recovery.rs` | `Stage`, `Relationship`, `Lifecycle` | 13 tests. |
| `runtime/runner_tests/recovery/terminal_tail.rs` | 158 | `runtime/runner/recovery/replay.rs` | — | 3 tests. |

### Test-only data files

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `runtime/runner_tests/fixtures/synthetic-profile.json` | 1 (1,466 B) | Delete; a single `#[cfg(test)] pub(crate) fn synthetic_definitions() -> Value` in `coding_agent/profiles/catalog.rs` replaces it | 7 `include_str!`/`include_bytes!` sites: `catalog.rs:21,83`, `runner/input.rs:8361`, `runner/writer.rs:851`, `observation_services.rs:96`, `input_service.rs:133`, `native_capture/support.rs:28` | Test-only. Semantically identical to `profiles.rs:8418-8437` `definitions()` except for `source_ref` (TST-08). |
| `runtime/runner_tests/fixtures/claude-2.1.302-blocked-form-blink-1.raw` | 0 (441 B, no newline) | Inline as a generated byte vector in the `runner/input.rs` `permission_response_contracts` test | Single user `runner/input.rs:1046` | Test-only. The content is 11 regular frames (6 with `●`, 5 with a space) of `ESC[H\r ESC[9B ESC[38;5;246m {●\|space} ESC[39m ESC[80;1H ESC[23;2H`. |
| `coding_agent/profiles/contract_tests/synthetic-permission-screen.json` | 39 (604 B) | Inline as a byte literal in the test that hashes it (target `catalog.rs`, TST-03), then delete the `contract_tests/` directory | Single user `profiles.rs:64`; sha256 `6898…d773` is pinned at `profiles.rs:8421` and in `synthetic-profile.json` | Test-only. Its `source_ref` path is never resolved for `kind: synthetic` (`catalog.rs:1674`). Keep the bytes exact (trailing newline) or update both digests together. |

### `#[cfg(test)]`-gated module files

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `runtime/runner/native_capture.rs` | 1,108 | TST-02 | — | Gated by `runner/mod.rs:11-14`; no production caller. |
| `runtime/runner/native_capture/exporter.rs` | 240 | TST-02 | — | — |
| `runtime/runner/native_capture/files.rs` | 612 | TST-02 | — | — |
| `runtime/runner/native_capture/verification.rs` | 1,595 | TST-02 | — | Holds the ignored Director-owned driver `native_capture_prepare_start_driver` :1585 and the child-path literal :944. |
| `runtime/runner/native_capture/review_service_tests.rs` | 107 | `runtime/runner/native_capture/verification.rs` (beside the verification service setup it repeats, TST-12) | `ResizeArm`, `write_pair` | 1 ignored child `runner_collector_resize_child` (path literal at `control_service.rs:282`). Its own `#[cfg(test)]` at `native_capture.rs:14` is redundant because the parent is already gated. |
| `runtime/runner/process/ownership_diagnostics.rs` | 381 | `runtime/runner/process.rs`, inside `ownership_refresh_tests` (:1210) as its private support | `OwnershipDiagnostics`, `OwnershipDiagnosticBuffer`, `enable_/take_ownership_diagnostics` | 0 tests. Gated by `process.rs:11-14`. All users are `process.rs` tests (:1154, :1185, :1320, :1669–:1934) plus the `#[cfg(test)]` seams at `process.rs:329-586`. |

### Mis-housed in-file test modules

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `coding_agent/profiles.rs` `contract_tests` | ~8,510 (21–8531) | TST-03 split by submodule | Top-level helpers `definitions()` :8418, `tracker()` :8506 etc. | 174 tests. |
| `broker/mod.rs` `runtime_contract_tests` | ~6,222 (14–6236) | TST-05 split by section | Fixture and helpers at :3946–:4184 | 142 tests. |

## Findings

### TST-01: Dissolve `runtime/runner_tests/` into the implementation files it exercises
- Category: test-placement
- Location: `cafleet/src/runtime/runner_tests/**` (36 `.rs` files, 13,016 lines, 212 `#[test]`); `cafleet/src/runtime/mod.rs:6-18`
- Evidence: `runtime/mod.rs:6-7` mounts the tree with `#[cfg(test)] mod runner_tests;`. Every file does `use super::runner::*` or `use super::*` and drives `runtime/runner/*` entry points. Callers flow in both directions: 13 `runner/*.rs` inline test modules import `crate::runtime::runner_tests::{Fixture, read_fixture_bytes, write_fixture_bytes, owned_child::OwnedChild, control_service::...}` (for example `runner/artifacts.rs:528`, `control.rs:327/710`, `input.rs:7100`, `input_client.rs:1454/1797`), and `runtime/mod.rs:9-18` re-exports `ServiceFixture`/input-service helpers/`OwnedChild` crate-wide to `delivery.rs`, `cli/member.rs`, `cli/runtime.rs`.
- Proposal: Move each file per the inventory. Target modules:

  | Content | Target module |
  |---|---|
  | `Fixture` and the fixture byte I/O helpers | `runtime/runner/mod.rs` `#[cfg(test)] pub(crate) mod test_support` |
  | `OwnedChild` | `runtime/runner/process.rs` |
  | `ServiceFixture` and the input-service helpers | `runtime/runner/service.rs` `#[cfg(test)] pub(crate) mod test_support` |
  | The recovery `Case`/`Host` fixture | `runtime/runner/recovery.rs` `#[cfg(test)] mod test_support` |

  Re-point `runtime/mod.rs:9-18` at these paths and delete the directory. Rough per-target growth:

  | Target file | Grows from | By |
  |---|---|---|
  | `service.rs` | 1,497 | ~2,700 |
  | `recovery.rs` | 508 | ~3,000 |
  | `recovery/replay.rs` | 246 | ~1,700 |
  | `input.rs` | 1,776 | ~1,300 |
  | `native_capture.rs` | 1,108 | ~1,200 |

  Nested `mod tests { mod crash { … } }` blocks keep today's file-level grouping readable inside one file. Apply TST-04 (path literals) in the same change.
- Spec impact: none — only `#[cfg(test)]` code moves; `cargo test --lib` (`cafleet/mise.toml:2`) still discovers every test.
- Risk: medium — path-sensitive `include_str!` and re-exec literals (TST-04, TST-08), and visibility widening (`Fixture::new` `pub(super)` → `pub(crate)` for the `input.rs` targets).
- Estimated delta: -150 lines (14 `mod` declarations, repeated `use` headers, `runtime/mod.rs` re-export block); the consolidation wins are counted separately in TST-06…TST-14.

### TST-02: Fold the `#[cfg(test)]` `runner/native_capture/` module tree into an in-file test module
- Category: test-placement
- Location: `cafleet/src/runtime/runner/mod.rs:11-14`; `cafleet/src/runtime/runner/native_capture.rs` and `native_capture/{exporter,files,verification,review_service_tests}.rs` (3,662 lines)
- Evidence: The whole tree is compiled only under `cfg(test)`, so it is test-only code shaped like production modules. Production wiring reaches it only through `#[cfg(test)]` seams: `observation.rs:923-1003,1218` (`attach_capture`, `capture: Option<NativeCapturePort>`) and `service.rs:863-912`. It is the harness that produces the native captures the docs require (`docs/docs/contributing.md:70`, `docs/docs/spec/member-runtime.md:73`), entered through the ignored driver `verification.rs:1585`, so it stays.
- Proposal: Make it one in-file test module of the implementation file whose observation site it instruments: `runtime/runner/observation.rs` gains `#[cfg(test)] pub(crate) mod native_capture { … }`, with nested `exporter`, `files` and `verification` submodules. The tests that today live in `runner_tests/native_capture/*`, `control_service/review_export.rs`, `review_collector.rs` and `review_service_tests.rs` become that module's `tests`. Drop the redundant `#[cfg(test)]` at `native_capture.rs:14`. Alternative if the Director prefers smaller files: `service.rs`, which owns `run_with_test_services`. Either target satisfies the rule; observation.rs is recommended because the capture port is an observation-controller attachment.
- Spec impact: none — the module is already absent from release builds.
- Risk: medium — about 3.7k lines join a 2,624-line file, and the re-exec literals at `verification.rs:944` and `control_service.rs:282/285/305` change (TST-04).
- Estimated delta: -10 lines (module declarations and the redundant gate).

### TST-03: Move `coding_agent/profiles.rs` `contract_tests` into the files they test
- Category: test-placement
- Location: `cafleet/src/coding_agent/profiles.rs:21-8531` (production code is ~80 lines of the 8,601)
- Evidence: The submodules exercise other files:

  | Submodule(s) | Exercises | Defined in |
  |---|---|---|
  | `catalog` :32, `gated_catalog` :798, `annotation_catalog` :7628, `profile_version_fallback` :8064 | `ProfileCatalog::from_definitions` | `catalog.rs:1647` |
  | `focus_resolution` :502, `continuation_episodes` :680, `detector` :1189 | `Detector` | `detector.rs:1410` |
  | `episodes` :1605 | `EpisodeTracker` | `runtime/runner/episodes.rs:175` |
  | `executable_composer` :2704 | `ComposerPlan` | `composer.rs` |
  | `bundled_claude` :3399, `bundled_claude_auto` :4424, `bundled_codex_auto` :5632, `bundled_codex` :6181, `bundled_opencode` :6644 | `ProfileCatalog::bundled()`, the embedded `BUNDLED_FIXTURES` and `bundled.json` | `catalog.rs:1146-1456`, `catalog.rs:1609` |

  The cross-module test-support exports `catalog::inspection_tests` (re-exported at `profiles.rs:8-13` and `coding_agent/mod.rs:25-29`) and `detector::inspection_tests::replay_native_fixture` (8 callers in `profiles.rs`) exist only because the tests live in the wrong file.
- Proposal: Move each submodule to the file in its "Defined in" row:

  | Submodule(s) | Target |
  |---|---|
  | `catalog`, `gated_catalog`, `annotation_catalog`, `profile_version_fallback` | `catalog.rs` |
  | the five `bundled_*` | `catalog.rs`, as nested `mod tests::bundled_*` |
  | `focus_resolution`, `continuation_episodes`, `detector` | `detector.rs` |
  | `executable_composer` | `composer.rs` |
  | `episodes` | `runtime/runner/episodes.rs` |

  The shared synthetic `definitions()` (:8418) becomes the single `#[cfg(test)] pub(crate) fn synthetic_definitions()` in `catalog.rs` (TST-08). Once each `replay_native_fixture` caller sits in `detector.rs` or uses a `catalog.rs`-owned helper, the `inspection_tests` re-export chains collapse to direct paths.
- Spec impact: none — tests only.
- Risk: medium — `catalog.rs` grows from 1,987 to ~7,000 lines; the bundled suites' `concat!("native/", $id, ".json")` (:4482, :5671) and `source(id)` (:6727) paths are relative to `profiles/` and stay valid from `catalog.rs`, which sits in the same directory as `native/`.
- Estimated delta: -40 lines (re-export chains `profiles.rs:6-18`, `coding_agent/mod.rs:3-29` test re-exports, duplicated `use` headers).

### TST-04: Derive re-exec child test names from `module_path!()` instead of hard-coded path literals
- Category: other
- Location: `runtime/runner_tests/control_service.rs:24,282,285,305`; `runtime/runner_tests/native_capture/files.rs:429`; `runtime/runner_tests/recovery/crash.rs:46`; `runtime/runner/native_capture/verification.rs:944`; `cli/member.rs:949,1310`
- Evidence: Each literal (for example `"runtime::runner_tests::native_capture::native_capture_service_child"`) is passed with `--exact` to `std::env::current_exe()`. A module move keeps compiling and silently runs zero tests in the child, so the parent then fails on a missing outcome far from the cause. The same native-capture child literal appears three times (`verification.rs:944`, `control_service.rs:305`, `native_capture/files.rs:429`).
- Proposal: Next to each child test, define one `#[cfg(test)] const` built from `module_path!()` with the crate prefix stripped, for example `fn child_test_name(name: &str) -> String { format!("{}::{name}", module_path!().strip_prefix("cafleet::").expect("crate-rooted module path")) }`. Every launcher references that item. Then TST-01/02 moves need no literal edits, and a bad path fails at the `expect`.
- Spec impact: none — test harness only.
- Risk: low.
- Estimated delta: -5 lines (three duplicate literals collapse to one item), plus removal of a whole class of silent breakage.

### TST-05: Move `broker/mod.rs` `runtime_contract_tests` into `broker/runtime/*.rs`
- Category: test-placement
- Location: `cafleet/src/broker/mod.rs:14-6236` (142 tests in a 6,326-line file)
- Evidence: The tests call `crate::broker::runtime` functions defined in sibling files: `update_interaction_evidence`/`persist_transition`/`get_event` (`broker/runtime/events.rs:738/513/72`), `reserve_permission_response` (`permissions.rs:320`), `reserve_input`/`settle_input`/`settle_preview` (`inputs.rs:101/227/298`), `persist_transition_with_effective_policy`/`record_effective_policy` (`effective_policy.rs:133/166`), `create_run`/`reserve_event_sequences`/`database_identity` (`broker/runtime/mod.rs:322/381/102`).
- Proposal: Split by section:

  | Section (lines) | Target |
  |---|---|
  | `interaction_refresh` :26-502 | `events.rs` |
  | `permission_responses` :503-2259 | `permissions.rs` |
  | `versioned_events` :2260-3074 | `events.rs` |
  | `automatic_review_events` :3076-3945 | `effective_policy.rs` (needs-verification per test: its calls are `persist_transition_with_effective_policy` and `persist_transition_counted`) |
  | shared `Fixture`/helpers :3946-4184 | `broker/runtime/mod.rs` `#[cfg(test)] pub(crate) mod test_support` |
  | run/sequence tests :4187-4363, 5761-5863 | `broker/runtime/mod.rs` |
  | transition/inbox tests :4366-4792 and lifecycle/event-shape regressions :5865-6235 | `events.rs` |
  | receipt/input tests :4795-5083 | `inputs.rs` |
  | evidence tests :5086-5402 | `evidence.rs` |
  | effective-policy reader tests :5521-5686 | `effective_policy.rs` |
  | runtime-presentation tests :5405-5519, :5689 | `presentation.rs` / `output/runtime.rs` |

  `broker::tests` (:6238) stays the crate-wide broker fixture (TST-13).
- Spec impact: none.
- Risk: low — mechanical move inside one module tree.
- Estimated delta: -30 lines (shared `use` header and module scaffolding).

### TST-06: Fold `process/ownership_diagnostics.rs` into `process.rs` test support
- Category: test-placement
- Location: `cafleet/src/runtime/runner/process.rs:11-14`; `cafleet/src/runtime/runner/process/ownership_diagnostics.rs` (381 lines, 0 tests)
- Evidence: The module is `#[cfg(test)]`-gated and every consumer is `process.rs` test code (`ownership_refresh_tests` :1210 onward) or `#[cfg(test)]` seams in `process.rs:329-586` (`refresh_owned_traced`, `pending_ownership_refresh`). There is no other caller (grep `ownership_diagnostics|OwnershipDiagnostic|pending_ownership_refresh` → `process.rs` only).
- Proposal: Move the types into `process.rs` as a `#[cfg(test)] mod ownership_diagnostics { … }` inline block and delete the `process/` directory. The seam shape (a test-only traced refresh path versus production `refresh_owned()`, a divergent execution path) belongs to the runner-slice auditor. Flagged here because the inline move keeps those seams.
- Spec impact: none.
- Risk: low.
- Estimated delta: -3 lines (module declaration); the folder disappears.

### TST-07: Replace test clocks that duplicate production clocks
- Category: duplication
- Location: `runtime/runner_tests/control_service/observation_services.rs:7-18`; `runtime/runner_tests/control_service/input_service.rs:90-112` (used by `input_service/native.rs:14,31,73`); `runtime/runner_tests/native_capture/support.rs:50-75`
- Evidence: `observation_services.rs` `Clock` is line-for-line `RealObservationClock` (`runner/observation.rs:261-272`, verified). The `input_service.rs` `Clock` matches `RealInputDeadlineClock` (`runner/input_clock.rs:19-48`, `pub(crate)` re-exported). The `support.rs` deterministic `Clock` duplicates `writer::contract_tests::support::Clock` (`runner/writer.rs:876-902`); only the `observed_at` literal differs, and no capture test asserts it.
- Proposal: Use `RealObservationClock` and `RealInputDeadlineClock` directly, and have the native-capture tests reuse the writer support `Clock`, made `pub(crate)` in the writer test support.
- Spec impact: none.
- Risk: low.
- Estimated delta: -60 lines.

### TST-08: One synthetic profile definition instead of a JSON fixture plus a `json!` copy
- Category: duplication
- Location: `runtime/runner_tests/fixtures/synthetic-profile.json`; `coding_agent/profiles.rs:8418-8437` (`definitions()`); 7 `include_str!` sites (inventory); `runtime/runner_tests/native_capture/support.rs:26-31` = `runner/writer.rs:855-857` (`catalog()`)
- Evidence: The JSON fixture and the `definitions()` literal describe the same synthetic profile, differing only in `source_ref`. Two `catalog()` wrappers parse it identically. `source_ref` is not resolved for synthetic fixtures (`catalog.rs:1674`).
- Proposal: Add `#[cfg(test)] pub(crate) fn synthetic_definitions() -> Value` and `synthetic_catalog()` in `coding_agent/profiles/catalog.rs` from the `definitions()` literal. Point all 7 include sites and both `catalog()` helpers at them, then delete the fixture file and the `fixtures/` directory. Inline `synthetic-permission-screen.json` as a `const` byte string in the test that pins its digest. Replace `claude-2.1.302-blocked-form-blink-1.raw` with a 6-line generator in `runner/input.rs` tests.
- Spec impact: none — no runtime path reads these files.
- Risk: low — keep the hashed bytes exact or update both pinned digests (`profiles.rs:8421` and the former JSON) together.
- Estimated delta: -25 lines (and 3 data files / 2 directories removed).

### TST-09: Single `sha256_hex` helper
- Category: duplication
- Location: `runtime/runner_tests/mod.rs:77-85`; `coding_agent/profiles/catalog.rs:1458` (production, private); `runtime/runner/native_capture/verification.rs:627` (`driver_digest`); `runtime/runner_tests/native_capture/support.rs:23-25` (`digest`) vs `native_capture/files.rs:8` (`file_digest`)
- Evidence: All produce lowercase hex SHA-256. `runner_tests` users: `control_service.rs:148`, `native_capture.rs:35`, `observation_services.rs:56,166`, `observation_contract.rs:29`, `review_export.rs:524`, `support.rs:24`, `crash.rs:156,416`.
- Proposal: Make `catalog.rs:1458` `pub(crate)` and use it from `driver_digest`'s call sites and all test users. Replace `support.rs` `digest` with `files::file_digest`.
- Spec impact: none — same digest function.
- Risk: low.
- Estimated delta: -20 lines.

### TST-10: One table-snapshot helper
- Category: duplication
- Location: `runtime/runner_tests/recovery.rs:389-422` (`TableSnapshot`, `rows`); `runtime/runner/health.rs:465-499`; `runtime/runner/input_recovery.rs:664-695`; `runtime/runner/input_client.rs:5199-5230`; `broker/mod.rs:4109-4131` (`rows`, `durable_state`)
- Evidence: The `recovery.rs` and `health.rs` copies are identical. `input_recovery.rs` adds an exclusion list; `input_client.rs` is near-identical.
- Proposal: One `#[cfg(test)] pub(crate) fn table_snapshot(conn, exclude: &[&str])` in `crate::broker::tests`, which already owns `migrated_conn`. Delete the copies.
- Spec impact: none.
- Risk: low.
- Estimated delta: -110 lines.

### TST-11: Collapse duplicated fake hosts and forwarding wrappers
- Category: over-abstraction
- Location: `runtime/runner_tests/recovery/review_clock.rs:22-94` (`ClockHost`); `runtime/runner_tests/recovery/review_descriptor/io.rs:187-237` (`UnlinkHost`); `runtime/runner_tests/recovery/deletion.rs:15-187` vs `runtime/runner_tests/fleet_deletion.rs:17-140`; `runtime/runner_tests/recovery.rs:170-315` (`Host`) vs `runtime/runner/input_recovery.rs:402-534` (`TypedHost`)
- Evidence: `ClockHost` and `UnlinkHost` forward every `RecoveryHost` method to the inner `Host` except one hook. The two `DeletionHost` fakes overlap, and `active` (`deletion.rs:170-187`) equals `assert_registry_active` (`fleet_deletion.rs:127-140`). Both land in `input.rs` after TST-01.
- Proposal:
  - Add optional clock and unlink hook fields to the recovery `Host` and delete both wrappers.
  - Merge the two deletion fakes into one `input.rs` test-support `Host` with a `Mutation` enum covering both suites' cases.
  - needs-verification for merging `Host` with `TypedHost`: `TypedHost` panics on replay/cleanup and records calls; confirm by checking each `input_recovery.rs`/`input_client.rs` caller's assertions.
- Spec impact: none.
- Risk: medium — fault-injection fidelity must be preserved per case.
- Estimated delta: -135 lines verified (wrappers ~60, deletion fakes ~75); -100 more needs-verification (`TypedHost`).

### TST-12: De-duplicate native-capture and control-service scaffolding
- Category: redundant-test
- Location and evidence: listed per item below. Removable lines are verified unless marked needs-verification.

  | Item | Evidence | Removable lines |
  |---|---|---|
  | `private_write` | `native_capture/support.rs:13-22` and `observation_services.rs:68-77` are byte-identical (verified); a third near-copy is `recovery/review_descriptor.rs:49-58`. One `#[cfg(test)] pub(crate)` helper in `runner/artifacts.rs` beside `validate_private_file` (:139) replaces them | ~20 |
  | `review_existing.rs` re-implements `CaptureFixture` internals | `prepared`/`producer`/`channel` (:5-30) copy `support.rs:145-164`; `files.rs:255-264` repeats the prepared-state SQL | ~25 |
  | `NativeCaptureConfig` / policy-file builder | `native_capture/files.rs:16-33` vs `observation_services.rs:172-189` | ~15 |
  | `NativeCaptureArm` literal | `native_capture/files.rs:6-15`, `review_export.rs:233-246`, `observation_contract.rs:8-15`; one test constructor in `native_capture/files.rs` covers all three | ~15 |
  | resize (`tcsetwinsize` and SIGWINCH) | `observation_contract.rs:154-169`, `review_collector.rs:16-31`, `review_failures.rs:271-286, 307-321`, `input_client.rs:1471-1486`; becomes `ServiceFixture::resize(rows, cols)` | ~45 |
  | event-kind query | `observation_contract.rs:186-194`, `review_failures.rs:492-500, 547-555`; becomes `kinds()` | ~17 |
  | `ServiceFixture::wait` and the Drop wait loop | `control_service.rs:499-511, 628-642`; use `OwnedChild::poll_until` (`owned_child.rs:17`) | ~19 |
  | polling loops | `wait_path` (`review_export.rs:222-231`) and `wait_until` (`observation_services.rs:102-108`) duplicate `writer.rs:1059-1074` `wait_until_within` | ~10 |
  | recovery `frame` and decoder | `recovery.rs:357-363` and `recovery/replay.rs:450-462` equal `writer.rs:1664` `frame` / `:1672` `independent_frames` | ~19 |
  | recovery invocation wrappers | `recover_claim`, `invoke` ×2, `recover_missing`, `recover` (`missing_paths.rs:24`, `review_completion.rs:35`, `review_history.rs:73`, `review_relationships.rs:71`, `io.rs:238`) and 9-argument `recover_member_with_held_pane_lock` calls (`identity.rs:118,164,207,224,299,332`, `review_clock.rs:112`, `deletion.rs:106`); becomes `Case::recover_with(host, claim)` / `Case::recover_held(...)` | ~85 |
  | lifecycle-mutation branches and terminal-history filters | `history.rs:15-36`, `missing_paths.rs:200-221`, `review_relationships.rs:266-300`; `review_history.rs:32-45` = `review_relationships.rs:302-315`; `review_history.rs:9-18` = `review_descriptor.rs:22-31` | ~70 |
  | exited / cleanup-pending setup and unavailable/exceptional case setup | `review_relationships.rs:26-51`, `review_history.rs:196-211`, `terminal_tail.rs:79-96, 131-146`; `replay.rs:19-39` = `:65-85`; `crash.rs:238-248` = `:310-320` | ~68 |
  | `NewRun` literal written three times | `runner_tests/mod.rs:114-138`, `fleet_deletion.rs:255-283`, `broker/mod.rs:3977-4009` | ~20, needs-verification (fields differ per case) |
  | JSON `health()` vs typed `ServiceFixture::health()` | `observation_services.rs:120-127` vs `control_service.rs:454-461` | ~8, needs-verification |
  | three test socket clients | `Wire` (`control_service.rs:76-105`), `Peer` (`input_service.rs:178-262`), `frame(body)` copies (`codec.rs:9`, `control.rs:722`, `service.rs:1315`). `Wire` must send invalid versions and `Peer` must send split frames, so only the framing helper merges cleanly | ~30, needs-verification |

- Proposal: Apply each row during the TST-01 move so every consolidated helper lands once in its owning module.
- Spec impact: none.
- Risk: low per item.
- Estimated delta: -448 lines verified; -58 needs-verification.

### TST-13: Name cross-file test support for what it is
- Category: other
- Location:

  | Module | Defined at | Tests |
  |---|---|---|
  | `broker::tests` | `broker/mod.rs:6238` | 0 |
  | `multiplexer::recording` | `multiplexer/mod.rs:1172-1293` | 0 |
  | `writer::contract_tests` | `runner/writer.rs:625` | 31 |
  | `input_recovery::tests` | `runner/input_recovery.rs:389` | 7 |
  | `input_client::permission_history` | `runner/input_client.rs:4964` | about 35 |
  | `catalog::inspection_tests` | `coding_agent/profiles/catalog.rs:14` | 3 |
  | `detector::inspection_tests` | `coding_agent/profiles/detector.rs:24` | 28 |

- Evidence: There are 81 `#[cfg(test)] mod` blocks under 4 naming families: `tests` ×43, `*_tests` ×31, `*_contracts` ×4, and unmarked names ×4 (`recording`, `permission_history`, `native_capture`, `ownership_diagnostics`). Two modules named `tests` hold no tests and serve 12–15 importing files. Five modules mix tests with helpers that other files import.
- Proposal: Use one convention, written into the rule (below). Shared helpers go in `#[cfg(test)] pub(crate) mod test_support` inside the owning implementation file; tests go in `#[cfg(test)] mod tests`, with nested submodules for grouping. For example, `broker::tests` becomes `broker::test_support`, and `writer::contract_tests::{support,worker}` becomes `writer::test_support`.
- Spec impact: none.
- Risk: low (renames only).
- Estimated delta: 0 lines; removes the misleading `tests`-with-no-tests modules.

### TST-14: Redundant tests asserting the same behavior twice
- Category: redundant-test
- Location and evidence: each row is one redundant test or section.

  | Redundant test | Duplicates | Removable lines | Status |
  |---|---|---|---|
  | `observation_contract.rs:311-325` `unknown_coverage` | The `(None, 1)` row of `fallback_profile` (:337-354): same setup and assertions, since `Fixture` defaults to `None`/`Unknown` (`runner_tests/mod.rs:132-135`). Move its 2 unique asserts (:323-324) into that row | ~13 | verified |
  | `review_failures.rs:245-258` quiet-timer detector test | The `child_output_parser_and_detector` table (:367-384); add it as a third row | ~13 | verified |
  | `native_capture/collection.rs:348-360` pattern-bound cases | The same three cases (count 65, 4097 B, 33×2048) in `review_policy.rs:24-79`, which also asserts the compiler is never invoked | ~14 | verified |
  | `recovery/replay.rs:271-297` | `recovery/terminal_tail.rs:43-72` is a superset (adds origin, exit code, zero counts) | ~27 | needs-verification: spool empty vs tail present |
  | `recovery/terminal_tail.rs:129-158` | `replay.rs:194` checksum case plus `outbox.rs:1173` | ~30 | needs-verification |
  | `recovery/replay.rs:395-427` | The `replay.rs:352` loop, which already includes `before_compact` | ~25 | needs-verification: move the spool-prefix assert into it |
  | `recovery/deletion.rs:307` `AfterClose` arm (:345-349, :149-163) | `fleet_deletion.rs:316` | ~20 | needs-verification |
  | `recovery/foundations.rs:203-215` | `input.rs:1047-1063` | ~13 | needs-verification |
  | `recovery/deletion.rs:469-548` outer-scope restoration | `lifecycle.rs:764,865,870` and `input.rs:439` cover the invariant | ~80 | needs-verification: the combination may be deliberate |

  Checked and kept: the service-layer vs controller-layer pairs (`observation.rs:1954/2010/2114/1403` vs `observation_contract.rs:83/101/179`, `review_failures.rs:246`), `creation.rs:803` vs service-side stale rejection, and `review_clock.rs` vs `lifecycle.rs:349,377`. These deliberately cover different layers.

- Proposal: Delete or merge the redundant tests. Confirm each needs-verification row by diffing the asserted state, so no unique assertion is lost.
- Spec impact: none.
- Risk: low (verified rows); medium (needs-verification rows).
- Estimated delta: -40 lines verified; -195 needs-verification.

### TST-15: Oversized test scaffolding
- Category: redundant-test
- Location:
  - `runtime/runner_tests/native_capture/review_policy.rs:65,87,109,126` and `collection.rs:357,416`
  - `native_capture/files.rs:217`
  - `runtime/runner_tests/recovery/crash.rs:431-626`
  - `eprintln!` diagnostics in `crash.rs:255,271-275,333,351-356,362-365,449-452,533,537`, `deletion.rs:525,594-598`, `review_clock.rs:82-84`, `review_history.rs:109-118`, `review_relationships.rs:199-205`
  - `.p194-` temp prefixes in `runner_tests/mod.rs:96`, `native_capture/files.rs:415`, `cli/mod.rs:43,832`, `cli/member.rs:938`
- Evidence: `NativeCapturePolicy::from_launch` reads only `spawn.evidence_policy` (`native_capture.rs:187-261`), yet those policy tests build a full `CaptureFixture` (database, run lock, executable, descriptor, producer), several of them inside loops. `files.rs:217` builds 9 fixtures for a stateless reader. The `crash.rs` child is a 195-line string-mode dispatcher for 7 behaviors. The `eprintln!` lines total ~45. The `.p194-` prefix embeds a design number in temp-directory names.
- Proposal: Use a `SpawnSpec`-only builder for the policy tests and one fixture with distinct filenames for `files.rs:217`. Split the crash child per mode, or into a typed mode enum parsed once. Drop the `eprintln!` lines (needs-verification: confirm they are not flake-investigation aids the Director wants kept). Rename the temp prefix to a neutral `.cafleet-test-`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -60 lines verified (builders, crash dispatcher); -45 needs-verification (`eprintln!`).

## Rule wording (drafts)

### `.claude/rules/code-quality.md`: append a section

```markdown
## Test placement

Every Rust test is an in-file unit test. It lives in a `#[cfg(test)] mod tests` block inside the implementation file whose behavior it exercises, and `mise //cafleet:test` runs it.

- Place each test in the file that defines the entry point it asserts on. When a test drives several files, the file owning the asserted behavior owns the test. Group related tests with nested modules inside `mod tests`.
- Put a helper that tests in several files share into the implementation file that owns the type it builds, as `#[cfg(test)] pub(crate) mod test_support`. Reuse an existing helper or production function (clocks, digests, frame codecs) before writing another.
- Express test inputs as literals or generated bytes inside the test module. Data files under `cafleet/src` are runtime data embedded by production code; tests may read them.
- Name a re-executed child test through `module_path!()` beside the child, so moving the module keeps the launcher correct.

**Forbidden patterns:**
- A test-only `.rs` file or directory: `*_tests.rs`, `*_tests/`, `cafleet/tests/`, a `#[path]`-mounted test file, or a `#[cfg(test)] mod name;` declaration that loads a separate file.
- A test-only fixture file or fixture directory.
- A test module that exercises another file's implementation.
- A module named `tests` that holds only shared helpers; name it `test_support`.
```

### `CLAUDE.md`: append to the `## Commands` paragraph

```markdown
Every Rust test is an in-file `#[cfg(test)]` unit test in the implementation file it exercises; `.claude/rules/code-quality.md` § Test placement owns the convention.
```

### `docs/docs/contributing.md`: append to `## Rust boundaries`

```markdown
Every Rust test is a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises; shared test helpers live in a `#[cfg(test)] test_support` module beside the type they build, and test inputs stay inline in the test module.
```

Existing consistent wording needs no edit: `cafleet/mise.toml:3` ("Run inline unit tests"), `skills/cafleet-design-doc/execute/roles/programmer.md:69`, `skills/cafleet-design-doc/execute/roles/tester.md:33`. No living doc mentions `runner_tests`, `contract_tests` or the fixture paths, so the Removal rule needs no further doc edits.

## Totals

15 findings. About 940 lines removable verified (TST-01 150, 02 10, 03 40, 04 5, 05 30, 06 3, 07 60, 08 25, 09 20, 10 110, 11 135, 12 448, 14 40, 15 60; TST-13 removes none) and about 400 more needs-verification (TST-11 100, 12 58, 14 195, 15 45). Applying the moves also removes 45 test-only files and 10 test-only directories:
- files: 36 `.rs` in `runner_tests/`, `review_service_tests.rs`, `ownership_diagnostics.rs`, 3 fixtures, and the 4 other `runner/native_capture` module files (`native_capture.rs`, `exporter.rs`, `files.rs`, `verification.rs`) after inlining
- directories: `runner_tests/` with its `control_service/`, `control_service/input_service/`, `native_capture/`, `recovery/`, `recovery/review_descriptor/` and `fixtures/` subtrees, plus `runner/native_capture/`, `profiles/contract_tests/` and `process/`
