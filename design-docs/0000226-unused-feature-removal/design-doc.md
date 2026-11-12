# Unused feature removal

**Status**: Approved
**Progress**: 15/49 tasks complete
**Last Updated**: 2026-11-12

## Overview

Remove every unnecessary feature, code path, parameter, field, enum variant, trait default, schema value, duplicate mechanism and test-only seam reported by an eight-slice unused-feature scan of HEAD `79627cb6`, together with the `SPEC.md`, `docs/`, `assets/skills/` and `V1__baseline.sql` edits each removal requires. The change also corrects the native-fixture embedding drift in `SPEC.md` and bumps the cafleet version to `0.34.0` because the baseline schema changes.

## Success Criteria

- [ ] Every row of § Removal catalog is applied as specified, and every row of § Retained by decision is still present.
- [ ] `assets/migrations/V1__baseline.sql` and the `SPEC.md` §8 DDL are identical for every changed table and match § Schema delta.
- [ ] Every pattern in § Verification greps returns only its listed allowed residuals.
- [ ] `SPEC.md` attributes native-fixture provenance validation to the bundled-catalog build guard (the test suite), not the production loader or binary embedding.
- [ ] The four `native_capture` driver entries still compile; its self-tests that assert only harness types are deleted.
- [ ] `Cargo.toml`, `Cargo.lock` and `.bumpversion.toml` record `0.34.0`.
- [ ] Every implementation step ends with `mise //:test` and `mise //:lint` passing, and the final step also passes `mise //docs:build`.

---

## Background

The scan read every production region of `src/` plus `assets/profiles`, `assets/presets`, `assets/extensions` and the baseline. `mise //:lint` already rejects code with no caller at all, so every finding is semantic: a value that is always the same, plumbing only tests read, a branch production invariants make unreachable, a constant-valued schema column, or test-only machinery inside production types. Overlapping findings are merged into one catalog row. The "Sources" column records the original finding as `<scanner letter>.<finding id>` (for example `C.C1`) for traceability only; every row states its complete scope inline.

The user decided to remove both the "certainly" and the "possibly" findings, with these clarifications:

| Question | Decision |
|---|---|
| Single-implementor fault/race test-injection traits | Keep, recorded in § Retained by decision; remove only `HermesConfig` and `RecoveryBoundary`/`checkpoint` |
| `native_capture` | Keep the capture drivers (the fixture-recording capability); delete only its self-tests that exercise harness types |
| Exclusive receipt proof | Remove the mechanism and narrow the receipt contract to the behavior production already has |
| Schema/vocabulary removals | Remove every listed item, including the write-only audit columns |
| Constant wire keys | Drop the event and effective-policy `schema_version`, the always-null `native_request_id` and the always-false profile `sensitive` flag |
| Read-side CHECK re-validation | Drop the pure-CHECK duplicates; keep typed enum conversion |
| CLI and error-row narrowing | Remove every listed contract row |
| Unreachable invariant branches | Replace with `.expect("<why>")` / `unreachable!("<why>")` and tighten types |
| Native fixture drift | Remove fixture-registry validation from the production loader; keep what `native_capture` needs under `#[cfg(test)]` |
| Run-less ordinary members | Every ordinary-target caller expects a run (INV5) |
| Tests, docs, version | Docs first; delete tests of removed behavior; bump the version in the implementation commit sequence |
| Visibility/naming tidy | Include |

---

## Specification

### Invariants the removals rely on

Every replacement `expect`/`unreachable!` message names its invariant.

| ID | Invariant | Evidence |
|---|---|---|
| INV1 | Every active member has a `member_placements` row (`mux_pane_id` may be NULL while pending). | `lifecycle.rs:276-282` always passes `Some(&member.placement)`; `fleets.rs:54-55` inserts the Director's row in the fleet transaction; placements are deleted only with deregistration (`members.rs:186-197`, `fleets.rs:147-152`). |
| INV2 | A committed fleet records `director_member_id`, and the root Director's placement records a pane. | `fleets.rs:38-60` asserts the pane and sets the column in one transaction; nothing clears either. |
| INV3 | Every readable database row was written by the running version. | The command guard accepts only `cafleet_database.cafleet_version == VERSION` (`db.rs:248-257`), and each version uses its own `cafleet_<VERSION>.db`. |
| INV4 | Every fleet name is non-null. | `fleet create --name` is a required `String` (`cli/fleet.rs:43-46`). |
| INV5 | Every ordinary member is registered together with its run; only the root Director has none. | `lifecycle.rs:270-300` registers member, placement and run in one transaction. |
| INV6 | The production profile catalog is exactly `assets/profiles/bundled.json`. | `ProfileCatalog::bundled()` is the only production catalog (`cli/member.rs:390`, `runtime/service.rs:563`). |
| INV7 | Production `ScreenObserver` implementors are `OrderedObservation` and `DeadlineObserver`; both treat failures as fatal and only `OrderedObservation` reaches `quiesce_for_input`. | `observation.rs:213-228`, `transport.rs:514-529`, `service.rs:152-158`. |
| INV8 | An `InputCoordinator`'s `self.run` is fixed for its lifetime, and every production caller passes that same run. | `service.rs:1155-1182`; no production reassignment of `self.run`. |
| INV9 | The native startup process probe is unavailable, so receipt-only exclusive ownership never succeeds. | `recovery.rs:479-489` always returns `ProcessEvidence::Unknown`. |

### Removal catalog

Line numbers are at HEAD `79627cb6`. Before each removal the implementer re-locates the item by name and confirms the cited callers with `grep -rn` across `src/`; a row whose evidence no longer holds is reported to the Director instead of being forced through. A step applies every consumer edit its type or signature change forces, including test code.

