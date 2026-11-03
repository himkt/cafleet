# Remove Embedded Data Content Tests and Schema Migrations

**Status**: Approved
**Progress**: 20/26 tasks complete
**Last Updated**: 2026-11-03

## Overview

Remove the tests that restate the content of embedded data files instead of asserting what CAFleet code decides or produces, and revise the test convention so every test asserts code behavior. Replace the schema migration practice with one in-place baseline and a database file named after the cafleet version, so each version starts from a fresh database and the successor, behind/ahead and chain machinery disappears. Skills and presets stay embedded and their installation tests stay.

## Success Criteria

- [ ] Every test in § Audit result and § Database tests with action `delete` is absent; every `reduce`, `rename`, `move` and `adapt` row holds the listed result under its listed name; every `new` row exists.
- [ ] Helpers and constants left without callers by those edits are absent.
- [ ] Every installation-behavior test in `src/assets.rs` and the `hermes` test module remain unchanged.
- [ ] `migrations/` holds only `V1__baseline.sql`, and its schema contains the `hermes` backend value, the `member_runs.native_session_json` column and the `cafleet_database` table.
- [ ] With `CAFLEET_DATABASE_URL` unset and `HOME` set to a fresh directory, `cafleet setup` creates `$HOME/.local/share/cafleet/cafleet_<VERSION>.db`, where `<VERSION>` is the package version (the version field of `cafleet --version`, e.g. `0.31.1` from `cafleet 0.31.1`), and prints `Initialized database at PATH.`; a second run prints `Database already initialized at PATH.`; `cafleet doctor` reports `ok database: cafleet <VERSION>`.
- [ ] A file recorded by another cafleet version, a file whose ledger holds a row beyond the baseline, and a file with another application identity are each reported with the incompatible-format error by `cafleet setup`, `cafleet fleet list` and `cafleet doctor`, and keep their checksum.
- [ ] Outside `design-docs/`, no file mentions `cafleet_v9.db`, `V2__hermes_backend.sql`, `head_version`, `migration_chain`, `migrate_to_head`, `SchemaState::Behind`, `SchemaState::Ahead`, `database schema is outdated` or `is newer than this cafleet`.
- [ ] `.claude/rules/code-quality.md` § Test placement states that tests drive production code and assert none of the text, keys, counts, ids or layout of embedded data files, with a matching Forbidden-patterns bullet; its former § Database migrations is headed `## Database schema` and reads as in § Revised convention text; `.claude/rules/documentation-maintenance.md` names "the schema baseline" instead of "migration files".
- [ ] `docs/docs/contributing.md` carries the same test convention in its test paragraph.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.
- [ ] The test count reported by `mise //:test` equals the baseline minus 29: 31 tests are deleted and 2 are added; reduced, renamed, moved and adapted tests count as kept.

---

## Background

`src/assets.rs` holds two kinds of tests. The installation tests run `install_agent` against temporary directories and compare the installed files with the embedded `SKILLS` / `PRESETS` bytes, so they verify setup behavior. Six content tests instead parse the embedded preset files and assert their literal text, JSON key order, paragraph count and sentence wording. They exercise no CAFleet code beyond `lookup`, fail on every preset wording edit, and duplicate the preset files inside the test module. The same pattern appears in `src/coding_agent/catalog.rs`, where tests parse `bundled.json` and assert profile counts, rule-id sets and raw keys of the bundled profile catalog, and in `src/db.rs`, where tests assert the ids, count and order of the embedded migration files.

The test convention invites these tests: `.claude/rules/code-quality.md` § Test placement says "Tests may read the runtime data that production code embeds (`coding_agent/native/<backend>.json`, `bundled.json`, presets, migrations)", and § Database migrations requires a chain-guard test plus successor and exact-schema tests.

The schema itself carries a migration practice. The default database is `~/.local/share/cafleet/cafleet_v9.db` (`src/config.rs`). `migrations/` holds `V1__baseline.sql` and the hand-written successor `V2__hermes_backend.sql`, which rebuilds `member_runs` to admit the `hermes` backend and add `native_session_json`. `src/db.rs` derives a head version from the embedded chain, builds a reference schema per recorded version and classifies a file as `Missing`, `Incompatible`, `Behind`, `Head`, `Ahead` or `Unreachable`; setup migrates `Behind` files and the command guard reports `Behind` and `Ahead` with their own messages. Each successor costs a hand-written data-preserving rebuild and a module of preservation tests. Starting every cafleet version from a fresh database removes that cost.

Distributing skills or presets as GitHub release assets is out of scope; the embedding is unchanged.

---

## Specification

### Embedded data files

