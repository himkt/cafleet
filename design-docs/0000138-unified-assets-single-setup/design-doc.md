# Unified Assets Concept and Single Setup Command

**Status**: Approved
**Progress**: 0/26 tasks complete
**Last Updated**: 2026-07-18

## Overview

Unify the shipped skill/preset artifacts under a single "assets" concept — the GitHub release asset becomes `cafleet-assets-v<version>.zip` and the install-tracking table/model/module become `asset_installs` / `AssetInstall` / `broker/asset_installs.py` — and collapse the setup CLI to one command: `cafleet setup [--skip AGENT]...`. The `setup db` and per-agent `setup claude` / `setup codex` / `setup opencode` subcommands are removed (hard break, no aliases), along with coding-agent home auto-detection: bare `cafleet setup` always runs the DB half then installs assets for all three agents minus the skipped ones.

## Success Criteria

- [ ] `cafleet setup` is a plain Click command; `cafleet setup db`, `cafleet setup claude`, `cafleet setup codex`, and `cafleet setup opencode` fail with Click's standard "Got unexpected extra argument" error (no custom hint).
- [ ] Bare `cafleet setup` migrates the DB to head, then installs skills + presets for claude, codex, and opencode (creating agent homes as needed) and records one `asset_installs` row per agent.
- [ ] `cafleet setup --skip claude --skip codex --skip opencode` runs the DB half only and exits 0 — the documented schema-only path for contributors and `mise //cafleet:makemigration`.
- [ ] The publish workflow uploads `cafleet-assets-v<version>.zip` and the CLI downloads exactly that name; a missing release or asset remains a loud exit-1 failure.
- [ ] Migration `0003` moves every `skill_installs` row into `asset_installs` (create + copy + drop, data-preserving both ways); the chain-guard test asserts the 3-revision chain.
- [ ] No mention of `setup db`, per-agent setup subcommands, home auto-detection, `cafleet-skills-v`, or the `skill_installs` name family remains anywhere outside `design-docs/` and `cafleet/src/cafleet/db/alembic/versions/` (removal rule; migration scripts are immutable history, and migration `0003` itself necessarily references `skill_installs`); the project `CLAUDE.md` describes the single `setup` command.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

