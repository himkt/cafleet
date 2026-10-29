# Audit: AGT — coding-agent profiles (`cafleet/src/coding_agent/**`)

## Summary

The slice is 15,444 lines, of which roughly 13,000 are tests. The biggest problem is placement: `profiles.rs` is 8,601 lines, but 8,511 of them (21-8531) are one `#[cfg(test)] mod contract_tests` that exercises `catalog.rs`, `detector.rs`, `composer.rs` and `runtime/runner/episodes.rs`, which has no tests of its own. In the same way, `mod.rs` carries the tests for `policy.rs` and `native_version.rs`. Inside those tests, the five per-backend `bundled_*` modules copy the same fixture tables, helpers, guard tests and assertion bodies (Claude/Codex/OpenCode). Every bundled native fixture is also `include_bytes!`-ed up to four times. Production code has smaller items: test-only public API (`answer_capability`, `build_spawn_argv`), an unreachable second native-provenance check that keeps three provenance fields alive, and `Result` returns that can never fail. Fixture status: all 77 `profiles/native/*.json` files are runtime data, embedded and digest-verified by `ProfileCatalog::bundled()`. 19 of them (54 KB of 215 KB) are cited by no capability and are consumed only by tests. `profiles/contract_tests/synthetic-permission-screen.json` is test-only. Estimated removable: ~2,250 lines, plus the `contract_tests/` folder and one duplicate synthetic-profile JSON.

## Test placement inventory

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `cafleet/src/coding_agent/profiles.rs` `mod contract_tests` (21-8531) | 8,511 | `profiles/catalog.rs`: `catalog` 32-501, `gated_catalog` 798-1188, catalog-validation half of `annotation_catalog` 7628-8063, `profile_version_fallback` 8064-8378, the bundled guard/registration tests. `profiles/detector.rs`: `focus_resolution` 502-679, `detector` 1189-1604, `annotation_detector_*` 7900-8043, 8379-8410, bundled detection tests. `profiles/composer.rs`: `executable_composer` plan/encode tests 2704-3398; the submission-validation cases go to `catalog.rs`, which owns `composer_submission`. `runtime/runner/episodes.rs`: `continuation_episodes` 680-797, `episodes` 1605-2703, every `tracker_*` / `*_episode_*` bundled test | Shared helpers 8412-8531: `definitions`/`simple_definition` → catalog.rs test support; `lines`/`terminal`/`observation`/`permission`/`state_observation` → detector.rs test support; `tracker`/`open`/`NOW` → episodes.rs test support | Not a test-only file: lines 8533-8601 are production (`AnswerCapability`). After the move, `profiles.rs` is ~90 lines. `episodes.rs` currently has no `#[cfg(test)]` module. |
| `cafleet/src/coding_agent/mod.rs` `mod policy_contract_tests` (31-535) | 505 | `coding_agent/policy.rs` | `version()` / `permission_argv()` oracles move with it | Everything it calls (`resolve_policy`, `policy_choices`, `permission_mode_help`, `build_spawn_argv_with_policy`) is defined in `policy.rs`. |
| `cafleet/src/coding_agent/mod.rs` `tests::native_version_probe` (830-1012) | 183 | `coding_agent/native_version.rs` | `stub()` shell-script helper, `tempfile` | It exercises `probe_native_version` and `native_launch_identity`, both in `native_version.rs`, which already has a `tests` module (153-196). |
| `cafleet/src/coding_agent/profiles/contract_tests/` (`synthetic-permission-screen.json`) | 39 | Delete (AGT-03) | Loaded only by `include_bytes!` at `profiles.rs:64` | Test-only fixture folder; nothing in production reads it. |
| `cafleet/src/runtime/runner_tests/fixtures/synthetic-profile.json` (outside the slice; loaded by `catalog.rs:20-22,82-84`) | 1 (long) | `#[cfg(test)]` synthetic-definition builder in `profiles/catalog.rs` (AGT-03) | Also loaded by `runtime/runner/input.rs:8361`, `runtime/runner/writer.rs:851`, `runner_tests/control_service/input_service.rs:133`, `observation_services.rs:96`, `runner_tests/native_capture/support.rs:28` | Same content as the `definitions()` JSON literal at `profiles.rs:8418-8437` (only `source_ref` differs). |
| `catalog.rs` `pub(super) mod inspection_tests` (14-319) and `detector.rs` `pub(super) mod inspection_tests` (24-1304) | 306 / 1,281 | Stay in place; rename the helper half to `test_support` | Exported through a three-level `#[cfg(test)] pub(crate) use` chain (`profiles.rs:6-18`, `mod.rs:8-9,25-29`) to `runtime/runner/{input,input_client,observation}.rs` | Correct placement. Mixing `#[test]`s with crate-shared helpers under one module is what forces the re-export chain. |
| `profiles/native/*.json` (77 files, 214,597 B) | — | Stay: runtime data | `include_bytes!` in `catalog.rs:1147-1456` (production `BUNDLED_FIXTURES`), re-included by tests at `profiles.rs` (5 tables), `detector.rs:107-110,400-405,762-765,946-950`, `runtime/runner/input.rs:101-170,1073-1074` | All 77 are registered in `bundled.json` and verified on every `ProfileCatalog::bundled()`. 58 back a capability. 19 are embedded but uncited (AGT-22). |

