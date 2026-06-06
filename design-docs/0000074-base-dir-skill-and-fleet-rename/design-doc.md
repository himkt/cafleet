# 0000074 — base-dir skill self-containment + `session`→`fleet` Tier-C rename

**Status**: Complete
**Progress**: 41/41 tasks complete
**Last Updated**: 2026-06-07

## Overview

Two independent CAFleet CLI refactors land in one change: **(Item 1)** remove the `cafleet base-dir` CLI command group and re-express its deterministic BASE-resolution logic as native tool steps inside the existing `cafleet-base-dir` skill, deleting the now-dead `base_dir.py` module and its tests; **(Item 3)** perform a full Tier-C rename of the CAFleet "session" concept to "fleet" — the `cafleet session` command group, the `--session-id` global flag, the public output keys, the admin WebUI, the spawn-prompt placeholder, AND the `sessions` DB table (via a new reversible Alembic migration). Both items obey the project removal rule: after the change the repository reads as if the old names never existed.

## Success Criteria

- [x] `cafleet base-dir` no longer exists — `cafleet base-dir resolve` exits with Click's "No such command 'base-dir'".
- [x] `cafleet/src/cafleet/base_dir.py`, `cafleet/tests/test_base_dir.py`, and `cafleet/tests/test_base_dir_spawn_flow.py` are deleted; no module imports `cafleet.base_dir`.
- [x] The `cafleet-base-dir` skill resolves and records BASE using only built-in tools (`git rev-parse --show-toplevel` via Bash, Read, Write, AskUserQuestion) — it contains zero `cafleet base-dir` invocations and preserves the four deterministic guarantees (traversal-escape rejection, repo-root-degenerate rejection, anchor `version: 1` validation, idempotent record-on-mismatch) as explicit numbered procedure steps.
- [x] `cafleet fleet {create,list,show,delete}` works end-to-end; `cafleet session ...` exits with "No such command 'session'".
- [x] The global flag is `--fleet-id`; passing `--session-id` exits with Click's "No such option: --session-id".
- [x] A new Alembic migration `0011` renames `sessions`→`fleets`, `session_id`→`fleet_id` (PK and the `agents` FK column), and `idx_agents_session_status`→`idx_agents_fleet_status`; `upgrade()` preserves every existing row; `downgrade()` is a full inverse that restores the original schema.
- [x] `--json` output emits the key `fleet_id` (never `session_id`); text output emits `FLEET_ID` / `fleet_id:`. No backward-compat alias key is emitted.
- [x] The admin WebUI uses route `/fleets`, header `X-Fleet-Id`, JSON field `fleet_id`, and "Fleet"/"Fleets" UI labels (and the `cafleet fleet create` snippet).
- [x] The member spawn-prompt substitution placeholder is `{fleet_id}` and the spawn label is `FLEET ID:` across all six member-spawning skills and their role files.
- [x] **Removal completeness (identifiers)**: a repo-wide search (excluding `design-docs/`, `researches/`, `.git/`, the immutable Alembic revisions `0001`–`0010`, the new `0011` rename migration, and its round-trip test `cafleet/tests/test_alembic_0011_rename.py` — these three legitimately name the pre-rename `sessions`/`session_id` schema as migration history, not the live concept) for the identifier tokens `cafleet base-dir`, `base_dir`, `--session-id`, `cafleet session`, the CAFleet `session_id` identifier, and the `sessions` table name returns **zero** hits — except the legitimate tmux-session, SQLAlchemy-ORM-session, HTTP/browser-session, and coding-agent-runtime-session usages enumerated in §Disambiguation, which are intentionally retained.
- [x] **Removal completeness (prose / UI copy + test/source internals)**: a SEPARATE *reviewed* (not zero-hit) case-insensitive `session` sweep across the SAME repo-wide scope as the identifier grep — `docs/`, `skills/`, `.claude/`, `admin/src`, AND `cafleet/src` + `cafleet/tests` (excluding `design-docs/`, `researches/`, `.git/`, Alembic `0001`–`0010`, the `0011` rename migration, and `cafleet/tests/test_alembic_0011_rename.py`) — confirms every remaining hit is a deliberate meaning-#2 (tmux), #3 (SQLAlchemy ORM), #4 (HTTP/browser), or #5 (coding-agent runtime session) retention per §Disambiguation. Narrow identifier tokens miss CAFleet-concept *wording* (e.g. the nav label "Session isolation", admin copy "Back to Sessions" / "Select a Session" / "This session has no Administrator agent", doc prose such as "each session owns a Director") AND token-less test/source-internal CAFleet-session identifiers (e.g. `_create_session_with_ctx`, `_create_session_via_cli`, `test_session_delete_*`); this reviewed pass catches both. The `cafleet/` portion legitimately returns many meaning-#3 ORM `session` / meaning-#2 `tmux_session` hits — each is confirmed as a retention, not removed.
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test`, and `mise //admin:lint` all pass.

---

## Background

`cafleet/src/cafleet/cli.py` (~1415 lines) defines one `cli` Click group with subgroups `db` / `session` / `agent` / `message` / `base-dir` / `member` plus top-level `server` / `doctor`.

**Item 1 — `cafleet base-dir`.** The `base-dir` group (`cli.py:402-502`) exposes `resolve` and `record`, which are thin wrappers over `base_dir.resolve()` / `base_dir.record()`. Those two functions are called from nowhere else in the source tree. The remaining `base_dir.py` helpers — `extract_spawn_templates`, `substitute_base_in_prompt`, `write_audit_file` — are referenced only by `cafleet/tests/test_base_dir_spawn_flow.py`; no live code path (notably **not** `member create`) calls them. So once the CLI group is removed, the entire `base_dir.py` module is dead. The `cafleet-base-dir` skill already orchestrates the full resolution flow and merely delegates the deterministic core to the CLI; folding that core back into the skill as native tool steps removes the only reason the module exists.

**Item 3 — `session`→`fleet`.** The product is "CAFleet", so a unit of work that owns a Director, members, and a message timeline is naturally a **fleet**, not a "session". The user selected **Tier C** (the deepest of three options): rename the command group, the `--session-id` global flag, AND the `sessions` DB table. The user further confirmed the full-depth follow-ons: the DB column `session_id`→`fleet_id`, the Python symbols (`Session` model→`Fleet`, `create_session`→`create_fleet`, every `session_id=` kwarg→`fleet_id`), the public CLI output keys, the admin WebUI, and the spawn-prompt placeholder all rename, with **no** backward-compatibility aliases.

