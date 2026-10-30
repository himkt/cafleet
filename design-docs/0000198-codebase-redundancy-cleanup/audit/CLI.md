# Audit: CLI surface and support

## Summary

The slice has no test-only files, but `cafleet/src/cli/mod.rs` is ~92% tests (1,694 of 1,845 lines), with five test modules that cover commands owned by `member.rs`, `runtime.rs` and `setup.rs`. Several of those tests repeat the same behavior: five near-identical "refused database bytes stay unchanged" tests, mirrored inspect/respond accept/reject loops, a second clap `Harness` in `member.rs`, and overlapping OpenCode preset and "former base" tests in `assets.rs`, `doctor.rs` and `config_dir.rs`. On the production side, the biggest structural win is making the library modules `pub(crate)`. The only consumer of the library is `main.rs`, so today's blanket `pub mod` stops rustc's `dead_code` lint from finding unused items anywhere in the crate. Smaller wins: `config_dir.rs` resolvers collapse into one (`ResolvedDir` and three per-agent functions duplicate `CodingAgent::config_variable`), the `format_utc(now_utc())` pair (34 sites) and the random-hex nonce code become shared helpers, and `prompt`/`exec` in `member.rs` share one body. Estimated removable: ~630 lines inside the slice, plus ~110 more across the crate if the shared test `Settings` constructor and hex helpers are adopted everywhere.

## Test placement inventory

None in slice. Every test in the slice is already an inline `#[cfg(test)]` module. The slice has no `*_tests.rs` files, test folders or fixture files. Tests in `assets.rs` read the build-generated `SKILLS`/`PRESETS` tables, which are runtime data embedded by `build.rs`, not test fixtures.

Within-file misplacement (inline test modules that sit in the wrong implementation file) is reported as CLI-02.

## Findings

