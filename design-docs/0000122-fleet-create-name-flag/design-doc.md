# Rename fleet `--label` to `--name` (end-to-end)

**Status**: Approved
**Progress**: 39/39 tasks complete
**Last Updated**: 2026-07-07

## Overview

Rename the fleet's human-readable identifier from `label` to `name` across every layer — CLI flag, DB column, broker API, output formatters, JSON/HTTP contract, admin frontend, docs, and skills — as a hard-break with no `--label` alias. The new `cafleet fleet create --name` becomes a **required** option, mirroring `cafleet member create --name` exactly. This removes a terminological asymmetry between the two sibling `create` subcommands that repeatedly trips up coding agents.

## Success Criteria

- [x] `cafleet fleet create --name "X"` sets the fleet's name; `cafleet fleet create` with no `--name` errors with Click's missing-required-option message (exit 2).
- [x] `cafleet fleet list` shows a `NAME` column; `cafleet fleet show` shows a `name:` field; the `--full` create render shows a `name:` line.
- [x] The JSON output of `fleet create` / `fleet list` / `fleet show --json` and the WebUI `/fleets` HTTP response emit the key `name` (never `label`).
- [x] The `fleets` table column is `name`; a data-preserving, reversible Alembic migration `0009` renames `label → name` (upgrade) and `name → label` (downgrade); `mise //cafleet:test` passes including the alembic smoke test.
- [x] The admin frontend reads `fleet.name`; `mise //admin:build` regenerates the bundle to read `fleet.name`. (`cafleet/src/cafleet/webui/dist/` is a gitignored build artifact, not committed; the publish flow rebuilds it fresh — see Step 7.)
- [x] `grep -rn "label"` over source, tests, docs, SPEC.md, README.md, and skills returns **no** fleet-`label` reference (only unrelated incidental `label` uses remain, plus the immutable `0001` + the `0009` rename migrations).
- [x] The package version is bumped `0.16.0 → 0.17.0` to signal the breaking JSON/HTTP contract change.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //admin:lint` all pass.

---

## Background

`cafleet` has two sibling `create` subcommands that both attach a human-readable name to the thing they create:

| Subcommand | Flag today | Requiredness |
|:--|:--|:--|
| `cafleet member create` | `--name` | required (`member.py:158`, `required=True`) |
| `cafleet fleet create` | `--label` | optional (`fleet.py:24`, `default=None`) |

The `cafleet` skill documents `member create --name` heavily — every member-spawn example in every workflow body passes it — so after a coding agent loads the skill it internalizes the dominant "creating a thing takes `--name`" pattern. It then reaches for `--name` at `fleet create` and hits `No such option: --name`. The asymmetry is purely terminological: a fleet's "label" and a member's "name" are the same concept (a free-form human-readable identifier). Renaming `--label` to `--name`, and making it required to fully mirror `member create`, removes the asymmetry at its source.

This is a **breaking change** to two contract surfaces — the JSON output of the fleet commands and the WebUI `/fleets` HTTP response both rename the key `label → name`. Per this project's culture, SPEC.md JSON key order and text layouts **are** the contract, so the change is stated explicitly in the Specification and carries a minor version bump. Per the project's removal rule, the rename is a total hard-break: after it lands the repository reads as if fleets never had a `label` — no alias, no deprecation notice.

---

## Specification

### The rename, layer by layer

| Layer | Site (file) | Before | After |
|:--|:--|:--|:--|
| CLI flag | `cli/fleet.py:24` | `@click.option("--label", default=None, help="Optional human-readable label.")` | `@click.option("--name", required=True, help="Human-readable name for the fleet.")` |
| CLI param | `cli/fleet.py:38,54` | `label: str \| None`, `label=label` | `name: str`, `name=name` |
| CLI list header | `cli/fleet.py:80` | `{'LABEL':<20}` | `{'NAME':<20}` |
| CLI list row | `cli/fleet.py:86` | `{r['label'] or '':<20}` | `{r['name'] or '':<20}` |
| CLI show field | `cli/fleet.py:106` | `f"label:      {result['label'] or ''}"` | `f"name:       {result['name'] or ''}"` |
| Formatter | `output/formatters.py:127` | `f"label:            {data['label'] or ''}"` | `f"name:             {data['name'] or ''}"` |
| Broker param | `broker/fleets.py:16` | `create_fleet(label: str \| None = None, ...)` | `create_fleet(name: str \| None = None, ...)` |
| Broker write | `broker/fleets.py:63` | `Fleet(label=label, ...)` | `Fleet(name=name, ...)` |
| Broker returns | `broker/fleets.py:117,163,192` | `"label": ...` | `"name": ...` |
| Broker select | `broker/fleets.py:136,152` | `Fleet.label` | `Fleet.name` |
| DB model | `db/models.py:15` | `label: Mapped[str \| None]` | `name: Mapped[str \| None]` |
| WebUI API | `webui/api.py:87-89` | (returns `broker.list_fleets()` verbatim) | unchanged code — the key renames automatically via the broker dict |
| Admin type | `admin/src/types.ts:48` | `label: string \| null` | `name: string \| null` |
| Admin consumers | `App.tsx`, `Dashboard.tsx`, `FleetPicker.tsx`, `AppHeader.tsx` | `fleet.label`, `fleetLabel`, `label` | `fleet.name`, `fleetName`, `name` |

