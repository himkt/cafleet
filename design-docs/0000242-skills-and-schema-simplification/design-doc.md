# Skills and Schema Simplification

**Status**: Complete
**Progress**: 54/54 tasks complete
**Last Updated**: 2026-11-23

## Overview

Cut the prose of every bundled skill under `assets/skills/` by at least a quarter, and simplify the database schema layer. Part 1 runs two passes: the reviewed edits of the clean-docs simplification run, then an aggressive restructure that leaves every file at least 25% shorter than on `main`. Part 2 removes the vestigial `refinery_schema_history` ledger and the `assets/migrations/` directory, inlines the complete schema DDL into `src/db.rs` as the `SCHEMA_SQL` constant, and drops the required-shape check, so compatibility is the application identity plus the recorded version. The two parts share one document but have separate implementation steps and commits.

## Success Criteria

- [x] Every file under `assets/skills/` has at least 25% fewer words (`wc -w`) than on `main`, as listed in § Measured word counts.
- [x] Every relative link and `#anchor` in `assets/skills/`, and every inbound link into `assets/skills/` from `SPEC.md`, `docs/` and `.claude/`, resolves to an existing file and heading, and every `### Model catalog`, `### Role defaults` and `#### OpenCode Go` section in `coding-agents.md` is byte-identical to `main`, all established by § Verifying the aggressive pass.
- [x] The `SCHEMA_SQL` constant in `src/db.rs` is the only schema file the binary embeds; neither `assets/schema.sql` nor `assets/migrations/` exists.
- [x] Neither `SCHEMA_SQL`, `SPEC.md` §8 nor any source file declares or reads `refinery_schema_history`; `LedgerState` and `ledger_state` no longer exist.
- [x] No source file defines or calls `schema_matches`, `reference_schema`, `read_schema`, `all_match`, `SchemaTable`, `SchemaIndex`, `metadata_rows`, `COLUMNS`, `FOREIGN_KEYS`, `INDEX_COLUMNS` or `Metadata`.
- [x] `initialize` on a non-empty file succeeds only when the application identity matches and `cafleet_database` records the running version, and refuses every other file — another recorded version, a missing version row, a missing `cafleet_database` table, an unrelated table, a foreign identity — with the incompatible-format error, leaving its bytes unchanged (asserted by `OTHER_FILE_CASES`).
- [x] `git grep` finds no `migrations`, `refinery`, `V1__`, `LedgerState`, `ledger_state`, `BASELINE_SQL`, `migrated_conn`, `schema.sql`, `schema_matches`, `SchemaTable` or `reference_schema` in the tracked tree outside `design-docs/`, and no schema-ledger, schema-baseline or required-shape wording in `SPEC.md`, `docs/`, `.claude/rules/`, `CLAUDE.md` or `src/`.
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

Four maintainer-local research artifacts feed this design; their content needed for implementation is copied into this document, so none of them is needed to implement it.

| Input | Content |
|---|---|
| clean-docs simplification run, merged `findings.md` | 165 rows over every file under `assets/skills/`, each annotated by the run's reviewer: 156 APPROVE, 3 REVISE, 3 REJECT and 3 superseded inside their slice; plus per-slice Observations |
| `db-proposal.md` | Analysis of `assets/migrations/`, the ledger and the compatibility classifier in `src/db.rs` |
| Skills follow-up brief | The cutting rules, preserved surfaces and 25% per-file target of the aggressive pass |
| Schema follow-up brief | The user decisions to inline the schema SQL and drop the required-shape check |

The schema layer predates the per-version database file model. Commit `67ea6551` replaced the `refinery` crate with a hand-written ledger that kept refinery's table name and its `V1__` file convention; commit `760d03e9` introduced per-version `cafleet_<VERSION>.db` files and the `cafleet_database` version row, which supersede the ledger. No migration runner exists or can exist under the per-version model, nothing reads the ledger's `applied_on` or `checksum`, and `Cargo.toml` has no `refinery` dependency.

The first Part 1 pass removed 5.5% of the skill words; the user judged that too small and approved the aggressive pass. After the first Part 2 implementation the user decided to inline the schema SQL into `src/db.rs` and to drop the required-shape check, leaving identity and recorded version as the whole compatibility rule.