### CLI-01: Library-wide `pub` visibility hides dead code from the compiler
- Category: dead-code
- Location: cafleet/src/lib.rs:1-23; cafleet/src/cli/mod.rs:2-4; cafleet/src/output/mod.rs:9-15
- Evidence: The only consumer of the `cafleet` library is `cafleet/src/main.rs:2` (`cafleet::cli::run()` plus `CafleetError::message`/`exit_code`). The package has no `tests/`, `benches/` or `examples/` targets (`git ls-files cafleet`). Even so, 15 of 21 modules are `pub mod`, and their items are `pub`. rustc's `dead_code` lint skips items reachable through a public module path, so `cargo clippy -D warnings` never reports unused functions or types in them. The crate has only one `#[allow(dead_code)]` (runtime/runner/terminal.rs:16), which suggests unused items are being hidden by visibility rather than by suppressions. Inside `cli`, the `pub(crate) mod fleet/helpers/member` declarations are referenced only from inside `cli` (grep for `cli::fleet|cli::helpers|cli::member` finds only `cli/` files).
- Proposal: Keep `pub mod cli` (for `run`) and `pub mod error` (`run`'s return type must stay public to satisfy `private_interfaces`), and make every other `lib.rs` module `pub(crate)`. Make `cli`'s submodules private (`mod fleet;` and so on). Then run `mise //cafleet:lint` and delete every item the new `dead_code` warnings name. This finding is the enabler for a crate-wide dead-code sweep, and every other auditor's slice benefits from it.
- Spec impact: none — visibility is not part of the CLI contract, and the binary's behavior is unchanged.
- Risk: low — compile-time only. needs-verification: the count of newly reported dead items is unknown until the lint runs.
- Estimated delta: -20 lines directly (visibility keywords, unused re-exports); the dead-code removal it unlocks is likely much larger (unmeasured).

### CLI-02: `cli/mod.rs` test modules live in the wrong implementation file and precede the production code
- Category: test-placement
- Location: cafleet/src/cli/mod.rs:9-1311 (four test modules before the production code at 1313-1453), cafleet/src/cli/mod.rs:1455-1845 (`parser_contracts`)
- Evidence: The production code in `cli/mod.rs` is 141 lines (CliArgs, Command, `run`, `invocation_connection*`, `dispatch`). The test modules are:
  - `launch_policy_parser_tests` (9-222): exercises `member create` flags and `member::run`.
  - `press_parser_tests` (223-309): exercises `member press` parsing plus `output::format_member_press` and `presentation::member_press`.
  - `runtime_parser_tests::permission_entry` (316-632): exercises `cli/runtime.rs` `PermissionCommand` value parsers.
  - `runtime_parser_tests::recovery_entry` (633-913): exercises `member recover`.
  - `runtime_parser_tests` body (915-1058): exercises `event show`, `input show` and `input reconcile` from `cli/runtime.rs`.
  - Help-text and `__member-run` tests (1060-1310), plus `parser_contracts` (1455-1845): these belong to `mod.rs`. The exception is `setup_rejects_unknown_options_positionals_and_agent_values` (1815-1844), which belongs in `setup.rs`, where `parser_uses_enum_choices_and_supports_repeated_multi_value_selection` (setup.rs:105-132) already tests the same parser.

  The modules carry three separate parse/error helpers that do the same thing: `usage_error` (915-920), `parse` + `assert_error` (1513-1527), and the `member.rs` `Harness`/`parse` (member.rs:1827-1835).
- Proposal: Move each module into the `#[cfg(test)] mod tests` of the file that owns the command:
  - `member.rs`: launch policy, press parsing, recovery entry.
  - `runtime.rs`: permission entry, event/input read and reconcile.
  - `setup.rs`: setup parser rejection.
  - `mod.rs`: global help, `__member-run`, option scoping and dispatch-level storage tests, placed after the production code.

  Keep one `#[cfg(test)] pub(super) fn parse_cli(args) -> Result<Command, clap::Error>` helper in `cli/mod.rs` and have every moved module use it. This removes `usage_error`, `assert_error`'s duplicate parse and the `Harness` struct.
- Spec impact: none — test-only relocation.
- Risk: low — `CliArgs`/`Command` are module-private, so moved tests reach them through the `pub(super)` helper or `super::super`.
- Estimated delta: -60 lines (duplicate helpers and repeated `use` blocks)

### CLI-03: Five copies of the "refused database stays byte-identical" test
- Category: redundant-test
- Location: cafleet/src/cli/mod.rs:886-912 (`runner_recovery_entry_refused_registry_preserves_wal_database_and_sidecar_bytes`), cafleet/src/cli/mod.rs:1030-1058 (`runtime_read_dispatch_preserves_refused_database_bytes`), cafleet/src/cli/mod.rs:1460-1503 (`ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes`), cafleet/src/cli/setup.rs:183-201 (`foreign_ahead_and_malformed_databases_are_unchanged_on_refusal`), cafleet/src/cli/setup.rs:203-229 (`refused_wal_databases_and_sidecars_remain_byte_identical_with_a_held_reader`)
- Evidence: The three `cli/mod.rs` tests drive the same guard: every non-setup/doctor/press/permission command goes through the `mut command` arm of `dispatch` (mod.rs:1432-1433), which calls `invocation_connection` → `invocation_connection_with_deadline` (mod.rs:1368-1400). Each test rebuilds the same WAL setup (`PRAGMA journal_mode=WAL`, a mutation, `BEGIN; SELECT * FROM refinery_schema_history`), the same `["", "-wal", "-shm"]` byte snapshot (mod.rs:898, mod.rs:1480, setup.rs:218; the only three sites in the crate) and the same compare. Their mutation lists overlap (`application_id=42`, renamed columns). In `setup.rs`, the non-WAL test is a strict subset of the WAL test (same three mutations; the WAL test also checks the main file). Its only extra assertion is that `.claude` does not exist, but `prepare_database` alone is called and it never touches `.claude`.
- Proposal: Add one `#[cfg(test)] fn wal_snapshot(db_path) -> [(PathBuf, Vec<u8>); 3]` helper next to `db::connect` in `db/mod.rs`, since `db` owns the storage format. Replace the three `cli/mod.rs` tests with one table-driven test over (mutation × command), with commands `fleet list`, `member show`, `message poll`, `member recover`, `event show` and `input show`. In `setup.rs`, delete `foreign_ahead_and_malformed_databases_are_unchanged_on_refusal`.
- Spec impact: none — the same guarded behaviors stay asserted.
- Risk: low.
- Estimated delta: -90 lines

### CLI-04: Mirrored accept/reject loops for `permission inspect` and `permission respond`
- Category: redundant-test
- Location: cafleet/src/cli/mod.rs:394-491, cafleet/src/cli/mod.rs:509-531
- Evidence:
  - `..._inspect_accepts_positive_id_boundaries_and_json_selection` (395-409) and `..._respond_accepts_positive_id_epoch_boundaries_and_json_selection` (411-431) are the same triple loop over different fixture/field lists. `..._inspect_rejects_invalid_assigned_ids` (451-467) and `..._respond_rejects_invalid_assigned_ids_and_epochs` (469-491) duplicate each other the same way.
  - Every test starts with `accepted(&respond())` / `accepted(&inspect())` (412, 434, 452, 470, 534, 543, 563, 581, 605, 619). `required()` (493) runs the same check again.
  - All of these fields use one value parser, `positive_id` (cli/runtime.rs:9-18).
- Proposal: Use one test that iterates `[(inspect(), INSPECT_IDS), (respond(), RESPOND_IDS)]` and checks both the accepted and the rejected value lists. Drop the repeated baseline `accepted` calls; one fixture-sanity test is enough.
- Spec impact: none — same value sets asserted.
- Risk: low.
- Estimated delta: -80 lines

### CLI-05: Test `Settings` built field-by-field at every call site
- Category: duplication
- Location: in slice: cafleet/src/cli/mod.rs:62, 274, 846, 1039, 1293, 1469; cafleet/src/cli/member.rs:984, 1198, 1574, 1615; cafleet/src/cli/setup.rs:97-103; cafleet/src/cli/runtime.rs:356. Crate-wide: 22 `max_text_len: ` struct literals in test code plus 7 `Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL")…)` test calls (grep of `max_text_len: ` and `Settings::from_lookup`).
- Evidence: Each site writes out `Settings { database_url: format!("sqlite:///{}", …), max_text_len: 200, multiplexer: … }`, or wraps `from_lookup` around a single database URL. These sites encode the production default `200` (config.rs:31) by hand.
- Proposal: Add `#[cfg(test)] impl Settings { pub(crate) fn for_database(path: &Path, multiplexer: Option<&str>) -> Settings }` to `config.rs`. It goes through `from_lookup` so the default text limit has one source. Replace every test construction with it. The one `max_text_len: 1000` site (cli/mod.rs:276) does not depend on the value, so it can use the helper too.
- Spec impact: none — test-only.
- Risk: low.
- Estimated delta: -35 lines in slice; -110 lines crate-wide

### CLI-06: `config_dir.rs` duplicates the per-agent variable mapping and wraps a lone `PathBuf`
- Category: over-abstraction
- Location: cafleet/src/config_dir.rs:7-28, 40-54, 63-73; cafleet/src/coding_agent/mod.rs:575-589; cafleet/src/cli/doctor.rs:9, 178, 237, 255; cafleet/src/assets.rs:22
- Evidence:
  - `ResolvedDir { path }` has one field. All four consumers immediately read `.path` (assets.rs:22, doctor.rs:178/219/255, coding_agent/mod.rs:653).
  - `claude_config_dir`, `codex_home` and `opencode_preset_base` each repeat a variable name that `CodingAgent::config_variable()` already defines (coding_agent/mod.rs:575-581). `CodingAgent::config_dir` (583-589) then maps the agent back to those three functions. Their only other callers are the `config_dir.rs` tests.
  - `opencode_skills_base` is used both as the OpenCode default (config_dir.rs:27) and for the skills directory (assets.rs:24).
- Proposal: Replace the three resolvers and `ResolvedDir` with a single `pub(crate) fn resolve(var: &str, env, default: PathBuf) -> Result<PathBuf, CafleetError>`. `CodingAgent::config_dir` calls it with `self.config_variable()` and a per-agent default (`.claude`, `.codex`, `opencode_skills_base(home)`), and returns `PathBuf`. Rewrite the `ENV_RESOLVERS` table tests (config_dir.rs:63-135) to iterate `CodingAgent::all()`, so every agent added later is covered automatically.
- Spec impact: none — same variables, defaults and the error string `"{var} must be an absolute path (got '{value}')"`.
- Risk: low.
- Estimated delta: -35 lines

### CLI-07: `time::now_utc` only forwards, and `format_utc(now_utc())` is repeated at 34 sites
- Category: duplication
- Location: cafleet/src/time.rs:5-7; call sites: broker/{fleets,members,messaging,asset_installs}.rs, broker/runtime/{mod,inputs,permissions,events}.rs, delivery.rs:311, runtime/runner/{artifacts,creation,input_client,input,observation,service}.rs, runtime/runner/recovery/{replay,exceptional}.rs, runner_tests/control_service/observation_services.rs:12 (34 matches for `format_utc((crate::time::)?now_utc())`)
- Evidence: `now_utc()` returns `Utc::now()` and nothing else. Its uses outside a `format_utc` pair: members.rs:242, cli/member.rs:907, and 2 test sites. All the other calls exist only to produce the canonical storage stamp, and many sit inside one-line SQL `params![…]` lists that exceed rustfmt width, e.g. creation.rs:184 and service.rs:894.
- Proposal: Replace `now_utc` with `pub(crate) fn now_stamp() -> String { format_utc(Utc::now()) }`. Call `Utc::now()` directly at the four raw-instant sites.
- Spec impact: none — identical timestamp format.
- Risk: low — mechanical.
- Estimated delta: -10 lines (plus shorter SQL parameter lines)

### CLI-08: `member prompt` and `member exec` duplicate the same command body
- Category: duplication
- Location: cafleet/src/cli/member.rs:789-832 (`prompt`), cafleet/src/cli/member.rs:834-884 (`exec`)
- Evidence: Both functions run the same steps: `with_database_budget(load_member(.., false))`, then a Director self-target refusal (differing only in the `"prompt"` / `"exec in"` wording), then `require_pane`, then `dispatch_input(.., kind, label, ..)`, then a JSON/text `emit` that appends `"\nInput: {id}"` when present.
- Proposal: Extract `fn dispatch_to_member(conn, settings, member_id, text, kind, label, director_refusal, deadline) -> Result<(MemberRecord, String, Option<i64>), CafleetError>` for the shared steps. `prompt` and `exec` keep only their validation and their payload/text shape.
- Spec impact: none — identical error strings, JSON key order and text.
- Risk: low.
- Estimated delta: -25 lines

### CLI-09: Forwarding wrappers and repeated multiplexer error mapping in the CLI
- Category: duplication
- Location: cafleet/src/cli/helpers.rs:5; cafleet/src/cli/mod.rs:1368-1370; cafleet/src/cli/member.rs:404-406, 619-623, 894-896
- Evidence:
  - `helpers.rs` re-exports `crate::runtime::resolve_mux` (`pub use`). Its importers (doctor.rs:39, fleet.rs:5, member.rs:6) could import the original path.
  - `invocation_connection` (mod.rs:1368-1370) only forwards to `invocation_connection_with_deadline` with `Deadline::new()`. It has one production caller (mod.rs:1433) and one test caller (mod.rs:1509).
  - In member.rs, `resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?` followed by `mux.ensure_available().map_err(|e| CafleetError::App(e.to_string()))?` appears three times (create, delete, capture). `crate::input` has the same pattern (input.rs:1421-1424).
- Proposal: Delete the re-export and import `crate::runtime::resolve_mux` directly. Inline `invocation_connection`. Add `fn available_mux(settings) -> Result<AnyMultiplexer, CafleetError>` to `cli/helpers.rs` for the three member.rs sites. `fleet create` and `dispatch_input` keep their own error mappings because their messages differ.
- Spec impact: none — identical error messages.
- Risk: low.
- Estimated delta: -15 lines

### CLI-10: Random 32-byte hex nonce read written twice in `member.rs`; byte-to-hex loop repeated crate-wide
- Category: duplication
- Location: cafleet/src/cli/member.rs:455-462, 711-717; cafleet/src/runtime/runner/service.rs:120-124; hex loop: 28 `.map(|byte| format!("{byte:02x}"))` sites (grep), e.g. capture.rs:23, input.rs:543, broker/mod.rs:620, broker/runtime/evidence.rs:261, broker/runtime/permissions.rs:144, coding_agent/profiles/catalog.rs:1462, runtime/runner/native_capture/files.rs:22
- Evidence: Both member.rs sites open `/dev/urandom`, `read_exact` 32 bytes and hex-encode them; only the error prefix differs. `SystemRequestTokens::fill` (service.rs:121) is a third copy of the read. The hex encoding of digests and nonces appears in production code 12 times and in tests 16 times.
- Proposal: Add `pub(crate) fn hex(bytes: &[u8]) -> String` and `pub(crate) fn random_hex32() -> std::io::Result<String>` to one support module, e.g. `runtime/system.rs` next to `SystemProbe`. Use them at all sites. Callers keep their own error messages through `map_err`.
- Spec impact: none — same 64-character lowercase hex.
- Risk: low.
- Estimated delta: -12 lines in slice; -50 lines crate-wide

### CLI-11: `member create` wraps an always-present runtime snapshot in `Option`
- Category: other
- Location: cafleet/src/cli/member.rs:521-539
- Evidence: `let runtime = Some(broker::runtime::runtime_snapshot(...)?)` is followed by `runtime.as_ref().map(presentation::runtime_snapshot)` and `if let Some(runtime) = &runtime`. The `None` branch can never run.
- Proposal: Bind the snapshot directly. Emit `"runtime": presentation::runtime_snapshot(&runtime)` and append the runtime text unconditionally.
- Spec impact: none — `serde_json` serializes `Some(v)` as `v`, so the JSON bytes and key order are unchanged.
- Risk: low.
- Estimated delta: -4 lines

### CLI-12: Near-identical clap value-parser adapters
- Category: duplication
- Location: cafleet/src/cli/runtime.rs:20-26 (`evidence_reference`), 130-142 (`permission_reference`, `request_token`, `operation_key`); cafleet/src/cli/member.rs:197-203 (`recovery_reference`)
- Evidence: The three runtime.rs adapters share one body, `validate_x(value).map_err(to_string)?; Ok(value.into())`. `evidence_reference` and `recovery_reference` are the same nonblank check with different messages. Cross-slice note (needs-verification by the runner auditor): the same token, key and reference fields are validated again in `input_client::validate_respond` (input_client.rs:853-871). That function's callers are input_client.rs:1099 and input.rs:4566, and `respond_permission`'s only caller is the CLI (cli/runtime.rs:262), so the second validation may be redundant.
- Proposal: Use one adapter, `fn checked(validate: fn(&str) -> Result<(), CafleetError>) -> impl Fn(&str) -> Result<String, String> + Clone`, plus one `fn nonblank(message: &'static str)` for the two nonblank checks. Each keeps its current message.
- Spec impact: none — same usage messages and exit code 2.
- Risk: low.
- Estimated delta: -10 lines

### CLI-13: The permission formatter re-spells `Visibility`'s stored names
- Category: duplication
- Location: cafleet/src/output/runtime.rs:125-130; cafleet/src/runtime/runner/episodes.rs:21-28
- Evidence: `Visibility` derives `Serialize` with `rename_all = "snake_case"`, which already defines `visible`, `obscured`, `not_visible` and `unknown`. The text formatter writes out a second, hand-maintained match for the same names. Every other runtime enum in the formatter uses `.as_str()`, which the `runtime_enum!` records provide.
- Proposal: Give `Visibility` an `as_str()`, either through the existing `runtime_enum!` pattern or a small impl in `episodes.rs` (runner slice), and call it from the formatter.
- Spec impact: none — same strings.
- Risk: low.
- Estimated delta: -5 lines

### CLI-14: Text formatting split between `output::formatters` and inline CLI closures
- Category: other
- Location: cafleet/src/cli/fleet.rs:63-80, 86-97; cafleet/src/cli/member.rs:628-632, 820-830, 869-882; cafleet/src/cli/message.rs:134-139, 169-178, 186-191
- Evidence: `output/formatters.rs` holds the formatters for messages, members, the roster and fleet create/delete. The `fleet list` table, the `fleet show` block, the `member delete` lines, the `prompt`/`exec` lines and the `message wait`/`ack`/`send` headers are built in closures inside the CLI handlers instead. `output/formatters.rs` has tests for its formatters; the inline ones have none.
- Proposal: Move the inline text builders into `output/formatters.rs` as `format_fleet_list` and similar functions, so text layouts have one home next to their tests. This is consolidation, not deletion.
- Spec impact: none — identical text.
- Risk: low.
- Estimated delta: 0 lines (one owner for text layouts)

### CLI-15: Stale `rerun-if-changed` on the docs tree in `build.rs`
- Category: stale-doc
- Location: cafleet/build.rs:10
- Evidence: `build.rs` embeds only `../skills` and `../presets` (lines 13-18). No `include_str!`/`include_bytes!` in `cafleet/src` references `docs/` (grep). The `../docs/docs` rerun trigger therefore rebuilds the crate on every docs edit for no reason.
- Proposal: Delete line 10.
- Spec impact: none.
- Risk: low.
- Estimated delta: -1 line

### CLI-16: `cafleet/rustfmt.toml` only repeats the manifest edition
- Category: other
- Location: cafleet/rustfmt.toml:1; cafleet/Cargo.toml:4
- Evidence: The file contains only `edition = "2024"`, which `Cargo.toml` already declares. `cargo fmt`, run by the `format` and `lint` tasks (cafleet/mise.toml:7-14), passes the manifest edition to rustfmt.
- Proposal: Delete `cafleet/rustfmt.toml`.
- Spec impact: none.
- Risk: low. needs-verification: confirm `mise //cafleet:lint` stays clean, and that no editor workflow calls bare `rustfmt` outside cargo.
- Estimated delta: -1 file / -1 line

### CLI-17: Asset-install tests repeat the generic per-agent assertions
- Category: redundant-test
- Location: cafleet/src/assets.rs:307-342 (`opencode_installation_prints_both_preset_targets_after_the_skills_line`), cafleet/src/assets.rs:451-455 (`embedded_presets_include_both_opencode_agents`)
- Evidence: `installation_prints_the_skills_line_then_one_preset_line_per_target_in_install_order` (267-305) already loops over every agent with a custom `OPENCODE_CONFIG_DIR`. For OpenCode it asserts exactly the two preset lines that the 307-342 test checks again. That test's only extra assertion, `asset_install_count == 1`, is implied by the successful-install record check at 236-243. The presence check at 451-455 is implied by `frontmatter_and_body`'s `.expect("embedded preset")` (165) in both preset tests and by the install tests' `lookup(PRESETS, source).unwrap()`.
- Proposal: Delete both tests.
- Spec impact: none — the same lines and presets stay asserted.
- Risk: low.
- Estimated delta: -42 lines

### CLI-18: "Former base" (`~/.opencode`) sentinel tests
- Category: redundant-test
- Location: cafleet/src/cli/doctor.rs:451-503 (`opencode_installation_at_the_former_base_is_not_the_resolved_installation`); cafleet/src/assets.rs:392-394, 443-448 (former-base setup and asserts inside `opencode_default_installation_uses_the_native_config_directory_for_skills_and_presets`); cafleet/src/config_dir.rs:144-147 (`assert_ne!(…, home.join(".opencode"))`)
- Evidence: These assertions name a preset location that no longer exists, keeping the history of a removed location in the tests. The global Removal rule treats sentinel-style tests for removed behavior as residue. Current behavior is already covered elsewhere:
  - The OpenCode default `~/.config/opencode` is asserted by config_dir.rs:79-85 and 150-156, and by assets.rs:395-420.
  - The installation-row mismatch path ("no installation recorded at …") is asserted by doctor.rs:393-398.
- Proposal: Delete the doctor test. In `assets.rs`, delete the `former` file setup and the two former-base assertions but keep the native-directory assertions. In `config_dir.rs`, delete the `assert_ne!`.
- Spec impact: none — current resolution stays asserted.
- Risk: low.
- Estimated delta: -65 lines

### CLI-19: Overlapping OpenCode preset tests in `doctor.rs`
- Category: redundant-test
- Location: cafleet/src/cli/doctor.rs:423-449, 505-526, 528-560, 562-593
- Evidence:
  - The "configuration fails → info `unavailable:`" path for both preset checks is asserted by `independent_failures_…` (350-360) and again by `opencode_preset_checks_are_info_when_…` (565-581).
  - The preset "error" status is asserted by `healthy_database_…` (399-400) and again by `missing_cafleet_ask_preset_…` (540-559).
  - `preset_checks(&checks).len() == 2` is asserted three times (448, 571, 592).
  - Each test rebuilds the same `Probe { home, executable, invalid }`.
- Proposal: Use one table-driven test over (config valid?, executable present?, installed presets), asserting each preset check's (name, status, detail prefix) in catalog order. Keep `independent_failures_…` and `healthy_database_…` for the cross-check behavior.
- Spec impact: none — same check names, statuses and detail strings.
- Risk: low.
- Estimated delta: -50 lines

### CLI-20: A second clap harness in `member.rs` repeats `cli/mod.rs` parse coverage
- Category: redundant-test
- Location: cafleet/src/cli/member.rs:1827-1883
- Evidence: `create_parses_the_supplied_assignment_and_inline_prompt` and `create_parses_file_input_and_backend_model_selection` parse `member create` through a local `Harness` derive. `cli/mod.rs` already covers both: inline and `--file` sources in `member_and_message_body_sources_are_exclusive` (1714-1768), and `--coding-agent`/`--model`/`--effort` in `runtime_policy_parser_keeps_omitted_selection_and_model_effort_boundaries` (134-158).
- Proposal: Delete both tests and `Harness`/`parse`. The name/description assertions can be added to the mod.rs inline-source arm in one line.
- Spec impact: none.
- Risk: low.
- Estimated delta: -55 lines

### CLI-21: Design-step provenance in test identifiers
- Category: stale-doc
- Location: cafleet/src/cli/mod.rs:228, 271, 293, 395-617 (18 occurrences of `step5_` / `step5_a0_` / `.p194-`), cafleet/src/cli/member.rs:938, 980 (`.p194-member-delete-`, `.m194-`)
- Evidence: The test names and tempdir prefixes encode an implementation-plan step and a design-doc number rather than the behavior under test. Project rules (code-quality "No unnecessary comments", the global Removal rule) keep provenance in git and design docs. The `tempdir_in("/tmp")` calls next to these prefixes are needs-verification: if they exist to keep Unix-socket paths short, that constraint should be stated once in a shared helper.
- Proposal: Rename the tests by behavior, e.g. `permission_respond_rejects_invalid_ids`, and drop the numeric prefixes. Route the short-path tempdir through one named helper, `short_tempdir()`, whose name carries the reason.
- Spec impact: none.
- Risk: low.
- Estimated delta: 0 lines

### CLI-22: Single-use generic helper and split rejection tests in `config.rs`
- Category: over-abstraction
- Location: cafleet/src/config.rs:28-33, 43-55, 117-133
- Evidence: `parse_numeric<T>` has one caller (`CAFLEET_MAX_TEXT_LEN`). The `non_integer_max_text_len_fails_loudly` and `negative_max_text_len_fails_loudly` tests differ only in their input.
- Proposal: Inline the parse into `from_lookup`, keeping the message `"{name} must be a non-negative integer (got '{raw}')"`. Merge the two rejection tests into one loop over `["twenty", "-1"]`.
- Spec impact: none — identical error string.
- Risk: low.
- Estimated delta: -12 lines

### CLI-23: Duplicate `HOME is not set` resolution in `setup` and `doctor`
- Category: duplication
- Location: cafleet/src/cli/setup.rs:22-24; cafleet/src/cli/doctor.rs:68-70
- Evidence: Both build `PathBuf::from(std::env::var("HOME"))` with the identical `CafleetError::App("HOME is not set")`. `SystemProbe::home_dir` (runtime/system.rs:442-443) reads the same variable but panics instead of returning an error. `config.rs:18-22` uses a different, contract-visible message and stays as it is.
- Proposal: Add one `pub(crate) fn home_dir() -> Result<PathBuf, CafleetError>` in `config_dir.rs` and use it from `setup` and `doctor`.
- Spec impact: none — same message.
- Risk: low.
- Estimated delta: -4 lines