Text-layout note: `"name"` is one character shorter than `"label"`, so the padding after the colon in the `fleet show` and `format_fleet_create --full` lines is widened by one space to keep the value column aligned. The exact new layout is recorded in SPEC.md (below).

The broker `create_fleet` keyword stays typed `str | None` (the schema column remains nullable for pre-existing rows), but the CLI now always supplies a value because `--name` is required.

### `--name` becomes required

`cafleet fleet create` gains `required=True` on `--name`, matching `member create`. Consequences that MUST be reflected in the docs/spec:

- `cafleet fleet create` (and `cafleet fleet create --json`) with no `--name` now fails with Click's standard `Missing option '--name'` message and **exit code 2** (a `UsageError`, not the exit-1 runtime errors elsewhere). The `fleet create` error rows in `docs/spec/cli-options.md` and `SPEC.md` gain this case.
- Every doc, skill, and workflow-body example that runs `fleet create` MUST pass `--name`. Bare `cafleet fleet create --json` examples are updated to supply a name value. The create/execute/interview/research workflow bodies already pass a label string, so those become `--name "<same string>"`.

### JSON / HTTP contract change (breaking)

The renamed key appears in the same position it occupied before (key order is otherwise unchanged):

| Command / endpoint | Emitting site | Contract shape (after) |
|:--|:--|:--|
| `fleet create [--json]` | `create_fleet` return | `{fleet_id, name, created_at, administrator_agent_id, director:{…}}` |
| `fleet list [--json]` / WebUI `GET /fleets` | `list_fleets` return / `webui/api.py:89` | one record `{fleet_id, director_agent_id, name, created_at, agent_count}` |
| `fleet show [--json]` | `get_fleet` return | `{fleet_id, name, created_at, deleted_at, director_agent_id}` |

`docs/spec/webui-api.md` documents the `/fleets` `label` key and MUST be updated to `name`. The WebUI code path needs no change — `webui/api.py` returns the broker dicts verbatim.

### Persistence: migration `0009`

A data-preserving, reversible column rename using the project's established SQLite-safe batch pattern (as in `0008`). The `fleets` table declares `sqlite_autoincrement=True`, so the batch recreate MUST carry the `table_kwargs` (as `0007` does for `tasks`) or it silently drops the AUTOINCREMENT clause.

Mirror `0008_backend_neutral_placement_columns.py` exactly for the module docstring (`Revision ID` / `Revises` / `Create Date`), the imports, and the TYPED module-level identifiers — only the `upgrade`/`downgrade` bodies differ:

```python
"""rename fleet label to name

Revision ID: 0009
Revises: 0008
Create Date: 2026-07-07 00:00:00.000000

Rename the ``fleets.label`` column to ``fleets.name``. Data-preserving: the
existing values carry over unchanged. The column stays nullable.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0009"
down_revision: str | None = "0008"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table(
        "fleets", schema=None, table_kwargs={"sqlite_autoincrement": True}
    ) as batch_op:
        batch_op.alter_column(
            "label", new_column_name="name", existing_type=sa.String()
        )


def downgrade() -> None:
    with op.batch_alter_table(
        "fleets", schema=None, table_kwargs={"sqlite_autoincrement": True}
    ) as batch_op:
        batch_op.alter_column(
            "name", new_column_name="label", existing_type=sa.String()
        )
```

Existing rows' values carry over unchanged (a rename preserves data). The column stays `nullable=True`.

### Version bump

Bump the minor version `0.16.0 → 0.17.0` via the project's `bump-my-version` flow (`.bumpversion.toml`, `current_version = "0.16.0"`), which updates `.bumpversion.toml`, `cafleet/pyproject.toml`, and `uv.lock`, and creates the `Bump version: 0.16.0 → 0.17.0` commit — mirroring the existing `0.15.1 → 0.16.0` commit. The bump signals the breaking JSON/HTTP contract change.