#### OBS — Runtime observation, transport and press (`src/runtime/{observation,transport,press,mod,artifacts}.rs`)

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| OBS1 | Make `ObservationPhase`, `OrderedObservation.phase`, `observer_for` and `poll_for` `#[cfg(test)]`; production `service.rs` (`:447-463`, `:513-517`, `:1223`) calls `observer()`/`poll()`. Test phases reach hooks through `Transport::set_observation_phase`; `native_capture`'s `ObservationPhase::Resize` use (`observation.rs:9975-9978`) stays. | A.A1 | — |
| OBS2 | Drop `EpisodeTracker::finish`'s `reason` parameter and its `live finish requires process exit` ensure; use `InteractionCloseReason::ProcessExited` inside. | A.A2 | — |
| OBS3 | Drop `EpisodeTracker::reset_observation`'s `reason` parameter and its ensure. | A.A3 | — |
| OBS4 | Drop `EpisodeTracker::new`'s `next_episode_seq`; initialize `next_episode: Some(1)`; delete the `EpisodeTracker::new(1, 0, ..)` rejection assertion (`observation.rs:12130`). | A.A4 | — |
| OBS5 | Delete `status.retained_transition \|\|` from `ObservationController::has_pending` (`observation.rs:962`), `input.rs:989` and `input.rs:1892-1893`; keep the standalone use at `input.rs:362`. | A.A5 | — |
| OBS6 | Remove `ScreenObserver::failures_are_fatal` (INV7): `TerminalState::step` returns `Err(Observer(cause))` immediately and loses its `failure` accumulator (`transport.rs:1041-1072`); delete the non-fatal `Observer` arm and `observation_complete` restore guard in `Transport::tick` (`:234-245`); delete `TransportProgress.observation_error` and `observation_failure`; delete `service.rs::record_observation_failure` (`:372-385`) with its consistency ensure and its calls (`:458`, `:1238`). The `#[cfg(test)]` parser-hook block (`:218-226`) returns its error directly. | A.B1, C.C1, D.C2 | — |
| OBS7 | `ScreenObserver::input_health` becomes a required method returning `ObservationHealth`; delete the `None` branch (`transport.rs:634-636`) and its `observation health unavailable` pending reason. | A.B1, C.C2 | `SPEC.md:1173` pending-reason row; `cli.md:430` reason list |
| OBS8 | `ScreenObserver::parser_failure` becomes a required method. Every test observer (`LegacyObserver`, `ControlledObservation`, `Measured`, `ParserAlgorithmBridge` and the observers at `transport.rs:1523`, `:2043`, `:2423`, `:4247`, `observation.rs:11216`) implements OBS6–OBS8 explicitly. | C.C3 | — |
| OBS9 | Delete `TransportProgress.child_bytes` and `display_bytes` and their writes (`transport.rs:212`, `:247`), keeping `flush_display(display)?`; tests assert observable display/pty bytes. | C.C4 | — |
| OBS10 | `InputDrain::Quiescent` becomes a unit variant; tests read the values from `take_input_drain()`. | C.C5 | — |
| OBS11 | Drop `ObservationController::new`'s `initial` parameter; call `pool.supply(1..=256)` after the `event_seq_reserved_through == 256` check. | A.B3 | — |
| OBS12 | Delete the `member press` backend-mismatch check (`press.rs:71-75`) and its test. | A.B2 | `SPEC.md:1052` and `cli.md:329` rows `press backend does not match the recorded placement` |
| OBS13 | Drop the launch descriptor `schema_version` field and its reader check (`artifacts.rs:338-357`). | A.B5 | `SPEC.md:2819` "schema version" item |
| OBS14 | `press_member` passes its loaded member into the inner function, which keeps its revalidation (`press.rs:90-100`) and drops its second `active_member` load. | A.B6 | — |
| OBS15 | Make `validate_private_file` private (`artifacts.rs:124-126`); `native_capture` calls `private_metadata(.., false)` directly. | A.B7 | — |
| OBS16 | Move `runtime::test_support::mark_exited` (`mod.rs:200-209`) into `input_client.rs`'s `mod tests`. | A.B8 | — |

#### INP — Runtime input and writer (`src/runtime/{input,writer,input_client}.rs`)

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| INP1 | Drop the `run: &RunRecord` parameter (INV8) from `inspect_received`, `permission_history`, `retained_permission_response`, `permission_received`, `receipt_received`, `retire_received`, `handle_received`, `deferred_target`, `start_deferred`, `attempt_deferred`, `immediate_preflight`, `validate_write_now`, `acquire_write_guard`, `advance`, `reserve_pending`, `arm_pending`, `interrupt_composer`, `write_pending_stage`, `stop_composer_stages`, `retain_permission_failure`, `validate_live`/`validate_live_run`, `validate_receipt_owner`, `receipt_guard_snapshot`, `acquire_receipt_guard` and `acquire_rearm_guard`, and from their callers (`service.rs:222`, `:264`, `:281`, `:1271`). Delete the ensures `input coordinator run identity changed`, `receipt caller run identity changed` and `retirement caller run identity changed`, the `run.run_id == self.run.run_id` conjuncts (`input.rs:1253`, `:1958`) and the first `deferred_target` branch (`:2289-2291`). Keep every comparison of the database `current` row against `self.run`. | B.C1 | — |
| INP2 | Delete `CurrentRequest.identity`, its initializer (`input.rs:819`) and both conjuncts (`same_request`, `:1336`). | B.C2 | — |
| INP3 | `inspect_received` takes `PermissionInspectInput` directly, deleting its `_ => None` fallback, `permission inspection operation required` error and duplicate `identity.validate`; `handle_received`'s `_ =>` arm (`:1975`) becomes `unreachable!("serve_input routes only prompt and exec submissions here")`. | B.C3 | — |
| INP4 | Drop the `shutdown: Option<Instant>` parameter of `receive_input_expiration` and `InputExpiration::receive` and its clip (`input.rs:3983-3985`); delete the duplicate `expires_at_mono_ns > 0` ensure (`input.rs:4001`), which `InputControlRequest::validate` enforces. | B.C4, D.C3, D.C7 | `SPEC.md:3286` drops "or any earlier shutdown deadline" |
| INP5 | Delete `Outbox::initialize`'s repeated deadline and lock checks (`writer.rs:892-893`). | B.C5 | — |
| INP6 | Drop `EventWriter::start`'s `outbox: PathBuf` and the first `validate_run_paths` + equality ensure (`writer.rs:181-185`); check emptiness through the opened `Outbox`'s file. | B.C6 | — |
| INP7 | `InputCoordinator::result` writes `input_id: operation.input_id.into()`; `InputCheckpoint.input_id` becomes `#[cfg(test)]`. | B.C7 | — |
| INP8 | `advance`'s `let Some(pending) … else { return Ok(None) }` (`input.rs:3116-3118`) becomes `.expect("pending input survives preflight")`. | B.P1 | — |
| INP9 | `retire_received` calls `validate_receipt_owner` inside its transaction instead of inlining it. | B.P2 | — |
| INP10 | `EventWriter.sender` becomes a plain `SyncSender`; delete `sender.take()` and the `event writer is stopped` branch. | B.P3 | — |
| INP11 | Delete `InputServices.stage_settle`; production reads `STAGE_SETTLE`, and the test override moves behind `#[cfg(test)]` like `writer.rs:203-206`. | B.P4 | — |
| INP12 | Delete `OutboxScan.maximum_event_seq`; production uses `!scan.frames.is_empty()` (`writer.rs:1045`), tests use `frames.last()`. | B.P6 | — |
| INP13 | Delete `WriterStatus.degraded`; consumers (`observation.rs:753`, `:785`, `:808`, `:815`, `:994`, `:1009`) read `diagnostic.is_some()`. | B.P7 | — |
| INP14 | `Outbox` keeps the path `open_outbox` validated, and `validate_with` checks it instead of rebuilding `"events.outbox"` (`writer.rs:832`). | B.P8 | — |
| INP15 | Drop `InputCommandHost::lock`'s unused `run` parameter from the trait, every impl (`NativeInputHost`, `NoLiveHost`, `UnavailableReceiptHost`, `SocketHost`, `LockTakingHost`, `delivery.rs` `DeliveryHost`) and every call site. | C.C6 | — |

