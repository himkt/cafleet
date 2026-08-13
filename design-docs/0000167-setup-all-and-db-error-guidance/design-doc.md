# One-Command Setup and Database-Error Guidance

**Status**: Approved
**Progress**: 11/24 tasks complete
**Last Updated**: 2026-08-13

## Overview

Make `cafleet setup` a true one-command onboarding: plain `cafleet setup` installs the coding-agent assets for all three agents, and `--coding-agent` accepts multiple values in one occurrence. Additionally, every non-setup command gains a schema-version guard so a missing or outdated database produces "run 'cafleet setup'" guidance instead of a raw SQLite error.

## Success Criteria

- [ ] On a fresh machine, plain `cafleet setup` migrates the database AND installs assets for `claude`, `codex`, and `opencode` — no second command needed.
- [ ] `cafleet setup --coding-agent claude codex` (space-delimited) installs exactly the named agents; the repeated-flag form keeps working.
- [ ] Running any fleet-scoped command, `monitor`, or `server` against a missing or schema-outdated database fails with an error that names `cafleet setup` (or "upgrade cafleet" for a newer-than-CLI schema) — never a raw SQLite error.
- [ ] `cafleet doctor` against a pre-head-schema database completes its three-section render (exit 1 with the database issue) instead of aborting with `database error: no such column: path …`.
- [ ] `mise //cafleet:test` and `mise //cafleet:lint` pass; SPEC.md, docs, and skills reflect the new contract with no drift.

---

## Background

Three UX gaps, confirmed by reading v0.24.0:

1. **Fresh-machine setup takes two commands.** `cafleet setup` runs the db half and the assets half in order, but the no-flag assets half only *refreshes* agents already recorded at their resolved identity paths. On a fresh machine it installs nothing and prints a guidance line telling the user to re-run with `--coding-agent <agent>`.
2. **`--coding-agent` takes one value per occurrence.** Installing two agents requires `--coding-agent claude --coding-agent codex`.
3. **Stale schemas surface raw SQLite errors.** Both the stale-assets guard (`cafleet/src/cli/helpers.rs`) and `doctor` (`cafleet/src/cli/doctor.rs`) call `list_asset_installs` whenever the `asset_installs` *table exists* — but on a pre-V6 schema the table exists without the `path` column, so the SELECT fails with the raw error `database error: no such column: path in SELECT coding_agent, path, … at offset 21`. In `doctor` the error propagates with `?` and aborts the whole diagnosis, contradicting its no-early-abort design (the `✗ database — schema M, head is N — run: cafleet setup` rendering exists but is unreachable in this state).

User decisions (clarification round 1): plain setup installs all three agents unconditionally — both on a fresh machine and when installs are already recorded (the refresh-only semantics are replaced); the multi-value syntax is space-delimited; the schema guard is systematic (every non-setup command, `server` included; `doctor` stays report-only); no `--coding-agent all` shorthand.

---

## Specification

### 1. Plain `cafleet setup` installs all three agents

The assets-half selection table collapses to:

| Invocation | Assets-half behavior |
|---|---|
| `--coding-agent` given (one or more values) | Install exactly the named agents, in the fixed order `claude`, `codex`, `opencode`. Unchanged. |
| No flag | Install all three agents, in the fixed order — identical to `--coding-agent claude codex opencode`. |

Consequences:

- The empty-table guidance line (`no assets install recorded; run 'cafleet setup --coding-agent <agent>' …`) and the superseded-path hint line (`<agent>: no install at <resolved> (previously set up at <old>); …`) are removed — the states that produced them now install instead. Per the removal rule, every mention of both lines is deleted from SPEC.md, `docs/docs/spec/cli-options.md`, and any other surface in the same change. `refresh_recorded` and `most_recent_path` in `cafleet/src/cli/setup.rs` are deleted.
- SPEC §6.3 *Schema-only invocation* is rewritten: plain `cafleet setup` is no longer naturally schema-only on a records-free database — it installs assets. Consequence for contributors: `cafleet setup` overwrites the agent-home skills with the binary's build-time-embedded assets, clobbering working-tree installs made via `mise //:skill-install`. `contributing.md` therefore changes in two places: the first-time-setup comment on the `cafleet setup` line becomes "migrate the database schema and install the embedded assets (idempotent)", and § *Installing the skills from your checkout* adds the ordering rule: run `cafleet setup` first, then `mise //:skill-install` — and re-run `mise //:skill-install` after any later `cafleet setup` to restore the working-tree skills.
- Lazy config-dir validation narrows to the selector form: plain setup resolves all three identity paths, so an invalid config-path variable (e.g. a relative `CODEX_HOME`) now fails the plain-setup assets half. `cafleet setup --coding-agent claude` with an invalid `CODEX_HOME` still succeeds.
- Doctor's superseded-row footnote and per-agent `cafleet setup --coding-agent <agent>` cells stay — targeted install remains supported.
- Flag help text becomes: `Install the named agent's assets (space-delimited, repeatable; default: all agents).`

### 2. `--coding-agent` accepts multiple values