---

## Specification

### Part 1: Skill prose simplification

#### Scope

- Files: every file under `assets/skills/`. Skills elsewhere (such as `.claude/skills/`) are out of scope.
- `SPEC.md`, `docs/` and `README.md` receive no Part 1 edits, and Part 1 adds no tests: no test asserts skill text, and the embedded skill bytes follow the edited files automatically.

#### Passes

| Pass | Method | Words, all files |
|---|---|---|
| First pass | The run's APPROVE and REVISE rows plus three adopted Observations (a redundant model-selection trigger list in `director.md`, a "without a probe" contradiction with `cli.md` § `member show`, and plain-text Hermes reception references in `coding-agents.md` turned into links); REJECT and superseded rows excluded | 38,180 → 36,094 (−5.5%) |
| Aggressive pass | Per-file restructure under § Cutting rules, targeting 25% fewer words per file than the first pass | 36,094 → 26,641 (−26.2%; −30.2% against `main`) |

The aggressive pass rewrites the same spans the first pass edited, so the current files, not per-row edit tables, are the result of Part 1.

#### Cutting rules

- Replace a restatement of a fact another reference owns with a one-clause link: `cli.md` owns command facts, `coding-agents.md` backend specifics, `coordination.md` verbs, pointers and markers, and `director.md` supervision.
- Keep one statement of a rule a file repeats, such as a section intro, a table and the prose under it.
- Remove hedged, doubled or stacked qualifiers, explanations of why that an agent does not act on, and mechanism or internal timing an agent never observes.
- Remove procedure steps that restate their heading or the previous step; turn a table whose rows read better as one sentence into that sentence; merge sections that cover one topic.

#### Preserved surfaces

- Every fact an agent acts on: invocations, flags, required arguments, output fields it reads, error strings it must recognize and next commands. In `cli.md` these are the content; elsewhere they may become links to `cli.md`.
- Every `IMPORTANT:` line, spawn-skeleton line, delta-table cell and verbatim start cue a workflow renders into spawn prompts.
- Every heading or anchor another file links to. A heading with no inbound link may be renamed or merged: the `execute.md` step headings drop their `(Director)` suffixes, the Verifier's phase sections become top-level sections, and the `guidelines.md` format sections merge into § Writing Standard.
- Backend neutrality: base files stay backend-neutral and point at `coding-agents.md` for specifics.
- In `coding-agents.md`, every `### Model catalog` and `### Role defaults` subsection and the `#### OpenCode Go` table stay byte-identical, because the `cafleet-model-list-refresh` skill owns them.
- The project rules: affirmative writing, current-state wording and no historical narration.

#### Verifying the aggressive pass

No task or test checks skill links (`mise //docs:build` covers only the documentation site), so Part 1 is verified with three explicit checks:

1. **Links resolve.** For every file under `assets/skills/`, and for every link into `assets/skills/` from `SPEC.md`, `docs/` and `.claude/`, list each Markdown link target `](…)`. A relative path must name an existing file, resolved from the linking file's directory. Each `#anchor`, in-file or after a path, must equal the GitHub slug of a heading in its target file: lowercase the heading text, drop backticks and every punctuation character except hyphens and spaces, then replace each space with a hyphen (`` ### `member show` `` → `member-show`, `## COMMENT(role) Marker` → `commentrole-marker`).
2. **Refresh-owned sections unchanged.** Every `### Model catalog`, `### Role defaults` and `#### OpenCode Go` section of `coding-agents.md` is byte-identical to `main`.
3. **Word counts.** `wc -w` on each file is at least 25% below its `main` count.

#### Measured word counts

`wc -w` per file; "First pass" is the tree after `docs: tighten bundled skill prose`, and the change column is against `main`.

