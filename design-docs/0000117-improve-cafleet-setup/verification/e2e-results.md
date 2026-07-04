# Phase D verification results (Verifier, agent 138, 2026-07-04)

Overall: **PASS** — every success criterion verifiable offline is confirmed by live E2E (Director-dispatched via `member exec`, transcripts landed in the Verifier pane); everything else is green via the sanctioned gates.

## Live E2E (17/17 pass, scratch DB `/tmp/cafleet-verify-0000117/`, cleaned up)

| # | Check | Result |
|---|-------|--------|
| 1 | `setup db` fresh path | `schema ready at /tmp/cafleet-verify-0000117/verify.db`; parent dir auto-created |
| 2 | `setup db` idempotent | identical output on re-run |
| 3 | Table inventory | exactly the seven baseline tables (`agent_placements`, `agents`, `fleets`, `monitor_config`, `monitor_runtime`, `skill_installs`, `tasks`) + SQLite-internal `sqlite_sequence`; no `alembic_version` |
| 4 | Empty-table guard | `Error: no skills install is recorded; run 'cafleet setup' first` |
| 5 | `setup skill` pre-flight (no schema) | `Error: the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first` — fired offline, before any network |
| 6 | Bare `setup`, empty HOME | db half ran first (schema created, so the skills pre-flight passed), skills half failed independently: `skills half failed: no coding-agent homes detected (…)` then `Error: skills half failed` |
| 7 | `setup --agent` removed | Click `Error: No such option: --agent` |
| 8 | `db` group removed | Click `Error: No such command 'db'.` |
| 10 | Stale guard string | `Error: stale skills detected (claude=0.5.0, codex=0.6.0; CLI 0.14.0); run 'cafleet setup skill' to reinstall` — ascending agent order |
| 11 | Group help under stale | `cafleet fleet --help` prints help |
| 12 | Subcommand help under stale | `cafleet fleet create --help` errors with the guard message (documented contract) |
| 13 | `doctor` text | `skills:` block after the tmux block; `cli_version: 0.14.0`; per-agent lines with verbatim microsecond timestamps and `STALE`; doctor exempt from the guard |
| 14 | `doctor` JSON | sibling `"skills"` key: `{"cli_version": "0.14.0", "installs": [... "current": false ...]}` |
| 16 | Matching rows pass | `fleet list` → `No fleets found.` (exit 0) |

(9/15 were sqlite3 seeding steps; 17 was cleanup.)

## Sanctioned-gate evidence

- `mise //cafleet:test`: 925 passed; targeted verbose run of the 69 new contract tests (setup group, skills guard incl. help contract, doctor skills section, db-group removal, schema baseline, skill_installs helpers) all pass.
- `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint`: all pass.
- Removal sweep: no `db init` / `alembic` / `Alembic` mention outside `design-docs/` (lock files included); `db_group` only in the removal regression test; `cli/db.py`, `alembic.ini`, `alembic/` tree deleted.
- Docs: `install.md`, `contributing.md`, `storage.md`, `data-model.md`, `cli-options.md`, README, SPEC, CLAUDE.md, and the Dashboard hint all reflect the new surface (no `db init` / backfill wording remains).
- `tests/conftest.py` seeds a `SkillInstall` row so fleet-scoped CLI tests pass the guard.
- GitHub release `0.14.0` exists with exactly `cafleet-skills-v0.14.0.zip` (checked via the public API), so the real `setup skill` lookup path is satisfiable.

## Noted gap (accepted scope, not a failure)

The network-dependent skills-install flow (release download, extraction, per-home install, `record_skill_install` on success, live upsert on re-install) was NOT exercised end-to-end against GitHub or the real skill homes, per the Director's Phase D constraint (offline only, no real registry/homes). It is covered by the mocked contract tests in `tests/cli/test_setup.py` (download/extract/validate/record, upsert, partial-failure row retention).