Today `cafleet setup` is a Click group (`invoke_without_command=True`): bare `setup` runs a DB half plus an assets half over auto-detected agent homes, `setup db` runs the DB half alone, and generated per-agent subcommands (`setup claude` / `codex` / `opencode`) run the assets half for one named agent. The release asset is named `cafleet-skills-v<version>.zip` even though it has carried both `skills/` and `presets/` since presets shipped, and the install-tracking surface (`skill_installs` table, `SkillInstall` model, `broker/skill_installs.py`, the "stale skills" guard, `doctor`'s `skills` report) still says "skills". Two problems follow: the naming misrepresents the artifact, and removing `setup db` requires a defined schema-only path — `mise //cafleet:makemigration` needs the DB at head, and on an unreleased dev version the assets half has no matching GitHub release, so bare `cafleet setup` would fail.

---

## Specification

### Naming map

| Surface | Old | New |
|---|---|---|
| Release asset | `cafleet-skills-v<version>.zip` | `cafleet-assets-v<version>.zip` |
| `publish.yml` job | `upload-skills` | `upload-assets` |
| `publish.yml` upload step | `Upload skills archive to the release` | `Upload assets archive to the release` |
| Temp download filename (`setup.py`) | `skills.zip` | `assets.zip` |
| DB table | `skill_installs` | `asset_installs` |
| SQLAlchemy model | `SkillInstall` | `AssetInstall` |
| Broker module | `cafleet/broker/skill_installs.py` | `cafleet/broker/asset_installs.py` |
| Broker functions | `record_skill_install` / `list_skill_installs` / `skill_installs_table_exists` | `record_asset_install` / `list_asset_installs` / `asset_installs_table_exists` |
| CLI guard function (`cli/_helpers.py`) | `ensure_skills_current` | `ensure_assets_current` |
| Spec/guard concept | "Stale-skills guard" | "Stale-assets guard" |
| `doctor` JSON key / text section | `"skills"` / `skills:` | `"assets"` / `assets:` |
| SPEC.md entity | `SkillInstalls` | `AssetInstalls` |

Table columns are unchanged: `coding_agent` (TEXT, PK), `cafleet_version` (TEXT, NOT NULL), `installed_at` (TEXT, NOT NULL). The word "skills" stays wherever it literally means the three skill directories — e.g. the per-agent echo `<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>` and the `zip -r ... skills presets` packaging step.

There is no dual or fallback asset-name lookup: each CLI version queries only its own version's release, and every release from this change forward carries the new name. Old releases are untouched.

### CLI: `cafleet setup [--skip AGENT]...`

`setup` becomes a plain `click.command` (the group, `setup db`, and the generated per-agent commands are deleted). Former subcommand invocations fail with Click's standard `Got unexpected extra argument (<word>)` error.

| Option | Spec |
|---|---|
| `--skip AGENT` | Repeatable. `click.Choice(["claude", "codex", "opencode"])`; an unknown value fails with Click's standard invalid-choice error (exit 2). Duplicates are deduplicated. Help: `Skip the named agent's assets install (repeatable).` |

Command help: `Migrate the database schema and install the coding-agent assets (skills and presets).`

Execution — two halves, in order:

1. **DB half** — `run_db_init()` unchanged (idempotent migration to the bundled Alembic head; same output lines and same two refused states as today).
2. **Assets half** — targets are the fixed list `["claude", "codex", "opencode"]` (in that order) minus the skipped agents. Auto-detection is removed entirely: no home-directory probing, no "no coding-agent homes detected" error, and `_resolve_targets` is deleted. Installation creates each target's directories as needed (the existing `mkdir(parents=True, exist_ok=True)` calls in the install path already do this), so the forced-install ability of the old per-agent subcommands is now universal.
   - If **all three** agents are skipped, the assets half is skipped entirely: the command echoes `assets half skipped (all agents skipped)` and the half counts as not-run.
   - Otherwise the half runs the existing sequence: schema preflight (`asset_installs_table_exists()`), release lookup for the installed CLI version, download + archive validation, then per target: install the three skill directories (delete-and-reinstall), install the bundled preset where one exists (codex, opencode), upsert the agent's `asset_installs` row, echo the existing install lines. Per-target install order, echo shapes, and abort-on-failure semantics are unchanged.

Failure semantics (unchanged shape): each half that runs and fails prints `db half failed: <msg>` / `assets half failed: <msg>`; the command exits 1 with `Error: <halves> half failed` (halves joined by `' and '`) when any half that ran failed. A skipped assets half is not-run and cannot contribute a failure. A missing release (`no release found for version <version>`) or missing asset (`asset cafleet-assets-v<version>.zip not found in release <version>`) remains a loud assets-half failure — no exit-code downgrade.

The schema preflight error becomes: `the database schema is missing or outdated; run 'cafleet setup' first`. (Within the single command the DB half always runs first, so this fires only after a DB-half failure or an externally broken schema; it is kept as defense.)

The opencode backend's missing-preset error (`coding_agent/opencode.py`) becomes: `opencode agent preset not found at <path>; run 'cafleet setup' first`.

#### Schema-only path (contributors and CI)

The documented invocation for "bring the DB to head without touching assets" is:

```bash
cafleet setup --skip claude --skip codex --skip opencode
```

It is deterministic (independent of which agent homes exist), exits 0 when the DB half succeeds, and never contacts GitHub — so it works on unreleased dev versions. `mise //cafleet:makemigration`'s description, `.claude/rules/database-migrations.md`, `.claude/rules/commands.md`, and `docs/contributing.md` reference this invocation wherever they previously said `cafleet setup db`. (CI workflows never ran `setup db` — the test suite builds its schema in fixtures — so no workflow change is needed for this.)

### Data model and migration `0003`

`db/models.py`: `SkillInstall` → `AssetInstall` with `__tablename__ = "asset_installs"`; columns unchanged. `broker/skill_installs.py` is renamed to `broker/asset_installs.py` with the three functions renamed per the naming map; `asset_installs_table_exists()` inspects `has_table("asset_installs")`. Consumers of the broker module (`cli/_helpers.py`, `cli/doctor.py`, `cli/setup.py`) update their imports; the `ensure_skills_current` → `ensure_assets_current` rename additionally updates its importers `cli/fleet.py`, `cli/member.py`, `cli/message.py`, and `cli/monitor.py`.

Migration `0003` (chain kept: `0001` → `0002` → `0003`) is create + copy + drop, data-preserving in both directions:

```python
def upgrade() -> None:
    op.create_table(
        "asset_installs",
        sa.Column("coding_agent", sa.String(), primary_key=True),
        sa.Column("cafleet_version", sa.String(), nullable=False),
        sa.Column("installed_at", sa.String(), nullable=False),
    )
    op.execute(
        "INSERT INTO asset_installs (coding_agent, cafleet_version, installed_at) "
        "SELECT coding_agent, cafleet_version, installed_at FROM skill_installs"
    )
    op.drop_table("skill_installs")


def downgrade() -> None:
    op.create_table(
        "skill_installs",
        sa.Column("coding_agent", sa.String(), primary_key=True),
        sa.Column("cafleet_version", sa.String(), nullable=False),
        sa.Column("installed_at", sa.String(), nullable=False),
    )
    op.execute(
        "INSERT INTO skill_installs (coding_agent, cafleet_version, installed_at) "
        "SELECT coding_agent, cafleet_version, installed_at FROM asset_installs"
    )
    op.drop_table("asset_installs")
```

Generation follows `.claude/rules/database-migrations.md`: with the DB at head, run `mise //cafleet:makemigration "rename skill_installs to asset_installs"`, then hand-edit the autogenerated drop+add into the create + copy + drop shape above (autogenerate cannot see the rename) and write the mirrored `downgrade()`. The table has no FK relationships, so no batch-recreate concerns apply.

Chain-guard updates in `tests/db/test_alembic_smoke.py`: rename `test_two_revision_migration_chain_exists` → `test_three_revision_migration_chain_exists` asserting 3 revisions with links `0003` → `0002` → `0001` → `None` and head `0003`; update `test_alembic_version_table_records_head_0002` → `..._0003`; retarget `test_skill_installs_table_created_by_migration` to assert `asset_installs`; and switch `skill_installs` → `asset_installs` in the expected head-table set of `test_alembic_upgrade_head_creates_expected_tables`.

### Guard and doctor wording

`ensure_assets_current` (renamed from `ensure_skills_current`) keeps its logic; only the strings change:

| Condition | New message |
|---|---|
| No row recorded (or table/DB missing) | `no assets install is recorded; run 'cafleet setup' first` |
| Any stale row | `stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall` |

`doctor`: `_skills_report` → `_assets_report`; the JSON payload key `"skills"` becomes `"assets"` (inner keys `cli_version` / `installs` and the per-row shape unchanged); the text section heading becomes `assets:` and the empty-state line becomes `(no assets install recorded; run 'cafleet setup')`; docstrings say "assets-install diagnostics".

### Publish workflow

`.github/workflows/publish.yml`: rename the `upload-skills` job to `upload-assets`, change both occurrences of the zip name to `cafleet-assets-v${{ github.event.release.tag_name }}.zip`, and rename the upload step per the naming map. The packaging step (`zip -r ... skills presets`) and its name are unchanged — it literally packages those two directories.

### Documentation surface (removal-rule sweep)

Per `.claude/rules/removal.md`, every mention of the removed surface is updated in the same change — no deprecation notices, no historical pointers. Affected files (from a repo-wide sweep):

| Area | Files |
|---|---|
| Spec | `SPEC.md` (setup contract, `AssetInstalls` entity, asset name); `docs/spec/cli-options.md` (rewrite § `cafleet setup`, delete the `setup db` / per-agent sections, "Stale-assets guard", asset/table names, `permissions.allow` coverage rows); `docs/spec/data-model.md`; `docs/spec/coding-agent-backends.md` |
| Docs | `docs/api/broker.md`; `docs/concepts/overview.md`; `docs/concepts/storage.md`; `docs/contributing.md`; `docs/quickstart.md`; `README.md` (only if its thin install surface mentions the old form) |
| Skills / rules | `skills/cafleet/reference/director.md`; `.claude/rules/commands.md`; `.claude/rules/database-migrations.md`; `.claude/skills/update-readme/SKILL.md` |
| Project memory | `CLAUDE.md` — the stale "with `setup` — and its `db` / `skill` subcommands …" line becomes: `Unified CLI command: cafleet (with a single setup command for onboarding and schema management, and all member/messaging commands)` |
| Build | `cafleet/mise.toml` (`makemigration` description references the schema-only invocation) |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [ ] Rewrite `docs/spec/cli-options.md` § `cafleet setup`: single command with `--skip`, fixed three-agent target list, skip-all schema-only semantics, unchanged half-failure/exit contract, new preflight string; delete the `setup db` and per-agent sections; rename "Stale-skills guard" → "Stale-assets guard" with the new messages; update asset name, table name, and `permissions.allow` coverage rows <!-- completed: -->
- [ ] Update `docs/spec/data-model.md`, `docs/spec/coding-agent-backends.md`, and `docs/api/broker.md` to the `asset_installs` / `broker/asset_installs.py` names and the single setup command <!-- completed: -->
- [ ] Update `docs/concepts/overview.md`, `docs/concepts/storage.md`, `docs/contributing.md`, and `docs/quickstart.md` (schema-only invocation replaces `setup db`; all-agents-by-default install with `--skip`) <!-- completed: -->
- [ ] Sync `SPEC.md` (and `README.md` if its thin surface drifted) via the `/update-readme` skill <!-- completed: -->
- [ ] Update `skills/cafleet/reference/director.md`, `.claude/rules/commands.md`, `.claude/rules/database-migrations.md`, and `.claude/skills/update-readme/SKILL.md` to the new setup surface <!-- completed: -->
- [ ] Update the project `CLAUDE.md` unified-CLI line per the Specification <!-- completed: -->
- [ ] Update the `makemigration` task description in `cafleet/mise.toml` to reference the schema-only invocation <!-- completed: -->

### Step 2: Data layer — rename + migration (lands as one unit)

- [ ] Rename `SkillInstall` → `AssetInstall` (`__tablename__ = "asset_installs"`) in `cafleet/src/cafleet/db/models.py` <!-- completed: -->
- [ ] Rename `broker/skill_installs.py` → `broker/asset_installs.py`; rename the three functions per the naming map; inspect `asset_installs` <!-- completed: -->
- [ ] Update imports in `cli/_helpers.py`, `cli/doctor.py`, and `cli/setup.py`; rename `ensure_skills_current` → `ensure_assets_current` with the new guard messages, updating its importers `cli/fleet.py`, `cli/member.py`, `cli/message.py`, and `cli/monitor.py` <!-- completed: -->
- [ ] Update `doctor`: `_assets_report`, JSON key `"assets"`, text section `assets:`, empty-state line, docstrings <!-- completed: -->
- [ ] Generate migration `0003` with `mise //cafleet:makemigration "rename skill_installs to asset_installs"` (DB at head first) and hand-edit to the create + copy + drop `upgrade()` / mirrored `downgrade()` in the Specification <!-- completed: -->
- [ ] Update the chain-guard tests in `tests/db/test_alembic_smoke.py` (3-revision chain, head `0003`, `asset_installs` assertions including the expected head-table set in `test_alembic_upgrade_head_creates_expected_tables`) <!-- completed: -->
- [ ] Rename `tests/broker/test_skill_installs.py` → `tests/broker/test_asset_installs.py`; update `tests/conftest.py` and `tests/_helpers.py` to the new model/table names <!-- completed: -->
- [ ] Update guard/doctor string assertions in `tests/cli/test_skills_guard.py` (rename the file to `test_assets_guard.py`) and `tests/cli/test_doctor.py` <!-- completed: -->

### Step 3: CLI collapse

- [ ] Rewrite `cli/setup.py`: plain `click.command` with repeatable deduplicated `--skip` (`click.Choice`); delete the group, `setup_db`, `_make_agent_command`, `_resolve_targets`, and the detection error; targets = fixed three-agent list minus skips; skip-all echoes `assets half skipped (all agents skipped)` and counts as not-run; new preflight string; asset name `cafleet-assets-v<version>.zip`; temp filename `assets.zip` <!-- completed: -->
- [ ] Update the opencode missing-preset error in `coding_agent/opencode.py` to `run 'cafleet setup' first` <!-- completed: -->
- [ ] Rework `tests/cli/test_setup.py`: default install covers all three agents; `--skip` (single, repeated, duplicate, all-three, invalid-choice) cases; removed-subcommand invocations fail with Click's extra-argument error; new asset name; missing-release/asset still exit 1 <!-- completed: -->
- [ ] Update `tests/db/test_init.py` to drive the DB half via the schema-only invocation instead of `setup db` <!-- completed: -->
- [ ] Update `tests/cli/test_member.py` and `tests/coding_agent/test_opencode.py` for the new opencode error string <!-- completed: -->

### Step 4: Publish workflow

- [ ] Update `.github/workflows/publish.yml`: `upload-assets` job name, `cafleet-assets-v<tag>.zip` in both steps, renamed upload step <!-- completed: -->

### Step 5: Verification

- [ ] `mise //cafleet:test` passes <!-- completed: -->
- [ ] `mise //cafleet:lint` and `mise //cafleet:typecheck` pass <!-- completed: -->
- [ ] Repo-wide sweep confirms no mention of `setup db`, `setup claude|codex|opencode`, home auto-detection, `cafleet-skills-v`, or the `skill_installs` name family outside `design-docs/` and `cafleet/src/cafleet/db/alembic/versions/` <!-- completed: -->
- [ ] Smoke-check the schema-only invocation on a fresh DB: `cafleet setup --skip claude --skip codex --skip opencode` exits 0 and reports head `0003` <!-- completed: -->
- [ ] Smoke-check upgrade data preservation: a DB at `0002` with a seeded `skill_installs` row migrates to `0003` with the row present in `asset_installs` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-18 | Initial draft |