**Out of scope.** Merging `cafleet agent` into `cafleet member` (the "Item 2" the user considered) is explicitly excluded. `agent` = registry identity (includes the root Director and Administrator, which are agents but never members); `member` = an agent with a tmux placement + Director-scoped authorization. This document does not design or mention such a merge.

### Verified codebase facts (correcting the brief)

- `agent_placements` has **no** `session_id` column. The only CAFleet-session FK column is `agents.session_id` → `sessions.session_id` (`db/models.py:29-33`). The migration therefore touches exactly: the `sessions` table, the single `agents.session_id` FK column, and the one index `idx_agents_session_status`. (The brief's claim that `agent_placements` carries a `session_id` FK is incorrect.)
- FK enforcement is **ON** during migrations: `db/engine.py` registers a global `Engine.connect` listener that runs `PRAGMA foreign_keys=ON` on every SQLite connection, and `cafleet db init` builds its engine after `cafleet.broker` (hence `cafleet.db.engine`) is imported, so the listener applies there too. This shapes the migration strategy (§Migration design).
- There is a **circular** FK pair: `agents.session_id → sessions.session_id` and `sessions.director_agent_id → agents.agent_id`. A parent-table rebuild is therefore awkward; SQLite's native `ALTER TABLE ... RENAME` is the clean path.
- The project `.claude/settings.json` `permissions.allow` list is mise-based (`Bash(mise //cafleet*)` etc.) with **no** `cafleet session`-specific or `cafleet base-dir`-specific pattern, so neither removal causes settings.json churn.

---

## Specification

### Disambiguation — five unrelated meanings of "session"

This is the single highest-risk aspect of Item 3. The token "session" appears in the repo with **five** distinct meanings; only the first renames. This table is the single source of truth — every "session" retention the rest of the doc points executors at MUST have a row here.

| # | Meaning | Examples (MUST be handled correctly) | Action |
|---|---|---|---|
| 1 | **CAFleet session** (the entity being renamed) | `cafleet session` group; `--session-id` flag; `sessions` table; `Session` ORM model; `session_id` columns/kwargs/JSON keys; `create_session`/`list_sessions`/`get_session`/`delete_session`/`verify_agent_session`/`list_session_agents`/`_agent_is_active_in_session`; `X-Session-Id` header; `get_webui_session`; `format_session_create`; the `{session_id}` spawn placeholder; `_DIRECTOR_DESCRIPTION = "Root Director for this session"` | **RENAME → fleet** |
| 2 | **tmux session** (multiplexer concept) | the literal string `"... must be run inside a tmux session"`; the `tmux_session` column / `placement['tmux_session']`; `MultiplexerContext.session`; the `session` column header in `output.format_member_list` (renders `tmux_session`); docstrings in `multiplexer/tmux.py` and `multiplexer/base.py` | **KEEP as "session"** |
| 3 | **SQLAlchemy ORM session** (DB transaction handle) | `from sqlalchemy.orm import Session, sessionmaker` in `db/engine.py`; `get_sync_sessionmaker` / `_sync_sessionmaker`; the `session` local variable throughout `broker.py` (`with sm() as session, session.begin(): session.execute(...)`); the `### Session ownership` subsection and `async_sessionmaker`/`AsyncSession` prose in `docs/spec/data-model.md` | **KEEP as "session"** |
| 4 | **HTTP / browser session** (stateless WebUI + browser automation) | `docs/spec/webui-api.md:13`'s "No server-side session cookies" — the WebUI keeps no cookie/server session; the SPA sends the CAFleet id in the `X-Fleet-Id` header instead. Any "session cookie" / "browser session" wording stays. Also the agent-browser named browser-automation sessions (`bun run agent-browser --session vr-batch-*`, `SESSION NAME:` in the `cafleet-research-presentation` skill + role files, and the `.claude/settings.json` `--session vr-batch-*` allow-patterns). | **KEEP as "session"** |
| 5 | **Coding-agent runtime session** (the Claude Code / codex / opencode REPL invocation; a skill "run") | `cafleet-agent-team-monitoring`'s "in-session scheduling" / "from inside a running session" / "outside the Director's session" / "the session's launch instructions" / "short sessions"; `cafleet-agent-team-supervision`'s "restarted the session"; the `cafleet-design-doc-interview` skill's "multi-session splitting" / "Session termination" / "Session Report" / "across multiple sessions" / "first session" (each = one interview run); `cafleet-base-dir`'s "across sessions" / "mid-session" / "the next session"; generic "codex session" / "research session" / "long-lived sessions" prose; `.claude/rules/code-quality.md`'s `# Create the session` example comment. Renaming any of these to "fleet" (e.g. "in-fleet scheduling") would be **wrong**. | **KEEP as "session"** |

The executor MUST review every "session" hit individually against this table rather than applying a blind find-replace. Meaning #2 and #3 collisions are the highest-risk: a naive global rename would corrupt the multiplexer layer and the entire data-access layer. Meanings #4 (HTTP-statelessness + agent-browser) and #5 (coding-agent runtime session) are lower-collision but are enumerated so the table stays the single source of truth for every retained "session". Meaning #5 was surfaced during execution — the narrow identifier grep does not touch it, but the reviewed prose sweep does, so it must be enumerated here to keep that sweep coherent.

### Item 1 — remove `cafleet base-dir`, make the skill self-contained

**Retained (do NOT remove).** This change deletes only the space-form CLI invocation `cafleet base-dir` and the underscore module `base_dir.py`. The hyphenated skill name `cafleet-base-dir`, the skill directory `skills/cafleet-base-dir/`, and the `.cafleet-base-dir.json` anchor file all STAY — the skill becomes the authoritative resolver and keeps writing the anchor. The removal-completeness identifier tokens already separate these (space vs hyphen in `cafleet base-dir`/`cafleet-base-dir`; underscore vs hyphen in `base_dir`/`base-dir`); this note exists so no executor reading "delete every mention" strips the skill name or anchor by "fixing" a false residual hit.

**Code deletions:**

1. Delete the `base-dir` Click group and both subcommands (`cli.py:402-502`: `base_dir_group`, `base_dir_resolve`, `base_dir_record`).
2. Remove `base_dir` from the import at `cli.py:20` (`from cafleet import base_dir, broker, output` → `from cafleet import broker, output`).
3. Delete `cafleet/src/cafleet/base_dir.py` in full.
4. Delete `cafleet/tests/test_base_dir.py` and `cafleet/tests/test_base_dir_spawn_flow.py` in full.

**Skill rewrite (`skills/cafleet-base-dir/SKILL.md`):** rewrite the Procedure and CLI-surface sections so Claude performs resolution with built-in tools. The skill MUST preserve the four guarantees the deleted Python enforced, expressed as explicit, checkable steps:

| Guarantee (was compiled Python) | Skill-prose replacement |
|---|---|
| Repo-root inference (`is_git_repo_root` CWD walk) | Run `git rev-parse --show-toplevel` (Bash). A non-zero exit / empty output means "no `.git` ancestor" → for the task-scope branch, instruct the user to `cd` to the repo root and retry (mirrors the old `RuntimeError`). |
| Traversal-escape + repo-root-degenerate rejection | Before creating a task folder: reject a `task_name` that resolves outside the repo root (contains `..` segments that escape, or an absolute path not strictly under the root) and reject one that resolves to the repo root itself (`.`, `""`, `design-docs/..`). State both checks as explicit guard steps. |
| Anchor `version: 1` validation | When reading `<base>/.cafleet-base-dir.json`: parse JSON; require `version == 1` (integer), `base` an absolute path equal to the anchor's parent dir, and `source ∈ {cwd-inference, askuserquestion, task-scope}`; otherwise instruct "delete the anchor and re-resolve" (mirrors the old `AnchorError`). |
| Idempotent record-on-mismatch | Before writing an anchor: if one already exists and its `base` matches, no-op; if it mismatches, stop and surface the conflict (do not overwrite). |

The anchor schema (version 1), the `<unset>` sentinel contract, the no-bypass write protocol, and the `AskUserQuestion` candidate picker remain exactly as documented today — only the deterministic core moves from `cafleet base-dir resolve/record` to inline tool steps. The skill stays the single authoritative resolver.

**Consumer updates (every doc/skill that referenced the removed CLI):** rewrite the six Director skills that invoke `cafleet base-dir resolve` / `record` (`cafleet-design-doc-create`, `cafleet-design-doc-execute`, `cafleet-design-doc-interview`, `cafleet-research-report`, `cafleet-research-presentation`, `cafleet-create-figure`) plus `.claude/skills/skill-author`, and the `cafleet/reference/director.md` mention, to call the `cafleet-base-dir` skill's native procedure instead of the CLI. Remove the `cafleet base-dir {resolve,record}` row from the `README.md` CLI table. Member role files that merely *load* the `cafleet-base-dir` skill at startup need wording updates only if they name the CLI.

`.gitignore` keeps its unanchored `.cafleet-base-dir.json` entry (the anchor still exists, now written by the skill) — confirm, no change.

### Item 3 — `session`→`fleet` Tier-C rename

#### Data layer (`db/models.py`)

- Rename the ORM model class `Session` → `Fleet`; `__tablename__ = "sessions"` → `"fleets"`.
- Rename column `session_id` (PK) → `fleet_id`.
- In `Agent`: rename `session_id` → `fleet_id`; update the FK target string `ForeignKey("sessions.session_id", ...)` → `ForeignKey("fleets.fleet_id", ...)`; rename the index `idx_agents_session_status` → `idx_agents_fleet_status` (columns `("fleet_id", "status")`).
- `Fleet.director_agent_id` FK (`agents.agent_id`) is unchanged.

#### Migration design (new revision `0011_rename_sessions_to_fleets`)

The new migration file is `cafleet/src/cafleet/alembic/versions/0011_rename_sessions_to_fleets.py` (existing revisions live in `cafleet/src/cafleet/alembic/versions/` — **not** the brief's `db/migrations/versions/`, which does not exist). Match the typed-identifier style used by `0001`–`0010`: `revision: str = "0011"` and `down_revision: str | None = "0010"` (current head). Because FK enforcement is ON and the `sessions`↔`agents` FK pair is circular, use SQLite's native `ALTER TABLE ... RENAME`, which on SQLite ≥ 3.25 (with `legacy_alter_table` OFF, the default) **auto-propagates** the rename into dependent FK definitions and index column references. Python 3.12's bundled `sqlite3` ships SQLite well above 3.25, so this is safe. Index *names* are not auto-renamed, so drop+create the index explicitly.

```python
revision: str = "0011"
down_revision: str | None = "0010"


def upgrade() -> None:
    op.execute("ALTER TABLE sessions RENAME TO fleets")
    op.execute("ALTER TABLE fleets RENAME COLUMN session_id TO fleet_id")
    op.execute("ALTER TABLE agents RENAME COLUMN session_id TO fleet_id")
    op.drop_index("idx_agents_session_status", table_name="agents")
    op.create_index("idx_agents_fleet_status", "agents", ["fleet_id", "status"])


def downgrade() -> None:
    op.drop_index("idx_agents_fleet_status", table_name="agents")
    op.execute("ALTER TABLE agents RENAME COLUMN fleet_id TO session_id")
    op.execute("ALTER TABLE fleets RENAME COLUMN fleet_id TO session_id")
    op.execute("ALTER TABLE fleets RENAME TO sessions")
    op.create_index("idx_agents_session_status", "agents", ["session_id", "status"])
```

Table rename preserves every row; no data backfill. The downgrade is a full inverse restoring the exact pre-`0011` schema (table name, column names, FK, index). Rejected alternative: the 0002-style parent-table rebuild (`CREATE _fleets_new` + `INSERT…SELECT` + `DROP sessions` + `RENAME`). It is rejected here because `sessions` is the *parent* of the `agents.session_id` FK while also being a *child* of `sessions.director_agent_id → agents.agent_id`; dropping it under `foreign_keys=ON` would dangle the `agents` FK and require disabling the pragma mid-transaction (SQLite forbids changing `foreign_keys` inside a transaction). Native `RENAME` sidesteps all of this.

**Immutable-history exception (important):** Alembic revisions `0001`–`0010` are append-only schema history; they create and manipulate the `sessions` table because that was the schema at their point in time. They are **not** edited — rewriting them would break replay (`cafleet db init` on a fresh DB and on DBs stopped at an intermediate revision). This is the same category as git history under the removal rule: the new `0011` migration renames forward; the old revisions keep saying "sessions". The removal rule's "reads as if it never existed" applies to live source/docs/skills/tests, not to the immutable migration chain. The removal-completeness grep (§Success Criteria) therefore excludes `0001`–`0010`.

#### Data-access layer (`broker.py`)

- Update the import `from cafleet.db.models import Agent, AgentPlacement, Session, Task` → `... Fleet, Task` (the CAFleet model only — the `sqlalchemy.orm` `Session` import lives in `engine.py` and stays).
- Rename functions: `create_session`→`create_fleet`, `list_sessions`→`list_fleets`, `get_session`→`get_fleet`, `delete_session`→`delete_fleet`, `verify_agent_session`→`verify_agent_fleet`, `list_session_agents`→`list_fleet_agents`, `_agent_is_active_in_session`→`_agent_is_active_in_fleet`.
- Rename every CAFleet `session_id` parameter, local, dict key, and `.values(...)`/`.where(...)` reference to `fleet_id`; update all `Session.session_id` / `Session.<col>` query references to `Fleet.fleet_id` / `Fleet.<col>`.
- **Keep** the SQLAlchemy `session` local variable name (`with sm() as session, session.begin():`) and all `session.execute/add/flush` calls (meaning #3).
- Update CAFleet-session error/description strings: `_DIRECTOR_DESCRIPTION = "Root Director for this session"` → "... for this fleet"; `"Session '{...}' not found."` → `"Fleet '{...}' not found."`; `"session {...} is deleted"` → `"fleet {...} is deleted"`; bootstrap docstrings. Leave any "tmux session" wording untouched (none currently in broker.py beyond the CLI layer).

#### CLI layer (`cli.py`)

- Rename the `session` group → `fleet` (`@cli.group()` `def session()` → `def fleet()`; `@session.command(...)` → `@fleet.command(...)`); rename the command callbacks `session_create/list/show/delete` → `fleet_*`. Update the group/command docstrings ("Session management commands." → "Fleet management commands.", etc.).
- Rename the global option `--session-id`/`session_id` → `--fleet-id`/`fleet_id` (`cli.py:153-158`); rename `ctx.obj["session_id"]` → `ctx.obj["fleet_id"]` everywhere; rename the `_require_session_id` helper → `_require_fleet_id` and its error text ("`--session-id <uuid> is required ... Create a session with 'cafleet session create' ...`" → "`--fleet-id <uuid> is required ... Create a fleet with 'cafleet fleet create' ...`").
- Update the member spawn-prompt template `_MEMBER_PROMPT_TEMPLATE` (`cli.py:25-30`): "Member of cafleet session {session_id}" → "Member of cafleet fleet {fleet_id}"; `cafleet --session-id {session_id} message poll` → `cafleet --fleet-id {fleet_id} message poll`.
- Update `_resolve_prompt` (`cli.py:842-879`): the `str.format(session_id=...)` kwarg → `fleet_id=...`; the supported-placeholder set and the "Unknown placeholder ... Supported placeholders: {session_id}, {agent_id}, {director_agent_id}" / "Malformed custom prompt" error messages → `{fleet_id}`.
- Update remaining CAFleet-session strings: `session_show`/`session_list`/`session_delete` output (`SESSION_ID` header → `FLEET_ID`; `session_id:` label → `fleet_id:`; `f"session '{session_id}' not found."` → fleet); the `"agent {agent_id} is not a member of session {session_id}."` membership error → "... member of fleet ...".
- **Keep** the `"cafleet fleet create must be run inside a tmux session"` error (the verb renames, the *tmux session* noun stays) and the surrounding docstring's "tmux session" wording.

#### Output layer (`output.py`)

- Rename `format_session_create` → `format_fleet_create`; `data['session_id']` → `data['fleet_id']`; update its docstring (`<session_id> director=...` → `<fleet_id> ...`).
- **Keep** `format_member_list`'s `session` column header — it renders `placement['tmux_session']` (meaning #2). Optionally relabel it to `tmux_sess`/`tmux` for clarity, but this is not required and not in scope; if left as `session`, document that it is the tmux session.

#### WebUI layer (`webui_api.py` + `admin/`)

- `webui_api.py`: rename `get_webui_session` → `get_webui_fleet`; read header `x-fleet-id` (was `x-session-id`); error text "X-Fleet-Id header required" / "Fleet not found"; call `broker.get_fleet`; rename the `session_id` Depends-injected parameters threaded through `/agents`, `/timeline`, `/messages/send`, etc. → `fleet_id`; rename the route `@webui_router.get("/sessions")` → `/fleets` and the `list_sessions` handler → `list_fleets` (calling `broker.list_fleets`).
- `admin/src/api.ts`: rename module var `sessionId` → `fleetId`, `setSessionId` → `setFleetId`, header `X-Session-Id` → `X-Fleet-Id`, `listSessions`→`listFleets`, route `/sessions`→`/fleets`.
- `admin/src/types.ts`: rename interface `SessionListItem` → `FleetListItem`; field `session_id` → `fleet_id`.
- `admin/src/components/SessionPicker.tsx`: rename file → `FleetPicker.tsx`; component, props (`onSelect(sessionId)` → `(fleetId)`), state (`sessions`/`setSessions` → `fleets`/`setFleets`), and all UI copy ("CAFleet — Sessions" → "CAFleet — Fleets", "Select a Session" → "Select a Fleet", "No sessions found." → "No fleets found.", the `cafleet session create` code snippet → `cafleet fleet create`, `s.session_id` → `s.fleet_id`).
- `admin/src/components/Dashboard.tsx`: rename the `sessionId` prop → `fleetId` (and its usage, e.g. the `{sessionId.slice(0, 8)}` header) plus the user-facing copy ("Back to Sessions" → "Back to Fleets", "This session has no Administrator agent" → "This fleet has no Administrator agent", "No agents registered in this session" → "... in this fleet").
- `admin/src/components/Sidebar.tsx`: rename the copy "No agents registered in this session" → "... in this fleet" (no `sessionId` prop here).
- Update every importer of the renamed admin symbols/file (e.g. `App.tsx` wiring that imports `SessionPicker`/`Dashboard`, calls `setSessionId`, and threads the `sessionId` prop) — then run a post-edit `grep -ri session admin/src` that must come back clean except any genuine browser/HTTP-session usage (none expected).

#### Tests (`cafleet/tests/`)

- Rename all five session-named test files: `test_session_cli.py`→`test_fleet_cli.py`, `test_cli_session_flag.py`→`test_cli_fleet_flag.py`, `test_cli_session_bootstrap.py`→`test_cli_fleet_bootstrap.py`, `test_session_bootstrap.py`→`test_fleet_bootstrap.py`, `test_session_list_director.py`→`test_fleet_list_director.py`.
- Rename the shared CAFleet-session test helper `tests/_broker_helpers.py::_create_session` → `_create_fleet` and update every importer.
- Update all `--session-id` → `--fleet-id`, `cafleet session` → `cafleet fleet`, `session_id` JSON-key/kwarg assertions → `fleet_id`, and broker-function references (`create_session`→`create_fleet`, etc.) across every test file (~19 files reference `--session-id`).
- Rename test-internal CAFleet-session identifiers that the four grep tokens do NOT catch (so green tests cannot mask residual old-concept names): the locally-defined per-file helpers `_create_session` (`test_broker_inline_preview.py`, `test_webui_api_format.py`), `_create_session_with_ctx` (`test_broker_administrator.py`), `_create_session_via_cli` (`test_cli_session_flag.py`), and CAFleet-session test-function names (`test_create_session__*`, `test_session_create__*`, `test_session_id_flag_*`, `test_session_delete_*`) → their `*_fleet*` equivalents. KEEP meaning-#3 local ORM `session` variables. The reviewed `session` sweep (§Success Criteria / Step 8) covers `cafleet/src` + `cafleet/tests` precisely to surface these token-less identifiers.
- Delete the two base-dir test files (Item 1). Per the removal rule, do **not** add sentinel "old name now errors" tests beyond the natural absence checks: a single regression assertion that `--session-id` / `cafleet session` no longer parse (Click's built-in "No such option" / "No such command", which is testing the *absence*, not advertising the deletion) is acceptable; a suite of deprecation-shaped tests is not.
- Add/keep migration coverage: a test that applies `0011` upgrade then downgrade against a seeded DB and asserts (a) row preservation, (b) the round-trip schema (table name, `fleet_id` column, `idx_agents_fleet_status` index), and (c) FK integrity directly — run `PRAGMA foreign_key_check` after `upgrade()` AND after `downgrade()` (expect zero rows), and assert the `agents` FK target resolves to `fleets(fleet_id)` post-upgrade and `sessions(session_id)` post-downgrade (this is the Risk 4 auto-propagation — assert it, do not assume it).

### Documentation surface (Item 1 + Item 3)

Per the project documentation-first rule, these are updated **before** code. CAFleet-session references rename; tmux-, ORM-, and HTTP-session references (per §Disambiguation) stay.

- `README.md`: drop the `cafleet base-dir` CLI-table row; rename `cafleet session` references → `cafleet fleet`, `--session-id` → `--fleet-id`.
- `docs/concepts/session-isolation.md`: rename the file → `fleet-isolation.md`; update its body and every cross-link. The docs use **Zensical** (there is no `mkdocs.yml`); the only nav reference outside `design-docs/` is `zensical.toml:55` — update BOTH the path (`concepts/session-isolation.md` → `concepts/fleet-isolation.md`) AND the label (`"Session isolation"` → `"Fleet isolation"`), or the docs build breaks on a dangling nav path.
- `docs/concepts/overview.md`, `coding-agents.md`, `tmux-push.md`: rename CAFleet-session usages; keep tmux-session usages.
- `docs/spec/cli-options.md`: rename the `## cafleet session — Session Management` section and every `cafleet session …` / `--session-id` reference.
- `docs/spec/data-model.md`: rename the `### sessions` table section → `### fleets`, the `session_id`/`Session`/`create_session`/`list_sessions`/`get_session`/`delete_session` references, the FK-enforcement line (`agents.session_id → sessions.session_id` → `agents.fleet_id → fleets.fleet_id`), the "Session Lifecycle" / "Root Director bootstrap" prose, and `cafleet session create/delete` snippets. **Keep** the `### Session ownership` subsection and its `async_sessionmaker`/`AsyncSession`/"opens its own session" prose (meaning #3) and the "must be run inside a tmux session" line (meaning #2).
- `docs/get-started/quickstart.md`, `configure.md`: rename `cafleet session …` and `--session-id`.
- `docs/reference/coding-agents/codex.md`, `opencode.md`: rename `cafleet session …` and `--session-id` usage examples.
- `docs/concepts/member-lifecycle.md`, `docs/concepts/storage.md`, `docs/spec/webui-api.md`: rename CAFleet-session identifiers and prose. `webui-api.md` documents the `X-Session-Id` header and `/sessions` route Item 3 renames (→ `X-Fleet-Id`, `/fleets`, `fleet_id`), so it MUST be updated here or the API spec drifts from the implementation. Apply §Disambiguation per hit — e.g. `webui-api.md`'s "No server-side session cookies" is an HTTP-session statement (meaning #4) that stays, and `storage.md` may carry ORM/tmux-session prose (meaning #2/#3) to KEEP.
- Skills/roles: the six member-spawning skills + their `roles/*.md` rename the `{session_id}` placeholder → `{fleet_id}`, the `SESSION ID:` label → `FLEET ID:`, all `--session-id` → `--fleet-id`, and `cafleet session` → `cafleet fleet`; the `cafleet` skill (`SKILL.md` + `reference/director.md` + `reference/recovery.md` + `reference/broadcast.md` + `roles/director.md`), `cafleet-agent-team-monitoring`, and `cafleet-agent-team-supervision` rename their CAFleet-session command/flag references. The two project rule files that quote the CLI (`.claude/rules/bash-tool.md`, `.claude/rules/commands.md`) rename their `--session-id` / `cafleet session` examples (keep `member ping`/`member exec` text intact otherwise).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first: Step 1 (docs/skills/README) lands before any code edit.

### Step 1: Documentation + skills + README (both items, FIRST)

- [x] Rewrite `skills/cafleet-base-dir/SKILL.md` Procedure + CLI-surface to native tool steps; preserve the four guarantees as numbered steps; remove all `cafleet base-dir` invocations <!-- completed: 2026-06-06T12:48 -->
- [x] Update the six base-dir-consuming skills + `.claude/skills/skill-author` + `cafleet/reference/director.md` to call the skill procedure, not the CLI <!-- completed: 2026-06-06T13:13 -->
- [x] `README.md`: remove the `cafleet base-dir {resolve,record}` row; rename `cafleet session`→`cafleet fleet`, `--session-id`→`--fleet-id` <!-- completed: 2026-06-06T12:57 -->
- [x] Rename `docs/concepts/session-isolation.md`→`fleet-isolation.md`; fix body + all cross-links + `zensical.toml:55` nav entry (path → `concepts/fleet-isolation.md`, label → "Fleet isolation") <!-- completed: 2026-06-06T13:00 -->
- [x] Rename CAFleet-session usages in `docs/concepts/{overview,coding-agents,tmux-push}.md` (keep tmux-session usages) <!-- completed: 2026-06-06T13:03 -->
- [x] Rename `docs/spec/cli-options.md` `cafleet session` section + flag references <!-- completed: 2026-06-06T13:10 -->
- [x] Rename `docs/spec/data-model.md` `sessions`→`fleets` table + prose; KEEP `### Session ownership` (ORM) + "tmux session" wording <!-- completed: 2026-06-06T13:13 -->
- [x] Rename `docs/get-started/{quickstart,configure}.md` + `docs/reference/coding-agents/{codex,opencode}.md` CAFleet-session usages (also cleaned `docs/get-started/{index,install}.md`) <!-- completed: 2026-06-06T13:18 -->
- [x] Rename CAFleet-session identifiers/prose in `docs/concepts/member-lifecycle.md`, `docs/concepts/storage.md`, `docs/spec/webui-api.md` (webui-api: `X-Session-Id`→`X-Fleet-Id`, `/sessions`→`/fleets`, `session_id`→`fleet_id`; KEEP "session cookies" / ORM / tmux prose) <!-- completed: 2026-06-06T13:24 -->
- [x] Rename `{session_id}`→`{fleet_id}`, `SESSION ID:`→`FLEET ID:`, `--session-id`→`--fleet-id`, `cafleet session`→`cafleet fleet` across the six member-spawning skills + their `roles/*.md` (incl. `[session-id]`→`[fleet-id]` bracket placeholders in research-* roles) <!-- completed: 2026-06-06T13:39 -->
- [x] Rename CAFleet-session refs in the `cafleet` skill (`SKILL.md` + all `reference/*.md` + `roles/director.md` + `roles/member.md`), `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision` (+ `cafleet-my-slidev` spawn example) <!-- completed: 2026-06-06T13:34 -->
- [x] Rename `--session-id` / `cafleet session` examples in `.claude/rules/bash-tool.md` and `.claude/rules/commands.md` (completed by Director) <!-- completed: 2026-06-06T13:13 -->

### Step 2: Item 1 — remove `cafleet base-dir` and delete the dead module

- [x] Delete the `base-dir` Click group + `resolve` + `record` (`cli.py:402-502`) <!-- completed: 2026-06-06T13:56 -->
- [x] Drop `base_dir` from the `cli.py:20` import <!-- completed: 2026-06-06T13:56 -->
- [x] Delete `cafleet/src/cafleet/base_dir.py` (Director ran git rm) <!-- completed: 2026-06-06T13:59 -->
- [x] Delete `cafleet/tests/test_base_dir.py` and `cafleet/tests/test_base_dir_spawn_flow.py` (Director ran git rm) <!-- completed: 2026-06-06T13:59 -->
- [x] Confirm `.gitignore` keeps the unanchored `.cafleet-base-dir.json` entry (no change) <!-- completed: 2026-06-06T13:56 -->

### Step 3: Item 3 — data layer + migration

- [x] `db/models.py`: `Session`→`Fleet`, `__tablename__`→`fleets`, `session_id`→`fleet_id` (PK), `Agent.session_id`→`fleet_id`, FK string→`fleets.fleet_id`, index→`idx_agents_fleet_status` <!-- completed: 2026-06-06T14:31 -->
- [x] Add `cafleet/src/cafleet/alembic/versions/0011_rename_sessions_to_fleets.py` (`revision: str = "0011"`, `down_revision: str | None = "0010"`) with the upgrade/downgrade from §Migration design <!-- completed: 2026-06-06T14:31 -->
- [x] Do NOT edit revisions `0001`–`0010` (immutable replay history) <!-- completed: 2026-06-06T14:31 -->

### Step 4: Item 3 — broker.py

- [x] Update the models import (`Session`→`Fleet`); keep the `engine.py` `sqlalchemy.orm.Session` import untouched <!-- completed: 2026-06-06T14:31 -->
- [x] Rename the six public fns + `_agent_is_active_in_session`; rename all `session_id`→`fleet_id` params/keys/query refs; KEEP the `session` ORM local var <!-- completed: 2026-06-06T14:31 -->
- [x] Rename CAFleet-session error/description/docstrings (`_DIRECTOR_DESCRIPTION`, not-found, is-deleted, bootstrap) <!-- completed: 2026-06-06T14:31 -->

### Step 5: Item 3 — cli.py

- [x] Rename the `session` group→`fleet` + the four command callbacks + docstrings <!-- completed: 2026-06-06T14:43 -->
- [x] Rename `--session-id`→`--fleet-id`, `ctx.obj["session_id"]`→`fleet_id`, `_require_session_id`→`_require_fleet_id` + its error text <!-- completed: 2026-06-06T14:43 -->
- [x] Update `_MEMBER_PROMPT_TEMPLATE` and `_resolve_prompt` (`format` kwarg + placeholder error messages) to `fleet_id`/`{fleet_id}` <!-- completed: 2026-06-06T14:43 -->
- [x] Rename remaining CAFleet-session output/error strings; KEEP "must be run inside a tmux session" <!-- completed: 2026-06-06T14:43 -->

### Step 6: Item 3 — output.py + WebUI + admin frontend

- [x] `output.py`: `format_session_create`→`format_fleet_create` + `fleet_id` key/docstring; KEEP the tmux `session` column in `format_member_list` <!-- completed: 2026-06-06T15:16 -->
- [x] `webui_api.py`: `get_webui_session`→`get_webui_fleet`, `X-Fleet-Id`, `/fleets` route, `list_fleets`, `broker.get_fleet`/`list_fleets`, `session_id`→`fleet_id` params <!-- completed: 2026-06-06T15:16 -->
- [x] `admin/src/api.ts` + `types.ts`: rename vars/fns/header/route/interface/field <!-- completed: 2026-06-06T15:16 -->
- [x] Rename `SessionPicker.tsx`→`FleetPicker.tsx` (component, props, state, UI copy, snippet) + update every importer (`App.tsx` etc.) <!-- completed: 2026-06-06T15:16 -->
- [x] Rename CAFleet-session refs in `admin/src/components/Dashboard.tsx` (`sessionId` prop→`fleetId` + copy "Back to Sessions"/"This session has no Administrator agent"/"No agents registered in this session") and `Sidebar.tsx` (same copy) <!-- completed: 2026-06-06T15:16 -->

### Step 7: Item 3 — tests

- [x] Rename all five session-named test files → fleet-named (incl. `test_session_list_director.py`); rename the shared `tests/_broker_helpers.py::_create_session` → `_create_fleet` AND the token-less test-internal identifiers (`_create_session_with_ctx`, `_create_session_via_cli`, per-file `_create_session`, `test_session_*`/`test_create_session__*` function names) → `*_fleet*`; update `--session-id`/`cafleet session`/`session_id`/broker-fn references across all affected tests; KEEP local ORM `session` vars <!-- completed: 2026-06-07T16:00 (content+references done; the 5 git mv file renames routed to Director) -->
- [x] Add an `0011` upgrade→downgrade migration test: row preservation + schema round-trip + `PRAGMA foreign_key_check` (zero rows) after both directions + assert the `agents` FK target resolves to `fleets(fleet_id)` post-upgrade / `sessions(session_id)` post-downgrade <!-- completed: 2026-06-07T16:00 (tests/test_alembic_0011_rename.py, 8 tests) -->
- [x] Keep at most a minimal absence regression (`--session-id`/`cafleet session` no longer parse); add no deprecation-shaped tests <!-- completed: 2026-06-07T16:00 (one test in test_cli_session_flag.py asserts Click's "No such option"/"No such command") -->

### Step 8: Verification + removal-completeness sweep

- [x] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck` pass <!-- completed: 2026-06-07T16:21 (format reformatted 11 files — cosmetic line-collapses from shorter fleet_id identifiers; lint/typecheck clean) -->
- [x] `mise //cafleet:test` passes; `mise //admin:lint` passes <!-- completed: 2026-06-07T16:21 (697 passed; admin eslint clean) -->
- [x] Smoke: `cafleet fleet create` → `list` → `show` → member spawn → `delete`; confirm `cafleet session ...` and `cafleet base-dir ...` error; confirm `--session-id` rejected <!-- completed: 2026-06-07 (covered by the 697-test suite: fleet CRUD via CliRunner [test_fleet_cli.py / test_fleet_bootstrap.py], and `session` / `--session-id` / `base-dir` absence via the regression in test_cli_fleet_flag.py; the live `uv run cafleet` subprocess smoke was skipped — denied team-wide by the repo deny-rule/hook, user approved relying on the test suite + gates) -->
- [x] `cafleet db init` on a fresh DB reaches head `0011`; the WebUI loads at `/fleets` and the picker works <!-- completed: 2026-06-07 (db-init→`0011` covered by test_db_init.py + test_alembic_0011_rename.py [8 round-trip tests, in the 697]; WebUI type-consistency by `mise //admin:build` `tsc -b` clean + test_webui_api_format.py / test_server_routing.py [`/fleets`, `X-Fleet-Id`]; live browser render skipped per user decision) -->
- [x] **Removal-completeness grep (identifiers)** for `cafleet base-dir`, `base_dir`, `--session-id`, `cafleet session`, the CAFleet `session_id` identifier, and the `sessions` table name returns zero hits outside `design-docs/`, `researches/`, `.git/`, Alembic `0001`–`0010`, the `0011` migration + its round-trip test, and the §Disambiguation meaning-#2/#3/#4/#5 retentions <!-- completed: 2026-06-07 (`git grep` clean: zero `session_id` outside the migration + its test; forbidden tokens zero except the intentional absence regression in test_cli_fleet_flag.py; the grep caught + I fixed a stale `cafleet base-dir record`/`resolve` reference in .gitignore comments) -->
- [x] **Reviewed prose/UI + test/source sweep**: case-insensitive `session` sweep across `docs/`, `skills/`, `.claude/`, `admin/src`, `cafleet/src`, `cafleet/tests` (same scope as the identifier grep) — confirm every remaining hit is a deliberate meaning-#2/#3/#4 retention per §Disambiguation (catches CAFleet-concept wording AND token-less test/source-internal identifiers the grep misses) <!-- completed: 2026-06-07 (Tester: cafleet/tests, suite green 697; Programmer: cafleet/src clean no-edits; Director: docs/skills/.claude/admin/src clean no-edits; all remaining hits confirmed #2/#3/#4/#5 retentions) -->

---

## Risks / Trade-offs

1. **Loss of compiled enforcement for BASE resolution (Item 1, accepted).** The deterministic checks (traversal-escape rejection, repo-root-degenerate rejection, anchor `version: 1` validation, idempotent record-on-mismatch) move from test-covered Python into `cafleet-base-dir` skill prose that Claude executes by following instructions. This is a genuine reduction in rigor — there is no longer a unit test asserting `../outside` is rejected. Mitigation: the guarantees are preserved as explicit, numbered procedure steps (not implicit), and the skill remains the single authoritative resolver so behavior stays centralized. The user explicitly accepted this trade-off.
2. **Four-way "session" disambiguation (Item 3).** A blind find-replace would corrupt the multiplexer layer (`tmux_session`, "tmux session") and the entire data-access layer (the SQLAlchemy ORM `session` variable, `sqlalchemy.orm.Session`, `### Session ownership`). Mitigation: §Disambiguation enumerates every retained usage (tmux, SQLAlchemy ORM, and the WebUI HTTP-statelessness note); the executor reviews each hit individually; the removal-completeness sweeps explicitly exclude the meaning-#2/#3/#4 set.
3. **Breaking output/API contract (Item 3, intentional).** Renaming the `session_id` JSON key, the `--session-id` flag, the `/sessions` route, and the `X-Session-Id` header with no backward-compat alias breaks any external consumer that hardcoded the old names. This is the user's explicit choice (no aliases). The blast radius is internal (skills, tests, the bundled WebUI) with no known external integrators.
4. **Migration on live DBs (Item 3).** `0011` is a forward table/column rename with a full-inverse downgrade; SQLite native `RENAME` auto-propagates FK references on the bundled SQLite (≥ 3.25). Risk: an exotic environment with an ancient SQLite (< 3.25) would not auto-propagate. Mitigation: Python 3.12's bundled `sqlite3` is far newer; the migration test exercises upgrade+downgrade row preservation; if a sub-3.25 environment is ever in play, the fallback is the 0002-style rebuild (documented as the rejected alternative, available if needed).
5. **Spawn-prompt placeholder break (Item 3).** Renaming `{session_id}`→`{fleet_id}` means any spawn-prompt template still using `{session_id}` would raise the "Unknown placeholder" `UsageError`. All such templates are in-repo (the six skills) and are updated in the same change, so no external custom prompt is silently broken.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-06 | Initial draft |
| 2026-06-06 | Addressed Reviewer markers: Zensical nav (not MkDocs), 5th test file + `_broker_helpers` helper, admin `Dashboard`/`Sidebar` components, 3 extra docs (member-lifecycle / storage / webui-api), migration file path + typed identifiers + FK-check test, base-dir retention note, prose/UI removal-completeness criterion |
| 2026-06-06 | Round-2 review: added §Disambiguation meaning #4 (HTTP/browser session) + reconciled meaning-# numbering across the doc; widened the reviewed `session` sweep to `cafleet/src` + `cafleet/tests` and added token-less test-internal identifier renames (`_create_session_with_ctx`, `_create_session_via_cli`, `test_session_*`) |
| 2026-06-06 | Round-3 review: fixed stale Risk 2 heading ("Three-way" → "Four-way"); §Documentation-surface intro now lists the HTTP/#4 retention alongside tmux/ORM |
| 2026-06-06 | User approved; Status → Approved. Spec frozen; ready for implementation (0/41 tasks) |
| 2026-06-06 | Step 1 execution: Director added §Disambiguation meaning #5 (coding-agent runtime session — "in-session scheduling", interview-run "session", agent-browser `--session`) surfaced during the reviewed prose sweep; updated the two removal-completeness criteria to enumerate #5. Documents always-retained usages, not a scope change. |
| 2026-06-06 | Step 3 execution: Director extended the removal-completeness criteria (3 places) to exclude the new `0011` rename migration and its round-trip test `cafleet/tests/test_alembic_0011_rename.py` — both name the pre-rename `sessions`/`session_id` schema by necessity (migration-history retention, same class as immutable `0001`–`0010`). |
| 2026-06-07 | Step 7 execution: Tester flagged `cafleet/src/cafleet/multiplexer/{base.py,tmux.py}` `send_poll_trigger` as a source rename the spec's source steps (3–6) did not enumerate — it still took `session_id` and emitted `cafleet --session-id {session_id} message poll`, which would `TypeError` against the renamed `cli.py:1287` call site. Programmer renamed the param to `fleet_id` and the keystroke to `cafleet --fleet-id {fleet_id} ...` (KEEP `MultiplexerContext.session`/`tmux_session`); the 4-line source fix is folded into the Step 7 commit. No scope change — closes a source-step omission. |
| 2026-06-07 | Step 8 execution: Programmer flagged the compiled admin bundle `cafleet/src/cafleet/webui/assets/index-*.js` as carrying stale pre-rename strings (`X-Session-Id`, `/sessions`, "Select a Session"). Resolved as a non-blocker: `cafleet/src/cafleet/webui/` is gitignored (`.gitignore:16`), so it is not committed source — the `git grep` identifier sweep auto-excludes it, and `mise //cafleet:publish` chains `mise //admin:build` before packaging. Action: regenerate via `mise //admin:build` as the first step of the Verifier WebUI smoke so the smoke loads the renamed UI. Director-side reviewed `session` sweep of `docs/`/`skills/`/`.claude/`/`admin/src` = clean (all hits are #2 tmux / #3 ORM / #4 HTTP+agent-browser / #5 coding-agent retentions); Programmer-side `cafleet/src` sweep = clean (no edits). |
| 2026-06-07 | Step 8 completion: gate green (lint/typecheck/`mise //cafleet:test` 697 passed/`admin:lint`); `ruff format` reformatted 11 files (cosmetic line-collapses from shorter `fleet_id` identifiers, folded into the Step 8 commit). Identifier grep clean — caught + fixed a stale `cafleet base-dir record`/`resolve` reference in `.gitignore` comments. **Live `uv run --package cafleet cafleet …` CLI/WebUI smoke is denied team-wide** (repo deny-rule/hook over the `uv run --package *` allow; Director rule-forbidden); the Verifier confirmed `mise //admin:build` `tsc -b` passes (admin TS rename type-consistent). User decision: **rely on the test suite + gates** — the 697 CliRunner tests against isolated SQLite already exercise fleet CRUD, `--fleet-id`, `session`/`--session-id`/`base-dir` rejection, `db init`→`0011`, and `/fleets`+`X-Fleet-Id`; the live subprocess smoke is skipped and documented as covered-by-tests. |
| 2026-06-07 | PR #96 opened (8 commits) with `@copilot` review. Copilot round 1: one inline comment on `docs/get-started/quickstart.md` (compact default output contradicts the "first two lines" export prose) — routed to the Programmer, fixed by adding `--full` (commit `ce3dc5b`), pushed, re-requested. Copilot round 2: reviewed 114/115 files, **no new comments** (clean pass; the Copilot reviewer submits `COMMENTED`, never `APPROVED`). User approved finalize + teardown. Status → Complete (41/41); team torn down. PR #96 left open for the user to merge. |