## Findings

### AGT-01: Move the 8.5k-line `profiles.rs` test module into the files it exercises
- Category: test-placement
- Location: cafleet/src/coding_agent/profiles.rs:21-8531
- Evidence: `profiles.rs` itself defines only re-exports (1-19) and `AnswerCapability` (8533-8601). Its test submodules call `ProfileCatalog::from_definitions`/`from_bundled_definitions` (catalog.rs), `Detector::{new,observe,match_answer}` (detector.rs), `ComposerPlan::encode` (composer.rs) and `EpisodeTracker::{new,observe,reset,finish}` (`runtime/runner/episodes.rs:175`; 7 `EpisodeTracker::new(` sites, all in profiles.rs plus one in observation.rs). `grep '#\[cfg(test)\]' runtime/runner/episodes.rs` finds nothing: the tracker's only tests live in this slice.
- Proposal: Split per the inventory row. Give each target a `#[cfg(test)] mod tests` and, where helpers are shared across files, a `#[cfg(test)] pub(crate) mod test_support` inside the module that owns the type: synthetic definitions in catalog.rs, screen builders and replay in detector.rs, `tracker()`/`open()` in episodes.rs. Rename `contract_tests` away.
- Spec impact: none — only test code moves.
- Risk: low — mechanical; `use` paths change.
- Estimated delta: -0 lines on its own (the move enables AGT-04…08, AGT-17)

### AGT-02: Move `policy.rs` and `native_version.rs` tests out of `mod.rs`
- Category: test-placement
- Location: cafleet/src/coding_agent/mod.rs:31-535, 830-1012
- Evidence: `policy_contract_tests` tests only `policy.rs` items (`resolve_policy` policy.rs:67, `policy_choices` :37, `permission_mode_help` :22, `build_spawn_argv_with_policy` :96). `native_version_probe` tests `probe_native_version` (native_version.rs:74) and `native_launch_identity` (:22); native_version.rs already has `mod tests` at :153.
- Proposal: Move `policy_contract_tests` into `policy.rs` `#[cfg(test)] mod tests`, and `native_version_probe` into the existing `native_version.rs` `mod tests`. `mod.rs` keeps the tests of its own items (argv literal, model/effort validation, preflight, spawn environment, launch argv).
- Spec impact: none.
- Risk: low.
- Estimated delta: -0 lines (relocation; see AGT-18/19 for trimming)