The content-test audit subject is every file the binary embeds:

| Mechanism | Files |
|---|---|
| `build.rs` `SKILLS` / `PRESETS` tables | every file under `skills/` and `presets/` |
| `include_str!` / `include_bytes!` in `src/coding_agent/catalog.rs` | `src/coding_agent/bundled.json` and `src/coding_agent/native/<backend>.json` |
| The schema baseline in `src/db.rs` | `migrations/V1__baseline.sql` |

### Audit criterion

The classification depends on where an assertion's **actual** value comes from; the expected value does not decide it.

- A **content assertion** is one whose actual value is an embedded data file itself: a raw read (`include_str!`, `include_bytes!`, `lookup`, `SKILLS`, `PRESETS`, `serde_json` over the file, a filesystem read of the source tree) or a pass-through that returns stored data unchanged. Negative and property checks on raw data — "no rule declares key X", "no screen line contains Y", "every profile has key Z" — are content assertions too.
- A **behavior assertion** is one whose actual value is output of production logic: detecting or classifying a screen, accepting or rejecting a definition, selecting a profile or answer plan by version, installing bytes at a target, initializing or classifying a database. It stays a behavior assertion when its expected value names ids or text from the file.

| Test shape | Action |
|---|---|
| Every assertion is a content assertion | Delete the test |
| Content assertions mixed with behavior assertions | Reduce: delete the content assertions and the setup only they use |
| Every assertion is a behavior assertion | Keep |

A reduced test is renamed to the behavior it still asserts when its current name describes a removed assertion. A reduced test whose remaining assertions are already made by another kept test is deleted instead. In either case, helpers and constants left without callers are deleted.

Consequences:

- Installation tests that compare installed bytes with the embedded bytes are behavior assertions and stay.
- Tests that build a modified definition from `bundled.json` and pass it to `ProfileCatalog::from_definitions`, `from_bundled_definitions` or the detector are behavior tests and stay (for example `bundled_guard_rejects_every_unreviewed_change_to_each_bundled_fixture` and the detector's `shell_rule` input helper).
- Database tests whose actual value is an initialized database — `PRAGMA application_id`, `refinery_schema_history` rows, the recorded version, table and column shapes, constraint enforcement, classification — are behavior assertions and stay. Tests whose subject is migration machinery this change removes (successor migrations, behind/ahead classification, chain numbering) are deleted with that machinery; § Database tests lists them.

### Audit result

The audit covered every `#[cfg(test)]` module under `src/`. Selected tests in `src/assets.rs` and `src/coding_agent/catalog.rs`; `src/db.rs` is covered by § Database tests:

| File | Test | Action | Remaining assertions or reason |
|---|---|---|---|
| `src/assets.rs` | `codex_rules_preset_is_the_single_broad_cafleet_allow` | delete | — |
| `src/assets.rs` | `opencode_shell_maps_end_with_the_broad_cafleet_allow_and_hold_no_director_only_entry` | delete | — |
| `src/assets.rs` | `cafleet_ask_preset_frontmatter_differs_only_in_description_and_shell_base` | delete | — |
| `src/assets.rs` | `opencode_preset_bodies_hold_three_unwrapped_paragraphs_and_one_trailing_newline` | delete | — |
| `src/assets.rs` | `opencode_preset_authority_paragraph_routes_director_only_needs_to_the_director` | delete | — |
| `src/assets.rs` | `cafleet_ask_preset_body_replaces_only_the_permission_paragraph` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_fixtures_carry_no_personal_paths_or_account_names` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_claude_and_codex_operations_declare_neither_row_option` | delete | — |
| `src/coding_agent/catalog.rs` | `runtime_profile_bundled_entries_are_identified_by_backend_and_min_version` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_claude_detection_and_catalog_cite_the_unwrapped_wrapped_and_boxed_native_captures` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_claude_auto_profile_is_the_exact_2_1_302_profile_with_its_rule_groups` | delete | Its behavior checks are already made by kept tests: the `PERMISSION_RULE` answer-plan stage bytes `1` / `4` by `single_byte_answer_plans_are_the_measured_stages`, and the `capabilities` checks by `each_bundled_profile_declares_detection_idle_composer_both_operations_and_both_answers` |
| `src/coding_agent/catalog.rs` | `bundled_claude_auto_fixtures_produce_exactly_their_primary_and_annotation_sets` | reduce, rename to `bundled_claude_auto_fixtures_produce_their_primary_and_annotation_sets` | The `CLASSIFICATION` loop over detector observations and annotation blocks; the raw fixture-id set equality goes |
| `src/coding_agent/catalog.rs` | `bundled_claude_read_outside_rule_joins_detection_with_its_detection_fixtures` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_claude_read_outside_answers_never_select_a_persistent_choice` | delete | Its only behavior check (plan stages avoid `1`, `2`, Enter, Tab, Escape) is implied by `bundled_claude_read_outside_dialog_answers_allow_with_4_and_deny_with_3_ungated`, which asserts the single stage bytes exactly |
| `src/coding_agent/catalog.rs` | `bundled_codex_auto_profile_is_the_exact_0_163_0_profile_with_its_rule_groups` | delete | Its only behavior check (`capabilities(Codex, CODEX_VERSION).min_version`) is made by `each_bundled_profile_declares_detection_idle_composer_both_operations_and_both_answers` |
| `src/coding_agent/catalog.rs` | `bundled_codex_auto_fixtures_produce_exactly_their_primary_and_annotation_sets` | reduce, rename to `bundled_codex_auto_fixtures_produce_their_primary_and_annotation_sets` | The `CLASSIFICATION` loop and the predecessor-fixture loop over detector observations; the raw fixture-id set equality goes |
| `src/coding_agent/catalog.rs` | `bundled_opencode_definitions_declare_the_form_reason_dialog_and_idle_rules` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_opencode_form_rule_matches_the_wide_and_split_layouts` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_opencode_reject_reason_reads_the_placeholder_through_one_region_matcher` | delete | — |
| `src/coding_agent/catalog.rs` | `bundled_opencode_compact_rules_mirror_the_wide_and_split_rules_without_a_margin` | delete | — |
| `src/coding_agent/catalog.rs` | `annotation_bundled_profiles_each_declare_the_annotations_capability` | delete | Its only behavior check (`ProfileCatalog::bundled().is_ok()`) is made by every test that unwraps the bundled catalog |
| `src/coding_agent/catalog.rs` | `bundled_profiles_declare_their_initial_boundary_and_keep_capture_versions` | delete | — |

Totals: 20 deleted, 2 reduced. Helpers expected to lose every caller, deleted with them: in `src/assets.rs`, `frontmatter_and_body`, `paragraphs`, `key_order` and `OPENCODE_PRESETS`; in `src/coding_agent/catalog.rs`, the Claude module's `bundled_claude_profile` and `ids`, and the OpenCode module's `definitions`, `profile`, `rule`, `matcher` and `id_set`. Lint is the authority on which helpers become unused. The `assert_key_order` helper in `src/broker/runtime/mod.rs` is a separate function that stays.

### Database format

Each cafleet version owns one database file and one schema baseline:

| Aspect | Behavior |
|---|---|
| Default URL | `sqlite:///` + `$HOME/.local/share/cafleet/cafleet_<VERSION>.db`, where `<VERSION>` is the package version `crate::cli::VERSION` (`env!("CARGO_PKG_VERSION")`, the version field of `cafleet --version`, e.g. `0.31.1` from `cafleet 0.31.1`), for example `cafleet_0.31.1.db` |
| Explicit `CAFLEET_DATABASE_URL` | Used verbatim; no version substitution |
| Schema source | `migrations/V1__baseline.sql`, the only file in `migrations/`, edited in place for every schema change |
| Application identity | `PRAGMA application_id` `1128351289` (`0x43414639`, ASCII `CAF9`), unchanged |
| Version record | One `cafleet_database` row holding the `<VERSION>` of the binary that initialized the file |
| Ledger | `refinery_schema_history` holds exactly the row `(1, 'baseline')` |
| Older files | Never opened for writes, migrated, renamed or deleted; the user deletes them by hand |

`V1__baseline.sql` absorbs `V2__hermes_backend.sql`: the `member_runs.backend` check becomes `CHECK(backend IN ('claude','codex','opencode','hermes'))`, `native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object'))` is appended as the last `member_runs` column, and this table follows `runtime_database_identity`:

```sql
CREATE TABLE cafleet_database (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    cafleet_version TEXT NOT NULL CHECK(length(trim(cafleet_version)) > 0)
);
```

`V2__hermes_backend.sql` is deleted. The SQL file cannot know the binary's version, so the binary builds the refinery migration at runtime instead of through `embed_migrations!`:

```rust
const BASELINE_SQL: &str = include_str!("../migrations/V1__baseline.sql");

fn baseline() -> refinery::Migration {
    let sql = format!(
        "{BASELINE_SQL}\nINSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, '{}');\n",
        crate::cli::VERSION
    );
    refinery::Migration::unapplied("V1__baseline", &sql)
        .expect("the embedded baseline has a valid migration name")
}

pub(crate) fn initialize(conn: &mut Connection) -> Result<(), CafleetError> {
    refinery::Runner::new(&[baseline()])
        .set_grouped(true)
        .run(conn)
        .map_err(|e| CafleetError::App(format!("database initialization failed: {e}")))?;
    Ok(())
}
```

The version row is therefore written in the same transaction as the baseline, and a failed initialization leaves no identity, table, ledger or version row.

### Classification

`SchemaState` keeps four states:

| State | Condition | `require_current` result |
|---|---|---|
| `Missing` | `application_id` 0 and no user table | `no cafleet database; run 'cafleet setup'` |
| `Current` | Every check below passes | `Ok(())` |
| `Incompatible { path }` | Any check below fails | `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` |
| `Unreachable { cause }` | A read fails | The read's cause |

`classify_schema(conn)` and `classify_selected_schema(conn, database_url)` take no head argument and check, in order:

1. `PRAGMA application_id` equals `APPLICATION_ID`.
2. `refinery_schema_history` exists and its `(version, name)` rows are exactly `[(1, "baseline")]`. A file with a successor row, such as a `cafleet_v9.db` at version 2, is incompatible.
3. `schema_matches(conn)` compares required tables, columns, foreign keys and indexes with a reference built by `initialize` on an in-memory connection; auxiliary tables, indexes and triggers stay compatible, as today.
4. `SELECT cafleet_version FROM cafleet_database WHERE singleton_id=1` returns exactly `crate::cli::VERSION`; another value or no row is incompatible.

Any read error during classification, including the `Missing` probe and the step-3 comparison, is `Unreachable`, as today. Check 3 keeps catching a file of the same `<VERSION>` whose baseline was edited in place during development.

Removed from `src/db.rs`: `mod embedded`, `migration_chain`, `head_version`, `migrate_to_head` (replaced by `initialize`), `recorded_version`, the version argument of `reference_schema` / `schema_matches_version` (renamed `schema_matches`) and its ahead branch, the `Behind`, `Head` and `Ahead` states and their messages, and `test_support::migrate_to_version`. `test_support::migrated_conn` keeps its name and calls `initialize`.

### Commands

| Surface | Behavior |
|---|---|
| `cafleet setup` | `Missing`: run `initialize`, re-classify with `require_current`, print `Initialized database at PATH.`. `Current`: print `Database already initialized at PATH.`. Any other state: fail with its `require_current` error before database mutation, asset installation or asset-install recording |
| Command guard (`invocation_connection` in `src/cli/mod.rs`, the runner in `src/runtime/service.rs`) | Require `Current`; other states fail with their `require_current` error |
| `cafleet doctor` `database` check | `Current`: `ok`, detail `cafleet <VERSION> at PATH`; otherwise `error` with the `require_current` message |
| `cafleet doctor` `database` summary | `Current`: `ok database: cafleet <VERSION>`; otherwise `error` with the check's cause |

`PATH` is the file path of the selected URL (`db::database_path`). Every other `migrate_to_head` caller, including test code and `src/runtime/observation.rs`, calls `initialize`.

A live fleet does not survive an upgrade: its members carry the old URL in their spawn environment and the new binary refuses the old file as incompatible. The user finishes and deletes active fleets before upgrading cafleet; the old file is left untouched.

### Database tests

| File | Test | Action | Result |
|---|---|---|---|
| `src/db.rs` | `migration_chain_is_contiguous_from_one_with_a_single_baseline` | delete | Chain numbering no longer exists |
| `src/db.rs` | `ahead_versions_validate_the_ledger_without_requiring_the_head_application_shape` | delete | The ahead state no longer exists |
| `src/db.rs` | `hermes_backend_successor::the_chain_is_the_baseline_then_the_hermes_backend_successor` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_migrates_a_populated_v1_database_to_the_current_head` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_preserves_every_member_run_and_every_other_table_row` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_keeps_the_v1_columns_in_order_and_appends_the_native_session_column` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_preserves_the_run_index_and_every_foreign_key_to_and_from_member_runs` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_enforces_the_active_run_index_and_the_preserved_foreign_keys` | delete | Successor removed; `every_nonterminal_status_enforces_one_live_run_per_member` and `runtime_foreign_keys_retain_history_and_reject_orphans` cover the baseline |
| `src/db.rs` | `hermes_backend_successor::successor_continues_run_ids_after_the_preserved_autoincrement_sequence` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_preserves_the_v1_run_check_constraints` | delete | Successor removed; `run_sql_domains_and_json_shapes_reject_corrupt_values` covers the baseline |
| `src/db.rs` | `hermes_backend_successor::a_fresh_database_and_a_migrated_v1_database_share_the_head_run_shape` | delete | Successor removed |
| `src/db.rs` | `hermes_backend_successor::successor_backend_check_accepts_hermes_and_every_existing_backend_only` | move to `runtime_tests::run_backend_check_accepts_every_supported_backend_only` | Against an initialized database: `hermes`, `claude`, `codex`, `opencode` insert; `gemini`, `Hermes`, `""`, `"hermes "` fail; the "baseline still refuses hermes" assertion goes |
| `src/db.rs` | `hermes_backend_successor::successor_native_session_column_accepts_only_null_or_a_json_object` | move to `runtime_tests::native_session_column_accepts_only_null_or_a_json_object` | The same accepted and refused values against an initialized database |
| `src/db.rs` | `baseline_is_the_first_migration_with_the_v9_identity_and_shape` | reduce, rename to `baseline_initialization_records_the_identity_ledger_version_and_shape` | Drop the `migration_chain()` assertion; add `cafleet_database` to the table list and `native_session_json` to the `member_runs` columns; assert the `cafleet_database` row equals `crate::cli::VERSION`; `schema_matches(&conn)` holds |
| `src/db.rs` | `classification_preserves_missing_behind_ahead_identity_and_sql_causes` | reduce, rename to `classification_preserves_missing_current_identity_and_sql_causes` | Drop the future-column / ahead-ledger step; `Missing`, `Current`, `Incompatible` for `application_id=1` and `Unreachable` for a renamed ledger column remain |
| `src/db.rs` | `repeated_migration_at_head_preserves_all_application_rows` | rename to `repeated_initialization_preserves_all_application_rows` | Calls `initialize` twice; rows and ledger unchanged; `require_current` passes |
| `src/db.rs` | `required_schema_components_and_identity_are_refused` | adapt | Add the damages `DROP TABLE cafleet_database` and `DELETE FROM cafleet_database` |
| `src/db.rs` | `a_v8_family_file_is_incompatible_and_left_byte_identical`, `selected_reference_rechecks_ddl_and_accepts_auxiliary_objects`, `structural_reference_checks_required_keys_types_defaults_and_foreign_keys`, `connection_pragmas_and_foreign_keys_protect_real_writes`, `failed_baseline_initialization_rolls_back_identity_tables_and_ledger`, `runtime_tests::database_instance_identity_is_singleton_stable_and_distinct`, `runtime_tests::current_schema_guard_requires_each_runtime_table` | adapt | Call `initialize`, `schema_matches(&conn)` and the head-free classifiers; assertions unchanged |
| `src/db.rs` | `another_versions_file_is_incompatible_and_left_byte_identical` | new | Initialize a file, set `cafleet_version` to `0.0.0`, close it; `classify_selected_schema` is `Incompatible` with the file path, `require_current` gives the incompatible message, and the file bytes are unchanged |
| `src/db.rs` | `a_ledger_row_beyond_the_baseline_is_incompatible` | new | Initialize, insert a `refinery_schema_history` row with version 2; `classify_schema` is `Incompatible` |
| `src/config.rs` | `default_database_url_points_at_expanded_home_cafleet_v9` | rename to `default_database_url_names_the_running_version` | Expected URL ends in `cafleet_{VERSION}.db`; the `~`-expansion assertion stays |
| `src/cli/setup.rs` | `healthy_database_distinguishes_absent_stale_current_and_failed_install_queries`, doctor tests using `database_line` | adapt | Use `SchemaState::Current`; `database_line` returns `ok database: cafleet {VERSION}` |

Totals: 11 deleted, 2 moved, 2 reduced, 2 new. Helpers expected to lose every caller, deleted with them: the `hermes_backend_successor` module's `V1_RUN_COLUMNS`, `RUN_REFERENCES`, `insert_run`, `populated_v1`, `v1_run_rows`, `ddl`, `foreign_keys`, `migrated`, `SEQUENCES`, `unchanged_table` and `migrated_in_place`, and the test module's `at_version` (replaced by a helper that returns an initialized in-memory connection). A moved test needs a run-insert helper that sets `backend` and `native_session_json`; extend the `runtime_tests` fixture rather than keeping `insert_run`.

### Contract changes

`SPEC.md`:

| Section | Change |
|---|---|
| §1 component table, `Persistence` row | "embedded migrations" → "the embedded schema baseline" |
| §6.1 Persistence & Schema | In the validation paragraph, edit the sentences listed below the table; the connection-reuse sentence and the trailing incompatible-message line stay |
| §6.3 subcommand summary, `setup` row | "Initialize the version's database and install assets" |
| §6.3 environment table, `CAFLEET_DATABASE_URL` row | Default `sqlite:///` + `~/.local/share/cafleet/cafleet_<VERSION>.db`, `<VERSION>` being the package version (the version field of `cafleet --version`, e.g. `0.31.1`) |
| §6.3 Shared diagnosis and connection reuse | "Compatibility includes identity, ledger version and expected table/column shapes." → "Compatibility includes identity, baseline ledger, recorded cafleet version and expected table/column shapes." |
| §6.3 `cafleet setup` | State the § Commands setup behavior and the two output strings; delete the migrate-to-head and ahead clauses |
| §6.3 Schema-version guard | Keep the empty and incompatible messages, name another-version files as incompatible, delete the behind/ahead sentences and "initializes/repairs compatible known versions" |
| §6.3 `cafleet doctor` | "missing, outdated or incompatible" → "missing or incompatible"; `database` check `ok` cell `cafleet <VERSION> at <path>`, `error` cell "for an empty or incompatible database"; summary `ok` detail `cafleet <VERSION>`, problem line names `run 'cafleet setup'`; every "not at head" / "at head" / "at-head" → "current" |
| §6.3 error table | "A missing, empty, behind, ahead or incompatible database" → "A missing, empty or incompatible database" |
| §6.9 runtime artifacts and runtime records | "The migration creates" / "initialized once during migration" → baseline initialization; replace the v8-upgrade sentence with: each cafleet version starts from its own database file, a file of another version is never opened for writes, migrated, renamed or deleted, and the running CLI does not show its fleets, messages or runtime history |
| §6.9 event-key paragraph | The last sentence states that a binary of another cafleet version never reads these records, because it refuses the file as incompatible |
| §6.9 measurement table | Row `Migration and reporting` → `Schema and reporting`; "Fresh/head/previous-head upgrade" → "Fresh-baseline initialization" |
| §7.1 | HOME expansion applies to the default `sqlite:///` URL naming `cafleet_<VERSION>.db` |
| §7.6 | "Embed migrations, skills and presets" → "Embed the schema baseline, skills and presets" |
| §8 Database schema | Opening paragraph per § Database format and § Classification (identity, single baseline, version record, per-version default file, incompatible other files); add the `cafleet_database` row to the table list; `refinery_schema_history` → "Baseline ledger"; fold the `hermes` check, `native_session_json` and `cafleet_database` into the exact DDL and state the binary-appended version insert; delete the Version 2 paragraph and keep its `native_session_json` meaning sentence; replace the format-family evolution sentences with "Schema changes edit `V1__baseline.sql` in place; each cafleet version starts from a fresh database." |

§6.1 validation paragraph sentence edits:

| Current sentence | Change |
|---|---|
| "Validate database identity, ledger version and required table/column shapes for the recorded supported version before command writes and before setup installs assets." | → "Validate database identity, the baseline ledger, the recorded cafleet version and required table/column shapes before command writes and before setup installs assets." |
| "Schedule schema upgrades between active command invocations." | Delete |
| "Current-family known older versions migrate; an ahead ledger version fails setup before database mutation, asset installation or asset-install recording. Other populated/identified or malformed files fail without content writes:" | → "Every other populated, identified or malformed file, including one recorded by another cafleet version, fails before database mutation, asset installation or asset-install recording:" |

"An invocation's validated connection serves its internal send, ACK and notification stamp operations; standalone public broker write functions retain their own format guard.", "Empty zero-identity files initialize with the baseline atomically." and the incompatible-message line after the paragraph stay unchanged.

`docs/`:

| Page | Change |
|---|---|
| `docs/docs/concepts/storage.md` § Backend | Default path `~/.local/share/cafleet/cafleet_<VERSION>.db`, with `<VERSION>` the package version (the version field of `cafleet --version`, e.g. `0.31.1`) |
| `docs/docs/concepts/storage.md` § Schema management | Describe identity, ledger, recorded version and shapes; the state table keeps the empty-file row, "Valid file of this version → proceed idempotently" and "Other populated/identified file, another version's file or malformed schema → report incompatible format before writes"; the behind and ahead rows go |
| `docs/docs/concepts/storage.md` § Upgrading from `cafleet_v8.db` | Replace with § Upgrading cafleet: finish and delete active fleets before upgrading; after upgrading run `cafleet setup` once, which creates the version's file (doctor reports it absent until then), and create fleets anew; database files of earlier versions in `~/.local/share/cafleet/` stay byte for byte, are reported incompatible when selected through `CAFLEET_DATABASE_URL`, and may be deleted by hand. A user with an explicit `CAFLEET_DATABASE_URL` points it at a new empty path, or deletes the old file by hand, then runs `cafleet setup` |
| `docs/docs/quickstart.md` install paragraph after the Homebrew / GitHub Releases commands | Add: rerun `cafleet setup` after every upgrade, patch releases included, because each version starts from a fresh database; link `concepts/storage.md` § Upgrading cafleet |
| `docs/docs/contributing.md` project-structure table | `migrations/` row → "Embedded SQL schema baseline" |
| `docs/docs/contributing.md` § Rust boundaries | Test-paragraph sentence per § Revised convention text |

`skills/cafleet/reference/cli.md`: the `setup` summary row as in §6.3; the `CAFLEET_DATABASE_URL` default as in §6.3, with `<VERSION>` the package version (e.g. `0.31.1`); "prepares/migrates the database" → "initializes the version's database"; "a database not at head" → "a database that is not current"; delete the behind and ahead error rows; the incompatible row's situation reads "Incompatible database file, including one from another cafleet version".

`CLAUDE.md` § Project: "rusqlite and embedded refinery migrations" → "rusqlite and an embedded refinery schema baseline".

`README.md` and every other `skills/` file stay unchanged: neither names the database path, schema states or migrations.

### Revised convention text

`.claude/rules/code-quality.md` § Test placement, replace the bullet:

```markdown
- Write test inputs as literals or generated bytes inside the test module. Tests may read the runtime data that production code embeds (`coding_agent/native/<backend>.json`, `bundled.json`, presets, migrations).
```

with:

```markdown
- Write test inputs as literals or generated bytes inside the test module. Every test drives production code and asserts what that code decides or produces. A test uses the data files that production code embeds (for example the backend fixtures, `bundled.json`, skills, presets and the schema baseline) only as input to that code or as the expected value of its output, such as comparing installed bytes with the embedded bytes, and asserts none of their text, keys, counts, ids or layout.
```

and append to the same section's **Forbidden patterns** list:

```markdown
- A test that asserts the text, keys, counts, ids or layout of an embedded data file instead of the behavior of the code that consumes it.
```

`.claude/rules/code-quality.md` § Database migrations, rename the heading to `## Database schema` and replace the section body with:

```markdown
`migrations/V1__baseline.sql` is the complete schema and the only file in `migrations/`; edit it in place for every schema change. Each cafleet version uses its own default database file, `cafleet_<VERSION>.db`, so every release starts from a fresh database and no data carries across versions. Baseline initialization records the creating version in `cafleet_database`. Setup and the command guard accept only a file whose application identity, ledger, recorded version and required table/column shapes match the running binary, and refuse every other populated file as incompatible without writing to it.
```

`.claude/rules/documentation-maintenance.md` § First-class documentation targets, in "Paths that are part of a specified contract (embedded preset paths and their install targets, migration files) are `SPEC.md` content.", replace "migration files" with "the schema baseline".

`docs/docs/contributing.md` § Rust boundaries, replace the first sentence of the test paragraph:

```markdown
Every Rust test is a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises.
```

with:

```markdown
Every Rust test is a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises, and it asserts the behavior of the code under test rather than the text, keys, counts, ids or layout of embedded data files such as skills, presets, the profile catalog and the schema baseline.
```

### Accepted consequences

- No replacement guard is added for content. Properties the deleted content assertions pinned — for example the broad `cafleet *` allow, the absence of Director-only shell entries in the OpenCode presets, one bundled profile per backend and the absence of personal paths in native fixtures — are maintained by reviewing data-file changes like any other file change.
- No data carries across cafleet versions, patch releases included. Fleets, messages, runtime history and asset-install records of an earlier version stay in its file; the user reruns `cafleet setup` after every upgrade. A user with an explicit `CAFLEET_DATABASE_URL` finds every command refusing that file after an upgrade, and points the variable at a new empty path, or deletes the old file by hand, before running `cafleet setup`.
- Database files of earlier versions accumulate in `~/.local/share/cafleet/` until the user deletes them; neither setup nor doctor reports them.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] Apply the §1, §6.1 and §6.3 rows of § Contract changes <!-- completed: 2026-11-03T08:23 -->
- [x] Apply the §6.9, §7.1, §7.6 and §8 rows of § Contract changes, including the folded exact DDL <!-- completed: 2026-11-03T08:24 -->