| File | `main` | First pass | Now | Change |
|---|---|---|---|---|
| `cafleet-design-doc/SKILL.md` | 604 | 544 | 398 | −34.1% |
| `cafleet-design-doc/create/create.md` | 802 | 716 | 537 | −33.0% |
| `cafleet-design-doc/create/roles/drafter.md` | 930 | 845 | 588 | −36.8% |
| `cafleet-design-doc/create/roles/reviewer.md` | 428 | 372 | 257 | −40.0% |
| `cafleet-design-doc/execute/execute.md` | 1,228 | 1,171 | 888 | −27.7% |
| `cafleet-design-doc/execute/roles/programmer.md` | 981 | 904 | 612 | −37.6% |
| `cafleet-design-doc/execute/roles/reviewer.md` | 672 | 608 | 431 | −35.9% |
| `cafleet-design-doc/execute/roles/tester.md` | 816 | 753 | 541 | −33.7% |
| `cafleet-design-doc/execute/roles/verifier.md` | 706 | 625 | 422 | −40.2% |
| `cafleet-design-doc/reference/coordination.md` | 1,439 | 1,380 | 923 | −35.9% |
| `cafleet-design-doc/reference/guidelines.md` | 914 | 904 | 674 | −26.3% |
| `cafleet/SKILL.md` | 1,092 | 999 | 747 | −31.6% |
| `cafleet/reference/cli.md` | 9,801 | 9,619 | 7,211 | −26.4% |
| `cafleet/reference/coding-agents.md` | 8,956 | 8,365 | 6,273 | −30.0% |
| `cafleet/reference/model-selection.md` | 1,693 | 1,632 | 1,170 | −30.9% |
| `cafleet/roles/director.md` | 6,247 | 5,875 | 4,384 | −29.8% |
| `cafleet/roles/member.md` | 871 | 782 | 585 | −32.8% |
| Total | 38,180 | 36,094 | 26,641 | −30.2% |

Against the first pass, every file loses at least 25% except `execute.md`, which loses 24.2%.

### Part 2: Schema layer simplification

#### Retained and removed elements

| Element | Decision | Reason |
|---|---|---|
| `PRAGMA application_id` `1128351289` | Keep | File identity; the empty-file rule and the foreign-identity refusal depend on it |
| `cafleet_database` version row | Keep | The only cross-version guard for an explicit `--database-url` path |
| Empty-file rule, `BEGIN IMMEDIATE` initialization | Keep | Serializes concurrent setup and refuses to stamp a schema onto a populated file |
| Incompatible-format refusal with byte preservation, `require_current`, `classify_selected_schema` | Keep | Fail-loud protection of a wrong file |
| `refinery_schema_history` table, its row, `applied_on`, `checksum` | Remove | No migration runner exists; nothing reads the columns |
| `LedgerState`, `ledger_state` | Remove | Ledger reads only |
| `assets/migrations/` directory, `V1__` prefix | Remove | refinery's discovery convention |
| `assets/schema.sql` | Remove | The DDL lives in the `SCHEMA_SQL` constant, its only consumer |
| Required-shape check: `schema_matches`, `reference_schema`, `read_schema`, `all_match`, `SchemaTable`, `SchemaIndex`, `metadata_rows`, `COLUMNS`, `FOREIGN_KEYS`, `INDEX_COLUMNS`, `Metadata` | Remove | User decision: compatibility is application identity plus recorded version |
| "baseline" terminology | Replace with "schema" | Nothing builds on a baseline; keep the `#### Schema-version guard` heading, whose anchor `docs/` and `cli.md` link |

Schema digests, `PRAGMA user_version` and dropping `application_id` stay out of scope.

#### Schema constant

`src/db.rs` holds the complete DDL as a raw string literal, the former `V1__baseline.sql` content without its `CREATE TABLE refinery_schema_history (...)` statement:

```rust
const SCHEMA_SQL: &str = r#"PRAGMA application_id = 1128351289;

CREATE TABLE members (
...
CREATE INDEX idx_member_runner_diagnostics_recent ON member_runner_diagnostics(run_id, last_at);
"#;
```

No other file embeds or reads the schema. `SPEC.md` §8 keeps its exact DDL block as the contract.

#### Classifier

`classify_schema` keeps its empty-file check, then tests identity and recorded version:

```rust
fn records_running_version(conn: &Connection) -> Result<bool, CafleetError> {
    let has_version_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='cafleet_database')",
        [],
        |row| row.get(0),
    )?;
    if !has_version_table {
        return Ok(false);
    }
    let recorded: Option<String> = conn
        .query_row(
            "SELECT cafleet_version FROM cafleet_database WHERE singleton_id=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(recorded.as_deref() == Some(crate::cli::VERSION))
}

fn classify_schema(conn: &Connection) -> Result<Classification, CafleetError> {
    if is_empty_file(conn)? {
        return Ok(Classification::Missing);
    }
    let identity: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if identity != crate::db::APPLICATION_ID || !records_running_version(conn)? {
        return Ok(Classification::Incompatible);
    }
    Ok(Classification::Current)
}
```

The table-existence check makes a file with CAFleet's identity but no `cafleet_database` table classify incompatible rather than fail with a SQL error. A genuine SQL failure, such as a file of non-SQLite bytes, still propagates as an error.

#### Initialization

`initialize` branches once on the classifier inside the same `IMMEDIATE` transaction:

```rust
pub(crate) fn initialize(conn: &mut Connection, database_url: &str) -> Result<(), CafleetError> {
    let failed =
        |e: rusqlite::Error| CafleetError::App(format!("database initialization failed: {e}"));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failed)?;
    match classify_schema(&tx)? {
        Classification::Current => return Ok(()),
        Classification::Incompatible => {
            return Err(incompatible_format(database_path(database_url)?));
        }
        Classification::Missing => {}
    }
    tx.execute_batch(SCHEMA_SQL).map_err(failed)?;
    tx.execute(
        "INSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, ?1)",
        [crate::cli::VERSION],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}
```

`initialize` calls neither `crate::digest::sha256_hex` nor `crate::time::now_stamp`; both modules keep their other callers.

| File state | Condition | Result |
|---|---|---|
| Empty | Application identity zero and no user tables | Applies the schema, inserts the `cafleet_database` row for the running version, commits |
| Current | Application identity matches and the `cafleet_database` row records the running version | Writes nothing and succeeds |
| Other | Every other file, including another recorded version, a matching identity without the version row or the `cafleet_database` table, an unrelated table and a foreign identity | Writes nothing and fails with the incompatible-format error naming the database path |
| Unreadable | A SQL error while classifying | Writes nothing and fails with that error |

Setup's observable behavior is unchanged, because setup classifies before it calls `initialize`. A current-version development file created before this change still contains `refinery_schema_history`; the classifier reads only identity and version, so the file stays current and needs no cleanup. The table and column shapes of a current file are outside the compatibility diagnosis. This change does not bump the version; release CI owns versioning.

#### Documentation and rule edits

| Location | Current text or action |
|---|---|
| `SPEC.md` §3 module table | `\| Persistence \| Connection factory, format diagnosis and the embedded schema \|` |
| `SPEC.md` §6.1 format sentence | "Format validation, schema initialization, incompatible-file refusal and file preservation follow §6.3 Schema-version guard." (rest unchanged) |
| `SPEC.md` §6.3 asset table | Delete the `assets/migrations/` row; the table carries no schema row |
| `SPEC.md` §6.3 Schema-version guard | "with the embedded schema atomically"; "Every other populated or identified file, including one recorded by another cafleet version, fails …" |
| `SPEC.md` runtime records sentence | "The schema creates the runtime records with the broker tables (§8)." |
| `SPEC.md` §8 intro | Replace the paragraph with the §8 intro text below |
| `SPEC.md` §8 table list and DDL | Delete the `refinery_schema_history` table row and DDL statement and the ledger-row paragraph; retitle "Exact schema DDL:" and "Column semantics:" |
| `SPEC.md` §8 initialization | Replace the initialization table and its closing paragraph with the §8 initialization text below; the sentence before the table stays |
| `SPEC.md` §8 closing paragraph | Delete the required-shape sentence, leaving "Runtime validates the free-text defer/outcome reasons of §6.9 Delivery reporting." |
| `docs/docs/concepts/storage.md` § Schema management | "A compatible file has CAFleet's application identity and records that the running CLI's cafleet version initialized it."; Missing-row result "Apply the embedded schema transactionally"; incompatible row "Other populated/identified file or another version's file" |
| `docs/docs/contributing.md` project-structure table | The `src/` row names "the SQL schema, the `SCHEMA_SQL` constant in `src/db.rs`"; the `assets/` row names no schema file |
| `docs/docs/contributing.md` test table | "Fresh-baseline initialization" → "Fresh-schema initialization" |
| `.claude/rules/code-quality.md` § Database schema | Replace the paragraph with the rule text below |
| `.claude/rules/documentation-maintenance.md` contract-path sentence | Drop the schema-asset entry: "(the embedded `assets/` tree, embedded preset paths and their install targets)" |
| `CLAUDE.md` project paragraph | "an embedded SQL schema baseline" → "an embedded SQL schema" |

