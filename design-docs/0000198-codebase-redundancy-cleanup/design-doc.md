# Codebase redundancy cleanup

**Status**: Approved
**Progress**: 13/56 tasks complete
**Last Updated**: 2026-10-29

## Overview

Remove waste from the CAFleet repository without changing the shipped specification. Every Rust test moves into a `#[cfg(test)]` module of the implementation file it exercises, and every test-only file and folder is deleted. Duplicated code, tests and documentation collapse to one owner each, with SPEC.md as the single authoritative specification, the docs site for users, and the skills for agents.

## Success Criteria

- [ ] `git ls-files cafleet/src` lists no test-only file or folder: no `runtime/runner_tests/`, no `runtime/runner/native_capture/`, `runtime/runner/native_capture.rs`, `runtime/runner/process/`, `coding_agent/profiles/contract_tests/`, no `*_tests.rs` and no fixture directory. Both checks print nothing: `git grep -n -A1 -F '#[cfg(test)]' -- cafleet/src | grep -E '^[^:]+-[0-9]+-\s*(pub(\([a-z ]+\))? )?mod [a-z_]+;'` (a `#[cfg(test)]` module loaded from a separate file) and `git grep -n -F '#[path' -- cafleet/src`.
- [ ] Every row of § Test relocation map is applied: each test lives in the file that defines the entry point it asserts on, and cross-file helpers live in `#[cfg(test)] pub(crate) mod test_support` of the owning file (§ Shared test-support homes).
- [ ] `CLAUDE.md`, `.claude/rules/code-quality.md` § Test placement and `docs/docs/contributing.md` state the test-placement rule with the wording in § Test placement rule.
- [ ] The test ledger reconciles exactly: the leaf-name multiset of `tests-after.txt` equals `tests-before.txt` with the ledger's additions, deletions, merges and renames applied (§ Test-change ledger).
- [ ] `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build` pass.
- [ ] The help text of `cafleet` and of every subcommand is byte-identical before and after (`help-before.txt` = `help-after.txt`, § Test-change ledger).
- [ ] `cafleet/src/lib.rs` declares only `cli` and `error` as `pub`; every other module is `pub(crate)` or private, and `mise //cafleet:lint` reports no dead code.
- [ ] `docs/docs/spec/` and `skills/cafleet/reference/runtime/` no longer exist; `git grep -n -e 'docs/docs/spec' -e 'reference/runtime' -e 'runtime/spec' -- . ':!design-docs'` prints nothing.
- [ ] SPEC.md contains every contract statement previously held only by a docs spec page, and no link into `docs/docs/`.
- [ ] `docs/docs/contributing.md` links SPEC.md; README links SPEC.md as the specification.
- [ ] `skills/cafleet/reference/cli.md` exists and covers every `cafleet` command, flag, output field and error string that a shipped skill tells an agent to run, parse or act on (§ Agent CLI reference).
- [ ] The removed contracts have zero hits: `git grep -n -F -e 'fleet capture' -e 'stale assets detected' -e 'no assets install is recorded' -e 'issues found' -e 'member_permissions' -e 'Upgraded from' -e 'nothing to do' -e 'db half failed' -e 'assets half failed' -e "cannot resolve the member's coding agent" -- SPEC.md docs skills .claude` prints nothing.
- [ ] Every adopted finding in § Production simplification catalog and § Test de-duplication catalog is applied, or recorded in the ledger as kept with the failed verification that kept it (§ Verify-then-act protocol).

---

## Background

A ten-slice audit found the following. Its findings files, `audit/{INP,OBS,SVC,TST,AGT,BRK,MUX,CLI,SPC,SKL}.md` beside this document, are committed with it. They hold the evidence and line locations behind every finding ID this document cites, as observed on the base commit.

- **Test-only files and folders.** `runtime/runner_tests/` has 36 `.rs` files (13,016 lines, 212 tests) and 2 fixture files. The `#[cfg(test)]`-only module tree `runtime/runner/native_capture*` has 5 files (3,662 lines), `runtime/runner/process/ownership_diagnostics.rs` is 381 lines, and `coding_agent/profiles/contract_tests/` holds one fixture.
- **Mis-housed in-file test modules.** `coding_agent/profiles.rs` has 8,510 test lines that exercise `catalog.rs`, `detector.rs`, `composer.rs` and `runtime/runner/episodes.rs`. `broker/mod.rs` has 6,222 test lines that exercise `broker/runtime/*`. `cli/mod.rs`, `coding_agent/mod.rs`, `listen.rs`, `multiplexer/mod.rs`, `input_client.rs` and `transport.rs` also have smaller mis-housed modules.
- **Documentation duplication.** SPEC.md (3,537 lines) repeats the `docs/docs/spec/*` pages almost verbatim. Commit 6712b036 deleted the guard that kept the copies in sync, and both copies still describe features that commit removed: `fleet capture`, the stale-assets guard, the sectioned `doctor` layout and member-permissions section, and the old `setup` messages.
- **Code and skill-text duplication.** Many shared helpers, wrappers and validations exist in several copies. The skills restate procedures in four to eight files each.

---

## Specification

### Contract invariants

The cleanup keeps every item below unchanged. Any edit that would change one of them is out of scope and must be reverted.

| Contract | Kept identical |
|---|---|
| CLI surface | Subcommands, flags, defaults, help text, exit codes |
| Output | Text layouts, JSON keys and key order, preview/notification lines |
| Errors | Every error string the binary can produce, and which error a given failure produces first |
| Storage | SQLite schema, migrations `V1`…`V6`, application identity, WAL/locking behavior |
| Wire | Runner control-frame bytes, including `"protocol_version":1` at its current key position |
| Shipped data | `coding_agent/profiles/native/*.json`, `bundled.json`, `presets/**` (all 77 bundled fixtures keep shipping) |
| Tasks | Every mise task, including `//cafleet:typecheck` (presets reference it) |

Removing an error string is in scope only when the string is unreachable (§ Verify-then-act protocol). Skill text, docs text and SPEC wording change; SPEC changes only to match the shipped binary or to absorb content from the deleted docs pages.

### Decisions

| Topic | Decision |
|---|---|
| Delivery | One feature branch and PR; one commit per Implementation step; each step leaves `test` and `lint` green |
| Scope | Whole repository, including the maintainer-only surfaces: `.claude/skills`, CI, `.gitignore`, `rustfmt.toml`, docs CSS |
| Specification owner | SPEC.md owns every contract. The docs site is for users (concepts, usage, setup). Skills carry only the subset an agent needs to drive cafleet |
| Docs spec section | `docs/docs/spec/` is deleted after SPEC.md absorbs its unique content. `docs/docs/contributing.md` links SPEC.md |
| Skill bundle | `skills/cafleet/reference/runtime/` is deleted entirely, including its concepts and how-to symlinks (user-confirmed). One hand-written `skills/cafleet/reference/cli.md` replaces it, and the single `director.md` concepts link is retargeted to `cli.md#member-create` (§ Skill bundle) |
| Native-capture harness | Kept. It becomes an inline `#[cfg(test)] pub(crate) mod native_capture` in `runtime/runner/observation.rs`, its tests become that module's `tests`, and its internal duplication is removed (OBS-08…11, OBS-13) |
| Ownership tracer | `ownership_diagnostics.rs`, its `#[cfg(test)]` hooks in `process.rs` and its self-tests are deleted (SVC-01) |
| Large files | Accepted. Tests are grouped in nested modules inside one `mod tests` at the end of the file |
| Needs-verification items | Applied through § Verify-then-act protocol |
| Unreachable validation | Removed when proven unreachable, keeping one owner per invariant. Kept when the proof fails |
| Uncited bundled fixtures (AGT-22) | Out of scope; all 77 keep shipping |
| Fixture-registration pins (AGT-07) | Replaced by one loop that asserts only what the loader does not enforce |
| Dead code | Visibility narrows to `pub(crate)`. An item with no caller in either the normal or the test build is deleted; an item whose only callers are tests becomes `#[cfg(test)]` (§ Production simplification catalog, Sweep) |
| Spawn delta tables | `create.md` and `execute.md` gain per-role delta tables, matching `interview.md` |

### Test placement rule

These texts are normative; write them verbatim.

**`.claude/rules/code-quality.md`**: append this section.

```markdown
## Test placement

Every Rust test is an in-file unit test. It lives in one `#[cfg(test)] mod tests` block at the end of the implementation file whose behavior it exercises, and `mise //cafleet:test` runs it.

- Place each test in the file that defines the entry point it asserts on. When a test drives several files, the file that owns the asserted behavior owns the test. Group related tests in nested modules inside `mod tests`.
- Put a helper shared by tests in several files into the implementation file that owns the type it builds, as `#[cfg(test)] pub(crate) mod test_support`. A test-only constructor or method on a production type is a `#[cfg(test)] impl` block beside that type. A helper used only by one file's tests stays inside that file's `mod tests`. Crate-wide test macros are defined at the top of `lib.rs`, before the first `mod` declaration. Reuse an existing helper or production function (clocks, digests, frame codecs, fixtures) before writing another.
- Write test inputs as literals or generated bytes inside the test module. Tests may read the runtime data that production code embeds (`coding_agent/profiles/native/*.json`, `bundled.json`, presets, migrations).
- Name a re-executed child test with `test_path!` beside the child, so moving a module keeps every launcher correct.
- Test-only harness code is itself test code: it is an inline `#[cfg(test)]` module of the implementation file it instruments.

**Forbidden patterns:**
- A test-only `.rs` file or directory: `*_tests.rs`, `*_tests/`, `cafleet/tests/`, a `#[path]`-mounted test file, or a `#[cfg(test)] mod name;` declaration that loads a separate file.
- A test-only fixture file or fixture directory.
- A test module that exercises another file's implementation.
- A module named `tests` that holds only shared helpers; name it `test_support`.
```

**`CLAUDE.md`**: add this section after `## Commands`.

```markdown
## Tests

Write every Rust test as a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises; never add a test-only file or folder. `.claude/rules/code-quality.md` § Test placement owns the convention.
```

**`docs/docs/contributing.md`**: append this paragraph to `## Rust boundaries`.