### AGT-03: Delete the `contract_tests/` fixture folder and keep one synthetic definition
- Category: test-placement
- Location: cafleet/src/coding_agent/profiles/contract_tests/synthetic-permission-screen.json; cafleet/src/coding_agent/profiles.rs:61-90, 8418-8437; cafleet/src/runtime/runner_tests/fixtures/synthetic-profile.json; cafleet/src/coding_agent/profiles/catalog.rs:20-22, 82-84
- Evidence: The screen JSON's only reader is the test at profiles.rs:62-90. That test checks the file's sha256 against the test definition's literal digest and its geometry against a terminal rendering, so it tests test data. `from_definitions` never reads a synthetic `source_ref` (catalog.rs:1655-1686 validates only the format; byte verification runs only for bundled fixtures, catalog.rs:1619-1621). `synthetic-profile.json` is byte-for-byte the `definitions()` literal except `source_ref` ("../../../coding_agent/profiles/contract_tests/…" vs "synthetic-permission-screen.json"). No docs reference either file (`grep` over SPEC.md/docs/skills: 0 hits).
- Proposal: Delete `contract_tests/` and the test at profiles.rs:61-90. Add one `#[cfg(test)] pub(crate) fn synthetic_definition() -> Value` in catalog.rs test support (the current `definitions()` literal) and use it from the existing `inspection_fixture_definition`/`gated_fixture_definition` and from the five out-of-slice `include_str!("…/synthetic-profile.json")` loaders. Then delete `synthetic-profile.json`.
- Spec impact: none — only test catalogs change; SPEC.md:2519 keeps "Synthetic fixtures remain available to explicitly selected test catalogs".
- Risk: low.
- Estimated delta: -85 lines (+ 2 test-only files)

### AGT-04: Embed each bundled fixture once and look it up from tests
- Category: duplication
- Location: cafleet/src/coding_agent/profiles/catalog.rs:1147-1456; cafleet/src/coding_agent/profiles.rs:3404-3501, 4478-4527, 4687-4694, 5667-5697, 5779-5785, 6194-6237, 6661-6741; cafleet/src/coding_agent/profiles/detector.rs:106-110, 400-405, 762-768, 945-950
- Evidence: `BUNDLED_FIXTURES` lists all 77 fixtures as 4-line tuples that spell each path twice (310 lines). The tests rebuild the same registry as five hand-maintained `FIXTURES` arrays: bundled_claude 9 entries, claude_auto 38, codex 4, codex_auto 19, opencode 7, two of them through duplicated `macro_rules! native`. A chained `embedded()` (profiles.rs:3484-3493) feeds them to `from_bundled_definitions`, and per-module `fixture()`/`bytes()`/`source()` lookups (×5) resolve them. detector.rs and runtime/runner/input.rs `include_str!` the same files a fourth time.
- Proposal: In catalog.rs, generate `BUNDLED_FIXTURES` from a one-line-per-id macro (`bundled!("claude-2.1.302-default-permission-1", …)` → `(concat!("native/",id,".json"), include_bytes!(…))`). Add `#[cfg(test)] pub(crate) fn bundled_fixture(id) -> (&'static str, &'static [u8])` and `#[cfg(test)] pub(crate) fn bundled_embedded() -> Vec<…>` over it. Delete every test-side `FIXTURES`, `native!`, `embedded()` and lookup function, and replace the detector.rs/input.rs consts with `bundled_fixture(id)`.
- Spec impact: none — the same bytes are embedded and verified.
- Risk: low.
- Estimated delta: -500 lines

### AGT-05: Replace per-module copies of bundled-test helpers with shared test support
- Category: duplication
- Location: cafleet/src/coding_agent/profiles.rs:3503-3579, 4696-4877, 5787-5944, 6239-6298, 6743-6838
- Evidence: `digest()` is defined 5× (3526, 4765, 5939, 6248, 6756), all identical to production `sha256_hex` (catalog.rs:1458). `definitions()` from bundled.json appears 4× (plus 6 inline `include_str!("profiles/bundled.json")`), and `entry()` 3×. `guarded_fixture()` appears 3×; the Claude and Codex copies at 3556/6268 are identical, and OpenCode at 6794 drops one parameter. `lines()` appears 3×. `bundled_observation()` appears 2×, and `replay`/`screen`/`detector`/`observe` 3×. `redacted()`, `frame()`, `Held`, `Run::{new,metadata,hold}` and `annotation_set` are copied between `bundled_claude_auto` (4751-4877) and `bundled_codex_auto` (5819-5918) with backend constants swapped.
- Proposal: One catalog.rs test-support set (`sha256_hex` exposed `#[cfg(test)] pub(crate)`, `bundled_definitions()`, `entry()`, `guarded_fixture(id, change_definitions, change_screen)`), one detector.rs set (`bundled_screen(id)`, `bundled_observe(backend, version, id)`), and one episodes.rs set (`Run`/`Held` parameterized by an event filter).
- Spec impact: none.
- Risk: low.
- Estimated delta: -250 lines