`README.md` and `assets/skills/` mention none of these and need no Part 2 edit.

§8 intro text:

> The binary embeds one complete schema, whose application identity is `PRAGMA application_id` `1128351289` (`0x43414639`, ASCII `CAF9`). Each cafleet version owns its default database file, `cafleet_<VERSION>.db`, where `<VERSION>` is the release version (§7.6), so each version starts from a fresh file; schema initialization records that version in the `cafleet_database` row. A fresh initialization creates exactly the application tables declared in the schema. Compatibility diagnosis verifies, in order, the application identity and a `cafleet_database` row recording the running binary's version. Any other identified or populated file, including one recorded by another cafleet version and one with the application identity but no `cafleet_database` table, is an incompatible format whose bytes are preserved, and `cafleet doctor` reports the version's default file as absent until `cafleet setup` creates it.

§8 initialization text (the sentence before the table stays):

> | File state | Condition | Result |
> |---|---|---|
> | Empty | Application identity zero and no user tables | Applies the schema and inserts the `cafleet_database` row for the running version, then commits |
> | Current | A file the compatibility diagnosis above classifies current | Writes nothing and succeeds |
> | Other | Every other file | Writes nothing and fails with the §6.3 incompatible-format error naming the database path |
>
> A read error during that classification fails initialization with its cause and writes nothing. A failed initialization leaves no identity, table or version row, and two concurrent initializations of one empty file both succeed with exactly one version row.

`code-quality.md` § Database schema text:

> The `SCHEMA_SQL` constant in `src/db.rs` is the complete schema; edit it in place for every schema change. Each cafleet version uses its own default database file, `cafleet_<VERSION>.db`, so every release starts from a fresh database and no data carries across versions. Schema initialization records the creating version in `cafleet_database`. Setup and the command guard accept only a file whose application identity and recorded version match the running binary, and refuse every other populated file as incompatible without writing to it.

#### Test changes

Every change stays in the in-file `#[cfg(test)] mod tests` block of the file it edits. The table lists each change against `main`.