### Step 2: User documentation, skills and rules

- [x] Apply the `docs/docs/concepts/storage.md` rows of § Contract changes <!-- completed: 2026-11-03T08:26 -->
- [x] Add the upgrade note to `docs/docs/quickstart.md` as in § Contract changes <!-- completed: 2026-11-03T08:26 -->
- [x] Apply the `docs/docs/contributing.md` rows of § Contract changes <!-- completed: 2026-11-03T08:26 -->
- [x] Apply the `skills/cafleet/reference/cli.md` changes of § Contract changes <!-- completed: 2026-11-03T08:26 -->
- [x] Replace the embedded-data bullet in `.claude/rules/code-quality.md` § Test placement and append the content-assertion Forbidden-patterns bullet, as in § Revised convention text <!-- completed: 2026-11-03T08:26 -->
- [x] Rename `.claude/rules/code-quality.md` § Database migrations to `## Database schema` and replace its body as in § Revised convention text, and apply the `CLAUDE.md` change <!-- completed: 2026-11-03T08:26 -->
- [x] Replace "migration files" with "the schema baseline" in `.claude/rules/documentation-maintenance.md` § First-class documentation targets <!-- completed: 2026-11-03T08:26 -->

### Step 3: Baseline and audit confirmation

- [x] Run `mise //:test` and note the reported test count as the baseline (baseline: 1630 tests) <!-- completed: 2026-11-03T08:27 -->
- [x] Confirm every row of § Audit result and § Database tests against the current tree; if a test changed or a new content or migration test appeared, update those tables and the Success Criteria count in this document before editing code <!-- completed: 2026-11-03T08:27 -->

