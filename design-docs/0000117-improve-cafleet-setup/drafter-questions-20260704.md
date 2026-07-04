# Drafter clarifying questions — design 0000117 improve-cafleet-setup

Context I gathered before asking: `cafleet setup` (cafleet/src/cafleet/cli/setup.py) currently
runs two independent halves — skills install (GitHub Release asset matching the installed CLI
version) then `run_db_init()` (Alembic upgrade to head). `cafleet db init` (cafleet/src/cafleet/cli/db.py)
is a thin wrapper over the same `run_db_init()`. Design 0000111 (Status: Complete) already rewrote
SPEC.md and docs/ to a world with NO `db init` command and NO migration chain (single-baseline
`CREATE TABLE IF NOT EXISTS`), but its code refactor was explicitly out of scope — the code still
ships the Alembic chain (0001–0005). Issue #152 sits directly on that documentation-vs-code gap.

## Q1 — Purpose & Scope: Alembic vs the documented single-baseline schema

SPEC.md §8 and docs/concepts/storage.md (per design 0000111) promise a single-baseline
`CREATE TABLE IF NOT EXISTS` schema with no migration chain and no version table. The code still
uses Alembic. What should `cafleet setup db` be built on?

- (a) **Complete the 0000111 hard cut** (recommended): `setup db` runs the single-baseline
  metadata create (`Base.metadata.create_all` semantics); delete the Alembic chain, alembic.ini,
  and the upgrade-guard machinery. The new version table becomes part of the baseline. Docs
  already describe this world, so doc drift is minimal.
- (b) **Keep Alembic** under `setup db` (rename only) and add the version table as migration 0006.
  This contradicts the shipped SPEC/docs, which would then need to be rewritten back.

## Q2 — Data model: shape of the version table

Skills are installed per coding-agent home (claude/codex/opencode), and `setup --agent codex` can
run at a different time (and CLI version) than `setup --agent claude`. Which granularity?

- (a) **One row per coding-agent home** (recommended): e.g. table `skill_installs` with
  `coding_agent` (PK, one of claude/codex/opencode), `cafleet_version`, `installed_at`.
  Mismatch is then detectable per home.
- (b) **Single global row**: table stores one `cafleet_version` + `recorded_at`; last setup wins
  regardless of which homes it touched.

Also: any additional columns you want (e.g. skills-asset name), or keep it minimal?

## Q3 — Who checks the version, where, and how hard?

Issue says "if the runtime version is not matched with the recorded one, reinstalling skills is
needed". Where is that check enforced?

- (a) **Warn on every fleet-scoped CLI command** (e.g. `fleet create`, `member create`): print a
  warning to stderr suggesting `cafleet setup skill` (or `cafleet setup`), but do not block.
- (b) **Check only in `cafleet doctor`** — a diagnostics-only surface; nothing else changes.
- (c) **Hard error** on selected commands (block until reinstall).
- (d) Some combination (e.g. warn broadly + doctor reports detail).

Also: is a mismatch simple string inequality (so a downgrade also triggers), and what is the
behavior when the table is empty / DB predates the feature (treat as "setup never ran" → same
warning)?

## Q4 — Recording semantics and half-ordering

The version row lives in the DB, but the skills half is what it certifies. Proposal (confirm or
adjust):

- Bare `cafleet setup` runs **db half first, then skills half** (reversed from today), so the
  version row can be written after a successful skills install. Halves stay independent-failure
  (run both, exit non-zero if either failed) as today.
- `cafleet setup skill` records/updates the version row on success. If the DB/schema does not
  exist yet, does `setup skill` (a) auto-create the schema (effectively running the db half), or
  (b) fail with "run `cafleet setup` (or `setup db`) first"?
- `cafleet setup db` never touches the version row (schema only).

## Q5 — CLI surface details

- `cafleet setup` becomes a Click group whose bare invocation runs everything; subcommands
  `db` and `skill`. Confirm subcommand name **`skill`** (singular, as in the issue) rather than
  `skills`.
- `--agent` stays on bare `setup` and on `setup skill`; `setup db` takes no options. OK?
- `cafleet db` group is removed **hard** — no alias, no deprecation notice, every mention swept
  (CLAUDE.md, SPEC.md, docs, admin Dashboard.tsx error hint, tests) per the repo removal rule. OK?

## Q6 — Testing expectations

Existing suites: cafleet/tests/cli/test_setup.py and cafleet/tests/db/test_init.py. Assumption:
rework them to the new surface (`setup db` / `setup skill` / version-record + mismatch-check
tests), delete Alembic-specific tests if Q1=(a). Any additional acceptance criteria you want
covered (e.g. a doctor-output test)?