```markdown
Every Rust test is a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises. Shared test helpers live in a `#[cfg(test)] pub(crate) mod test_support` beside the type they build, and test inputs stay inline in the test module.
```

### Test relocation map

Paths are relative to `cafleet/src/`. "Split" rows name the target of each group; a test drives its listed entry point. When a row and the rule disagree for a specific test, the rule wins: the test goes to the file that defines the entry point it asserts on. Line-level detail for every row is in `audit/TST.md` § Test placement inventory.

#### `runtime/runner_tests/`

| Source | Target |
|---|---|
| `mod.rs` (`Fixture`, `read_fixture_bytes`, `write_fixture_bytes`) | `runtime/runner/mod.rs` `test_support`. `Fixture::new/artifacts/spec` become `pub(crate)`. `sha256_hex` is replaced by `digest::sha256_hex` |
| `owned_child.rs` | `runtime/system.rs` as `system::test_support::TestChild`, with its unwind test in `system.rs` `tests` (SVC-21). It gains an `identity()` accessor that replaces `writer` `ChildGuard` |
| `creation.rs` | `runtime/runner/creation.rs` tests |
| `fleet_deletion.rs`, `recovery/deletion.rs` | `deletion.rs` tests (the new module from INP-38). The two fake hosts merge into one `test_support::Host` with a `Mutation` enum (TST-11) |
| `native_capture.rs` | `observation.rs` `native_capture::verification` (beside `native_capture_prepare_start_driver`) |
| `control_service.rs` | `runtime/runner/service.rs`: `ServiceFixture` and `Wire` go to `test_support`, the tests to `tests` |
| `control_service/input_service.rs`, `input_service/native.rs` | `service.rs` `test_support` (helpers, pseudo-native child), with the five service-input tests in `service.rs` `tests`. Helpers take their final descriptive names (`with_input_service`, `InputServiceFixture`, `input_service_barrier`, `input_service_records`, `wait_input_service_record`) at the definition (INP-29, SVC-17) |
| `control_service/observation_services.rs` | `service.rs` `test_support` (`run_configured`, `capture`, `count`, `health`). `synthetic` goes to `observation.rs` `test_support` |
| `control_service/observation_contract.rs` | Split. Service-boundary tests (reservation, durable history, started health, sqlite/writer failures) go to `service.rs`. Controller tests (quiet timer, transient, shutdown, unknown coverage, fallback profile) go to `observation.rs`. `private_capture` goes to `observation.rs` `native_capture::tests` |
| `control_service/review_collector.rs`, `review_export.rs` | `observation.rs` `native_capture::{verification, exporter}` tests |
| `control_service/review_failures.rs` | Split between `observation.rs` controller tests and `service.rs`. `hooks` moves with `run_configured` |
| `native_capture/{collection,review_policy,support}.rs` | `observation.rs` `native_capture::tests`, with `support` as `native_capture::test_support` |
| `native_capture/files.rs`, `review_existing.rs` | Split between `native_capture::files` tests and `native_capture::tests`. `ignored_entry_requires_explicit_configuration` goes to `verification` |
| `recovery.rs` (`Case`, `Host`, `Evidence`, …) | `runtime/runner/recovery.rs` `test_support`. `Host` gains optional clock and unlink hook fields, which replace `ClockHost` and `UnlinkHost` (TST-11) |
| `recovery/{identity,review_completion,review_clock,review_relationships}.rs`, `recovery/crash.rs` (ordinary takeover, hard kill) | `recovery.rs` tests. Within `review_relationships.rs`, exceptional branches go to `recovery/exceptional.rs` and finalization after replay goes to `recovery/replay.rs` |
| `recovery/{history,replay,review_descriptor,terminal_tail}.rs` | `recovery/replay.rs` tests |
| `recovery/review_history.rs` | `recovery/exceptional.rs` tests |
| `recovery/missing_paths.rs` | Split between `recovery.rs` and `recovery/exceptional.rs` |
| `recovery/review_descriptor/io.rs` | Split between `artifacts/recovery_descriptor.rs` and `recovery/replay.rs` |
| `recovery/foundations.rs` | Split. Lock-transfer tests go to `artifacts.rs`, pane-guard tests to `input.rs` `tests::pane_lock`, the presentation-pin test to `presentation.rs` |

#### `#[cfg(test)]` module files and test-only data