### Out of scope / non-goals

- No `--label` alias, no deprecation shim, no dual-read of both keys. After this change the repo reads as if `label` never existed for fleets (removal rule).
- Incidental, unrelated uses of the word "label" are left untouched (e.g. Alembic `branch_labels` boilerplate, `member show --full` "labeled block" prose in `README.md:165`, admin CSS/aria labels, chart/axis labels in `cafleet-research` skills).

---

## Implementation

> Documentation-first order (per `.claude/rules/documentation-maintenance.md`): docs, README, SPEC, and skills are updated before code. The design doc is committed on this feature branch (project override).
>
> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`. Check the box and record the timestamp in the same edit.

### Step 1: Documentation — `docs/`

- [x] `docs/spec/cli-options.md`: `--label` row → `--name` (required); the `cafleet fleet create --label "my-project"` example (line ~60) → `--name`; text-output `label:` → `name:` (lines ~322, ~382); JSON `"label"` → `"name"` (line ~334); `LABEL` column (line ~367) → `NAME`; the list description (line ~365) `label` → `name`; add the missing-required-`--name` (exit 2) case to the `fleet create` error table. <!-- completed: 2026-07-07T13:02 -->
- [x] `docs/spec/data-model.md:20`: rename the fleet column row `| \`label\` | \`TEXT\` | nullable | Optional free-form text for human bookkeeping (e.g. \`"PR-42 review"\`). |` → `| \`name\` | \`TEXT\` | nullable | …` (its Notes prose now reads as the fleet's name). <!-- completed: 2026-07-07T13:02 -->
- [x] `docs/spec/webui-api.md`: the `/fleets` schema key `"label"` → `"name"` (and its example value). <!-- completed: 2026-07-07T13:02 -->
- [x] `docs/get-started/quickstart.md`: `cafleet fleet create --label "demo"` → `--name "demo"`. <!-- completed: 2026-07-07T13:02 -->
- [x] `docs/how-to/mixed-backend-team.md`: `cafleet fleet create --label "demo" --coding-agent claude` → `--name "demo" ...`. <!-- completed: 2026-07-07T13:02 -->
- [x] `docs/reference/coding-agents/claude.md`, `codex.md`, `opencode.md`: each `cafleet fleet create --label <…>-smoke …` → `--name <…>-smoke …`. <!-- completed: 2026-07-07T13:02 -->

### Step 2: README + SPEC

- [x] `README.md:78`: `cafleet fleet create --label "my-project"` → `--name "my-project"`. (Leave `README.md:165` "labeled block" — incidental, not the fleet label.) <!-- completed: 2026-07-07T13:08 -->
- [x] `SPEC.md`: line 262 schema-table row `label` → `name`; line 536 `create_fleet(label, …)` → `create_fleet(name, …)`; lines 550/551/557 return shapes `label` → `name`; **line 992** `--label` (string, optional) → `--name` (string, **required**) — call out the requiredness contract change, not just the rename; **line 998** the five-column list description `FLEET_ID / DIRECTOR / LABEL / AGENTS` → `NAME`; line 1002 text-output key `label` → `name`; the `fleet create --full` text layout (lines ~1558/1613) `label:` → `name:` with the widened padding; **line 2734** the manual-verification checklist item `cafleet fleet create (--label, …)` → `--name`; add the missing-required-`--name` (exit 2) case to the `fleet create` CLI error surface. <!-- completed: 2026-07-07T13:08 -->

### Step 3: Skills (first-class documentation targets)

- [x] `skills/cafleet-design-doc/create/create.md:92`: `--label "design-doc-create-{slug}"` → `--name "design-doc-create-{slug}"`. <!-- completed: 2026-07-07T13:12 -->
- [x] `skills/cafleet-design-doc/interview/interview.md:106`: `--label "design-doc-interview-{slug}"` → `--name …`. <!-- completed: 2026-07-07T13:12 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:157`: `--label "design-doc-execute-{slug}"` → `--name …`. <!-- completed: 2026-07-07T13:12 -->
- [x] `skills/cafleet-research/report/report.md:78`: `--label "research-[topic-slug]"` → `--name …`. <!-- completed: 2026-07-07T13:12 -->
- [x] `skills/cafleet-research/presentation/presentation.md:93`: `--label "present-[topic-slug]"` → `--name …`. <!-- completed: 2026-07-07T13:12 -->
- [x] `skills/cafleet/reference/cli.md`: `cafleet fleet create --label "my-project"` → `--name "my-project"`. <!-- completed: 2026-07-07T13:12 -->
- [x] Grep-sweep `skills/` for any remaining `fleet create --label` and any fleet-`label` prose; update each (distinguishing incidental `label` uses). <!-- completed: 2026-07-07T13:12 -->

### Step 4: Persistence — model + migration

- [x] `db/models.py:15`: `label: Mapped[str | None]` → `name: Mapped[str | None]`. <!-- completed: 2026-07-07T13:22 -->
- [x] Add `cafleet/src/cafleet/db/alembic/versions/0009_rename_fleet_label_to_name.py` (revision `0009`, down_revision `0008`) per the Specification code block: upgrade `label → name`, downgrade `name → label`, batch mode with `table_kwargs={"sqlite_autoincrement": True}`. <!-- completed: 2026-07-07T13:22 -->

### Step 5: Broker

- [x] `broker/fleets.py`: `create_fleet` param `label` → `name` (keyword + docstring); `Fleet(name=name, …)`; return-dict `"label"` → `"name"` in `create_fleet`/`list_fleets`/`get_fleet`; `Fleet.label` → `Fleet.name` in the `list_fleets` select + group_by; update the `create_fleet`/`get_fleet` Returns docstrings. <!-- completed: 2026-07-07T13:22 -->

### Step 6: CLI + formatters

- [x] `cli/fleet.py`: `--label` → `--name` (`required=True`, help "Human-readable name for the fleet."); param `label`→`name`; pass `name=name` to `broker.create_fleet`; `LABEL`→`NAME` header; row key `r['label']`→`r['name']`; `fleet show` `label:` → `name:` with widened padding. <!-- completed: 2026-07-07T13:22 -->
- [x] `output/formatters.py`: `format_fleet_create` — `data['label']`→`data['name']`, `label:` line → `name:` with widened padding, and the docstring reference `label` → `name`. <!-- completed: 2026-07-07T13:22 -->

### Step 7: Admin frontend + rebuild bundle

- [x] `admin/src/types.ts:48`: `label: string | null` → `name: string | null` in `FleetListItem`. <!-- completed: 2026-07-07T13:22 -->
- [x] `admin/src/App.tsx`: rename `fleetLabel`/`setFleetLabel`/`label` locals to `fleetName`/`setFleetName`/`name` and read `fleet.name` (lines ~30, 53, 69, 93, 98, 138). <!-- completed: 2026-07-07T13:22 -->
- [x] `admin/src/components/FleetPicker.tsx`: `onSelect(fleetId, name)` param + `fleet.name` (lines ~11, 19, 24, 30). <!-- completed: 2026-07-07T13:22 -->
- [x] `admin/src/components/Dashboard.tsx`: rename the `fleetLabel` prop it threads between `App.tsx` and `AppHeader.tsx` to `fleetName` — props type (line ~15 `fleetLabel: string | null;`), destructure (line ~23), and the `AppHeader` passthrough (line ~97 `fleetLabel={fleetLabel ?? String(fleetId)}` → `fleetName={fleetName ?? String(fleetId)}`). Leaving this unrenamed breaks the TS build once `App.tsx` passes `fleetName`. <!-- completed: 2026-07-07T13:22 -->
- [x] `admin/src/components/AppHeader.tsx`: `fleetName?` prop + usage + the breadcrumb comment (lines ~65, 66, 76, 102) and the `{fleetLabel !== undefined && onBack ? (` conditional (line ~90). <!-- completed: 2026-07-07T13:22 -->

- [x] Grep-sweep `admin/src/` for any remaining `fleet.label` / `fleetLabel`. <!-- completed: 2026-07-07T13:22 -->
- [x] Run `mise //admin:build` so the admin bundle regenerates to read `fleet.name` (verified: `tsc -b && vite build` passes; the emitted bundle reads `fleet.name`, 0× `fleet.label`). Note: `cafleet/src/cafleet/webui/dist/` is a **gitignored build artifact** (`cafleet/.gitignore:7: dist/`) — it is NOT committed (and MUST NOT be force-added). The publish flow `mise //cafleet:publish` rebuilds admin assets fresh (`//admin:build` → `//cafleet:build`), so the shipped wheel always carries a freshly-built bundle; a committed `dist/` is not the shipped source of truth. <!-- completed: 2026-07-07T13:22 -->

### Step 8: Tests

- [x] `cafleet/tests/cli/test_fleet.py`: `_seed_fleet(label=…)` param + `INSERT INTO fleets (…, label, …)` + `SELECT … label …` → `name`; `--label` invocations → `--name`; `data[0]["label"]` assertion → `["name"]`; rename `test_fleet_create__label_round_trip_and_default_none` (the default-none half is now covered by the required-flag behavior — assert the value round-trips and adjust the "default none" expectation). <!-- completed: 2026-07-07T13:11 -->
- [x] `cafleet/tests/cli/test_fleet_flag.py`: `--label` → `--name` at lines ~133/151/157. <!-- completed: 2026-07-07T13:11 -->
- [x] `cafleet/tests/cli/test_fleet_bootstrap.py`: `--label` → `--name`; `SELECT … label …` → `name`; `"label:" in text` → `"name:"`; top-level-keys assertion `"label"` → `"name"`; rename `test_fleet_create_json_output__label_propagates_to_json` → `…name_propagates…`. <!-- completed: 2026-07-07T13:11 -->
- [x] `cafleet/tests/broker/test_fleet_bootstrap.py`, `cafleet/tests/broker/test_fleet_list_director.py`, `cafleet/tests/broker/_helpers.py`: update every fleet-`label` seed/assertion to `name` — including `_helpers._create_fleet`'s `label=` param → `name=`. <!-- completed: 2026-07-07T13:11 -->
- [x] `cafleet/tests/broker/test_registry.py`: the primary broker-fleet test — every `_create_fleet(label=…)` call (lines ~24, 77, 111–113, 125) → `name=`, the test `test_create_fleet__shape_and_label_handling` (line ~17) → `…name_handling` and its `label` assertions → `name`. (These are fleet labels, **not** incidental.) <!-- completed: 2026-07-07T13:11 -->
- [x] `cafleet/tests/broker/test_member_activity.py:12` (`label="activity-test"`) and `cafleet/tests/cli/test_member.py:961` (`create_fleet(label=None, …)`): rename `label` → `name`. <!-- completed: 2026-07-07T13:11 -->

- [x] Add a regression test that bare `cafleet fleet create` (no `--name`) exits 2 with the missing-required-option error (tests the current required-flag behavior). <!-- completed: 2026-07-07T13:11 -->
- [ ] `cafleet/tests/db/test_alembic_smoke.py` (handle with care — a blind grep-rename breaks it): the seed at line ~399 `INSERT INTO fleets (fleet_id, label, created_at, …)` runs at a **pre-0008 revision** (old `tmux_*` placement columns), so `label` is CORRECT there and **must stay `label`** — the column is only renamed at `0009`. Update only post-upgrade (HEAD-schema) reads/assertions of the fleet identifier column → `name`. Add a `0009` round-trip assertion: upgrade renames `label → name` preserving the seeded value; downgrade reverses it. <!-- completed: 2026-07-07T13:11 -->
- [x] Grep-sweep `cafleet/tests/` for any remaining fleet-`label` reference. Genuinely incidental `label` uses (left untouched) are the field-label assertions/test-names in `test_render_task.py`, `test_member_show.py`, and `test_compact_formatters.py`. Note: `test_compact_formatters.py` also carried a *real* fleet-`label` ref — its `_fleet_create_data` helper key, renamed to `name` — and `test_member_delete.py`'s `get_fleet` stub key was renamed too; both were outside the explicit bullets but caught by this sweep. <!-- completed: 2026-07-07T13:11 -->


### Step 9: Version bump

- [x] Bump `0.16.0 → 0.17.0` via `bump-my-version bump minor` (updates `.bumpversion.toml`, `cafleet/pyproject.toml`, `uv.lock`; creates the bump commit). <!-- completed: 2026-07-07T13:29 -->

### Step 10: Verification

- [x] `mise //cafleet:test` (includes the alembic smoke test — 1044 pass, verified by the Tester), `mise //cafleet:lint`, `mise //cafleet:typecheck` all pass. <!-- completed: 2026-07-07T13:31 -->
- [x] `mise //admin:lint` passes; `grep -rn "label"` over source/tests/docs/SPEC/README/skills shows no fleet-`label` reference remains (the only fleet-`label` refs are the immutable `0001` historical migration and the `0009` rename migration itself; all other `label` hits are incidental — SQL `.label()` aliases, `branch_labels` boilerplate, HTML/aria labels, status-chip labels, field labels). <!-- completed: 2026-07-07T13:31 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-07 | Initial draft |
| 2026-07-07 | Reviewer round resolved; approved |