### Step 4: Remove content assertions

- [x] Delete the six `src/assets.rs` tests in § Audit result and their four helpers <!-- completed: 2026-11-03T08:34 -->
- [x] Apply every `src/coding_agent/catalog.rs` row of § Audit result (deletions, reductions and renames) and delete helpers left without callers <!-- completed: 2026-11-03T08:34 -->

### Step 5: Single baseline and versioned database

- [x] Fold `V2__hermes_backend.sql` into `V1__baseline.sql`, add the `cafleet_database` table as in § Database format, and delete `V2__hermes_backend.sql` <!-- completed: 2026-11-03T08:29 -->
- [x] In `src/db.rs`, replace `embed_migrations!` with `baseline()` / `initialize`, implement § Classification, and delete the items listed there <!-- completed: 2026-11-03T08:29 -->
- [x] In `src/config.rs`, derive the default URL from `crate::cli::VERSION` as in § Database format <!-- completed: 2026-11-03T08:29 -->
- [x] In `src/cli/setup.rs`, implement the setup and doctor rows of § Commands <!-- completed: 2026-11-03T08:29 -->
- [x] Update every remaining caller of the removed `src/db.rs` items, including `src/cli/mod.rs`, `src/runtime/service.rs`, `src/runtime/observation.rs` and test callers <!-- completed: 2026-11-03T08:43 -->