| Test | File | Change |
|---|---|---|
| `repeated_initialization_preserves_all_application_rows` | `src/db.rs` | Replace the ledger snapshot with a `SELECT * FROM cafleet_database` snapshot, asserted unchanged after each repeated `initialize`; delete the `schema_matches` assertion |
| `baseline_initialization_records_the_identity_ledger_version_and_shape` | `src/db.rs` | Rename to `initialization_records_the_identity_and_version`; assert the application identity and the version row only |
| `a_v8_family_file_is_incompatible_and_left_byte_identical` | `src/db.rs` | Make the `cafleet_v8.db` pair's damage `PRAGMA application_id=1128351288` (`0x43414638`), delete the `if name == "cafleet_v8.db"` `pragma_update` branch and the then-unused `V8_APPLICATION_ID` constant; both pairs run only `conn.execute_batch(damage)` |
| `OTHER_FILE_CASES` / `initialization_refuses_every_other_file` | `src/db.rs` | Five cases: `recorded-other-version` (`UPDATE cafleet_database SET cafleet_version='0.0.0'`), `identity-without-version-row` (`DELETE FROM cafleet_database`), `identity-without-version-table` (`DROP TABLE cafleet_database`), `unrelated-table-only` (renamed from `unrelated-table-without-ledger`) and `foreign-identity-without-tables`; `ledger-row-beyond-baseline` and `ledger-row-deleted` are deleted |
| `concurrent_initialization_of_one_fresh_file_applies_the_baseline_once` | `src/db.rs` | Rename to `…_applies_the_schema_once`; drop the ledger count, keep the `cafleet_database` count and `require_current` |
| `a_ledger_row_beyond_the_baseline_is_incompatible` | `src/db.rs` | Delete |
| `failed_baseline_initialization_rolls_back_identity_tables_and_ledger` | `src/db.rs` | Rename to `failed_initialization_rolls_back_identity_and_tables`; body unchanged |
| `classification_preserves_missing_current_identity_and_sql_causes` | `src/db.rs` | Replace the ledger-column rename with a genuine SQL failure: a temp file holding non-SQLite bytes, for which `classify_schema` returns `Err` (`file is not a database`); add an in-memory connection with CAFleet's `application_id` and no tables, which classifies `Incompatible` |
| `composer_receipt_required_columns_are_checked_without_modifying_the_selected_database`, `selected_reference_rechecks_ddl_and_accepts_auxiliary_objects`, `required_schema_components_and_identity_are_refused`, `structural_reference_checks_required_keys_types_defaults_and_foreign_keys` | `src/db.rs` | Delete: each exercises only the required-shape check; the missing-version-row case moves to `OTHER_FILE_CASES` |
| `ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes` | `src/cli/mod.rs` | Held reader `BEGIN; SELECT * FROM cafleet_database` |
| `refused_wal_databases_and_sidecars_remain_byte_identical_with_a_held_reader` | `src/cli/setup.rs` | Held reader `BEGIN; SELECT * FROM cafleet_database` |

The remaining identity, empty-file, concurrency, rollback, refusal and SQL-domain tests in `src/db.rs` stay unchanged.

#### Test helper rename

Rename `db::test_support::migrated_conn` to `initialized_file_conn` at its definition in `src/db.rs` and at every call site (74 occurrences across `src/broker/`, `src/cli/`, `src/pane_lock.rs`, `src/runtime/mod.rs` and `src/db.rs` at the time of writing). The rename is mechanical and compiler-checked.

### Commits

| Commit | Content |
|---|---|
| `docs: tighten bundled skill prose` | Step 4 |
| `refactor: drop the vestigial schema ledger and rename the schema asset` | Steps 1, 2, 5 and 6 |
| `refactor: rename the migrated_conn test helper` | Step 7 |
| `docs: add design doc 0000242-skills-and-schema-simplification` | This document's `design-doc.md` |
| `docs: cut bundled skill prose by a quarter` | Step 10 |
| `refactor: inline the schema SQL and drop the shape check` | Steps 9 and 11, including the removal of `assets/schema.sql` |
| `docs: revise design doc 0000242 for the follow-up` | This document's revision |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Steps 1–5 complete the documentation before Step 6 changes code, and Step 9 completes the follow-up documentation before Step 11 changes code. Part 1 is Steps 4 and 10; Part 2 is Steps 1, 2, 5, 6, 7, 9 and 11.

### Step 1: SPEC.md (Part 2)

- [x] Update the `SPEC.md` module table and §6.1 format sentence to the "embedded schema" / "schema initialization" wording <!-- completed: 2026-11-23T02:56 -->
- [x] Replace the `assets/migrations/` asset-table row with an `assets/schema.sql` row <!-- completed: 2026-11-23T02:56 -->
- [x] Update §6.3 Schema-version guard to "with the embedded schema atomically", keeping the heading <!-- completed: 2026-11-23T02:56 -->
- [x] Update the runtime records sentence to "The schema creates the runtime records" <!-- completed: 2026-11-23T02:56 -->
- [x] Replace the §8 intro paragraph with the first-pass §8 intro text <!-- completed: 2026-11-23T02:56 -->
- [x] Delete the `refinery_schema_history` table row, DDL statement and ledger-row paragraph; retitle "Exact schema DDL:" and "Column semantics:" <!-- completed: 2026-11-23T02:56 -->
- [x] Replace the §8 initialization table and closing paragraph with the §8 initialization text, keeping the sentence before the table <!-- completed: 2026-11-23T02:56 -->

### Step 2: docs/ (Part 2)