### AGT-06: Test the backend-agnostic bundled guard once over every fixture
- Category: redundant-test
- Location: cafleet/src/coding_agent/profiles.rs:3708-3769, 3915-3953, 4287-4341, 4979-4992, 6518-6558, 7453-7469, 7560-7568
- Evidence: The guard is one function, `verify_native_screen` (catalog.rs:1466-1543), applied to every entry (catalog.rs:1619-1621). Six tests replay the same mutation table (synthetic provenance, extra field, nonempty masked row, backend, native version, rows/line count, cursor, synthetic catalog kind, appended-byte digest mismatch, missing embedded bytes) against different backends' fixtures.
- Proposal: Keep one catalog.rs test that iterates `bundled_fixture` over every bundled entry with the union mutation table. Keep only the genuinely distinct cases: restored masked status row (3944-3952), pending-wrap cursor (6561-6588) and the `cell_backgrounds` run table (7470-7559).
- Spec impact: none — every SPEC.md:2510-2519 rule stays covered, now on all 77 fixtures instead of a subset.
- Risk: low.
- Estimated delta: -200 lines

### AGT-07: Drop fixture-registration tests that re-assert the load-time guard
- Category: redundant-test
- Location: cafleet/src/coding_agent/profiles.rs:3582-3660, 3772-3850, 4009-4244, 4958-4978, 6301-6423, 6901-6997
- Evidence: Each test re-checks the entry JSON (`id/kind/backend/native_version/rows/cols/source_ref/sha256`), the exact field set, `provenance`, and that masked rows are empty. `ProfileCatalog::bundled()` already enforces all of these on every load (catalog.rs:1466-1543) and many tests call it. The rest pins capture literals (`screen_seq` 11424/10819/…, cursor positions, individual row strings), which detector tests already depend on implicitly.
- Proposal: Replace them with one loop over all bundled fixtures that asserts only what the loader does not enforce: the privacy-token scan (`/Users/`, `himkt`, …) and, if wanted, each fixture's declared `masked_rows` set. Delete the identity/digest/field-set re-assertions and the literal `screen_seq`/cursor pins.
- Spec impact: none.
- Risk: medium — confirm with the Director that pinning captured row text is not a deliberate review gate.
- Estimated delta: -400 lines

### AGT-08: Make the near-identical per-backend tests table-driven
- Category: redundant-test
- Location: cafleet/src/coding_agent/profiles.rs:3663-3705 / 6426-6462 / 6998 (capabilities "declares … only"); 3853-3912 / 6465-6515 (answer stages); 3956-4006 / 6591-6641 (episode closes after 250 ms idle); cafleet/src/coding_agent/profiles/detector.rs:219-264 / 884-943 / 1191-1239 (plans bind focus); 267-287 / 1242-1260 (idle composer verified); 290-312 / 1263-1290 (idle rule rejects drafts)
- Evidence: Each group has the same assertion sequence and differs only in constants (backend, version, profile, rule, fixture, focus glyph/row, stage byte). The two episode-closure tests are verbatim apart from the fixture bytes.
- Proposal: One test per behavior that iterates a `[(backend, version, fixture ids, rows, glyph, byte, …)]` table.
- Spec impact: none.
- Risk: low.
- Estimated delta: -350 lines

### AGT-09: Remove test-only `ProfileCatalog::answer_capability`
- Category: dead-code
- Location: cafleet/src/coding_agent/profiles/catalog.rs:1951-1974
- Evidence: Its only callers are tests: profiles.rs:146, 161, 167, 173, 8214. Production uses `answer_plan(...)` → `plan.capability()` (observation.rs:579, input.rs:2641). The body copies the `AnswerCapability` construction in `Profile::answer_plan` (catalog.rs:593-602). The same capability-identity assertions also appear twice: catalog.rs:191-275 and profiles.rs:132-178.
- Proposal: Delete the method, switch the five test sites to `answer_plan(..).map(|p| p.capability().clone())`, and merge the two overlapping capability-identity tests.
- Spec impact: none — not reachable from the CLI.
- Risk: low.
- Estimated delta: -45 lines