#### LIFE — Lifecycle, service, recovery and control (`src/runtime/{lifecycle,service,recovery,control}.rs`)

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| LIFE1 | Remove receipt-only exclusive ownership (INV9): delete `RecoveryHost::startup_process_evidence` and its impls, `exclude_previous_owner`, `ExclusiveReceiptGuard` (with `settle`, `zero_byte_retirement`, `retirement_binding`), `reconcile_input_with_host`, `ZeroByteRetirement::from_exclusive_guard` and the receipt-only CAS. `recover` treats a creator-owned run as `Unknown` directly and drops the `matches!(startup, Alive)` term (`recovery.rs:417`). In `input_client.rs`, a failed live reconciliation returns `original_failure` directly (`:790-807`), and an absent placement (`:718-729`: deregistered member or deleted fleet) fails with `receipt reconciliation requires the member's live runner; the input stays uncertain`. The live runner's receipt exclusion (`InputCoordinator::acquire_receipt_guard`) and the live-guard zero-byte proof (`from_live_guard`) stay. | D.P1, C.P1 | § Receipt contract narrowing |
| LIFE2 | Delete `RecoveryHealth::Mismatched`; the decode error maps to `Unavailable`; drop the merged alternatives (`recovery.rs:408`, `input_client.rs:1427`). | D.C1 | — |
| LIFE3 | `LifecycleRequest::Start.pane_id` becomes `String`; `lifecycle.rs:725` uses `.expect("published placement carries the created pane")`; `start` takes `&str`; delete `service.rs:1107`'s `ok_or_else`. | D.C4 | — |
| LIFE4 | Drop `member_id` from `recover_member_under_pane_lock`, `recover_member_with_held_pane_lock` and `DeletionHost::recover`; take it from the borrowed run; delete the `borrowed recovery member identity mismatch` ensure. | D.C5 | — |
| LIFE5 | Declare `RecoveryOutcome` as a plain enum with `as_str`, not through `stored_enum!`. | D.C6 | — |
| LIFE6 | Replace `RecoveryHost::checkpoint(boundary, run, deadline)` with a `check_deadline` call plus a `#[cfg(test)]` boundary hook, removing `RecoveryBoundary` from the production build. | D.P2 | — |
| LIFE7 | Production calls `ShutdownClock::new` (`service.rs:175`, `:590`, `:897`); `ShutdownClock::with_unit`, `NativeIo.shutdown_unit` and the `RunnerDependencies` `catalog`/`clock` fields become `#[cfg(test)]`. | C.P2, D.P3 | — |
| LIFE8 | Delete `HealthSnapshot.output_seq` and its write (`service.rs:1200`); the observation test synchronizes on another signal. | D.P4 | — |
| LIFE9 | Drop the re-validation after `HealthSnapshot::decode` (`recovery.rs:403-407`, `input_client.rs:152-153`); test hosts build snapshots through `decode`. | D.P5 | — |
| LIFE10 | `tick_running` uses `started(&mut self.coordinator)` (`service.rs:1226-1229`). | D.P6 | — |
| LIFE11 | Make `ShutdownOutcome.dropped_display_bytes`, `pending_event_frames` and `unsaved_observation_gap` locals of `shutdown()` that feed the same diagnostics. | C.C7 | — |
| LIFE12 | `LaunchHost::settle_hermes` becomes required; `CaptureVerificationLaunchHost` forwards to `self.inner.settle_hermes(..)`. | C.C8 | — |
| LIFE13 | `ZeroByteRetirement.input` becomes `InputOperation`; `validate_proof` always runs `same_identity`; the fixture takes a receipt. Applied after LIFE1 leaves `from_live_guard` as the only constructor. | G.C7 | — |

#### AGT — Coding agents, catalog, detector, Hermes and extensions (`src/coding_agent/`, `assets/extensions/`)

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| AGT1 | Delete the `#[cfg(test)] impl` with `CodingAgent::validate_spawn_policy_argv` (`mod.rs:457-550`) and the three `*_capture_argv_validation_*` tests. | E.C1 | — |
| AGT2 | `Settlement::Verified` becomes a unit variant. | E.C2 | — |
| AGT3 | Drop `&& !cap.fixture_ids().is_empty()` (`inputs.rs:1103`), `AnswerCapability.fixture_ids`, its accessor, `Answer.fixtures` and the test constructors' `fixture_ids` parameter. | E.C3 | `SPEC.md:2270` drops "and the cited fixture identities" |
| AGT4 | Shrink `ScreenRegion` to `{top, bottom}` without serde; `native_capture` gets its own `CaptureRegion {top, left, bottom, right}` with serde for its masks (`observation.rs:1153`, `:1293-1294`, `:1976`, `:2234`). | E.C4 | — |
| AGT5 | Replace the model/effort `?` re-validation in `build_spawn_argv_with_policy` (`mod.rs:393-394`) with `assert!(… .is_ok(), "member create validates model and effort before argv construction")`. | E.C5 | — |
| AGT6 | Delete `ObservationKind::{PermissionDenied, Working, ComposerDraft}` with their parse, name and fingerprint arms (`detector.rs:25-68`, `:907`, `catalog.rs:937`), `RuntimeEventKind::PermissionDeniedObserved` and its arms (`observation.rs:10570`, `:10665-10682`), `GateReason::Working` and `GateReason::Draft` (`observation.rs:10710-10711`), and `InteractionClass::{Trust, Update}` (`broker/runtime/mod.rs:489-492`). The same step removes `'trust'`, `'update'` and `'permission_denied_observed'` from the baseline CHECKs and updates every test that stores them (`events.rs:2123`, `:2197`, `:3156`, the `stored_enum!` `test_values` lists). | E.P1, G.P1 | § Schema delta; `SPEC.md:1590`, `:2965`, `:2968`, `:2970`, `:2972`, `:3024`, `:3025`, `:3044`, `:3354`; `cli.md:192`, `:362` |
| AGT7 | Remove `extract.native_request_id`: the `RawExtract` key, `Rule.native`, `Matched.native`, `ScreenObservation.native_correlation`, the always-false `active.native.is_some() && …` (`observation.rs:10678`), the `None == None` comparisons (`observation.rs:191`, `input.rs:1143`) and the `native_request_id` key in event context and evidence. | E.P2 | `SPEC.md:2502` ("`native_request_id` and sensitivity"), `:3023` catalog rule keys, `:3095` and `:3107` event examples, `:3119` key order, `:3121` evidence keys, `:3141` automatic-review nulls, `:3168` key table row; `cli.md:364`, `:366`, `:368` |
| AGT8 | Remove the profile `sensitive` flag: `RawRule`/`Rule.sensitive`, the detector `sensitive` parameter and raw evidence key (`detector.rs:688-695`, `:775`, `:896`) and the `\|\| sensitive` term and `sensitive` read in `evidence.rs:90-94`, `:185`. The operator `sensitive_rules` redaction file is unchanged; `native_capture` (`observation.rs:1889`) uses only its policy's `sensitive` set. | E.P3 | `SPEC.md:2502` sensitivity clause, `:3023` rule keys, `:3129` drops "a profile-marked sensitive form" |
| AGT9 | Delete `CursorRule::Outside`, its match arm and the test override (`detector.rs:3684`). | E.P4 | — |
| AGT10 | Delete `ChoiceScope::Session` and its test uses. | E.P5 | `SPEC.md:1107`, `:2737` scope enumerations |
| AGT11 | Fixture registry split per § Fixture registry split. | E.P6, E.P8, E Appendix A | § Fixture registry split item 4; `docs/docs/contributing.md` `assets/` row |
| AGT12 | `composer.submission`, `prompt` and `exec` become required (no `Option`, `catalog.rs:311-336`, `:573-576`, `:1136-1151`, `:1372`; `detector.rs:838-841`); missing-plan fallback tests remove the plan in test support. `annotations` stays optional. | E.P7 | `SPEC.md:2770` drops "an explicitly selected test catalog may omit it to exercise missing-plan fallback" |
| AGT13 | Reduce `Detector::match_answers`'s `applies` closure to `observed.match_evidence == MatchEvidence::Complete`, with `debug_assert!` for the backend, version and request-rule equalities. | E.P9 | — |
| AGT14 | Remove Hermes `--permission-mode manual`; `smart` is the only Hermes choice. | E.P10 | Every Hermes `manual` mention in `SPEC.md` (`:1814`, `:2180`, `:2206`, `:2573` "under either Hermes launch choice"); `cli.md:226` choices and `manual` class list; `docs/docs/concepts/coding-agents.md:45`, `:48`; hermes `{permission_flags}` in `assets/skills/cafleet/reference/coding-agents.md:357` |
| AGT15 | Delete the Hermes plugin's `_cafleet_env()` and its `env=` arguments (`assets/extensions/hermes/cafleet/__init__.py:88-98`, `:300`). | E.P11 | `SPEC.md:2084` Hermes cell becomes "The gateway's environment" |
| AGT16 | Remove the `HermesConfig` trait; call `HermesCli` directly and fake it with `test_support::executable_stub` (fakes at `hermes.rs:703`, `:872`, `assets.rs:528`). | E.P12 | — |
| AGT17 | Delete `pi_extension()`; derive the path from `assets::pi_extensions_dir` and the embedded file list. | E.P13 | — |