- [x] Update `docs/docs/concepts/storage.md` § Schema management <!-- completed: 2026-11-23T02:57 -->
- [x] Update the `docs/docs/contributing.md` project-structure and test tables <!-- completed: 2026-11-23T02:57 -->

### Step 3: README.md

- [x] Confirm `README.md` mentions neither the schema asset, the ledger nor any edited skill sentence; record "no change" <!-- completed: 2026-11-23T02:57 -->

### Step 4: assets/skills/ first pass (Part 1)

- [x] Apply the first-pass edits for `assets/skills/cafleet/SKILL.md` and `assets/skills/cafleet/roles/member.md` <!-- completed: 2026-11-23T02:56 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet/reference/cli.md` <!-- completed: 2026-11-23T02:58 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet/reference/coding-agents.md`, including the Hermes reception links, leaving every refresh-owned section untouched <!-- completed: 2026-11-23T03:01 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet/reference/model-selection.md` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet/roles/director.md`, including the two adopted Observations <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet-design-doc/SKILL.md` and every file under `create/` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet-design-doc/execute/execute.md` and every file under `execute/roles/` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the first-pass edits for `assets/skills/cafleet-design-doc/reference/coordination.md` and `reference/guidelines.md` <!-- completed: 2026-11-23T03:03 -->
- [x] Confirm no heading text changed and every link target and `#anchor` in the edited files resolves <!-- completed: 2026-11-23T03:03 -->

### Step 5: Rules and project instructions (Part 2)

- [x] Replace `.claude/rules/code-quality.md` § Database schema with the first-pass rule text <!-- completed: 2026-11-23T02:57 -->
- [x] Update the `.claude/rules/documentation-maintenance.md` contract-path sentence to "the embedded schema" <!-- completed: 2026-11-23T02:57 -->
- [x] Update `CLAUDE.md` to "an embedded SQL schema" <!-- completed: 2026-11-23T02:57 -->

### Step 6: Schema code and tests (Part 2)

- [x] `git mv assets/migrations/V1__baseline.sql assets/schema.sql`, delete its `refinery_schema_history` statement and remove `assets/migrations/` <!-- completed: 2026-11-23T02:59 -->
- [x] Rename `BASELINE_SQL` to `SCHEMA_SQL` with the new `include_str!` path in `src/db.rs` <!-- completed: 2026-11-23T02:59 -->
- [x] Delete `LedgerState` and `ledger_state` and drop the ledger clause from `classify_schema` <!-- completed: 2026-11-23T02:59 -->
- [x] Rewrite `initialize` per § Initialization <!-- completed: 2026-11-23T02:59 -->
- [x] Apply the first-pass `src/db.rs` test changes <!-- completed: 2026-11-23T03:05 -->
- [x] Apply the `src/cli/mod.rs` and `src/cli/setup.rs` rows of § Test changes <!-- completed: 2026-11-23T03:05 -->
- [x] Run `mise //:test` and `mise //:lint`; both pass <!-- completed: 2026-11-23T03:05 -->

### Step 7: Test helper rename (Part 2)

- [x] Rename `migrated_conn` to `initialized_file_conn` at its definition and every call site <!-- completed: 2026-11-23T03:10 -->
- [x] Run `mise //:test` and `mise //:lint`; both pass <!-- completed: 2026-11-23T03:10 -->

### Step 8: Validation

- [x] `git grep -n -E "migrations|refinery|V1__|LedgerState|ledger_state|BASELINE_SQL|migrated_conn" -- ":!design-docs"` returns nothing <!-- completed: 2026-11-23T03:05 -->
- [x] `git grep -n -i -E "ledger|schema baseline|baseline initialization|fresh-baseline|embedded baseline" -- SPEC.md docs .claude/rules CLAUDE.md src` returns nothing <!-- completed: 2026-11-23T03:05 -->
- [x] Run `mise //docs:build`, `mise //:test` and `mise //:lint` on the final tree; all pass <!-- completed: 2026-11-23T03:05 -->

### Step 9: Follow-up documentation and rules (Part 2)