| Source | Action |
|---|---|
| `runtime/runner/native_capture.rs`, `native_capture/{exporter,files,verification}.rs` | Inline as `#[cfg(test)] pub(crate) mod native_capture { mod exporter {…} mod files {…} mod verification {…} mod tests {…} }` in `observation.rs`. Delete the gate and re-export at `runner/mod.rs`, then re-export through `observation` |
| `native_capture/review_service_tests.rs` | `native_capture::verification` tests. Drop the redundant `#[cfg(test)]` |
| `runtime/runner/process/ownership_diagnostics.rs` | Delete with its hooks and self-tests (SVC-01). The 4 call sites call `self.refresh_owned()` unconditionally. The `step5_refresh_*` behavior tests that drive `refresh_owned_with(&mut Script)` stay |
| `runner_tests/fixtures/synthetic-profile.json` | Delete. Every reader calls `catalog::test_support::synthetic_definitions()`, built from the `profiles.rs` `definitions()` literal (TST-08, AGT-03) |
| `runner_tests/fixtures/claude-2.1.302-blocked-form-blink-1.raw` | Delete. A byte generator in the `runner/input.rs` test produces the same 11 frames (TST-08) |
| `coding_agent/profiles/contract_tests/synthetic-permission-screen.json` | Delete with the test that hashes it (`profiles.rs` test that checks the file's digest and geometry), and the `contract_tests/` folder (AGT-03) |

#### Mis-housed in-file modules

| Source module | Target |
|---|---|
| `coding_agent/profiles.rs` `contract_tests` | `catalog`, `gated_catalog`, `annotation_catalog` (catalog half), `profile_version_fallback` and the five `bundled_*` suites go to `catalog.rs`. `focus_resolution`, `detector` and `annotation_detector_*` go to `detector.rs`. `executable_composer` plan/encode goes to `composer.rs`, with the submission-validation cases to `catalog.rs`. `episodes`, `continuation_episodes` and every tracker test go to `runtime/runner/episodes.rs`. `profiles.rs` keeps only re-exports and `AnswerCapability` |
| `catalog.rs` / `detector.rs` `inspection_tests` | Helpers become `test_support`, tests join `tests`. The `#[cfg(test)] pub(crate) use` chains in `profiles.rs` and `coding_agent/mod.rs` collapse to direct paths |
| `coding_agent/mod.rs` `policy_contract_tests`, `tests::native_version_probe` | `policy.rs` tests, `native_version.rs` tests (AGT-02) |
| `broker/mod.rs` `runtime_contract_tests` | `interaction_refresh`, `versioned_events`, automatic review events, transition/inbox and event-shape tests go to `broker/runtime/events.rs`. `permission_responses` goes to `permissions.rs`, except the presentation-flattening test, which goes to `presentation.rs`. Effective-policy writes and readers go to `effective_policy.rs`. Receipt/input tests go to `inputs.rs`, evidence tests to `evidence.rs`, run/sequence tests to `broker/runtime/mod.rs`. Runtime presentation tests go to `presentation.rs` (JSON) and `output/runtime.rs` (text). The shared fixture goes to `broker/runtime/mod.rs` `test_support`. The `v4_migrations` embed is deleted (BRK-03) |
| `broker/mod.rs` `pub(crate) mod tests` | Split per § Shared test-support homes (BRK-02) |
| `cli/mod.rs` test modules | Launch policy, press parsing and recovery entry go to `member.rs`. Permission entry and event/input read and reconcile go to `runtime.rs`. Setup parser rejection goes to `setup.rs`. Help, `__member-run`, option scoping and dispatch-level storage tests stay in `mod.rs` after the production code. All use one `#[cfg(test)] pub(super) fn parse_cli` (CLI-02) |
| `listen.rs` snapshot tests | `broker/messaging.rs` tests. `listen_snapshot_with_clock` becomes private (MUX-05) |
| `multiplexer/mod.rs` PressKeys grammar test | `multiplexer/keys.rs` tests. The rest of `mod.rs` keeps one bottom `mod tests` (MUX-04) |
| `input_client.rs` external finite-wait delivery test | `delivery.rs` tests, reusing `wrapped_line`/`preview_stamp` (INP-23) |
| `input_clock.rs` prepared-submission tests | `input_client.rs` tests. `ScriptClock` moves to `input_clock::test_support` (shared by `input_clock.rs`, `input_client.rs` and `service.rs` tests, INP-27) |
| `input_protocol/wire.rs` reply tests | Merged with the codec into `input_protocol.rs` (INP-02) |
| `transport.rs` transient-screen test | `terminal.rs` tests (SVC-22) |
| `runner/health.rs` registry-show test | `presentation.rs` tests (OBS-23) |
| `system.rs` nested `press_tests` | Flattened into `system.rs` tests (SVC-16) |
| Files with several or mid-file test modules (`runner/input.rs`, `input_client.rs`, `input.rs`, `observation.rs`, `lifecycle.rs`, `codec.rs`, `db/mod.rs`, `delivery.rs` `notification_line_tests`) | One `mod tests` at the end of the file. Former modules become nested modules. Test support comes first, and visibility narrows to `pub(super)` (INP-07, INP-24, INP-46, OBS-22, SVC-03, SVC-11, BRK §db, MUX-13) |

#### Shared test-support homes

Each helper below has exactly one definition. The Form column gives its concrete form, following § Test placement rule:

- `test_support`: an item in the listed file's `#[cfg(test)] pub(crate) mod test_support`.
- `impl`: a `#[cfg(test)] impl <Type>` block beside the type in the listed file.
- `tests`: a helper used only by that file's tests, inside its `mod tests`.
- `macro`: a crate-level `#[cfg(test)]` macro at the top of `lib.rs`.

Delete every copy the Replaces column names.

| Helper | Home | Form | Replaces (source finding) |
|---|---|---|---|
| `Fixture`, fixture byte IO, `write_all_until` | `runtime/runner/mod.rs` | `test_support` | `runner_tests/mod.rs`; write-all loops in `input_service.rs` (INP-27) |
| `ServiceFixture` (with `resize(rows, cols)`), input-service helpers, `kinds()` | `runtime/runner/service.rs` | `test_support` | resize and event-kind copies (TST-12) |
| `TestChild` | `runtime/system.rs` | `test_support` | `runner_tests::owned_child::OwnedChild`, `writer` `ChildGuard`, `observation.rs` `Native` guard (OBS-22) |
| Recovery `Case` (with `recover_with`, `recover_held`), `Host` | `runtime/runner/recovery.rs` | `test_support` | `ClockHost`, `UnlinkHost`, recovery invocation wrappers (TST-11, TST-12) |
| `ManualObservationClock` | `runtime/runner/observation.rs` | `test_support` | three deterministic clocks (OBS-04). Copies of real clocks use `RealObservationClock` / `RealInputDeadlineClock` directly (TST-07, INP-27) |
| `LiveCase::started()` | `runtime/runner/observation.rs` | `tests` | eight repeated live setups within `observation.rs` (OBS-22) |
| `ScriptClock` | `runtime/runner/input_clock.rs` | `test_support` | `service.rs` `Clock::at` (INP-27) |
| `independent_frame(body)` | `runtime/runner/codec.rs` | `test_support` | nine length-prefix framers (SVC-05, TST-12) |
| `private_write` | `runtime/runner/artifacts.rs` | `test_support` | three copies (TST-12) |
| Transport fakes (`ByteInput`/`PtyIo`/`Display`) | `runtime/runner/transport.rs` | `test_support` | five fake sets (SVC-02) |
| `SLOW_SUM`/`FAST_SUM` probes | `runtime/runner/lifecycle.rs` | `test_support` | seven CTE literals in `lifecycle.rs` and `outbox.rs` (SVC-11) |
| Writer `Hooks`, `Release`, `wait_until_within`, `WriterFixture` | `runtime/runner/writer.rs` | `test_support` | polling loops `wait_path`, `wait_until` (TST-12). Writer `form` becomes `catalog::inspection_fixture_form` (SVC-15) |
| `RunnerFixture::new(catalog, profile, limits, until)`, `PanicTokens`, `pending_snapshot` | `runtime/runner/input.rs` | `tests` | `manual_progress` `Harness` (INP-06), INP-31, INP-36 |
| `HostProfile::metadata()` | `runtime/runner/input.rs` | `impl` | four `HostProfile::synthetic(EvidencePolicy::metadata(), "metadata")` calls (INP-36) |
| `synthetic_definitions()`, `synthetic_catalog()`, `bundled_fixture(id)`, `bundled_embedded()`, `bundled_definitions()`, `entry()`, `guarded_fixture(…)` | `coding_agent/profiles/catalog.rs` | `test_support` | JSON fixture, `definitions()`, five `FIXTURES` tables, `native!`, `embedded()`, lookups (TST-08, AGT-04, AGT-05) |
| Screen builders, `replay_native_fixture`, `bundled_screen`, `bundled_observe` | `coding_agent/profiles/detector.rs` | `test_support` | `native_screen_bytes` and per-module copies (INP-36, AGT-05) |
| `tracker()`, `open()`, `Run`, `Held` | `runtime/runner/episodes.rs` | `test_support` | AGT-05 |
| `migrated_conn`, `migrate_to_version`, `table_rows`, `wal_snapshot` | `db/mod.rs` | `test_support` | `broker::tests::migrated_conn`, six row dumps, five table snapshots, four migrate-to-version fixtures (BRK-03, TST-10, INP-13, CLI-03) |
| `create_fleet_fixture`; `placement`, `register` | `broker/fleets.rs`; `broker/members.rs` | `test_support` | `broker::tests`, `delivery.rs` copies (BRK-02, MUX-12) |
| Broker runtime `Fixture`, evidence builders, `assert_key_order` | `broker/runtime/mod.rs` | `test_support` | BRK-01 |
| `MessageRecord::fixture(id, from, to, text)` | `broker/records.rs` | `impl` | three literals (MUX-14) |
| `FixtureDeletionHost`, `guarded_delete_fleet`, deletion `Host` | `deletion.rs` | `test_support` | `broker::tests`, two fakes (BRK-02, TST-11) |
| `Settings::for_database(path, multiplexer)` (built through `from_lookup`) | `config.rs` | `impl` | 22 struct literals and 7 `from_lookup` wrappers (CLI-05, INP-14) |
| `TestClock` (implements `receive::Clock`, with `advance` and a sleep log) | `receive.rs` | `test_support` | `listen.rs`/`receive.rs` copies (MUX-14) |
| Fake command runner (current `recording`), one `argv` helper | `multiplexer/mod.rs` | `test_support` | TST-13 rename; three `argv` helpers (MUX-04) |
| `assert_no_debug_or_serialize!`, `test_path!` | `lib.rs` | `macro` | six `AmbiguousIfImplemented` copies (AGT-20, INP-36). Hard-coded child paths (TST-04) |

Module naming follows the rule: modules that hold only helpers are named `test_support` and test modules are named `tests`. This applies to `broker::tests`, `writer::contract_tests::{support,worker}`, `input_recovery::tests`, `input_client::permission_history`, `catalog::inspection_tests` and `detector::inspection_tests` (TST-13).

#### Re-executed child tests

Define `test_path!` and `assert_no_debug_or_serialize!` at the top of `lib.rs`, above the first `mod` declaration. `macro_rules!` is textually scoped, so only modules declared after the definition can invoke it. Precede the pair with one comment line stating that reason:

```rust
// Defined before every `mod` declaration: `macro_rules!` is visible only to modules declared after it.
#[cfg(test)]
macro_rules! test_path {
    ($name:ident) => {{
        let _: fn() = $name;
        concat!(module_path!(), "::", stringify!($name))
            .strip_prefix("cafleet::")
            .expect("test_path! names a test inside the cafleet crate")
    }};
}
```

Beside each ignored re-exec child (`runner_review_service_child`, `runner_collector_resize_child`, `runner_collector_export_child`, `native_capture_service_child`, `runner_recovery_owned_process_child`, and the two children launched from `cli/member.rs`), define `pub(crate) fn <child>_path() -> &'static str { test_path!(<child>) }`. Every launcher passes that value to `--exact` instead of a string literal, so the duplicated native-capture child literal collapses to one item. Keep the `DRIVER_NEUTRAL_ROOT`/`DRIVER_PRIVATE_ROOT` values (`/tmp/cafleet-194*`) unchanged; committed native fixtures cite them.

### Test-change ledger

The implementer keeps evidence under `<this folder>/.verification/` (dot-prefixed, so it is gitignored agent scratch):

1. Before any edit (the working tree equals the base commit), capture the baseline:
   1. Run `mise //cafleet:test -- -- --list`, which runs `cargo test --lib -- --list` and lists `#[ignore]` re-exec children too; keep them in the multiset. Save the output with the file writer (never a shell redirect) as `.verification/tests-before.txt`.
   2. Run `mise //cafleet:build`. Invoke `cafleet/target/release/cafleet --help`, then `cafleet/target/release/cafleet <subcommand> --help` for every subcommand that help lists, recursing into nested subcommands. Save the concatenated outputs in that order as `.verification/help-before.txt`. The installed `cafleet` stays untouched, preserving any active fleet's runtime.
2. Every step that adds, deletes, merges or renames a test appends one row per test to `.verification/test-ledger.md`:

   ```markdown
   | Step | Kind (added / deleted / merged / renamed / kept-after-verification) | Old leaf name | New leaf name or surviving test | Finding ID | Reason (unique assertions moved where; for added, the deleted or merged tests it absorbs) |
   ```

3. After the last code step, record `tests-after.txt` and `help-after.txt` with the same procedure.
4. Reconcile on leaf names (the last `::` segment), because relocation changes every module path. The multiset of leaf names in `tests-after.txt` must equal `tests-before.txt` minus deleted and merged names, with renamed names substituted and added names included. `help-after.txt` must be byte-identical to `help-before.txt`.

### Verify-then-act protocol

This protocol applies to every finding this document marks **(verify)**, and to any adopted finding whose premise the implementer cannot confirm:

1. Confirm the premise with evidence: compiler output, a grep over `cafleet/src` of callers or asserted strings, `git log -S`, or a diff of the asserted state between the two tests.
2. If the premise holds, act. Record a deleted test only when every one of its unique assertions already exists in, or was moved into, the surviving test, and name that test in the ledger.
3. If the premise fails, keep the code unchanged and record `kept-after-verification` with the evidence in the ledger.

To prove a check unreachable (D1), show that every production path into it passes an earlier check that raises first. A removal keeps the first-failing error identical for every input.

### Documentation model

| Audience | Home | Content |
|---|---|---|
| Everything (contract) | `SPEC.md` | Every contract: CLI, output, errors, environment, schema, runtime, multiplexer, backends, presets, wire |
| Users | `docs/` site | Concepts, usage, setup and configuration steps a user performs. Exact contract detail is a link to SPEC.md |
| Agents | `skills/` | Necessary-and-sufficient instructions to drive cafleet; command facts in `skills/cafleet/reference/cli.md` |

Skills never link `docs/` or SPEC.md; they carry the facts they need. SPEC.md never links `docs/`.

Anchors follow GitHub's heading-slug rule, because GitHub ignores `{#custom-id}` heading attributes. The rule: lowercase the heading text, drop every character other than letters, digits, spaces, hyphens and underscores, replace each space with a hyphen, and append `-1`, `-2`, … to repeated slugs.

| Reference | Form | Verification |
|---|---|---|
| Docs page → SPEC.md | `https://github.com/himkt/cafleet/blob/main/SPEC.md#<slug>` | Open the rendered SPEC.md on the PR branch and follow each link from the docs pages; the docs dead-link check covers only internal links |
| SPEC.md → SPEC.md | `§<number> <heading>` text, no anchor | The named section exists with that number and heading |
| Skill → `cli.md` | `cli.md#<slug>` | Each link's slug matches a `cli.md` heading under the rule |

During consolidation, delete every `{#…}` suffix from SPEC.md headings, so the visible heading text is the slug source.

#### SPEC.md consolidation

Absorb each docs spec page into its SPEC section, then delete the page:

| Docs page | SPEC section |
|---|---|
| `cli-options.md` | §6.3 CLI |
| `message-envelope.md` | §6.4 Output & Formatting |
| `multiplexer-backends.md` | §6.5 Multiplexer |
| `coding-agent-backends.md` | §6.7 Coding-agent backends (contract parts; setup steps go to docs, § Docs site) |
| `member-runtime.md` | §6.9 Member PTY runtime |
| `data-model.md` | §6.1 Persistence & Schema and §8 Database schema |

For each page:

1. Diff the page against its SPEC section.
2. Add every contract statement present only in the page to the section, after verifying it against the code. This covers, for example, the `operation_rows` sentence (`message-envelope.md:64`), the "in array order" rule, the coding-agent-backends model-selection, reasoning-effort, spawn-argv, Claude-profile-entries, spawn-time-allow-rules, minimum-rule-set, safety-floor-caveats and role-requirements contracts, and the data-model member and query/activity contracts.
3. Replace each of SPEC's 52 links into the page with an internal `§<number> <heading>` reference.
4. Delete the page.

SPEC keeps its numbered section structure. SPEC.md:3 ("self-contained") stays true.

Within SPEC, correct stale content to match the shipped binary:

| ID | Correction |
|---|---|
| SPC-02 | Delete every `fleet capture` mention, including the `Bash(cafleet fleet capture *)` coverage row. The shipped `FleetCommand` has only `Create`, `List`, `Show` and `Delete` |
| SPC-03 | Replace the `doctor` text layout, the sections, the Member permissions section, the footer and the nested JSON with the check-row model, built from `cli/doctor.rs`. It covers check names (`<agent>.configuration`/`.executable`/`.installation`, `opencode.preset[.<mode>]`, `member_runtime.<id>`, database and multiplexer rows), status values (version mismatch is `info`), the `"{status} {name}: {detail}"` line with newline escaping, JSON `{"checks":[{name,status,detail}]}`, and the `doctor found errors` exit rule. Keep the OpenCode preset and member runtime subsections |
| SPC-04 | Delete the stale-assets guard section, its error rows, the "schema/assets guards" wording and its lazy-validation paragraph, including §7.1's guard sentence |
| SPC-05 | Setup output becomes `Applied migrations to head ({head}) at {path}.` / `Already at head ({head}).`, with the shipped stop-at-first-failing-agent order. Delete the `cafleet-research` cleanup sentence and the dead error row |
| SPC-06 | `member create` errors become `Director (member {id}) not found.` and `Director (member {id}) has no placement; supply --coding-agent.`. Rewrite the compensation strings and order from `runtime/runner/creation.rs` **(verify)** the order table. Add `fleet {id}'s root Director (member {d}) has no concrete placement.` |
| SPC-10 | Replace the Python-era tmux/herdr method surfaces with a language-neutral operation / tmux argv / herdr argv / failure-handling table built from `tmux.rs`/`herdr.rs` (capture argv `-p -e`). The error taxonomy becomes one `MultiplexerError`, and `MultiplexerContext` becomes a plain value. Fix the dead "Creation ownership and compensation" reference. Delete the paragraphs that restate §6.5 Literal input and §6.6 |
| SPC-15 | §8 gains a two-line V2/V3 note (`messages.preview_started_at` added, then dropped). Headings become `V4 member runtime migration`, `V5 permission response migration` and `V6 automatic review migration`. The data-model wording names V4 |
| SPC-19 | §6.7 Interface: correct the method count, use behavior names instead of the absent `CODING_AGENTS`/`EFFORT_LEVELS`, add `SpawnProbe.home_dir`, state that the config dir is resolved before the PATH check, and turn the one-row table into a sentence |
| SPC-21 | Rewrite code identifiers (`split_window`, `PaneCleanup::Attempted`, `CaptureSnapshot::from_raw`, `register_member`, …) and change narration ("stay unchanged", "No new … contract") as behavior. Replace fixed counts with properties. Delete the agent follow-up rule that belongs to `member.md` |
| SPC-22 | Delete §9's process-advice paragraph, and scope "Escape-first" to unwrapped members. Reduce §7.7 to one sentence, §10 to its pointer and §11 to its statement without the index table. §6.2 Broker errors links the error table |

Within SPEC, give each repeated fact one owner:

| ID | Action |
|---|---|
| SPC-07 | Replace the three verbatim preset/rules copies with path-as-contract references to `presets/opencode/cafleet.md`, `presets/opencode/cafleet-ask.md` and `presets/codex/cafleet.rules`. Keep the format description and newline convention |
| SPC-08 | §6.7 keeps each backend fact once after absorbing `coding-agent-backends.md`. The opencode coordination sentence and the `cafleet-ask` cell follow the shipped presets |
| SPC-09 | The V5 DDL appears only in §8. §6.9 links it |
| SPC-11, SPC-12 | The runtime CLI presentation layouts live in §6.3. §6.9 keeps 2–3 sentences and a reference. Each repeated rule inside §6.9 is stated once |
| SPC-13 | Schema compatibility and the application identity live in §8 (and the §6.3 schema-version guard for errors). Other mentions are a reference |
| SPC-20 | Preview and delivery prose lives in §6.5 Push notifications. §6.4 and §8 keep column semantics |
| SPC-21 | The §6.3 Error Messages table keeps only rows not owned by a command section, and the schema-guard rows collapse into one. Delete the second capture-hash paragraph and the config-dir prose that restates its table |

#### Docs site

- Delete `docs/docs/spec/` and its `_meta.json` entry (`docs/docs/_meta.json` `spec`).
- Move setup steps from `coding-agent-backends.md` into `quickstart.md` and `concepts/coding-agents.md`: the permission profiles a user installs, codex rules and OpenCode preset installation, and model/effort choice guidance. Copy-paste configuration a user installs stays exact.
- Rewrite every internal link into `spec/` (concept, how-to, quickstart and contributing pages) as a self-contained user explanation, or as a SPEC.md URL where the reader needs the exact contract.
- `docs/docs/contributing.md` links SPEC.md as the specification for the CLI, runtime and storage contracts. It also gains the test-placement paragraph and lists `test` and `lint` as the required Rust validation (SKL-20); line 70 links SPEC §6.9 Native acceptance and coverage.
- README "Specification" links `SPEC.md`. `CONTRIBUTING.md` reads "The contributor guide lives at <https://himkt.github.io/cafleet/contributing> (source: `docs/docs/contributing.md`)." (SKL-26)
- Stale and duplicated user-page content:

| ID | Action |
|---|---|
| SPC-02 | Delete `fleet capture` from `concepts/member-lifecycle.md` and `how-to/design-doc-development.md` |
| SPC-04, SPC-14 | `storage.md` says doctor reports the recorded asset version. Delete its shape-caching sentences |
| SPC-16 | The concepts automatic-review table becomes one clause and a link. Exact launch-policy defaults appear only in SPEC |
| SPC-17 | Replace the "Known asymmetries" section with a one-clause summary. Reduce "One-shot command isolation" to two operator sentences |
| SPC-18 | Add `Bash(cafleet member press *)` to both quickstart permission profiles and the table row. Describe the codex rules by property ("Director-owned commands prompt"). Trim the restated rationale |
| SPC-23 | The Director reception lifecycle lives in `how-to/mixed-backend-team.md` § Manual lifecycle. Other pages keep one clause and a link; `member-lifecycle.md` says "members confirm readiness, then poll" |
| SKL-21 | `docs/theme/styles.css`: define the GitHub mask data URL once as a custom property. Align the `vw`/`%` width or state the reason **(verify)** visually |

#### Skill bundle

Delete `skills/cafleet/reference/runtime/` entirely and `cafleet/build.rs`'s `cargo:rerun-if-changed=../docs/docs` line (CLI-15). The only concepts link skills carry (`skills/cafleet/roles/director.md`, pane-discovery paragraph → `runtime/concepts/member-lifecycle.md`) is retargeted to `cli.md#member-create`. The user chose this over keeping that page, which would re-bundle the user pages it links transitively (`how-to/mixed-backend-team.md`, `concepts/member-permission-events.md`, `quickstart.md`, `concepts/coding-agents.md`).

##### Agent CLI reference

`skills/cafleet/reference/cli.md` contains, for every `cafleet` command, flag, output field, error string and permission pattern that a shipped skill tells an agent to run, parse or act on, the exact fact copied from SPEC.md. It contains nothing else. It replaces the anchors skills use today, with these sections:

| Section | Replaces |
|---|---|
| Subcommand summary, positional subject ids, environment variables | `cli-options.md#subcommand-summary`, `#positional-subject-ids`, `#environment-variables` |
| Output shapes and `--json` (including member list output) | `#output-shapes`, `#json-output`, `#member-list-output` |
| `permissions.allow` coverage | `#permissionsallow-coverage` |
| `message send` / `poll` / `wait` / `listen` / `ack` / `show`, and the push-notification line with its accepted trade-offs | `#message-send`, `#message-wait`, `#message-listen`, `multiplexer-backends.md#push-notifications`, `#accepted-trade-offs` |
| `member create` (including compensation and pane cleanup) / `delete` / `list` / `show` / `prompt` / `exec` / `press` / `capture` | `#member-create`, `#member-delete`, `#member-exec`, `#member-press`, `concepts/member-lifecycle.md` |
| Pane input and previews: the pane input lock (busy/exclusion outcomes), explicit key dispatch, prompt dispatch and the preview forms an agent sees in its pane | `multiplexer-backends.md` (whole page, `SKILL.md` "Pane input, exclusion and previews") |
| Member runtime command facts: `profile_match` values, runtime event kinds and the automatic-review event payload fields and limits, `event show` / `input show` / `input reconcile`, `permission inspect` / `permission respond` flags and output, receipt and operation-key fields, `member recover`, and the receipt-ownership facts behind `input reconcile` | `#permission-commands`, `member-runtime.md` (whole page, `SKILL.md` runtime row and `reference/member-runtime.md`), `#automatic-review-events`, `#input-arbitration-and-the-original-native-answer` |
| `setup` and `doctor` | `#cafleet-setup` |
| Error messages agents handle | `#error-messages` |

`cli.md` carries no procedure table that SKL-12 assigns elsewhere. The automatic-review event-handling table stays in `skills/cafleet/reference/member-runtime.md`, and the launch-policy defaults table stays in `skills/cafleet/roles/director.md`. Both link `cli.md` for the payload and command facts above.

Every existing link in `skills/` that targets the mirror is rewritten to the matching `cli.md` anchor. The SKILL.md command-index row "Read-only roster pane snapshot → Fleet capture" is deleted with the rest of SPC-02 (`SKILL.md`, `supervision.md`, `director.md` fleet-capture mentions).

#### Rules and maintainer skills for the documentation model

| File | Change |
|---|---|
| `.claude/rules/documentation-maintenance.md` | Rewrite the first-class targets to the three-audience model. SPEC.md owns every contract and is updated first. `docs/` explains concepts and usage for users and links SPEC for exact contracts. Skills carry the agent subset, with `skills/cafleet/reference/cli.md` for command facts. The `cli.md` target row's trigger is "any change to a SPEC fact that `cli.md` carries", updated in the same change; its drift consequence is that agents run stale commands, and drift blocks "documentation complete". README is the thin entry point. Implementation Order becomes SPEC.md → `docs/` → README → skills → rules. Delete the `docs/docs/spec/*` exemption row and describe path-as-contract mentions as SPEC content |
| `.claude/rules/coding-agent-overlay.md` | Replace "Two independent homes" with the three homes: SPEC §6.7 owns backend contracts, `skills/cafleet/reference/coding-agents.md` is the agent overlay, `docs/docs/concepts/coding-agents.md` is the user explanation. State that backend-specific `####` subsections may follow Note → applies at, and that the overlay comes first in the Required-reading block, table row or prose (SKL-25). Reduce the restated reader contract to author-facing requirements plus a link (SKL-08) |
| `.claude/skills/update-readme/SKILL.md` | Reconcile README with the docs site, and SPEC.md with the shipped binary and code. SPEC owns every contract; drop "embedded runtime mirrors" |
| `.claude/rules/database-migrations.md` | The chain guard derives the registry from the embedded migrations and asserts contiguous numbering from one baseline. Add a successor test for each new `V<N>__<slug>.sql` (BRK-19) |
| `.claude/rules/commands.md` | Required Rust validation is `mise //cafleet:test` and `mise //cafleet:lint`. `typecheck` stays a registry task (SKL-20) |
| `.claude/skills/clean-docs/reference/review-format.md` | The preserved-surface list names SPEC.md and `skills/cafleet/reference/cli.md` in place of "`docs/docs/spec/` contract pages" |

### Skill and maintainer-surface cleanup

| ID | Action |
|---|---|
| SKL-01 | Add `.claude/skills/clean-docs/roles/scanner.md` and `roles/reviewer.md` holding the shared startup, coordination, authority, apply-gate and shutdown text. Each workflow body (`residue.md`, `affirmative.md`, `simplification.md`) gains "Scanner obligations" / "Reviewer checks" carrying its role-specific text verbatim. Delete the six per-workflow role files |
| SKL-02 | Add `## Member startup` to `skills/cafleet-design-doc/SKILL.md` with the five-row read table and the loader/routing paragraph. Each of the seven role files replaces its block with one linking sentence and drops the repeated "Load the listed skills", "Broker protocol" and `## Shutdown` lines |
| SKL-03 | Condense `interview.md` Steps 0 and 2a–2e to pointers. Merge Prerequisites into Step 0, reword "Shared spawn deltas" for the single Analyzer role, and keep the Analyzer delta, progress table, loop and `question.md` format |
| SKL-04, SKL-05 | `clean-docs/SKILL.md`: delete "Skill file layout" and "Backend-neutrality". State the `${BASE}` convention once. Step 7 names "the repository's Rust validation tasks (`.claude/rules/commands.md`)". Replace the private 40-line spawn skeleton with a per-role delta table on the canonical frame |
| SKL-06 | `supervision.md` § Director wake is the only home of the wake loop. Delete "Team-facilitation instructions" (keep "Routing member command requests"), fold Stage 1 into one linking sentence, and replace the paraphrases in `director.md`, `interview.md`, `clean-docs/SKILL.md`, `skill-author/SKILL.md`, `create.md` and `execute.md` with the link |
| SKL-07 | `director.md`: delete Cross-references, the restated `--file` line and the double blank line. The model list becomes "Pass a token exactly as listed in the selected backend's Model catalog" plus the deregistration delta. Member Capture links `cli.md`. Director-only primitives become one sentence linking prompt-routing |
| SKL-08 | Keep the overlay resolution procedure in `skills/cafleet/SKILL.md` § Resolve your overlay. Reduce the preamble of `skills/cafleet/reference/coding-agents.md` to the selector table and the freshness/price notes. Delete the "Ownership boundary" section of `.claude/skills/cafleet-model-list-refresh/SKILL.md`. The `.claude/rules/coding-agent-overlay.md` part is in § Rules and maintainer skills for the documentation model |
| SKL-09 | Keep each obligation once in the design-doc role files (Accountability for the rule, Workflow for the step). Delete Drafter "COMMENT Processing" and design-doc `SKILL.md:53` |
| SKL-10 | `coordination.md`: delete Core Principle, reduce Finalize-Time Cleanup to the source-file marker fact, shorten the `--json` line to a link, and remove the per-file recovery re-explanations from `programmer.md`/`tester.md` |
| SKL-11 | `prompt-routing.md` § Member-side owns the routed-request evidence fields (exact command, absolute cwd, required environment, refusal). `member-runtime.md` step 5 owns the mode-dependent permission decision. Other sites keep a link plus at most one clause, and each pointer stays on the Director and member reading paths |
| SKL-12 | The automatic-review event table lives in `skills/cafleet/reference/member-runtime.md`; `director.md` links it. The launch-policy table lives in `director.md`; `member-runtime.md` links it. Neither table is copied into `cli.md`, which carries only the event payload and command facts (§ Agent CLI reference) |
| SKL-13 | `.github/workflows/ci.yml`: one job with `name: ${{ matrix.task }}` (so the check names stay `lint` and `test`), `strategy.matrix.task: [lint, test]`, cache key `${{ runner.os }}-cargo-${{ matrix.task }}-…` and `run: mise //cafleet:${{ matrix.task }}`. Pin `actions/cache` and the `renovate.yaml` actions by SHA |
| SKL-14, SKL-15 | The user-relay rule lives in `SKILL.md` § Soliciting user reactions and in `supervision.md` § User Delegation Protocol. Delete supervision's duplicate, Communication Model restatements and Cleanup Protocol (retarget inbound links to § Shutdown). Keep only the Director-specific sender follow-up clause |
| SKL-16, SKL-17 | Per-role delta tables (§ Spawn delta tables). In `skills/cafleet/roles/director.md`, delete the three paragraphs that restate the skeleton's fixed lines. They begin "Use an available non-shell text reader for prerequisite files.", "The ready-signal line is part of the **fixed frame**" and "Retain Bash hygiene and member-authority obligations" |
| SKL-18 | Delete the "**Backtick caveat (harness-dependent)**" paragraph in `skills/cafleet/roles/director.md`. `skills/cafleet-design-doc/execute/roles/verifier.md` uses "the host-permitted HTTP/browser tools". `skills/cafleet-design-doc/execute/roles/reviewer.md` checks "the host project's code-quality rules and validation tasks" |
| SKL-19 | `.gitignore`: delete lines 3, 5, 6 and 15 **(verify)** with `git check-ignore -v --no-index CLAUDE.md .claude/skills/x .claude/settings.json .prompts/x`, which must give identical results before and after |
| SKL-22 | Neutral wording in base files. `skills/cafleet/roles/director.md`: "via the Bash tool" becomes "your shell tool", and "in the Director's own tmux window" becomes "the Director's multiplexer window". `skills/cafleet-design-doc/interview/interview.md`: "Use `Edit`" / "Use `Grep`" become "edit the document" / "search the codebase". `skills/cafleet-design-doc/execute/roles/programmer.md`: "Use Grep" becomes "search the codebase". `skills/cafleet-design-doc/execute/roles/verifier.md`: the `mcp__*` prefix and "the system-reminder" wording become "inventory available tools, MCP servers and skills" |
| SKL-23 | Reference host rules by name ("the host's Affirmative Writing and Removal rules", "the host's Bash hygiene rule"), not `~/.claude/rules/*.md` paths. This covers the IMPORTANT line in `.claude/skills/clean-docs/SKILL.md`, `clean-docs/residue/residue.md`, `clean-docs/affirmative/affirmative.md` (including the text that SKL-01 moves there from the affirmative reviewer role) and the opening sentence of `.claude/rules/git-workflow.md` **(verify)** with `ls ~/.claude/rules` that the maintainer host ships no split rule files |
| SKL-24 | Fix the broken references: stall ladder → `supervision.md#stall-response`, interview § Coordination Protocol, and "your backend's decision-surface note" |
| CLI-16 | Delete `cafleet/rustfmt.toml` **(verify)** that `mise //cafleet:lint` stays clean |

#### Spawn delta tables

`create.md` (Step 1) and `execute.md` (Step 3) each gain a per-role delta table in the `interview.md` shape: Slot | role columns, with the rows ROLE TITLE / TEAM, role-file, EXTRA SKILL LOADS, CONTEXT LINES, IMPORTANT / ROLE-CONSTRAINT LINES (verbatim) and start cue (verbatim). The tables cover Drafter and Reviewer in create, and Programmer, Tester, Verifier and Reviewer in execute. Each role's IMPORTANT lines are its hard constraints from its role file, and every role carries the poll-handling line. The Drafter carries `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.`, `Send your questions to the Director who will relay them to the user.` and `Do NOT create any design document file until you have received answers.`. The execute Reviewer's CONTEXT LINES include `BASE BRANCH: [INSERT base branch name]`.

`director.md` § Canonical spawn-prompt skeleton names these `‹…›` markers **spawn-skeleton slots**. It states that they are filled from the consuming workflow's per-role delta table, and that they are distinct from the backend **runtime placeholders** (`{decision_surface}`, `{permission_flags}`, …) resolved from `coding-agents.md` and from the four CLI identity placeholders. The ‹CONTEXT LINES› row describes its members by property ("the role inputs the consuming workflow's delta table lists") instead of listing labels. The lossless rule points at those delta tables.

### Production simplification catalog

Each row is adopted; **(verify)** rows follow § Verify-then-act protocol. Evidence and line locations are in the named audit file. Every change keeps the § Contract invariants.

#### Crate-wide

| ID | Change |
|---|---|
| CLI-01, INP-45, MUX-19 | `lib.rs` keeps `pub mod cli` and `pub mod error`; every other module becomes `pub(crate)`, and `cli`'s submodules become private. Inside modules, `pub` becomes `pub(crate)` or narrower. Rename the read fields `_file`, `_guard` and `_guards` in `input.rs`. Make `resolve_multiplexer_name`, `recorded_version` and `system::{find_on_path, find_executable_in}` private |
| Sweep | After the visibility change, classify each item `mise //cafleet:lint` reports as dead (clippy `--all-targets` builds both the normal and the test configuration). An item with no caller in either build is deleted. An item whose only callers are tests becomes `#[cfg(test)]` beside its production variant, or moves into the owning `test_support`. A test seam this document keeps stays production. Record each item and its outcome (deleted / test-gated / kept as seam) in a "Dead-code sweep" table in `.verification/test-ledger.md`. Then delete the remaining `#[allow(dead_code)]`/`#[expect(dead_code)]` attributes |
| OBS-07, CLI-10, BRK-12, MUX-18, TST-09, SVC-23 | Add `cafleet/src/digest.rs` (`pub(crate)`) with `hex(&[u8])`, `sha256_hex(&[u8])`, `sha256_file_hex(&Path)` and `canonical_json(&Value)`. Replace every `format!("{byte:02x}")` site, `catalog::sha256_hex`, `driver_digest`, `files::file_digest`, `evidence::canonical` and the one-level sort in `permission_response_digest`. The existing independent-digest tests guard byte equality |
| CLI-10 | `runtime/system.rs::random_hex32()` replaces the two `/dev/urandom` reads in `cli/member.rs` and `SystemRequestTokens::fill`. Callers keep their error messages |
| BRK-04 | `impl From<rusqlite::Error> for CafleetError` in `error.rs`, with the exact `db_err` body. Replace only the sites that call `db_err` with `?`. Custom mappings stay explicit `map_err` calls. Delete `db_err` |
| CLI-07 | `time::now_stamp()` replaces `now_utc()` + `format_utc` pairs. The raw-instant sites call `Utc::now()` |
| INP-43 | `CafleetError::merge_cleanup(result, cleanup)` replaces the hand-written primary/cleanup merges. `combine_manual_progress` stays |
| BRK-07 | One `stored_enum!` (today's `runtime_enum!`) in `broker/records.rs` declares every stored enum |
| CLI-13, AGT-15 | `Visibility::as_str()` feeds the formatter. `ObservationKind::parse` delegates to `InteractionClass::try_from` |

#### Runtime runner

| ID | Change |
|---|---|
| OBS-02 | Delete `ScreenSnapshot.contents` (tests read `snapshot.screen.contents()`). Make `title`, its callback capture and `new_with_callbacks` `#[cfg(test)]`, and delete `#[allow(dead_code)]` |
| OBS-03, OBS-05 | `ObservationClockKind` and `kind()` become `#[cfg(test)]`, and the tautological `#[cfg(not(test))]` ensure is deleted. Delete the `ObservationSite` alias |
| OBS-06 | `broker::runtime::evidence::run_metadata(run, rule_id, screen_seq)` builds the 11-key lifecycle evidence for the controller, replay and tests |
| OBS-08…11, OBS-13 | Harness internals: one private publisher over `files::publish_file`, one `CaptureState::clip`, `publication_permit` via `operation_permit`, one bounded writer, patterns compiled once in `apply_limits` (delete the compiler trait and `from_launch_with_compiler` **(verify)** each `review_policy` case is expressible through limits), and `files.rs` uses `artifacts::private_metadata` |
| OBS-12 | One `DESCRIPTOR_LIMIT` and `LaunchDescriptor::parse` shared by the loader and the recovery pin; the two identity predicates stay |
| OBS-14…19 | `count_commit` replaces `count`/`count_exceptional`; `EpisodeTracker::clear_boundaries` and `earliest()`; `EpisodeUpdate::new`; `reset_observation` returns `Result<()>`; a `step` helper in `feed_observed_classified`; `supply` uses `exhausted()`; `#[derive(Default)]` on `EventSequencePool` |
| OBS-20 | Validate the health diagnostic against `ObservationFailure` codes, and drop `observation_gap` **(verify)** with `git log -S observation_gap -- cafleet/src` that no shipped runner emitted it |
| OBS-24 | `recover_member` calls `recover(…, &mut NativeRecoveryHost)` directly. The host-injecting entry becomes `#[cfg(test)]` |
| SVC-06 | `RunnerDependencies.writer_hooks: Arc<dyn WriterHooks>` (production passes `InertWriterHooks`). Delete `RunnerTestServices`, `run_with_test_services` and the writer start branch. Expose `RunnerDependencies` and `run_service` `#[cfg(test)] pub(crate)`. `EventWriter::start` becomes `#[cfg(test)]` once production stops calling it |
| SVC-07 | `run_service`: `quiesce(…)`, `observe_exit(…)`, `client.reply_input(result.as_ref())`, one placement read in the capture path, and per-request handlers taking a context struct |
| SVC-08 | `ShutdownOutcome::new(native_exit, diagnostics)` / `::complete(native_exit)` at every literal site |
| SVC-09 | `Outbox`: `open_private` shared by the recovery and owned open paths, `validate` via `validate_with`, `from_io` deleted (tests call `initialize`) |
| SVC-12 | `reconcile_exit(outcome, exit, conflict)`; `ShutdownHost` methods become required |
| SVC-13 | `check_deadline` moves to `runner/mod.rs`; delete `drain::before_deadline`. `nap_until(deadline)` replaces the nine bounded sleeps, including the unbounded 2 ms sleep in `service.rs` |
| SVC-16 | `system.rs`: `spawn_piped` and `decode` shared by the runners |
| SVC-17, MUX-06 | Inline `env_snapshot`. Delete `runtime::deliver_pending`; `delivery::deliver_pending(conn, settings, member_id, expected_fleet, deadline, attempted)` is the production entry, `deliver_pending_with(…, resolve, host)` is the test seam, and the `#[expect]` reason states the argument constraint |
| SVC-18 | `launch_registered` becomes private and takes `&mut PendingGuard`; `register_pending_run` becomes `pub(super)`; `ControlIdentity::from_descriptor` |
| SVC-19 | Callers use `control::request` directly; delete the re-export |
| INP-02 | Merge `input_protocol/{wire,wire/replies,wire/strict_json}.rs` into `input_protocol.rs`. Delete the reply forwarders; `decode_input_request` becomes private |
| INP-03 | Delete `InputControlRequest.protocol_version`; the encoder writes `CONTROL_PROTOCOL_VERSION` at the same key position. Delete the three constant checks |
| INP-04 | `CurrentRequest` stores `observed: ObservedRequest` plus its own identity fields and delegates to `ObservedRequest::same_request`. A field-for-field equivalence test pins the predicate, with `output_seq` excluded where it is excluded today |
| INP-05 | Delete client-side reply-binding conjuncts that `replies::decode` enforces, and the `InspectionHost` `wrong_event` subcase. Keep message_id, episode_seq, backend, versions, `missing_evidence`, capability rules, the 256-byte profile id limit, `actual == input_id` and the `director_member_id` binding unless `inspection_association` is proven to bind it **(verify)** |
| INP-10 | `submit_wrapped_input_with_host` becomes production; `submit_wrapped_input` calls it with `NativeInputHost`; delete `submit_wrapped_input_under_pane_lock` |
| INP-11 | `broker::runtime::inputs::input_payload_digest(run_id, &SubmitInput)` is used by the client and the runner, with one independent test oracle |
| INP-12 | `input::bounded_by_inherited(conn, until)` at every clamp site; delete the two no-op re-clamps |
| INP-18…22 | Delete the strict_json frame-size checks behind the framer. `codec::validate_version` replaces `wire::version`. `validate_order` uses `PermissionDecision::VALUES` and a `MissingEvidence::VALUES` const and drops its unreachable branch. Add `validate_reply_against_durable(…, messages)`. Derive `Clone` on the three protocol inputs |
| INP-25, INP-26 | The `input_client.rs` helpers: path check, `same_association`, `permission_placement`, `PermissionReceiptIdentity::for_run`, and `validate_respond(&input)?` in the decoder. `SocketHost::failure` uses `host_failure` |
| INP-30 | Delete `InputBoundary::Settled`/`Retired`, which are never constructed |
| INP-32, INP-33 | Drain accessors `human_generation()`/`output_seq()`; `current_drain_seq`; `validate_taken_drain`; drop the second binding validation in `fresh_drain`. Delete `retire_received`'s re-checks that `validate_receipt_cache` guarantees |
| INP-34, INP-35 | Production literals in `runner/input.rs`: an `InputFailure` builder, one permission-response prelude, `SubmissionResult::held`, `RECEIPT_CACHE_CAPACITY`, shared state mapping, one `WithDiagnostics` helper, `&PermissionRespondInput` parameters, `InputCheckpoint.input_id: i64`, and `observe_manual` via `and_then`. Add `settle_any_input_in_transaction` |
| INP-37 | Delete the duplicate pane-pair check and the real clock's self-validation |
| INP-38, INP-39, INP-40 | Move member and fleet deletion orchestration (`DeletionPlacement`, `DeletionHost`, `end_member`, `delete_fleet*`, `unplaced_member_outcomes`) from `input.rs` to the new `cafleet/src/deletion.rs`. `DeletionHost::shutdown` becomes a default method, `deletion_run_identity` is inlined, and `member_pane_input_error(member_id)` replaces the repeated mapping |
| INP-41 | `lock_deletion` uses `deadline.remaining()?` followed by `PaneLock::acquire_within(…)`, with the existing `member {member_id} pane input: {cause}` mapping applied to its error (`member_pane_input_error`) and `busy(member_id)` to its `None`; `Deadline::retry` is deleted **(verify)** behavioral equivalence for every input, including the expiry boundary: today an expired deadline after a sleep yields `busy` without another try, while `acquire_within` tries once more. When equivalence fails, keep the loop and only inline `retry` |
| INP-42 | `innermost_deadline`; `inherited_or_new` uses `unwrap_or_else(Deadline::new)`; delete `impl Default for Deadline` |
| INP-44 | Drop the duplicated connection-identity assertion from `CANCELLATION_SCOPES` **(verify)** that `database_cancelled` reads only the deadline stack |
| INP-49 | Remove the unused serde derives on protocol types **(verify)** that `mise //cafleet:typecheck` passes |

#### Coding agent

| ID | Change |
|---|---|
| AGT-04 | `BUNDLED_FIXTURES` is generated by a one-line-per-id `bundled!` macro (same paths and bytes) |
| AGT-09, AGT-10, AGT-16, AGT-17 | Delete `ProfileCatalog::answer_capability`, `CodingAgent::build_spawn_argv` and `binary_name()`. Re-export only `ComposerOperation` and delete the `expect(unused_imports)` |
| AGT-11 | Delete the unreachable second native-provenance check, `Answer.provenance`, `AnswerPlan.provenance` and `ComposerSubmission::native_provenance`. Composers store fixture ids |
| AGT-12 | `answer_plan` → `Option<AnswerPlan>`, `composer_plan` → `Option<ComposerPlan>`, `Detector::new` → `Self` |
| AGT-13, AGT-14 | Delete the detector's re-checks of catalog-validated invariants. Store region/matcher/choice selectors as private enums produced at the existing `one_of` sites |
| CLI-06 | Replace `ResolvedDir` and the three per-agent resolvers with `config_dir::resolve(var, env, default) -> Result<PathBuf, CafleetError>`. `CodingAgent::config_dir` returns `PathBuf`. Tests iterate `CodingAgent::all()` |

#### Broker and storage

| ID | Change |
|---|---|
| BRK-05 | `register_member`, `deregister_member`, `update_placement_pane_id`, `create_run`, `reserve_input`, `settle_input` and `release_permission_claim` become `#[cfg(test)]` helpers beside their production variants. `settle_input` drops its duplicated prechecks |
| BRK-06 | One `get_fleet`, plus `require_fleet` emitting `fleet '{id}' not found.` at its four sites. `cli/member.rs`'s `Usage` variant stays |
| BRK-08 | `member_fleet_status`; one destination-member branch. `historical_ownership` keeps its current error behavior |
| BRK-09 | `insert_member_row`, `insert_placement_row`, a shared member row mapper; delete the dead clippy allow **(verify)** with lint |
| BRK-10, BRK-11 | Delete the unreachable RunnerExited re-validation (extract with `expect` naming the `validate_cause` invariant). Add `permission_event_matches`; `validate_current_capability` takes the fetched event |
| BRK-13, BRK-14, BRK-17 | `PAYLOAD_LIMIT`, `UNRESOLVED_INPUT_STATES`, `latest_event_seq`, `interaction_id_for_episode`. Only the counted transition forms remain, renamed without the suffix. `SchemaShape` becomes `Vec<SchemaTable>` |
| BRK-16 | One deadline check per SQL step and one per row in `select_listen_snapshot` **(verify)** with `mise //cafleet:test listen`. The progress-handler unification stays out of scope |

#### CLI, multiplexer and delivery

| ID | Change |
|---|---|
| CLI-08, CLI-09, CLI-11, CLI-12 | `dispatch_to_member` shared by `prompt`/`exec`. Delete the `helpers.rs` `resolve_mux` re-export, inline `invocation_connection`, and add `available_mux`. Bind the `member create` runtime snapshot directly. Add the `checked`/`nonblank` value-parser adapters |
| CLI-14 | Move the inline text builders (`fleet list`/`show`, `member delete`, `prompt`/`exec`, message headers) into `output/formatters.rs` |
| CLI-22, CLI-23 | Inline `parse_numeric`. `config_dir::home_dir()` is shared by `setup` and `doctor` |
| MUX-01 | Private `BackendCore` in `multiplexer/mod.rs` with the shared runner, press, availability, key, prompt, capture-guard and error-mapping code. Messages are rebuilt byte-identically from the backend name, and one constant holds the placement-mismatch string |
| MUX-07, MUX-08 | Delete the `if deadline.remaining().is_err()` after the `?`. Compute the concrete placement once, and use one stage match without `unreachable!` |
| MUX-10, MUX-11 | `receive::wait` returns `Vec<MessageRecord>` and uses `if let AttemptEnd::Failed`. `EmittedMessages(HashSet<i64>)` |
| MUX-15, MUX-16, MUX-17 | herdr `list_panes`/`panes_of`/`run_json`/`sort_by_y`; tmux `caller_context`; `run_for` via `whole_seconds`, `DEADLINE_EXPIRED_BEFORE_LAUNCH`, `INPUT_RESERVATION` from shared consts **(verify)** const-fn support on the pinned toolchain |

### Test de-duplication catalog

Apply during or after relocation, recording each deleted, merged or renamed test in the ledger. Rows marked **(verify)** follow the protocol.

| Area | Findings |
|---|---|
| Native capture and control service | TST-12 items (private_write, `CaptureFixture` internals, config builder, `NativeCaptureArm` constructor, resize, kinds, `ServiceFixture::wait` via `TestChild::poll_until`, polling loops, frame decoders, recovery wrappers, lifecycle branches, case setup; `NewRun` literal, JSON health and socket clients **(verify)**). TST-14 verified rows; TST-14 needs-verification rows **(verify)**. TST-15 builders and a typed crash-child mode enum |
| Runtime services | SVC-02…05, SVC-10, SVC-11, SVC-14, SVC-19 tests (merged codec modules with `ControlDecoder::feed` replaced by a test `feed_values`, scoped-thread peers instead of `Peer`, dropped baseline preambles, outbox/lifecycle merges) |
| Observation | OBS-22, OBS-23 (keep the `recovery_entry` pair) |
| Input | INP-06, INP-08, INP-09, INP-15, INP-16, INP-17 (q4 and the r1 block; p2 shrink **(verify)**), INP-26, INP-27, INP-28, INP-31, INP-36, INP-37 helpers, INP-47 **(verify)** |
| Coding agent | AGT-05, AGT-06 (one guard test over every bundled fixture plus the three distinct cases), AGT-07 (one loop asserting the privacy-token scan and declared masked rows), AGT-08, AGT-09 merge, AGT-18, AGT-19 |
| Broker and storage | BRK-03, BRK-15, BRK-18 **(verify)** |
| CLI | CLI-03 (one table-driven refused-bytes test over mutation × command; delete the setup subset test), CLI-04, CLI-17, CLI-18 (delete the former-base `~/.opencode` sentinel assertions), CLI-19, CLI-20, CLI-22 |
| Multiplexer and delivery | MUX-02, MUX-03 (one table-driven `capture_window` test), MUX-12, MUX-13, MUX-14 |

Test hygiene, applied with each relocation:

- Rename every test, module and file whose name carries design provenance to the behavior it asserts. This covers `step5_*`, `step5_a0_`/`a1_`/`b0_`/`wire_`, `step5_composer_eN_`, `runner_batch2_*`, `runner_review_rN_*`, `runner_recovery_review_rN_*`, `collector_f1_`/`f4_`/`t1_`, `review_*.rs` file names and `mod review`. The `review-canary-194` literal becomes a neutral canary, and `.p194-`/`.m194-` temp prefixes become `.cafleet-test-`. When a `tempdir_in("/tmp")` exists to keep Unix-socket paths short, it routes through one `short_tempdir()` helper **(verify)** the reason. Keep `/tmp/cafleet-194*` driver roots. Rename `…validates_legacy_identity…` without "legacy" (OBS-21, CLI-21, INP-48, AGT-21, MUX-04).
- Delete diagnostic `eprintln!` calls in tests. A value that is genuinely needed folds into an assertion message (TST-15, INP-26, SVC-01).

### Retained by decision

| Item | Reason |
|---|---|
| `#[cfg(test)]` capture instrumentation in production files (SVC-20) | The native-capture harness is kept and needs it |
| `AttemptEnd` variants (MUX-09) | Delivery tests use them as an oracle; only the `receive.rs` match collapses (MUX-10) |
| 19 uncited bundled fixtures (AGT-22) | Shipped data stays identical |
| `InputCommandHost`, `InputDeadlineClock`, `RequestTokenSource`, `InputHooks`, `ListenIo`, `ListenOutput`, `ReceiveIo` traits | Real test seams |
| `presets/opencode/cafleet.md` vs `cafleet-ask.md` overlap | OpenCode loads each preset as a standalone file |
| Listen progress-handler cancellation (BRK-16 second half) | Unifying it with `with_database_deadline` changes cancellation wiring beyond redundancy removal |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Each step is one commit and leaves `mise //cafleet:test` and `mise //cafleet:lint` passing (documentation steps also pass `mise //docs:build`). Steps 1–4 complete the documentation before any code change, in the order the rewritten `.claude/rules/documentation-maintenance.md` prescribes: SPEC.md → `docs/` → README → skills → rules.

### Step 1: Baseline and SPEC.md as the single specification

- [x] Record `tests-before.txt` and `help-before.txt` per § Test-change ledger, and create `test-ledger.md` with its header <!-- completed: 2026-10-29T15:10 -->
- [x] Absorb `cli-options.md`, `message-envelope.md` and `data-model.md` into SPEC §6.3, §6.4, §6.1 and §8 per § SPEC.md consolidation steps 1–2 <!-- completed: 2026-10-29T15:10 -->
- [x] Absorb `multiplexer-backends.md`, `coding-agent-backends.md` (contract parts) and `member-runtime.md` into SPEC §6.5, §6.7 and §6.9 <!-- completed: 2026-10-29T15:15 -->
- [x] Apply the stale corrections SPC-02, 03, 04, 05, 06, 10, 15, 19, 21, 22 <!-- completed: 2026-10-29T15:24 -->
- [x] Apply the single-owner actions SPC-07, 08, 09, 11, 12, 13, 20, 21 <!-- completed: 2026-10-29T15:28 -->
- [x] Replace SPEC's links into `docs/docs/` with internal `§` references and delete every `{#…}` heading suffix; confirm `git grep -n "docs/docs" -- SPEC.md` prints nothing <!-- completed: 2026-10-29T15:32 -->

### Step 2: Docs site for users

- [x] Move the user setup content of `docs/docs/spec/coding-agent-backends.md` into `quickstart.md` and `concepts/coding-agents.md` <!-- completed: 2026-10-29T15:38 -->
- [x] Rewrite every internal link into `spec/` per § Docs site, and verify each SPEC anchor per the anchor table <!-- completed: 2026-10-29T15:38 -->
- [x] Apply SPC-02, SPC-04/14, SPC-16, SPC-17, SPC-18, SPC-23 and SKL-21 to the concept, how-to and quickstart pages and `docs/theme/styles.css` <!-- completed: 2026-10-29T15:38 -->
- [x] Update `docs/docs/contributing.md` (SPEC link, test-placement paragraph, `test` + `lint` validation, native-acceptance link), README's Specification link and `CONTRIBUTING.md` (SKL-26) <!-- completed: 2026-10-29T15:38 -->

### Step 3: Skills for agents

- [ ] Write `skills/cafleet/reference/cli.md` per § Agent CLI reference from SPEC.md <!-- completed: -->
- [ ] Retarget every skill link into the mirror to `cli.md` anchors; delete the SPC-02 fleet-capture mentions from `skills/cafleet/SKILL.md`, `skills/cafleet/reference/supervision.md` and `skills/cafleet/roles/director.md` <!-- completed: -->
- [ ] In one change, delete `skills/cafleet/reference/runtime/`, the `rerun-if-changed=../docs/docs` line in `cafleet/build.rs`, then `docs/docs/spec/` and its `docs/docs/_meta.json` entry. `cafleet/build.rs` embeds a dangling symlink as a file, so no symlink may outlive its target <!-- completed: -->
- [ ] Apply SKL-06, 07, 08, 11, 12, 14, 15, 17, 18, 22 and 24 to the files each row names: `skills/cafleet/**`, `skills/cafleet-design-doc/**` (SKL-06, 18, 22, 24), `.claude/skills/skill-author/SKILL.md` (SKL-06) and `.claude/skills/cafleet-model-list-refresh/SKILL.md` (SKL-08) <!-- completed: -->
- [ ] Apply SKL-02, 03, 09 and 10 to `skills/cafleet-design-doc/**` <!-- completed: -->
- [ ] Add the create/execute per-role delta tables and the slot/placeholder wording in `skills/cafleet/roles/director.md` (§ Spawn delta tables, SKL-16) <!-- completed: -->

### Step 4: Rules and maintainer surfaces

- [ ] Add § Test placement to `.claude/rules/code-quality.md` and `## Tests` to `CLAUDE.md` with the exact § Test placement rule texts <!-- completed: -->
- [ ] Rewrite `.claude/rules/documentation-maintenance.md`, `.claude/rules/coding-agent-overlay.md`, `.claude/rules/database-migrations.md` and `.claude/rules/commands.md` per § Rules and maintainer skills for the documentation model <!-- completed: -->
- [ ] Rewrite `.claude/skills/update-readme/SKILL.md` and `.claude/skills/clean-docs/reference/review-format.md` per the same table <!-- completed: -->
- [ ] Consolidate the clean-docs roles and SKILL.md (SKL-01, SKL-04, SKL-05) and apply SKL-23 to `.claude/skills/clean-docs/**` and `.claude/rules/git-workflow.md` <!-- completed: -->
- [ ] Merge the CI jobs into a matrix with stable check names and SHA-pin actions (SKL-13) <!-- completed: -->
- [ ] Apply SKL-19 (`.gitignore`) and CLI-16 (`rustfmt.toml`) under the verify-then-act protocol <!-- completed: -->

### Step 5: Crate-level test infrastructure

- [x] Add the `test_path!` and `assert_no_debug_or_serialize!` macros to `lib.rs` and the `child_path()` functions beside every re-exec child; replace every hard-coded child path literal <!-- completed: 2026-10-29T15:15 -->
- [x] Create the storage, config and broker homes from § Shared test-support homes (`db::test_support`, `Settings::for_database`, `fleets`/`members`/`records` helpers, `broker::runtime::test_support`); split `broker::tests` and update its 20 importers <!-- completed: 2026-10-29T15:33 -->
- [x] Create `catalog::test_support::synthetic_definitions()`/`synthetic_catalog()`; point every synthetic-profile reader at them; delete `runner_tests/fixtures/synthetic-profile.json` <!-- completed: 2026-10-29T15:39 -->

### Step 6: Dissolve `runtime/runner_tests/`

- [ ] Move `Fixture` to `runner/mod.rs` `test_support`, `OwnedChild` to `system.rs` `TestChild`, and the control-service harness to `service.rs` `test_support` with final names; delete the `runtime/mod.rs` re-export block <!-- completed: -->
- [ ] Move `creation.rs`, `control_service.rs`, `input_service*`, `observation_*` and `review_failures.rs` tests to their targets <!-- completed: -->
- [ ] Create `deletion.rs` (INP-38) and move `fleet_deletion.rs` and `recovery/deletion.rs` tests into it with the merged deletion `Host` <!-- completed: -->
- [ ] Move the recovery fixture to `recovery.rs` `test_support` and every `recovery/*` test to its target; merge `ClockHost`/`UnlinkHost` into `Host` <!-- completed: -->
- [ ] Replace the blink `.raw` fixture with a generator; delete `runtime/runner_tests/` <!-- completed: -->

### Step 7: Fold the native-capture harness and delete the tracer

- [ ] Inline `native_capture.rs` and `native_capture/{exporter,files,verification,review_service_tests}.rs` into `observation.rs` as `native_capture`, and move `runner_tests/native_capture*`, `review_collector.rs` and `review_export.rs` tests into it <!-- completed: -->
- [ ] Apply OBS-08…11 and OBS-13 inside the harness <!-- completed: -->
- [ ] Delete `process/ownership_diagnostics.rs`, its hooks and self-tests (SVC-01), and record them in the ledger <!-- completed: -->

### Step 8: Re-home mis-housed in-file test modules

- [ ] Split `coding_agent/profiles.rs` `contract_tests` into `catalog.rs`, `detector.rs`, `composer.rs` and `runtime/runner/episodes.rs`; rename `inspection_tests`; collapse the re-export chains; delete `contract_tests/` and its hashing test (AGT-03) <!-- completed: -->
- [ ] Move `coding_agent/mod.rs` policy and native-version tests to `policy.rs` and `native_version.rs` <!-- completed: -->
- [ ] Split `broker/mod.rs` `runtime_contract_tests` per § Mis-housed in-file modules <!-- completed: -->
- [ ] Re-home the `cli/mod.rs`, `listen.rs`, `multiplexer/mod.rs`, `input_client.rs`, `input_clock.rs`, `transport.rs`, `health.rs` and `system.rs` modules per the same table <!-- completed: -->
- [ ] Consolidate every file with several or mid-file test modules into one bottom `mod tests` and apply the TST-13 naming <!-- completed: -->

### Step 9: Visibility and crate-wide helpers

- [ ] Narrow visibility per CLI-01/INP-45/MUX-19 and delete every dead item the lint reports <!-- completed: -->
- [ ] Add `digest.rs` and `system::random_hex32`, and replace every copy <!-- completed: -->
- [ ] Add `From<rusqlite::Error>` and replace the `db_err` sites (BRK-04) <!-- completed: -->
- [ ] Apply CLI-07, INP-43, BRK-07, CLI-13 and AGT-15 <!-- completed: -->

### Step 10: Runtime runner simplification

- [ ] Apply OBS-02, 03, 05, 06, 12, 14–20 and 24 <!-- completed: -->
- [ ] Apply SVC-06, 07, 08, 09, 12, 13, 16, 17, 18 and 19 <!-- completed: -->
- [ ] Apply INP-02, 03, 04, 05, 10, 11, 12 <!-- completed: -->
- [ ] Apply INP-18–22, 25, 26, 30, 32–35, 37 <!-- completed: -->
- [ ] Apply INP-39–42, 44 and 49, plus MUX-06 <!-- completed: -->

### Step 11: Coding agent, broker, CLI and multiplexer simplification

- [ ] Apply AGT-04, 09–14, 16, 17 and CLI-06 <!-- completed: -->
- [ ] Apply BRK-05, 06, 08, 09, 10, 11, 13, 14, 16, 17 <!-- completed: -->
- [ ] Apply CLI-08, 09, 11, 12, 14, 22, 23 <!-- completed: -->
- [ ] Apply MUX-01, 07, 08, 10, 11, 15, 16, 17 <!-- completed: -->

### Step 12: Test de-duplication and hygiene

- [ ] Apply the § Test de-duplication catalog rows for native capture, runtime services, observation and input <!-- completed: -->
- [ ] Apply the rows for coding agent, broker and storage, CLI, multiplexer and delivery <!-- completed: -->
- [ ] Apply the provenance renames, temp-prefix changes and `eprintln!` removal <!-- completed: -->

### Step 13: Verification

- [ ] Record `tests-after.txt` and `help-after.txt`; reconcile the ledger and help text per § Test-change ledger <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build` and confirm every grep check in § Success Criteria <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-29 | Initial draft |
| 2026-10-29 | Review round 1: audit files committed under `audit/`; concrete `git grep` checks; explicit test-support forms and the `#[cfg(test)] impl` rule; macro placement; `added` ledger kind and the help-capture procedure; GitHub anchor rule; whole-page `cli.md` rows and its sync trigger; dead-code sweep split; INP-41 keeps its error mapping; documentation steps reordered (SPEC → docs → skills → rules) with mirror and docs-spec deletion in one commit |
| 2026-10-29 | User decision: delete the whole skill runtime mirror, including the one linked concepts page, and retarget that `director.md` link to `cli.md#member-create` |