clap change on `SetupArgs`: add `num_args = 1..` to the existing `Vec<String>` arg.

- `--coding-agent claude codex` and `--coding-agent claude --coding-agent codex` are both valid and equivalent.
- `setup` has no positional arguments, so greedy value consumption is unambiguous. Two distinct rejection cases, both exit 2: a bare `cafleet setup <word>` keeps failing with clap's native unexpected-argument error (unchanged); a word following the flag (`cafleet setup --coding-agent claude <word>`) is consumed as another flag value and fails with clap's native invalid-value error (or is a valid selection when `<word>` names an agent).
- Unknown values keep failing with clap's native invalid-value error (exit 2). Duplicates remain deduplicated (installation iterates the fixed `TARGET_AGENTS` order and membership-tests the selection).

### 3. Schema-version guard on every non-setup command

New prologue in `cafleet/src/cli/helpers.rs` (e.g. `schema_guard(settings) -> Result<(), CafleetError>`), evaluated before the command body — and before the stale-assets guard — for the `fleet`, `member`, and `message` groups, `monitor` (both forms), and `server`. `setup` (must remain runnable to repair) and `doctor` (reports instead of blocking) are exempt.

The guard connects and classifies via the existing `recorded_version` / `has_foreign_tables` helpers (moved or re-exported so `setup`, `doctor`, and the guard share them):

| Database state | Guard result (application error, exit 1) |
|---|---|
| Recorded version == head | Proceed silently. |
| Recorded version < head | `database schema is outdated (schema <M>, head <N>); run 'cafleet setup'` |
| No ledger, no app tables (missing or empty DB file) | `no cafleet database; run 'cafleet setup'` |
| No ledger, app tables present | `database has tables but no schema history — not a cafleet database?` |
| Recorded version > head | `database schema <M> is newer than this cafleet (head <N>); upgrade cafleet` |

Notes:

- `Connection::open` creates an empty DB file when missing; the guard therefore detects "missing" post-hoc as the no-ledger/no-tables state. This matches doctor's existing `Missing` classification.
- Connection-level failures (unreadable file, bad URL scheme) keep their existing `failed to open database at '<path>': <e>` / scheme errors — those are environment errors, not schema states.
- The guard's wording mirrors doctor's report lines (`schema <M>, head is <N> — run: cafleet setup` stays doctor's rendering; the guard uses the error phrasings above).
- The table strings are the `CafleetError::App` payloads; the CLI renders each with its uniform `Error: ` prefix, so the SPEC / `cli-options.md` error-catalog rows record the full form `Error: <payload>` (§5's table already shows the rendered form).
- With the schema guard in front, the stale-assets guard runs only against an at-head schema, so `list_asset_installs` can no longer fail on a missing column there. Its `asset_installs_table_exists` pre-check stays as belt-and-braces: on a hand-tampered database (ledger at head, table dropped) the missing table classifies as the no-rows case and yields the guard's guidance error, not a raw SQLite error.

### 4. Doctor never aborts on a stale schema

In `cafleet/src/cli/doctor.rs`, gate the recorded-rows read on the database report being at head AND the table existing — `db.ok() && asset_installs_table_exists(conn)` (the existing table-existence check is kept, the at-head check added). Whenever the rows are not read — any non-head state, or an at-head ledger with a hand-dropped table — the coding-agents section renders with no recorded-install data (each resolvable agent shows `– cafleet setup --coding-agent <agent>`, state `not_installed`, no issue), the database section renders its existing `✗` detail (`schema <M>, head is <N> — run: cafleet setup` / `no database — run: cafleet setup` / …), and doctor exits 1 for the database issue. With both gates, no missing-table or outdated-schema state reaches `list_asset_installs`, so doctor no longer aborts with a raw SQLite error from `asset_installs`.

### 5. Stale-assets guard message simplification

With plain `cafleet setup` now installing all three agents, the guard's no-rows error drops the per-agent selector:

| Guard case | New error (exit 1) |
|---|---|
| No agent has a row at its currently-resolved path | `Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install` |
| Stale row(s) at resolved path(s) | Unchanged: `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall` |

The assets-half pre-flight message (`the database schema is missing or outdated; run 'cafleet setup' first`) is unchanged — within `setup` it fires after a db-half failure, where re-running setup is still the remedy.

### Affected contract surfaces

| Surface | Change |
|---|---|
| `SPEC.md` §6.3 `setup` | Selection table, flag row (multi-value + new help string), guidance/hint line removal, *Schema-only invocation* rewrite, lazy-validation note |
| `SPEC.md` §6.3 *Stale-assets guard* | New schema-guard prologue subsection, ordering (schema guard → stale-assets guard), simplified no-rows error, `server` added to guarded surfaces |
| `SPEC.md` §6.3 `doctor` | Rows read gated on the at-head database report plus table existence; no-abort contract for the stale-schema state |
| `docs/docs/spec/cli-options.md` | Same contract edits: setup section, stale-assets guard tables, doctor section, error-string catalog rows |
| `docs/docs/quickstart.md` | One-command onboarding story (single `cafleet setup`) |
| `docs/docs/concepts/storage.md`, `docs/docs/concepts/coding-agents.md` | Plain-setup semantics ("all three, always"), schema-guard behavior description |
| `docs/docs/contributing.md` | First-time-setup comment update; skill-install ordering rule (`cafleet setup` first, `mise //:skill-install` after) |
| `README.md` | Verify via the `/update-readme` skill — the thin surface (single `cafleet setup` install step) is expected to already match |
| `skills/` | No skill embeds the two-step onboarding or the removed lines (verified by search); re-verify during implementation |

No database migration is needed; the schema is untouched.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Update `SPEC.md` §6.3 `setup`: multi-value flag row + help string, all-agents no-flag selection, delete the guidance and hint lines everywhere, rewrite *Schema-only invocation*, adjust the lazy-validation note <!-- completed: 2026-08-13T21:43 -->
- [x] Update `SPEC.md` §6.3 guard sections: add the schema-version guard (states, error strings, guarded surfaces incl. `server`, exemptions, ordering before the stale-assets guard), simplify the stale-assets no-rows error <!-- completed: 2026-08-13T21:43 -->
- [x] Update `SPEC.md` §6.3 `doctor`: rows read gated on the at-head report; stale-schema no-abort contract <!-- completed: 2026-08-13T21:43 -->
- [x] Apply the same contract edits to `docs/docs/spec/cli-options.md` (setup, guard, doctor, error-string catalog) <!-- completed: 2026-08-13T21:47 -->
- [x] Update `docs/docs/quickstart.md`, `docs/docs/concepts/storage.md`, `docs/docs/concepts/coding-agents.md`, `docs/docs/contributing.md` for the one-command story and the schema guard <!-- completed: 2026-08-13T21:50 -->
- [x] Run the `/update-readme` skill to sync `README.md` / SPEC drift; verify no skill under `skills/` mentions the removed lines <!-- completed: 2026-08-13T21:55 -->

### Step 2: Multi-value `--coding-agent`

- [x] Add `num_args = 1..` to `SetupArgs::coding_agent`; update the help string <!-- completed: 2026-08-13T21:53 -->
- [x] Tests: space-delimited form, repeated-flag form, mixed form, invalid value (exit 2), bare `cafleet setup <word>` still rejected with the unexpected-argument error, `--coding-agent claude <word>` rejected with the invalid-value error <!-- completed: 2026-08-13T21:53 -->

### Step 3: Plain setup installs all agents

- [x] Change `assets_half` empty-selection branch to install all `TARGET_AGENTS`; delete `refresh_recorded` and `most_recent_path` <!-- completed: 2026-08-13T21:58 -->
- [x] Delete the guidance-line and hint-line strings and their tests; add tests: fresh DB plain setup installs and records all three; recorded-elsewhere state installs at the resolved path anyway <!-- completed: 2026-08-13T21:58 -->
- [x] Test: plain setup with an invalid config-path variable fails the assets half with the pinned validation error; the selector form stays lazy <!-- completed: 2026-08-13T21:58 -->

### Step 4: Schema-version guard

- [ ] Add `schema_guard` to `cafleet/src/cli/helpers.rs` with the five-state classification and error strings; share `recorded_version` / `has_foreign_tables` with `setup` and `doctor` <!-- completed: -->
- [ ] Wire the guard into `cli/mod.rs` for `fleet` / `member` / `message` / `monitor` and into `server::run`; keep `setup` and `doctor` exempt <!-- completed: -->
- [ ] Simplify the stale-assets guard's no-rows error string (the `asset_installs_table_exists` pre-check stays) <!-- completed: -->
- [ ] Tests: behind-head fixture (hand-written ledger + pre-V6 `asset_installs` shape) → outdated error; empty DB → no-database error; ahead-of-head ledger → upgrade-cafleet error; foreign-tables-no-ledger → not-a-cafleet-database error; at-head → command proceeds <!-- completed: -->
- [ ] Test: a fleet-scoped command against the pre-V6 fixture no longer emits `no such column: path` <!-- completed: -->

### Step 5: Doctor hardening

- [ ] Gate the recorded-rows read on `db.ok() && asset_installs_table_exists(conn)` in `doctor.rs` <!-- completed: -->
- [ ] Tests: doctor against the pre-V6 fixture completes all three sections, renders the behind-head database line, exits 1, and emits no raw SQLite error; JSON shape unchanged <!-- completed: -->

### Step 6: Verification

- [ ] `mise //cafleet:test` passes <!-- completed: -->
- [ ] `mise //cafleet:lint` passes <!-- completed: -->
- [ ] `mise //cafleet:format` clean <!-- completed: -->
- [ ] `mise //cafleet:typecheck` passes <!-- completed: -->
- [ ] Grep sweep: no remaining mention of the removed guidance/hint lines anywhere in the repo <!-- completed: -->
- [ ] `mise //cafleet:install`, then a manual smoke of `cafleet doctor` against a stale fixture DB via a teammate with run permission (per the authorization-scope guard) <!-- completed: -->