- [x] Update `SPEC.md`: the module table's "format diagnosis", the deleted `assets/schema.sql` asset row, the Schema-version guard's "populated or identified file", the §8 intro text and the §8 closing paragraph without the required-shape sentence <!-- completed: 2026-11-23T04:32 -->
- [x] Update `docs/docs/concepts/storage.md` § Schema management and the `docs/docs/contributing.md` project-structure table <!-- completed: 2026-11-23T04:32 -->
- [x] Confirm `README.md` mentions neither the schema constant nor the shape check; record "no change" <!-- completed: 2026-11-23T04:46 -->
- [x] Replace `.claude/rules/code-quality.md` § Database schema with the rule text and drop the schema entry from the `.claude/rules/documentation-maintenance.md` contract-path sentence <!-- completed: 2026-11-23T04:32 -->

### Step 10: assets/skills/ aggressive pass (Part 1)

- [x] Restructure `assets/skills/cafleet/SKILL.md` and `assets/skills/cafleet/roles/member.md` <!-- completed: 2026-11-23T04:34 -->
- [x] Restructure `assets/skills/cafleet/reference/cli.md` <!-- completed: 2026-11-23T04:36 -->
- [x] Restructure `assets/skills/cafleet/reference/coding-agents.md`, keeping every refresh-owned section byte-identical <!-- completed: 2026-11-23T04:41 -->
- [x] Restructure `assets/skills/cafleet/reference/model-selection.md` <!-- completed: 2026-11-23T04:35 -->
- [x] Restructure `assets/skills/cafleet/roles/director.md` <!-- completed: 2026-11-23T04:37 -->
- [x] Restructure `assets/skills/cafleet-design-doc/SKILL.md` and every file under `create/` <!-- completed: 2026-11-23T04:34 -->
- [x] Restructure `assets/skills/cafleet-design-doc/execute/execute.md` and every file under `execute/roles/` <!-- completed: 2026-11-23T04:35 -->
- [x] Restructure `assets/skills/cafleet-design-doc/reference/coordination.md` and `reference/guidelines.md` <!-- completed: 2026-11-23T04:33 -->
- [x] Run the three checks in § Verifying the aggressive pass: every link and `#anchor` resolves, the refresh-owned sections match `main`, and every file meets the word-count target <!-- completed: 2026-11-23T04:57 -->

### Step 11: Inline the schema and drop the shape check (Part 2)

- [x] Inline the DDL into `SCHEMA_SQL` as a raw string literal and delete `assets/schema.sql` <!-- completed: 2026-11-23T04:33 -->
- [x] Delete the required-shape check and its `classify_schema` clause <!-- completed: 2026-11-23T04:33 -->
- [x] Add the `cafleet_database` table-existence check to `records_running_version` <!-- completed: 2026-11-23T04:33 -->
- [x] Apply the follow-up `src/db.rs` rows of § Test changes: the shape-only test deletions, the five `OTHER_FILE_CASES` and the identity-without-tables classification case <!-- completed: 2026-11-23T04:33 -->

### Step 12: Follow-up validation

- [x] Run `mise //:test` (1431 passed, 0 failed), `mise //:lint` and `mise //docs:build` on the final follow-up tree; all pass <!-- completed: 2026-11-23T04:58 -->
- [x] `git grep -n -E "migrations|refinery|V1__|LedgerState|ledger_state|BASELINE_SQL|migrated_conn|schema\.sql|schema_matches|SchemaTable|reference_schema" -- ":!design-docs"` returns nothing <!-- completed: 2026-11-23T04:46 -->
- [x] `git grep -n -i -E "schema\.sql|column shape|table/column|auxiliary|shape diagnosis" -- SPEC.md docs README.md .claude/rules CLAUDE.md assets/skills src` returns nothing <!-- completed: 2026-11-23T04:46 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-23 | Initial draft |
| 2026-11-23 | Revised for the follow-up: Part 1 replaces the per-file edit tables with the aggressive pass's cutting rules, preserved surfaces and measured word counts; Part 2 inlines the schema into `SCHEMA_SQL`, removes `assets/schema.sql` and the required-shape check, and updates the classifier, initialization, documentation and test specifications; adds Steps 9–12 and the follow-up commits |