### AGT-10: Remove test-only `CodingAgent::build_spawn_argv`
- Category: dead-code
- Location: cafleet/src/coding_agent/mod.rs:685-697, 74-90, 784
- Evidence: Its only callers are mod.rs:84 and :784, both tests. Production launches through `launch_argv` → `build_spawn_argv_with_policy` (cli/member.rs:505). The assertion at mod.rs:74-90 exists only to show that the two wrappers agree.
- Proposal: Delete the method and the equality clause. The test at :734 calls `build_spawn_argv_with_policy(…, &agent.resolve_policy(None, None)?)`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -30 lines

### AGT-11: Drop the unreachable second native-provenance check and the fields it keeps alive
- Category: over-abstraction
- Location: cafleet/src/coding_agent/profiles/catalog.rs:1622-1635, 444-453 (`Answer.provenance`), 495-502/606 (`AnswerPlan.provenance`), 1101-1114, 1842-1851; cafleet/src/coding_agent/profiles/composer.rs:28-38
- Evidence: `from_bundled_definitions` first runs `verify_native_screen` on every `fixtures` entry, which requires `kind == "native"` (catalog.rs:1468-1471). Every answer and composer fixture reference must resolve to one of those entries (`provenance`, catalog.rs:983-1006). The check at 1622-1635 ("bundled executable composer plans require native provenance") can therefore never fail. It is the only reader of `Answer.provenance` and `ComposerSubmission::native_provenance`. `AnswerPlan.provenance` is written at 606 and read nowhere except the derived `PartialEq`, and it is a pure function of `capability.fixture_ids`.
- Proposal: Delete the check, `Answer.provenance`, `AnswerPlan.provenance` and `ComposerSubmission::native_provenance`, and store composer fixture ids (`Vec<String>`) instead of cloned `Fixture`s.
- Spec impact: none — SPEC.md:2510-2519 rejection behavior is unchanged; the same inputs fail at `verify_native_screen`.
- Risk: low.
- Estimated delta: -40 lines

### AGT-12: Stop returning `Result` from functions that cannot fail
- Category: over-abstraction
- Location: cafleet/src/coding_agent/profiles/catalog.rs:1976-1986 (`answer_plan`); cafleet/src/coding_agent/profiles/composer.rs:95-126 (`composer_plan`); cafleet/src/coding_agent/profiles/detector.rs:1825-1835 (`Detector::new`)
- Evidence: All three bodies only construct `Ok(..)`. Callers: `Detector::new(` 31 sites (profiles.rs 16, detector.rs 11, input.rs 1, native_capture.rs 1, observation.rs 2), nearly all `.unwrap()`. `.answer_plan(` ~45 sites, mostly `.unwrap().unwrap()` / `.unwrap().is_none()`. `composer_plan` has one production caller (input.rs:5441, `?`).
- Proposal: Return `Option<AnswerPlan>`, `Option<ComposerPlan>` and `Self`, and drop the `.unwrap()`/`?` at callers.
- Spec impact: none.
- Risk: low — compile-checked churn across the runner slice.
- Estimated delta: -60 lines

### AGT-13: Remove the detector's re-checks of catalog-validated invariants
- Category: over-abstraction
- Location: cafleet/src/coding_agent/profiles/detector.rs:1931, 1785-1791
- Evidence: `!cap.fixtures.is_empty()` is always true because `provenance` rejects empty fixture ids (catalog.rs:989). The `single_request` scope re-check in `match_answer` is already guaranteed: compile requires answer choices to be `single_request` (catalog.rs:1797-1800), and the plan must equal the profile's own plan (detector.rs:1762-1769).
- Proposal: Delete both conditions. Keep an `expect` only where an `Option` must be unwrapped.
- Spec impact: none.
- Risk: low.
- Estimated delta: -8 lines