AGT7 and AGT8 also delete the `"native_request_id"` extract key and the `"sensitive"` rule key from every rule in `assets/profiles/bundled.json` (both keys appear on every rule) and from every test catalog definition (`catalog.rs` and `detector.rs` tests, `observation.rs` synthetic definitions including `native_capture`'s), in the same step, because `deny_unknown_fields` rejects them once the Rust fields are gone.

#### BRK — Broker, database and schema (`src/broker/`, `src/db.rs`, baseline)

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| BRK1 | Delete `member_runs.reachability`, the `Reachability` enum, `RunRecord.reachability`, the `reachability='reachable'` assignment (`input_client.rs:532`) and the runtime JSON `reachability` key (`output/json.rs:33`). | G.C1 | § Schema delta; `SPEC.md:1070` runtime key order; `SPEC.md:2864` becomes "Treat the run as unreachable until exclusive recovery establishes loss"; `SPEC.md:2881` drops "its normal bounded transaction can record `reachability=suspect` on failure or `reachable` on success"; `cli.md:348` runtime field list |
| BRK2 | Delete `RunStatus::Stopping` and `'stopping'` from both status CHECKs; update `events.rs:1527`. | G.C2 | § Schema delta |
| BRK3 | Delete `member_placements.backend DEFAULT 'tmux'`; the `db.rs:991` schema-damage case targets another clause. | G.C3 | § Schema delta |
| BRK4 | Delete `asset_installs.cafleet_version` (INV3), the `cafleet_version` parameter of `record_asset_install`/`install_agent`, and the version from `recorded_asset_install`, which returns `Option<String>` (the timestamp). `install_check` has one recorded arm, `ok` with `recorded at <installed_at>`, and keeps its `no installation recorded at <path>` warn. | F.F9, G.C4 | § Schema delta; `SPEC.md:551` ok cell `recorded at <installed_at>` and warn cell drops the other-version clause; `SPEC.md:554` comparison sentence, `:608`, `:617` example, `:3858` ("upserts its timestamp"); `cli.md:473` example and other-version sentence |
| BRK5 | `register_member_rows` takes `&NewPlacement`; paneless test members get a pending placement (`mux_pane_id: None`). | G.C5 | — |
| BRK6 | `MemberRecord.placement` becomes `Placement` via an inner `JOIN` (INV1); delete every placementless branch: CLI5–CLI7, CLI10, `press.rs:65` and `pane_lock.rs:299-301`. | G.C6, F.P6 | `SPEC.md:95`, `:190`, `:518` ("the root Director and members with a pending placement"), `:893` drops "Missing placement fails pane actions; list/show retain diagnostic nulls", `:972` ("Revalidate pending placement"), `:976`, `:980`; `cli.md:57`, `:276` |
| BRK7 | Remove run-less ordinary-member handling (INV5). Delete `finalize_deletion`; `cli/fleet.rs:544` calls `with_database_budget`. `delivery.rs:172` (ordinary recipients only; the root Director is excluded earlier) and `cli/member.rs:743` (prompt/exec; the Director is refused at `:801`) bind the run with `.expect("every ordinary member records its run")`, and `deliver_wrapped_pending` takes that run instead of calling `require_runtime` again (`delivery.rs:204`). `require_runtime` is deleted. `missing_run` stays only in recovery's `snapshot` (`recovery.rs:161`), where the root Director is the target that reaches it. | G.C10, G.P5, H.C7 | `SPEC.md:517` drops "A run-less ordinary row is ended as §6.3 `member delete` specifies for a run-less row"; `SPEC.md:893` drops "and an ordinary member without a recorded run"; `SPEC.md:968` drops the run-less ordinary row sentence; `SPEC.md:1070` "`runtime` is `null` for the root Director"; `SPEC.md:776` row, `:1233` row, `:1472` run-less clause, `:3418` cause; `cli.md:130`, `:216` drops "Prompt/exec fail before pane I/O for an ordinary member without a recorded run", `:264` drops "(none when run-less)", `:348`, `:537`. Delete the run-less ordinary-member tests (`cli/fleet.rs:1543`, `cli/member.rs:2720-2752`, `cli/message.rs:980-1030`). |
| BRK8 | `record_input_state` accepts only `InProgress → Ready \| Accepted`; delete the `(Ready, InProgress)` arm and the `expected != Ready` guard. | G.C8 | — |
| BRK9 | Drop `AND from_member_id!=to_member_id` from `PENDING_PREVIEW` and the preview clause `message.from_member_id != run.member_id` in `reserve_input_with_provenance` (`inputs.rs:419`). | G.C9, G.P15 | — |
| BRK10 | `classify_schema` becomes private and returns a unit `Incompatible` signal; delete the `":memory:"` fallback. | G.C11 | — |
| BRK11 | Declare `MemberKind` as a plain enum with `as_str`. | G.C12 | — |
| BRK12 | Drop the write-only audit columns `members.deregistered_at`, `member_runs.created_at`, `member_runs.exit_signal`, `member_runs.observation_gap_count` and `member_runtime_events.committed_at`, with their struct fields, writes, the nonnegative check (`broker/runtime/mod.rs:185`), the `db.rs` shape lists (`:1107`, `:1283`) and test inserts (`db.rs:815`). `recovery.rs:559` keeps its `cleanup_diagnostics_json` write. The runtime-event cause keys `exit_signal` and `gap_count` (`events.rs:456-491`, `observation.rs:878-881`) stay; `events.rs:746` writes only `status`, `ended_at` and `exit_code`. | G.P2 | § Schema delta; `SPEC.md:3069` drops "and `observation_gap_count`" |
| BRK13 | `get_fleet`/`list_fleets` expose `director_member_id: i64` (the column stays nullable for the circular FK insert) via `.expect("a committed fleet records its root Director")` (INV2); delete the downstream `Option` handling (`cli/member.rs:377-382` and its error, `lifecycle.rs:273-275`, `messaging.rs:284-286`, `broker/runtime/mod.rs:247`, `:320`, `output/text.rs:76-80`); `MemberRunRegistry.director_member_id` becomes `i64` and `director_label` takes `&[i64]`. | G.P3, F.F5, F.P1, F.P5 | `SPEC.md:1221` row |
| BRK14 | Delete `require_live_fleet`'s "root Director has no concrete placement" arm (INV2). | G.P4 | `SPEC.md:1223`; `cli.md:523` |
| BRK15 | Drop the pure-CHECK duplicates in `get_run` (`broker/runtime/mod.rs:169-199`), `events.rs:28-33`, `:152-176` and `inputs.rs:123-142`; keep typed enum conversion. | G.P9 | — |
| BRK16 | Drop the effective-policy `schema_version` from the record, struct and projection (`events.rs:891-916`, `:964-966`). | G.P10 | `SPEC.md:3185`, `:3196`; `cli.md:350` |
| BRK17 | Drop the event payload `schema_version` (`events.rs:561`). | G.P11 | `SPEC.md:3075`, `:3083`, `:3119`; `cli.md:364` |
| BRK18 | `delivery_projection`'s `None if delivery_reason.is_none()` arm for attempted inputs (`inputs.rs:71`) becomes an `unknown()` error; the `NotSubmitted` arm keeps `method_unrecorded`. | G.P12 | — |
| BRK19 | Drop the aliases `original_transition`/`validate_observed_transition` and update their call sites (`recovery.rs:2818`, `writer.rs:944`, `:979`, `:1061`). | G.P13 | — |
| BRK20 | Replace `idx_messages_from_member_status_ts` with `idx_messages_from_member_created_at ON messages (from_member_id, created_at)`. | G.P14 | § Schema delta |
| BRK21 | `select_listen_snapshot` keeps one `require_live_fleet` and validates runtime rows directly. | G.P7 | — |
| BRK22 | Express `active_member_fleet` through `member_fleet_status`. | G.P8 | — |
| BRK23 | `fleets.name` becomes `NOT NULL` (INV4); `create_fleet` takes `&str`; `FleetRecord`/`FleetSummary`/`CreatedFleet.name` become `String`; drop both `unwrap_or("-")` (`output/text.rs:81`, `:93`). | F.P4 | § Schema delta; `SPEC.md:51` drops "optional" |

#### CLI — CLI and output (`src/cli/`, `src/output/`)

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| CLI1 | Drop `read_body`/`resolve_body`'s `flag` parameter; inline `"--file"`. | F.F1 | — |
| CLI2 | `DeletionHost::settle_hermes` becomes required; `FixtureDeletionHost` implements it. | F.F2 | — |
| CLI3 | Bind the `listen` startup snapshot with `let InboxSnapshot::Open { fleet_id, .. } = &snapshot else { unreachable!("an unpinned listener snapshot is open or fails") }`. | F.F3 | — |
| CLI4 | `member create` reads the Director pane with `.expect("the root Director placement records its pane")` (INV2). | F.F4 | — |
| CLI5 | `resolve_coding_agent` replaces both `ok_or_else` calls with `.expect` naming INV1/INV2. | F.F6 | `SPEC.md:1243-1244`; `cli.md:524-525` |
| CLI6 | Delete `load_member`'s `tolerate_missing_placement` and its error; callers use `active_member`. | F.F7 | `SPEC.md:1232`; `cli.md:536` |
| CLI7 | Delete `NO_PLACEMENT_STATUS`; `unplaced_member_outcomes` uses an inner join that always yields `PENDING_PANE_STATUS`; `member.rs:601-605` always yields it; delete the placementless test case. | F.F8 | `SPEC.md:522` drops `(no placement)` |
| CLI8 | `probe_run` reads `conn.path().expect("doctor reads a file-backed database")`; `source` becomes `&str`. | F.P2 | — |
| CLI9 | Split `CallerReading` into `SharedServer { label, pid, message }`, `Unavailable(CafleetError)` and `Resolved { name, server, pane }`; delete both panicking arms. Applied with MUX8. | F.P3 | — |
| CLI10 | `format_member_list` and the roster JSON print the non-optional placement. | F.P6 | `SPEC.md:190`; `cli.md:57` drop "placement may be null" |
| CLI11 | `recovery::diagnostics` parses `Vec<String>` and fails loudly on a non-string; `format_runtime_recovery` prints strings directly. | F.P7 | — |
| CLI12 | `format_message` always emits `format!("{header}\n{body}")`. | F.P8 | — |

#### MUX — Multiplexer, delivery, pane lock, assets, config, time and errors

| ID | Change | Sources | Contract edits |
|---|---|---|---|
| MUX1 | Drop `timeout_secs` from `CommandRunner::run`, `BackendCore::{run, run_tolerating_missing, map_error}` and `map_run_error`; the `Timeout` arm keeps only the pane-input-deadline message; `DeadlineRunner::run` calls `run_for`; `SystemRunner::run` is unbounded. | H.C1 | `SPEC.md:1545-1546` drops `<backend> command timed out after <N>s: <argv>` |
| MUX2 | `CommandRunner::run_for`/`run_until` become required; `FakeRunner` carries the former default behavior and the other fakes (`AbsoluteRecorder`, press `Probe`, member `CloseRunner`) delegate to it. | H.C2 | — |
| MUX3 | Drop `ignore_missing` from `kill_pane`, `close_pane`, `close_reading_tab` and `run_tolerating_missing`; close always tolerates the recognized pane-gone cause. | H.C3 | `SPEC.md:1539` becomes "Close tolerates only a recognized pane-gone cause" |
| MUX4 | Replace `lock_member_with_reservation`'s `reservation` with `const PRESS_LOCK_RESERVE: Duration = Duration::from_secs(1)`. | H.C4 | — |
| MUX5 | `Verification::Verified` becomes a unit variant. | H.C5 | — |
| MUX6 | Drop `pid` from `Refusal::ProcessUnreadable` and `WalkEnd::Unreadable`. | H.C6 | — |
| MUX7 | Delete `CommandRunner::remaining_budget`; `DeadlineRunner` gets a private `remaining()` mapping expiry to `RunError::deadline_expired()`. | H.P1 | — |
| MUX8 | `shared_server` returns a dedicated `SharedServer { label, pid }`; `shared_server_message` takes it; delete `Refusal::SharedServer` and the panic arms (`caller.rs:239-242`, `:269-271`). | H.P2 | — |
| MUX9 | Drop `AgentPaths.identity`; pass `&paths.base.display().to_string()` (`assets.rs:208`). | H.P3 | — |
| MUX10 | `default_database_url` calls `home_dir()?`, keeping its database context with `map_err`. | H.P4 | — |
| MUX11 | Drop `validate_key_target`'s repeated `ensure_available` and one `check_deadline`; keep the `mux_pane_id` presence check. | H.P5 | — |
| MUX12 | `parse_lenient` returns `Option<DateTime<Utc>>`; its doc reads "parses a stored canonical timestamp". | H.P9 | — |
| MUX13 | Move `DEFAULT_TMUX_SOCKET` into `multiplexer/mod.rs`, use it in `caller_server` and delete `default_tmux_socket`. | H.P10 | `SPEC.md:363` uses the `env -u TMUX` form of `:464` |

#### CAP — `native_capture` self-tests (`src/runtime/observation.rs`, `mod native_capture`, lines 1042–10127)

Keep the four driver entries — `#[ignore]`d tests at `observation.rs:3950`, `:9135`, `:10002` and `:10072` — and every function they reach. Delete each other `#[test]` in the module whose entry point and asserted value are `native_capture` types:

| Module | Disposition |
|---|---|
| `tests::collection`, `tests::capture_policy`, `tests::binding` | Delete: capture seeds, masks, limits and compiled capture policy are harness types |
| `files::tests` | Delete: arm/config readers, producer binding and publication are harness types |
| `verification::tests` (`ApprovedCase` driver guards, overlays, separators) | Delete: case drivers and configuration overlays are harness types |
| `exporter::tests` non-ignored tests | Delete unless the entry point is the runner service |
| `tests::recorded_policy`, `tests::service_capture`, `verification::tests::runner_collector_common_service_arms_on_actual_resize_*` | Keep each test whose entry point is production code (the runner service, `Transport`, `Detector`, `ProfileCatalog`) and whose assertion is that code's decision; delete the rest |

Record every test's keep/delete classification with its entry point in `.verification/native-capture-tests.md` under BASE before deleting, then delete helpers that become unused. (Source: A.B4.)

#### VIS — Visibility and naming tidy

| ID | Change | Sources |
|---|---|---|
| VIS1 | Make private: `count_commit`, `loss_transitions`, `terminal_outcome`, `unlink_descriptor_file` (`recovery.rs`), `validate_version`, `frame_control_body`, `OwnerLockProbe`, `HealthProbe`, `RuntimeProbes` (`control.rs`); expose `ControlClient.stream` only under `#[cfg_attr(test, ...)]` for its test access. | D.P8, D.P9 |
| VIS2 | Make private: `PLUGINS_KEY`, `PLUGINS_ENABLED_KEY`, `GATEWAY_INJECTION_KEY`, `PluginState`, `read_plugin_state` (`hermes.rs`); `build_spawn_argv_with_policy`, `run_bounded`, `version_token`, `config_variable` (`coding_agent/mod.rs`); `ComposerPlan::encode`. | E Appendix B |
| VIS3 | Delete `assert_no_debug_or_serialize!`; rewrite its six call sites (`catalog.rs:3113`, `:7048`, `detector.rs:2529`, `:2536`, `input.rs:8890`, `observation.rs:12753`) as `assert_not_impl!(T: std::fmt::Debug, serde::Serialize)`. | H.P8 |
| VIS4 | Drop `CafleetError`'s `source()` override and `impl std::error::Error for MultiplexerError {}` when the build passes without them. | H.P12 |
| VIS5 | Rename `assets::lookup` to `preset(path)` with `PRESETS` built in. | H.P11 |

### Fixture registry split

`assets/profiles/native/*.json` are `include_bytes!`'d only inside `#[cfg(test)] mod test_support` (`catalog.rs:1418-1422`), so `SPEC.md:2280`'s claim that they are embedded in the binary is false, and the production loader's registry checks validate a compile-time constant the test suite already verifies.

1. Production `ProfileCatalog::from_definitions` reads `bundled.json` unchanged apart from the AGT7/AGT8 key deletions: `RawCatalog.fixtures` and every `fixture_ids` key deserialize as `serde::de::IgnoredAny`. `deny_unknown_fields` keeps rejecting unknown keys on every structure production parses; it no longer inspects the contents of the ignored `fixtures` entries or `fixture_ids` values. Production deletes `RawFixture`, `FixtureKind`, `Fixture`, `Interval`, `Scope`, `cite`, the continuation-native check and the digest shape check.
2. `catalog.rs`'s `#[cfg(test)] pub(crate) mod test_support` gains `validate_provenance(definitions: &Value) -> Result<()>`, which deserializes its own `deny_unknown_fields` provenance view (`FixtureKind::{Native, Synthetic}`; fixture keys exactly `id,kind,backend,native_version,rows,cols,source_ref,sha256`) and applies every former rule with its existing error text: unique ids, lowercase 32-byte digests, non-empty citations, citations inside the profile's backend interval, continuations citing native fixtures. `verify_bundled_fixtures` runs after it for the bundled catalog.
3. `synthetic_catalog()` (used by `native_capture`) and every test that asserts a provenance refusal call `validate_provenance`; `FixtureKind::Synthetic` stays, test-only.
4. `SPEC.md` attributes every provenance rule to the bundled-catalog build guard (the test suite): § Bundled profiles and native screen fixtures states that the bundled catalog is embedded in the binary and the native screen fixture files are test-suite inputs; "The loader validates every catalog fixture entry" and "Catalog validation requires every cited fixture's backend…" become build-guard requirements. Sweep §6.9 for every `fixture_ids`, provenance and "validation" sentence and assign each to the build guard; known hits are `SPEC.md:2270`, `:2276`, `:2280-2282`, `:2685`, `:2693`, `:2741`, `:2770` ("Unknown fields and incomplete or incompatible provenance fail validation"), `:3020` and `:3524`.
5. `docs/docs/contributing.md`'s `assets/` row says `assets/profiles/` holds the embedded bundled catalog and one native screen fixture file per backend that the test suite reads.

### Receipt contract narrowing

After LIFE1, `SPEC.md` §6.9 Durable input receipts and settlement and `cli.md` describe only live reconciliation:

- Reconciliation synchronizes with the pinned runner's receipt handler under the recorded placement's pane/input guard. An unreachable writer leaves the receipt uncertain, and the Director records `abandoned-uncertain` through `input reconcile`.
- An absent placement (deregistered member or deleted fleet) has no live runner, so reconciliation fails with the LIFE1 string and the receipt stays uncertain.
- Delete the receipt-only exclusive ownership path only: "the receipt-only exclusive ownership path for an unreachable writer" (`SPEC.md:1080`); "and any receipt-only exclusive ownership proof" and "an unreachable owner requires the exclusive proof below" (`:3323`); the absent-placement proof in `:3327`; the exclusion/takeover paragraph `:3329`; the receipt-only CAS paragraph `:3331`; the closing "The native startup probe reports unknown…" sentence of `:3339`; `cli.md:404` "unreachable owners require exclusive receipt-only ownership, otherwise uncertainty remains" becomes "an unreachable owner leaves uncertainty".
- Keep the live runner's receipt-only writer exclusion and its zero-byte proof wording (`SPEC.md:3335`, `:3337` and the rest of `:3339`).
- Add the LIFE1 string to `SPEC.md` §6.3 Error Messages beside the `input reconcile` rows.

### Schema delta

`assets/migrations/V1__baseline.sql` and the `SPEC.md` §8 DDL change identically. Each change lands in the implementation step that removes its Rust consumer (§ Implementation).

```sql
-- members: drop deregistered_at (BRK12)
CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL
);
-- fleets: name NOT NULL (BRK23)
    name TEXT NOT NULL,
-- asset_installs: drop cafleet_version (BRK4)
CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);
-- member_placements: no default (BRK3)
    backend TEXT NOT NULL,
-- messages: sender index covers MAX(created_at) (BRK20)
CREATE INDEX idx_messages_from_member_created_at ON messages (from_member_id, created_at);
-- member_runs: drop created_at, reachability, observation_gap_count, exit_signal (BRK1, BRK12); drop 'stopping' (BRK2)
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    recovery_origin_status TEXT CHECK(recovery_origin_status IS NULL OR recovery_origin_status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
-- member_interactions: drop 'trust','update' (AGT6)
    classification TEXT NOT NULL CHECK(classification IN ('permission_request','question','unknown_interaction')),
-- member_runtime_events: drop committed_at (BRK12); drop 'permission_denied_observed' (AGT6)
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','automatic_refusal_observed','automatic_review_unavailable','input_delivery_result','runner_degraded','runner_lost','runner_exited')),
```

`db.rs`'s required table/column shapes follow the baseline, and every `INSERT`/`SELECT` and row mapper naming a dropped column changes with it.

### Contract delta

| Surface | Removed |
|---|---|
| Error rows | `press backend does not match the recorded placement`; fleet has no root Director recorded; Director not found / has no placement, supply `--coding-agent`; has no placement row, it was not spawned via `cafleet member create`; root Director has no concrete placement; `<backend> command timed out after <N>s: <argv>`; ordinary-member `has no recorded run` rows for send and prompt/exec |
| Pending reasons and statuses | `observation health unavailable`; `(no placement)` |
| Doctor | Other-version installation warn row and example; the ok detail becomes `recorded at <installed_at>` |
| CLI choices | Hermes `--permission-mode manual` |
| JSON keys | runtime `reachability`; effective-policy `schema_version`; event `schema_version`; event and evidence `native_request_id` |
| Profile vocabulary | observations `permission_denied`, `working`, `composer_draft`; classes `trust`, `update`; cursor `outside`; scope `session`; rule key `sensitive`; extract key `native_request_id` |
| Descriptor | launch descriptor schema version |

Added contract string: `receipt reconciliation requires the member's live runner; the input stays uncertain` (LIFE1).

Operator-supplied files keep their own `schema_version` (event-redaction file, capture policy), and the bundled catalog and native fixture files keep theirs; they are external or test-suite inputs, not constant internal versions.

`SPEC.md` and `cli.md` prose that names reachability as a hold reason — a failed fresh health/ownership check, not the removed column or its values (`SPEC.md:1442`, `:1491`, `:3353`; `cli.md:190`, `:206`) — stays.

### Verification greps

Run each pattern with `grep -rnE --exclude-dir=native` over `src/ assets/ SPEC.md docs/docs .claude/rules README.md`; the exclusion removes the recorded native screens under `assets/profiles/native/`, whose captured native text is test input. Every hit outside the allowed residuals fails verification.

| Pattern | Allowed residuals |
|---|---|
| `'stopping'\|RunStatus::Stopping` | none |
| `permission_denied_observed\|PermissionDeniedObserved\|ObservationKind::(PermissionDenied\|Working\|ComposerDraft)\|GateReason::(Working\|Draft)\|composer_draft`, plus the backtick-quoted literals `permission_denied` and `working` | none |
| `InteractionClass::(Trust\|Update)\|'trust'\|'update'`, plus the backtick-quoted literals `trust` and `update` | none |
| `CursorRule::Outside\|"outside"\|ChoiceScope::Session` | none |
| `` `session` `` in `SPEC.md` | the multiplexer-context field (`SPEC.md:1509`) |
| `native_request_id\|native_correlation` | none |
| `reachability` | hold-reason prose listed in § Contract delta |
| `Reachability\|reachability=\|"reachability"\|'reachable'\|'suspect'` | none |
| `"schema_version": ?2\|schema_version=2\|schema_version=1\|"schema_version": ?1, ?"automatic_review"\|descriptor contains schema version` | none (event payloads, effective-policy records and the launch descriptor, in production code, tests and docs) |
| `schema_version` in `src/runtime/artifacts.rs` and in `src/broker/runtime/events.rs` outside its operator-file tests | none |
| `rule\.sensitive\|raw\["sensitive"\]\|"sensitive":\|profile-marked sensitive` | none (`sensitive_rules` and `native_capture`'s policy `sensitive` set do not match) |
| `\(no placement\)\|NO_PLACEMENT_STATUS\|has no placement\|placement may be null\|records no pane\|no root Director recorded\|no concrete placement\|tolerate_missing_placement` | none |
| `observation health unavailable\|press backend does not match\|timed out after` | none |
| `assets from cafleet\|for another version\|recorded <version>` | none |
| `cafleet_version` | `cafleet_database.cafleet_version` (baseline, `SPEC.md` §8 and its initialization sentence, `db.rs`) |
| `deregistered_at\|observation_gap_count\|committed_at` | none |
| `exit_signal` | the runtime-event `native_exit` cause key (`events.rs`, `observation.rs`, `SPEC.md` cause documentation) |
| `has no recorded run\|require_runtime\|finalize_deletion` | `missing_run`'s message in `broker/members.rs` and its recovery `snapshot` caller |
| `hermes: smart, manual\|either Hermes launch choice\|Explicit Hermes`, plus every backtick-quoted `manual` within a Hermes row or sentence | none; `manual` stays for the claude, codex, opencode and pi modes that keep it |
| `startup_process_evidence\|ExclusiveReceiptGuard\|exclude_previous_owner\|reconcile_input_with_host\|from_exclusive_guard\|receipt-only exclusive ownership\|exclusive receipt\|native startup probe\|RecoveryHealth::Mismatched` | none |
| `RecoveryBoundary\|failures_are_fatal\|validate_spawn_policy_argv\|HermesConfig\|remaining_budget\|assert_no_debug_or_serialize\|_cafleet_env\|fn pi_extension\|default_tmux_socket\|ignore_missing\|timeout_secs` | `RecoveryBoundary` only under `#[cfg(test)]` |
| `embedded in the CAFleet binary` near the native screen fixtures, `The loader validates every catalog fixture entry` | none |
| `run-less\|without a recorded run\|Missing placement\|missing placement` | none; the run-less ordinary-member tests (`cli/fleet.rs:1543`, `cli/member.rs:2720-2752`, `cli/message.rs:980-1030`) assert removed behavior and are deleted with BRK7 |

The `schema_version` patterns target only the removed constant versions; the operator event-redaction file, capture policy, bundled catalog, native fixture files, SQLite `PRAGMA schema_version` and `native_capture` producer/policy JSON keep their own `schema_version` and match none of them.

### Retained by decision

| Item | Source | Reason |
|---|---|---|
| `RequestTokenSource`, `InputDeadlineClock`, `ProcessObservations`, `OutboxIo` (with `Outbox<S>` generics), `ProcessTable` | B.P5 | Drive fault/race tests that cannot be produced against the real OS |
| `Clock`, `ReceiveIo`, `deliver_pending_with` host injection | H.P7 | Deterministic in-file unit tests for sleeps, busy errors and lock contention |
| `HermesSettlementHost`, `SpawnProbe` | E.P12 | The settlement host injects clock and sleep; the probe fakes native version probes |
| `RecoveryHost` trait (process/health evidence) | D.P2 | Crash-boundary tests; only `RecoveryBoundary`/`checkpoint` go (LIFE6) |
| `listen_snapshot_with_clock` | G.P6 | Same deterministic-timing seam class; expires the deadline mid-scan |
| `#[cfg(test)]` instrumentation inside `Transport`/`TerminalState` (observation hooks, observation phase, deferred child bytes, title callbacks, `feed_observed`, `transport.rs:100-105` … `:1089-1090`) | C.P3 | Compiled out of release builds and already inline `#[cfg(test)]` per § Test placement |
| Per-variant `LifecycleRequest.protocol_version` | D.P7 | `deny_unknown_fields` does not combine with `#[serde(flatten)]` on an internally tagged enum |
| `PaneCleanup::Attempted { error: None }` | H.P6 | Records the attempted pane for pane ownership during creation |
| `PtySession::shutdown` from `Drop` | B.P9 | Safety net for error returns between `PtySession::prepare` and `LiveShutdown` |
| `native_capture` drivers and their production seams (`ObservationClockKind`, capture fields, `attach_capture`, `begin_capture_shutdown`, `RunnerDependencies.capture`, `Running.capture`, the `service.rs:1664-1684` child-path dispatch) | A.B4 | User decision: the fixture-recording capability stays |

### Test policy

- Delete every test that asserts removed behavior; the absence is the test. Keep tests of current behavior and adapt their setup (explicit observer/host impls, pending placements instead of placementless members, `validate_provenance` for provenance refusals).
- Seams that only shorten timing move behind `#[cfg(test)]` (INP11, LIFE7).
- Every test stays in the implementation file it exercises (`.claude/rules/code-quality.md` § Test placement).

### Documentation edits

| Surface | Edits |
|---|---|
| `SPEC.md` | Every "Contract edits" cell, § Receipt contract narrowing, § Fixture registry split item 4 and § Schema delta in §8 |
| `docs/` | `concepts/coding-agents.md` Hermes mode row and paragraph (AGT14); `contributing.md` `assets/` row (AGT11) |
| `assets/skills/cafleet/reference/cli.md` | `:57`, `:130`, `:192`, `:216`, `:226`, `:264`, `:276`, `:329`, `:348`, `:350`, `:362`, `:364`, `:366`, `:368`, `:404`, `:430`, `:473`, `:523-525`, `:536`, `:537` per their catalog rows |
| `assets/skills/cafleet/reference/coding-agents.md` | hermes `{permission_flags}` binding drops `manual` (class `manual`) |
| `.claude/rules/code-quality.md` | § Test placement's "data files that production code embeds (every file under `assets/`)" becomes "data files under `assets/` (the files production code embeds and the native screen fixtures the bundled-catalog guard reads)" |
| `README.md` | Unchanged unless the `/update-readme` reconciliation reports drift |

### Version bump

Run `bump-my-version bump minor --no-commit` (or hand-edit the same lines) so `Cargo.toml`, the `cafleet` entry of `Cargo.lock` and `.bumpversion.toml` `current_version` read `0.34.0`. The change rides in the Step 4 implementation commit with the first baseline edit; no separate bump commit is created.

### Post-fleet-deletion action

Reinstalling the CLI replaces the binary the executing fleet runs on, and with `0.34.0` the default database becomes `cafleet_0.34.0.db`, which does not hold the executing fleet. The Director therefore runs `mise //:install` only after deleting the implementation fleet; no implementation step reinstalls.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Every code step applies every consumer edit (including tests) its type or signature change forces and ends with `mise //:test` and `mise //:lint` passing.

### Step 1: SPEC.md

- [x] Apply every `SPEC.md` "Contract edits" cell in § Removal catalog <!-- completed: 2026-11-12T11:06 -->
- [x] Rewrite §6.9 receipt reconciliation per § Receipt contract narrowing and add the LIFE1 error row <!-- completed: 2026-11-12T11:06 -->
- [x] Apply § Fixture registry split item 4, including the §6.9 provenance sweep <!-- completed: 2026-11-12T11:06 -->
- [x] Apply § Schema delta to the §8 DDL <!-- completed: 2026-11-12T11:06 -->

### Step 2: docs/

- [x] Update `docs/docs/concepts/coding-agents.md` for AGT14 <!-- completed: 2026-11-12T11:10 -->
- [x] Update `docs/docs/contributing.md`'s `assets/` row for AGT11 <!-- completed: 2026-11-12T11:10 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-12T11:10 -->

### Step 3: Skills, README and rules

- [x] Update `assets/skills/cafleet/reference/cli.md` per § Documentation edits <!-- completed: 2026-11-12T11:16 -->
- [x] Update the hermes binding in `assets/skills/cafleet/reference/coding-agents.md` <!-- completed: 2026-11-12T11:16 -->
- [x] Update `.claude/rules/code-quality.md` § Test placement <!-- completed: 2026-11-12T11:16 -->
- [x] Run the `/update-readme` reconciliation and apply any README drift <!-- completed: 2026-11-12T11:16 -->

### Step 4: Version and self-contained schema columns

- [x] Bump the version per § Version bump <!-- completed: 2026-11-12T11:25 -->
- [x] Apply BRK1, BRK2, BRK3 and BRK20 with their baseline and `db.rs` shape edits <!-- completed: 2026-11-12T11:25 -->
- [x] Apply BRK4 and BRK12 with their baseline and `db.rs` shape edits <!-- completed: 2026-11-12T11:25 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-12T11:25 -->

### Step 5: Placement, Director and run invariants

- [ ] Apply BRK5, BRK6, CLI5, CLI6, CLI7 and CLI10 <!-- completed: -->
- [ ] Apply BRK13, BRK14, BRK23 (with its baseline edit) and CLI4 <!-- completed: -->
- [ ] Apply BRK7 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 6: Remaining broker simplifications

- [ ] Apply BRK8–BRK11 and BRK15–BRK19, BRK21, BRK22 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 7: CLI and output

- [ ] Apply CLI1–CLI3, CLI8, CLI11 and CLI12 <!-- completed: -->
- [ ] Apply CLI9 together with MUX8 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 8: Multiplexer, delivery, assets, config, time

- [ ] Apply MUX1–MUX7 and MUX9–MUX13 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 9: Coding agents and the fixture registry split

- [ ] Apply AGT6 with its baseline CHECK edits <!-- completed: -->
- [ ] Apply AGT1–AGT5, AGT7–AGT10 and AGT12–AGT17 <!-- completed: -->
- [ ] Apply § Fixture registry split items 1–3 (AGT11) <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 10: Runtime observation and transport

- [ ] Apply OBS6–OBS8 and adapt every test observer <!-- completed: -->
- [ ] Apply OBS1–OBS5 and OBS9–OBS16 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 11: Runtime input and writer

- [ ] Apply INP1–INP7 <!-- completed: -->
- [ ] Apply INP8–INP15 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 12: Lifecycle, service, recovery and control

- [ ] Apply LIFE1 and then LIFE13 <!-- completed: -->
- [ ] Apply LIFE2–LIFE12 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 13: native_capture self-tests

- [ ] Write the CAP keep/delete classification ledger under BASE <!-- completed: -->
- [ ] Delete the classified harness self-tests and newly unused helpers <!-- completed: -->
- [ ] Confirm the four driver entries are listed by `mise //:test -- -- --list --ignored` <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 14: Visibility and naming tidy

- [ ] Apply VIS1–VIS5 <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` <!-- completed: -->

### Step 15: Verification

- [ ] Run every § Verification greps pattern and confirm only allowed residuals remain <!-- completed: -->
- [ ] Diff the baseline against the `SPEC.md` §8 DDL for every changed table <!-- completed: -->
- [ ] Confirm every § Retained by decision item is still present <!-- completed: -->
- [ ] Run `mise //:test`, `mise //:lint` and `mise //docs:build` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-12 | Initial draft |
| 2026-11-12 | Reviewer round 1: distinct catalog prefixes, inlined scope, compile-clean step order, exact verification greps, run-less removal across callers, receipt narrowing scope, provenance sweep, doctor ok output, audit-column boundaries, versioning and reinstall mechanics |
| 2026-11-12 | Reviewer round 2: `bundled.json` key deletions for AGT7/AGT8, remaining run-less/placementless contract sentences, targeted `schema_version` greps and native-screen grep exclusion |
| 2026-11-12 | Approved by the user |