### Step 6: Database tests

- [x] Apply every `src/db.rs` row of § Database tests and delete helpers left without callers <!-- completed: 2026-11-03T08:42 -->
- [x] Apply the `src/config.rs` and `src/cli/setup.rs` rows of § Database tests <!-- completed: 2026-11-03T08:42 -->

### Step 7: Verification

- [ ] Run `mise //:test`; confirm it passes and that the count is the baseline minus 29 <!-- completed: -->
- [ ] Run `mise //:lint`; confirm no unused-item or unused-import warnings remain <!-- completed: -->
- [ ] Run `mise //docs:build`; confirm it passes <!-- completed: -->
- [ ] With `HOME` set to a fresh temporary directory and `CAFLEET_DATABASE_URL` unset, run the built binary's `setup` twice and `doctor`; confirm the file name and strings in Success Criteria <!-- completed: -->
- [ ] For each of three copies of that initialized file — one after `UPDATE cafleet_database SET cafleet_version='0.0.0'`, one after inserting a `refinery_schema_history` row with version 2, one after `PRAGMA application_id=1` — record its checksum, then run the built binary's `setup`, `fleet list` and `doctor` with `CAFLEET_DATABASE_URL` pointing at the copy; confirm each prints the incompatible-format message and the checksum is unchanged <!-- completed: -->
- [ ] Search outside `design-docs/` for each removed name in Success Criteria and confirm none remains; confirm `git diff` touches no installation-behavior test in `src/assets.rs`, no file under `presets/` or `src/coding_agent/native/`, no `skills/` file other than `skills/cafleet/reference/cli.md`, and leaves `src/coding_agent/bundled.json` unchanged <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-03 | Include the migration-practice removal: per-version database file, single in-place baseline with a version record, four-state classification, and the matching contract, docs, rules and test changes |
| 2026-11-03 | Define `<VERSION>` as the package version; name the exact §6.1 sentences edited; add the §6.3 compatibility row, the quickstart upgrade note, the explicit-URL upgrade action, the `## Database schema` rule heading and the documentation-maintenance wording; add CLI-surface verification for incompatible files |