### AGT-14: Parse enum-like profile strings into Rust enums
- Category: over-abstraction
- Location: cafleet/src/coding_agent/profiles/catalog.rs:392-417, 738-742, 760, 779; cafleet/src/coding_agent/profiles/detector.rs:1574-1575, 1606-1615, 1634, 1669, 1788; catalog.rs:938, 1798
- Evidence: `Region.screen`, `Region.cursor`, `Matcher.scope` and `Choice.scope` are stored as `String` after `one_of` validation, then compared against string literals at runtime. `qualifying_regions` needs `_ => unreachable!("compiled cursor enum")`.
- Proposal: Use private enums (`ScreenSelector`, `CursorRule`, `MatcherScope`, `ChoiceScope`) produced by the existing `one_of` call sites; matches become exhaustive and the `unreachable!` goes away.
- Spec impact: none — the accepted JSON vocabulary and the error ("invalid profile enum") are unchanged.
- Risk: low.
- Estimated delta: -5 lines (removes the string-typo class and one unreachable arm)

### AGT-15: Delegate `ObservationKind` interaction strings to `InteractionClass`
- Category: duplication
- Location: cafleet/src/coding_agent/profiles/detector.rs:1305-1324
- Evidence: `parse` re-spells the five `InteractionClass` wire strings that `runtime_enum!` already provides (`TryFrom<&str>` / `VALUES`, broker/runtime/records.rs:7-27, 50-53). `name()` already delegates (`class.as_str()`, detector.rs:1327).
- Proposal: `if let Ok(class) = InteractionClass::try_from(value) { return Ok(Self::Interaction(class)); }`, then match the remaining kinds. Keep the existing `CafleetError::Usage("invalid profile observation")` for unknown input.
- Spec impact: none.
- Risk: low.
- Estimated delta: -5 lines

### AGT-16: Remove the `binary_name()` forwarder
- Category: dead-code
- Location: cafleet/src/coding_agent/mod.rs:571-573
- Evidence: `binary_name(self)` returns `self.name()`. Callers: cli/doctor.rs:182,187,189; mod.rs:650,651,720,1055; policy.rs:117; native_version.rs:80.
- Proposal: Call `name()` directly.
- Spec impact: none — identical strings.
- Risk: low.
- Estimated delta: -4 lines

### AGT-17: Remove the `expect(unused_imports)` suppression on the composer re-export
- Category: other
- Location: cafleet/src/coding_agent/mod.rs:17-24; cafleet/src/coding_agent/profiles.rs:16
- Evidence: Outside tests, `ComposerPlan` is never named through `crate::coding_agent` (input.rs uses only `ComposerOperation` and the method `.composer_plan`). The only path user is the test import at profiles.rs:2706, which is why non-test builds need `cfg_attr(not(test), expect(unused_imports, …))`.
- Proposal: After AGT-01 puts the composer tests in composer.rs, re-export only `ComposerOperation` and delete the `cfg_attr`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -7 lines

### AGT-18: Collapse overlapping launch-policy tests
- Category: redundant-test
- Location: cafleet/src/coding_agent/mod.rs:161-173, 212-226, 279-301, 318-325, 336-359, 362-419, 422-451, 454-503, 1200-1203
- Evidence: Test :95-158 already maps every registered explicit choice of every backend to its exact full argv (permission flags, model, effort, prompt). That covers :161 (Claude `default`/`manual`), :422 (explicit `approve-for-me`), :454 ("previous defaults remain explicit") and :229's argv. Test :59-92 covers the omitted default per backend, and :193 covers unknown native versions; together they cover :279, :362 and :396. :318 (help contains the OpenCode part) is strictly implied by :328 (exact help equality), which also fixes the choice lists that :336 restates. :1200-1203 repeats the loop's `cafleet-ask` case at :1194-1199.
- Proposal: Keep :59, :95, :176, :193, :304, :328, :506, :525. Fold the extra native versions from :212/:362/:396 into the `version()` table of :59/:95. Delete the rest.
- Spec impact: none — every argv/help/error string stays asserted.
- Risk: low.
- Estimated delta: -180 lines

### AGT-19: Trim native-version probe tests that re-test the tokenizer and the fallback
- Category: redundant-test
- Location: cafleet/src/coding_agent/mod.rs:849-869, 876-881, 963-995
- Evidence: The four success outputs at :853-858 and the token-shape failures ("no version token", "two version tokens", "v-prefixed and plain") are the inputs of the unit tests at native_version.rs:157-195. Only the process-specific cases are new (nonzero exit, first-line rule, empty, non-UTF-8, unstartable, timeout). `BUNDLED_PROBES` is copied at mod.rs:966-970 and profiles.rs:8280-8284, and :964-995 re-asserts the fallback selection already covered by profiles.rs:8287-8322; only its warning text is new.
- Proposal: Keep one success stub plus the process-specific failures. Have the `native_launch_identity` fallback test assert only the warning/coverage fields and share one `BUNDLED_PROBES` const.
- Spec impact: none.
- Risk: low.
- Estimated delta: -30 lines

