# Improve `cafleet setup` — version recording, `setup db` / `setup skill` subcommands, `db` group removal

**Status**: Approved
**Progress**: 0/22 tasks complete
**Last Updated**: 2026-07-04

## Overview

Restructure `cafleet setup` into a Click group — bare `cafleet setup` does everything, `cafleet setup db` runs only the schema create, `cafleet setup skill` runs only the skills download — and record the installed `cafleet` version per coding-agent home in a new `skill_installs` table so that fleet-scoped commands refuse to run against stale skills (GitHub issue #152). The `cafleet db` group is removed, completing the design-0000111 hard cut from the Alembic migration chain to the single-baseline schema that SPEC.md and docs/ already describe.

## Success Criteria

- [ ] `cafleet setup` (bare, no options) runs the db half first, then installs skills into every detected coding-agent home and records one `skill_installs` row per home; halves fail independently and the command exits non-zero if either failed.
- [ ] `cafleet setup db` creates the single-baseline schema (idempotent) and touches nothing else; `cafleet setup skill [--agent <name>]...` installs only the skills, fails with guidance when the `skill_installs` table is missing, and records the version rows on success.
- [ ] `cafleet db` no longer exists: `cafleet db init` fails with Click's standard no-such-command error, and no mention of the `db` group or Alembic remains anywhere in the repository outside `design-docs/` (per the removal rule).
- [ ] The Alembic chain (`0001`–`0005`), `alembic.ini`, `env.py`, the upgrade-guard machinery, and the `alembic` dependency are deleted; the schema is created by `Base.metadata.create_all` semantics (`CREATE TABLE IF NOT EXISTS`).
- [ ] Every fleet-scoped command (`fleet *`, `member *`, `message *`, `monitor *`) hard-errors with an actionable message when no skills install is recorded or when any recorded version differs from the runtime CLI version; `cafleet doctor` reports the per-home detail.
- [ ] README.md, SPEC.md, docs/, CLAUDE.md, and the admin Dashboard hint are updated in this cycle (documentation-first); `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //admin:lint` pass.

---

## Background

`cafleet setup` (`cafleet/src/cafleet/cli/setup.py`) currently runs two independent halves: a skills half (download `cafleet-skills-v<version>.zip` from the GitHub Release matching the installed CLI version, extract the three skill dirs into each target home) and a db half (`run_db_init()` — an Alembic upgrade to head). `cafleet db init` (`cafleet/src/cafleet/cli/db.py`) is a thin wrapper over the same `run_db_init()`.

Design 0000111 (Status: Complete) already rewrote SPEC.md §8/§11 and docs/ to a world with **no** `db init` command and **no** migration chain — a single-baseline `CREATE TABLE IF NOT EXISTS` schema — but its code refactor was explicitly out of scope, so the code still ships the Alembic chain `0001`–`0005`. This design completes that cut while delivering issue #152: nothing records which CLI version installed the skills, so after `uv tool upgrade cafleet` the skills silently go stale until the operator remembers to re-run `cafleet setup`.

---

## Specification

### Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | Complete the 0000111 hard cut: delete Alembic entirely; `setup db` runs the single-baseline metadata create. | User-confirmed (Q1). SPEC/docs already describe this world; the code catches up. Existing DBs are not migrated (unchanged 0000111 stance): delete the DB file and re-run `cafleet setup`. |
| 2 | New table `skill_installs`, one row per coding-agent home: `coding_agent` (PK), `cafleet_version`, `installed_at`. Minimal — no extra columns. | User-confirmed (Q2). `setup skill --agent codex` can run at a different time/version than the claude install, so per-home granularity is required to detect a partial-stale state. |
| 3 | `--agent` is removed from bare `cafleet setup` and lives only on `cafleet setup skill`. Bare `setup` targets **every detected** coding-agent home (the existing auto-detect: each home whose parent dir exists); zero detected homes → the skills half fails as today. | User-requested surface change (Q2 addendum: bare setup = all coding agents, `--agent` = single-home targeting on `setup skill`). "All" is realized as all *detected* homes: unconditionally creating `~/.codex` / `~/.config/opencode` on a machine without those agents would fabricate homes the auto-detect contract deliberately avoids. |
| 4 | Version mismatch is a **hard error** on every fleet-scoped command group (`fleet`, `member`, `message`, `monitor`); `doctor` reports the detail; `setup`, `doctor`, and `server` are exempt. | User-picked "error broadly + doctor detail" (Q3). Guarding the four fleet-scoped groups is the broadest surface that still leaves the repair commands (`setup`) and diagnostics (`doctor`) reachable. |
| 5 | Mismatch = simple string inequality against `importlib.metadata.version("cafleet")` (a downgrade also triggers). Missing table or empty table = "setup never ran" → same hard error. | User-accepted defaults (Q3). |
| 6 | Bare `setup` runs the **db half first, then the skills half** (reversed from today); halves stay independent-failure. The skills half records the version rows, which requires the schema. | User-confirmed (Q4). If the db half failed, the skills half fails its schema pre-flight and both halves are reported failed. |
| 7 | `setup skill` **fails with guidance** when the `skill_installs` table is absent — it does not auto-create the schema. `setup db` never touches `skill_installs` rows (schema only). | User-confirmed (Q4). |
| 8 | The `cafleet db` group is removed hard: no alias, no deprecation notice; every mention swept in this cycle. | User-confirmed (Q5); repo removal rule. |
| 9 | The guard checks **every existing row** of `skill_installs`: any row whose `cafleet_version` ≠ runtime version is stale. Homes with no row (agent never installed) are not checked. | With per-home rows there is no per-command coding-agent context on `message *` / `monitor *`; checking recorded rows only keeps the guard global, cheap, and free of false positives for agents the operator never installed. |

### Data model — `skill_installs`

Added to `cafleet/src/cafleet/db/models.py` and to the single baseline:

```python
class SkillInstall(Base):
    __tablename__ = "skill_installs"

    coding_agent: Mapped[str] = mapped_column(String, primary_key=True)
    cafleet_version: Mapped[str] = mapped_column(String, nullable=False)
    installed_at: Mapped[str] = mapped_column(String, nullable=False)
```

- `coding_agent` — one of `claude` / `codex` / `opencode` (the `AGENT_SKILLS_DIRS` keys); not FK-linked, not AUTOINCREMENT.
- `cafleet_version` — the exact `importlib.metadata.version("cafleet")` string at install time.
- `installed_at` — UTC ISO-8601 string, produced by `cafleet.broker._shared.now_iso()` (the existing timestamp convention).
- Writes are upserts (`session.merge`): re-installing replaces the row for that home.

Access helpers live in a new `cafleet/src/cafleet/broker/skill_installs.py` (the CLI accesses SQLite through the broker package):

```python
def skill_installs_table_exists() -> bool: ...        # sqlalchemy.inspect(engine).has_table
def list_skill_installs() -> list[dict]: ...          # [{coding_agent, cafleet_version, installed_at}], ORDER BY coding_agent
def record_skill_install(coding_agent: str, cafleet_version: str) -> None: ...  # merge + now_iso()
```

### Schema create — replacing Alembic

New module `cafleet/src/cafleet/db/schema.py`:

```python
def create_schema() -> Path:
    """Create the single-baseline schema; returns the DB file path."""
    # 1. force a sync sqlite URL from settings.database_url (as _sync_db_url does today)
    # 2. mkdir -p the DB file's parent
    # 3. Base.metadata.create_all(engine)   # checkfirst=True → CREATE TABLE IF NOT EXISTS
```

- `Base.metadata.create_all` is already exercised against this exact metadata by `tests/conftest.py`, so the fleets↔agents FK cycle is known to create cleanly on SQLite.
- Idempotent: re-running creates nothing new. Pre-existing tables are never altered or migrated; an old Alembic-era DB keeps its orphan `alembic_version` table untouched (harmless if it was at head `0005`; anything older is the documented delete-and-recreate case).

Deleted: `cafleet/src/cafleet/cli/db.py`, `cafleet/src/cafleet/db/alembic.ini`, `cafleet/src/cafleet/db/alembic/` (env.py, script.py.mako, versions `0001`–`0005`), the `alembic` dependency and the two `force-include` asset entries in `cafleet/pyproject.toml`, followed by `mise //:uv-sync`.

### CLI surface

| Command | Options | Behavior |
|---------|---------|----------|
| `cafleet setup` | none | Click group with `invoke_without_command=True`. The group callback **no-ops unless `ctx.invoked_subcommand is None`** (so `setup db` / `setup skill` never trigger the bare-setup sequence). Bare invocation runs the **db half** (`create_schema()`), then the **skills half** (auto-detect targets → download → install → record rows). Each half catches its own `ClickException` and prints `db half failed: <msg>` / `skills half failed: <msg>`; if anything failed, exit 1 with `<failed halves joined by ' and '> half failed` (db listed first, matching the new run order). |
| `cafleet setup db` | none | Runs `create_schema()` only. Prints `schema ready at <db_file>`. Never touches `skill_installs` rows. |
| `cafleet setup skill` | `--agent` (choice `claude`/`codex`/`opencode`, repeatable, deduped) | Pre-flight: `skill_installs` table must exist, else error `the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first`. Then resolve targets (`--agent` values, else auto-detect), download and install exactly as today, and after each home's install succeeds call `record_skill_install(agent, cli_version)`. Success output per home is unchanged: `<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>`. Install failure aborts the loop as today; rows recorded for homes completed before the failure remain. |

- The existing helpers (`_resolve_targets`, `_resolve_download_url`, `_download_and_extract`, `_install_skills`) and all their error strings are unchanged except that `_install_skills` gains the per-home `record_skill_install` call.
- `cli/__init__.py`: `cli.add_command(db_group)` is removed; the `setup` group registration is unchanged. `cafleet db init` afterwards fails with Click's standard `No such command 'db'.` (exit 2) — that absence is the regression test.
- No command in the `setup` group accepts `--fleet-id` (unchanged).

### Version guard

New helper `ensure_skills_current()` in `cafleet/src/cafleet/cli/_helpers.py`, called at the top of the `fleet`, `member`, `message`, and `monitor` group callbacks (one line per group; runs before any subcommand body):

1. If the DB file, the `skill_installs` table, or all rows are missing → `ClickException`: `no skills install is recorded; run 'cafleet setup' first`
2. Collect rows where `cafleet_version != importlib.metadata.version("cafleet")`. If any → `ClickException`: `stale skills detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup skill' to reinstall` — stale agents listed in ascending `coding_agent` order (`list_skill_installs()` applies `ORDER BY coding_agent`; for the three known agents this equals the `AGENT_SKILLS_DIRS` key order), e.g. `stale skills detected (claude=0.5.0, codex=0.5.0; CLI 0.6.0); run 'cafleet setup skill' to reinstall`
3. Otherwise return silently.

Exempt surfaces: the `setup` group (must remain runnable to repair), `doctor` (reports instead of blocking), `server` (WebUI is human-facing and not fleet-scoped).

Help interaction (intended, recorded as contract in cli-options.md/SPEC): group-level help (`cafleet fleet --help`) is parsed eagerly during the group's own context and prints help before the callback runs, so it always works. Subcommand help (`cafleet fleet create --help`) runs the group callback first, so under a missing/stale install the guard **errors instead of printing help** — accepted: the repair is a single command, and special-casing help would complicate an otherwise one-line guard.

### `cafleet doctor` — skills detail

After the existing tmux block (text and JSON forms both), report the runtime version and every `skill_installs` row:

```
skills:
  cli_version: 0.6.0
  claude:      0.6.0 (2026-07-04T00:12:09.123456+00:00) ok
  codex:       0.5.0 (2026-06-20T10:00:00.987654+00:00) STALE
```

- The stored `installed_at` string is printed **verbatim** (microsecond precision, as `now_iso()` produces it).
- No rows / table missing → `skills:` followed by `  (no skills install recorded; run 'cafleet setup')`.
- JSON form gains a sibling `"skills"` key: `{"cli_version": "<v>", "installs": [{"coding_agent": ..., "cafleet_version": ..., "installed_at": ..., "current": true|false}]}` (empty `installs` list when none).
- The existing tmux requirement (`ensure_tmux_or_die`) is unchanged.

### Removal sweep (per the repo removal rule)

| Surface | Change |
|---------|--------|
| `cafleet/src/cafleet/cli/db.py` | Delete file; drop registration in `cli/__init__.py`. |
| `cafleet/src/cafleet/db/alembic*` | Delete `alembic.ini` + `alembic/` tree. |
| `cafleet/pyproject.toml` | Drop `alembic` dep + the two `force-include` alembic asset lines. |
| `CLAUDE.md` | Rewrite the "Unified CLI command" bullet: `setup` (with `db` / `skill` subcommands) for onboarding and schema management — the stale "`db init` … `session` for session CRUD" text goes. |
| `admin/src/components/Dashboard.tsx` | Rewrite the no-Administrator hint: drop both `db init` code spans and the "backfill migration" sentence; new guidance = if you just upgraded from an old schema, delete the database file and re-run `cafleet setup`; a manually deleted Administrator still needs the operator. |
| `cafleet/tests/db/test_alembic_smoke.py`, `cafleet/tests/db/test_init.py`, `cafleet/tests/_helpers.py` (alembic cfg helper), alembic usage in `cafleet/tests/broker/test_monitor.py`, `alembic_version` / `alembic stamp head` assertions in `cafleet/tests/cli/test_setup.py` | Delete or rework (Step 6). |
| `cafleet/tests/cli/test_fleet.py` (a `db init` comment), `cafleet/tests/cli/test_fleet_flag.py` (two `db init` tests: the without-`--fleet-id` success test at line 148 and the `--fleet-id`-rejection test at line 177) | Reword the comment; delete both tests — their subject command no longer exists, and the `cafleet db` no-such-command regression test covers the absence (Step 6). |

### Documentation targets (updated first, Step 1)

| Target | Change |
|--------|--------|
| `docs/get-started/install.md` | `setup` as a group: bare = everything (db first, then skills for every detected home, no `--agent`); `setup db`; `setup skill [--agent]`; version recording + the stale-skills hard error; re-run guidance after upgrade becomes "any fleet-scoped command tells you". |
| `docs/get-started/contributing.md` | Adjust the `cafleet setup` line to the new surface (contributors typically want `cafleet setup db`). |
| `docs/concepts/storage.md` | Schema management: `cafleet setup` (or `setup db`) creates the baseline; reword "no version table" to "no schema-version table"; describe `skill_installs` (records the CLI version that installed the skills — not a schema version) and the guard. |
| `docs/spec/data-model.md` | Add the `skill_installs` table (columns, PK, upsert semantics); bump the table count. |
| `docs/spec/cli-options.md` | Rewrite §`cafleet setup` as the group + two subcommands with the exact flags/errors above; add subcommand-summary rows; document the guard error strings on the four fleet-scoped groups, including the subcommand-`--help` interaction; extend the `doctor` section. |
| `README.md` + `SPEC.md` | Via the `/update-readme` skill after docs/ land: quick start (`cafleet setup` unchanged as the one-liner), commands table row(s); SPEC §setup rewrite, §5.2/§8 seven-table baseline incl. `skill_installs`, §11 upgrade semantics (skills staleness + guard), doctor output contract, guard error strings as contract strings. |

### Test plan (Step 6, user-confirmed Q6)

- `cafleet/tests/db/test_schema.py` (replaces `test_init.py` / `test_alembic_smoke.py`): `create_schema()` creates **exactly** the seven baseline tables (asserted as set equality, which also proves no legacy schema-version table appears) + the four indexes, is idempotent, and creates the parent dir.
- `cafleet/tests/cli/test_setup.py` rework: bare `setup` runs db-then-skills and records rows; `--agent` on bare `setup` now exits 2 (`No such option`); `setup db` creates schema only and writes no rows; `setup skill` pre-flight error string; `setup skill --agent claude` records exactly one row with the runtime version; re-install upserts.
- New guard tests (e.g. `cafleet/tests/cli/test_skills_guard.py`): empty-table error string, stale-row error string (both halves of the message), matching rows pass, unguarded `setup` / `doctor` / `server` still run, and the help contract (`fleet --help` prints help under a stale install; `fleet create --help` errors).
- New `cafleet/tests/cli/test_db_group_removed.py` (mirroring `test_agent_group_removed.py`): `cafleet db init` fails with Click's standard no-such-command error.
- `cafleet/tests/cli/test_doctor.py`: skills section in text + JSON, `(no skills install recorded; run 'cafleet setup')` branch.
- `cafleet/tests/conftest.py`: after `Base.metadata.create_all`, seed one `skill_installs` row (`claude`, runtime version) so existing fleet-scoped CLI tests pass the guard.
- Sweep alembic imports out of `tests/_helpers.py` and `tests/broker/test_monitor.py` (rebuild those fixtures on `create_schema()` / `create_all`); reword the `db init` comment in `tests/cli/test_fleet.py` and delete both `db init` tests in `tests/cli/test_fleet_flag.py` (the without-`--fleet-id` success test and the `--fleet-id`-rejection test).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (first, per documentation-maintenance.md)

- [ ] Update `docs/get-started/install.md` to the `setup` group surface, version recording, and the stale-skills guard <!-- completed: -->
- [ ] Update `docs/get-started/contributing.md` (`setup db` for contributors) <!-- completed: -->
- [ ] Update `docs/concepts/storage.md` (schema management wording, `skill_installs`, guard) <!-- completed: -->
- [ ] Update `docs/spec/data-model.md` (add `skill_installs`, table count) <!-- completed: -->
- [ ] Update `docs/spec/cli-options.md` (setup group + subcommands, guard errors, doctor) <!-- completed: -->
- [ ] Update `README.md` and `SPEC.md` via the `/update-readme` skill <!-- completed: -->
- [ ] Update `CLAUDE.md` project bullet (drop `db init` / `session` text) <!-- completed: -->

### Step 2: Schema and Alembic removal

- [ ] Add `SkillInstall` to `cafleet/src/cafleet/db/models.py` <!-- completed: -->
- [ ] Add `cafleet/src/cafleet/db/schema.py::create_schema()` <!-- completed: -->
- [ ] Delete `cafleet/src/cafleet/db/alembic.ini` + `alembic/` tree; drop the `alembic` dep and force-include lines from `cafleet/pyproject.toml`; run `mise //:uv-sync` <!-- completed: -->
- [ ] Add `cafleet/src/cafleet/broker/skill_installs.py` (exists/list/record helpers) <!-- completed: -->

### Step 3: CLI restructure

- [ ] Rewrite `cafleet/src/cafleet/cli/setup.py` as the `setup` group (bare invocation, `db`, `skill` with `--agent`, pre-flight, version recording) <!-- completed: -->
- [ ] Delete `cafleet/src/cafleet/cli/db.py` and its registration in `cli/__init__.py` <!-- completed: -->

### Step 4: Version guard and doctor

- [ ] Add `ensure_skills_current()` to `cli/_helpers.py`; wire into the `fleet` / `member` / `message` / `monitor` group callbacks <!-- completed: -->
- [ ] Extend `cafleet doctor` with the skills section (text + JSON) <!-- completed: -->

### Step 5: Admin WebUI

- [ ] Reword the `Dashboard.tsx` no-Administrator hint (drop `db init`; delete-and-recreate guidance) <!-- completed: -->

### Step 6: Tests

- [ ] Replace `tests/db/test_init.py` + `tests/db/test_alembic_smoke.py` with `tests/db/test_schema.py` <!-- completed: -->
- [ ] Rework `tests/cli/test_setup.py` to the group surface (incl. `--agent` rejection on bare `setup`, pre-flight, upsert) <!-- completed: -->
- [ ] Add guard tests (incl. the help contract) + doctor skills-section tests; add `tests/cli/test_db_group_removed.py` <!-- completed: -->
- [ ] Seed `skill_installs` in `tests/conftest.py`; purge alembic from `tests/_helpers.py` and `tests/broker/test_monitor.py`; reword the `db init` comment in `tests/cli/test_fleet.py` and delete both `db init` tests in `tests/cli/test_fleet_flag.py` <!-- completed: -->

### Step 7: Verification and final sweep

- [ ] Grep sweep: no `db init` / `alembic` / `db` -group mention remains outside `design-docs/` <!-- completed: -->
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint` all pass <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-04 | Initial draft from issue #152 + user answers (Q1–Q5 in Decisions, Q6 in the Test plan; incl. `--agent` moved to `setup skill` only). |
| 2026-07-04 | Reviewer round 1: setup-group callback no-op condition; subcommand `--help` guard interaction specified as contract; deterministic stale-agent ordering; verbatim microsecond timestamps in doctor; test-sweep file list corrected (`db init` mentions vs `alembic_version` assertions); storage.md wording de-alembicized; regression test homed at `test_db_group_removed.py`. |
| 2026-07-04 | Reviewer round 2: both `db init` tests in `test_fleet_flag.py` covered for deletion. User-approved; Status: Approved. |
| 2026-07-04 | Post-approval user tweak: `setup skill` pre-flight error now suggests both repair commands (`run 'cafleet setup' or 'cafleet setup db' first`). |