### AGT-20: One helper for the "no Debug / no Serialize" compile-time check
- Category: duplication
- Location: cafleet/src/coding_agent/profiles/catalog.rs:306-318; cafleet/src/coding_agent/profiles/detector.rs:1292-1303; cafleet/src/coding_agent/profiles.rs:485-500, 3345-3356 (also runtime/runner/input.rs:721, runtime/runner/observation.rs:213 outside the slice)
- Evidence: The `AmbiguousIfImplemented<Marker>` trait, its two marker structs and three blanket impls are re-declared six times (`grep 'AmbiguousIfImplemented<Marker>'` = 6).
- Proposal: Add one crate-level `#[cfg(test)] macro_rules! assert_no_debug_or_serialize { ($($t:ty),+) => … }` (e.g. in `lib.rs`), so each test becomes a one-line invocation.
- Spec impact: none.
- Risk: low.
- Estimated delta: -35 lines in slice (-55 repo-wide)

### AGT-21: Remove design-step numbering from composer test names
- Category: stale-doc
- Location: cafleet/src/coding_agent/profiles.rs:2805-3345
- Evidence: 16 tests are named `step5_composer_e1_…` through `step5_composer_e16_…`, citing a design document's step/evidence numbering rather than the behavior.
- Proposal: Drop the `step5_composer_eN_` prefix and keep the behavioral remainder (e.g. `composer_prompt_plan_retains_exact_selected_metadata_and_independent_coverage`). Do it as part of the AGT-01 move.
- Spec impact: none.
- Risk: low.
- Estimated delta: -0 lines

### AGT-22: 19 bundled native fixtures are embedded in the release binary but only tests use them
- Category: other (needs-verification)
- Location: cafleet/src/coding_agent/profiles/bundled.json (fixture entries); cafleet/src/coding_agent/profiles/catalog.rs:1147-1456
- Evidence: These ids appear in `bundled.json` only as their own `fixtures[].id`: no `fixture_ids` of any detection/composer/annotation capability, answer or continuation rule cites them. Claude (13): `auto-approved-done-1`, `auto-approved-running-1`, `auto-ask-allow-continuation-1`, `auto-ask-deny-continuation-1`, `auto-fresh-first-launch-notice-1`, `auto-fresh-onboarding-theme-1`, `auto-model-switch-question-1`, `auto-refusal-notice-expired-1`, `trust-dialog-1`, `access-allow-continuation-1`, `access-allow-running-1`, `access-deny-continuation-1`, `auto-env-setup-cancelled-idle-1`. Codex (5): `auto-approved-escalated-1`, `auto-denial-limit-warning-1`, `auto-reviewing-1`, `auto-sandbox-failure-1`, `auto-second-launch-idle-1`. OpenCode (1): `focus-always-1`. Together they are 53,957 of 214,597 fixture bytes, sha256-hashed and JSON-parsed on every `ProfileCatalog::bundled()`. Their consumers are test classification tables (e.g. profiles.rs:4684 `trust-dialog-1` → UNKNOWN) and unmet-gate tests (design-doc 0000196 line 182: "Unmet-gate tests (no plan writes a byte on it)").
- Proposal: Register these 19 in a `#[cfg(test)]` extension of the catalog definitions/embedded set (built on the AGT-04 helpers) instead of the shipped `bundled.json`/`BUNDLED_FIXTURES`. To confirm: the Director/user decides whether reviewed native captures with no capability citation should ship; SPEC.md:2510-2519 defines checks per bundled fixture but does not enumerate the set.
- Spec impact: none on CLI/JSON/schema — capabilities and plans are unchanged; only the embedded uncited set shrinks.
- Risk: medium — policy decision about what the bundle carries.
- Estimated delta: -95 lines (19 `bundled.json` entries, 76 `BUNDLED_FIXTURES` lines; -19 if AGT-04 lands first)
