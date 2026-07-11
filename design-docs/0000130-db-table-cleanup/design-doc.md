# Rename the `tasks` table to `messages` (end-to-end)

**Status**: Approved
**Progress**: 10/27 tasks complete
**Last Updated**: 2026-07-11

## Overview

The DB entity that carries broker messages is still named `tasks` (an A2A-protocol leftover), while the CLI, docs, and product vocabulary call the domain object a "message". This design renames the entity end-to-end — table, columns, indexes, model class, internal identifiers, CLI flag, and JSON keys — so a single vocabulary remains, using a fresh-database approach (bump the DB file version, regenerate the initial migration) with no data migration.

## Success Criteria

- [ ] The schema defines a `messages` table with `message_id`, `owner_member_id`, `origin_message_id` columns and `idx_messages_*` indexes; no `tasks` table, `task_id` column, or `idx_tasks_*` index exists anywhere in `cafleet/src/`.
- [ ] `cafleet message ack` / `cancel` / `show` accept `--message-id`; `--task-id` fails with Click's standard "No such option" error (hard break, no alias).
- [ ] CLI `--json` output, the WebUI API, and the admin frontend use `message_id` / `origin_message_id` keys; the admin UI builds and renders against the renamed keys.
- [ ] The default database file is `cafleet_v5.db`; the Alembic chain is a single regenerated `0001` revision and the chain-guard test in `tests/db/test_alembic_smoke.py` passes.
- [ ] No identifier spelled `task` / `Task` / `task_id` remains in `cafleet/src/` or `admin/src/`; SPEC.md, `docs/`, all `skills/` trees (`cafleet`, `cafleet-design-doc`, `cafleet-research`), and `.claude/skills/skill-author/` carry no `tasks`-table / `task_id` / `--task-id` / `<task-id>` / `[task-id]` mention.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint`, and `mise //admin:build` all pass.

---

## Background

The original request asked to rename `agents` → `members`, rename `agent_placements` → `member_placements`, evaluate `agent_placements`' necessity, explain `monitor_config` / `monitor_runtime`, and drop unused tables. A verified codebase scan resolved most of that as already done or a no-op:

| Finding | Evidence |
|---|---|
| `agents` → `members` and `agent_placements` → `member_placements` renames are **already complete** | `cafleet/src/cafleet/db/models.py` defines `members` (L26) and `member_placements` (L46); no `agents` / `agent_placements` table exists anywhere in `cafleet/src/`. The only stale mentions live in historical `design-docs/`, which stay untouched. |
| All 7 tables are **actively used**; nothing is droppable | `fleets`, `members`, `member_placements`, `tasks`, `monitor_config`, `monitor_runtime`, `skill_installs` are each referenced by live broker/CLI/webui code. |
| `monitor_config` | Per-member ping schedule: `interval_seconds`, `last_ping_at`, `enabled`, keyed by `member_id`. Read/written by the monitor loop and `cafleet monitor config`. |
| `monitor_runtime` | Per-fleet monitor-loop process state: `pid`, `started_at`, `last_tick_at`, `tick_seconds`, keyed by `fleet_id`. Owned by `cafleet monitor start` / `status`. |

The one remaining rename is `tasks` → `messages`. The `Task` model (`models.py:61`) is the A2A-derived envelope row; every other surface already says "message" (`cafleet message send/poll/ack`, `POST /api/messages/send`, docs prose), so the table name is the outlier.

---

## Specification

### Rename map

One vocabulary everywhere. Every rename below is a hard break — no aliases, no compatibility shims, no deprecation notices (per the removal rule, no "task" spelling of the DB entity remains after this change).

| Surface | Current | New |
|---|---|---|
| Table | `tasks` | `messages` |
| Model class | `Task` (`db/models.py:61`) | `Message` |
| PK column | `task_id` | `message_id` |
| Owner FK column | `context_id` (FK → `members.member_id`, `ondelete="RESTRICT"`) | `owner_member_id` (same FK, same constraint) |
| Threading column | `origin_task_id` | `origin_message_id` |
| Index | `idx_tasks_context_status_ts` (`context_id`, `status_timestamp`) | `idx_messages_owner_member_status_ts` (`owner_member_id`, `status_timestamp`) |
| Index | `idx_tasks_from_member_status_ts` | `idx_messages_from_member_status_ts` |
| CLI flag (`message ack` / `cancel` / `show`) | `--task-id` | `--message-id` |
| CLI/JSON keys | `task_id`, `origin_task_id`, `context_id` (full projection) | `message_id`, `origin_message_id`, `owner_member_id` |
| WebUI API keys (`webui/api.py` `_format_messages`) | `task_id`, `origin_task_id` | `message_id`, `origin_message_id` |
| Admin frontend (`admin/src/types.ts` + components) | `task_id`, `origin_task_id` | `message_id`, `origin_message_id` |
| Docs/skills placeholder | `<task-id>` | `<message-id>` |
| Internal identifiers (broker/output) | `_insert_task`, `poll_tasks`, `ack_task`, `cancel_task`, `get_task`, `read_task`, `list_tasks_where`, `TASK_COLUMNS`, `render_task`, `expected_member_field="context_id"`, … | `_insert_message`, `poll_messages`, `ack_message`, `cancel_message`, `get_message`, `read_message`, `list_messages_where`, `MESSAGE_COLUMNS`, `render_message`, `expected_member_field="owner_member_id"`, … |

Unchanged columns: `from_member_id`, `to_member_id`, `type`, `created_at`, `status_state`, `status_timestamp`, `text`. Unchanged values: the `type` and `status_state` vocabularies, truncation semantics (`CAFLEET_MAX_TEXT_LEN`), and the compact `--json` projection keys (`id` / `from` / `ts` / `text` / `kind` / `origin` — already entity-neutral short names).

### `owner_member_id` rationale

`context_id` is the FK to `members.member_id` identifying **whose inbox the row lives in** — `message poll` / inbox queries filter on it, and the recipient-authorization guard in `broker/messaging.py` checks it. `owner_member_id` names that role directly. `recipient_member_id` was rejected to avoid confusion with `to_member_id` (the addressee field, which is `NULL` for broadcast rows while an owner row exists per recipient).

The column is omitted from the compact CLI projection and from the WebUI API output shape today; both behaviors are preserved. It surfaces only in `--json --full` (the raw typed-column dict), where the key follows the column name automatically.

### Fresh database, regenerated migration

A fresh database file is acceptable — no data migration and no v4 → v5 upgrade path. The old `cafleet_v4.db` is left untouched on disk.

The regenerated chain reuses revision id `0001`, so a pre-existing v4 database reached via `CAFLEET_DATABASE_URL` (or any database kept at the same path) is already stamped `0001` — `cafleet setup db` would otherwise report "Already at head (0001); nothing to do." while the stale `tasks` schema remains, and the broker would later fail with `no such table: messages` far from the cause. Per the fail-fast rule this case is guarded loudly at setup time (item 4 below), not documented away.

1. `config.py`: bump the default filename `cafleet_v4.db` → `cafleet_v5.db` at line 18, plus the docstring echo at line 33; the docstring's `Task.text` mention (line 43) becomes `Message.text`.
2. Delete `cafleet/src/cafleet/db/alembic/versions/0001_initial_schema.py` (the only revision).
3. Regenerate a fresh initial migration with `mise //cafleet:makemigration "initial schema"` after the model rename, against the new (empty) `cafleet_v5.db` brought to head via `cafleet setup db` (an empty DB with zero revisions is trivially at head). The `process_revision_directives` hook mints the id, so the file lands as `0001_<slug>.py` again. Review the generated DDL: `messages` table, both `idx_messages_*` indexes, `sqlite_autoincrement` for `fleets` / `members` / `messages`.
4. Stale-schema guard in `cafleet setup db` (`run_db_init`): when the revision check finds the database already at head **and the chain is non-empty** (`head_rev is not None`), verify the `messages` table exists in the connected database. If it is absent, exit 1 with an error naming the resolved database URL and stating that the file is a pre-rename (v4) database — point `CAFLEET_DATABASE_URL` at a fresh file (the default is already `cafleet_v5.db`). The `head_rev is not None` scope keeps item 3's regeneration transient working: an empty DB on an empty chain (`current_rev` and `head_rev` both `None`) is trivially at head with no `messages` table yet, and must pass through, while a v4 database stamped at the reused `0001` still fails loudly. Cover the guard with a test that builds a `tasks`-schema database stamped `0001` and asserts the loud failure.
5. Chain-guard: `test_single_initial_migration_revision_exists` keeps its shape (1 revision, id `0001`, `down_revision is None`) — the regenerated chain is again a single initial revision, so the guard's assertions are unchanged. The rest of `test_alembic_smoke.py` updates its literal table/column assertions (`"tasks"` → `"messages"`, `("tasks", "task_id")` → `("messages", "message_id")`, `test_tasks_table_has_origin_task_id_column` → renamed for `origin_message_id`, the AUTOINCREMENT table set, and `test_tasks_to_member_id_is_nullable_after_migration`).

### Edit surface by module

| Area | Files | What changes |
|---|---|---|
| Schema/model | `db/models.py`, `config.py`, `db/alembic/versions/` | Rename map rows 1–6; v5 filename; regenerated `0001` |
| Broker | `broker/messaging.py`, `broker/queries.py`, `broker/_shared.py`, `broker/members.py`, `broker/monitor.py`, `broker/__init__.py` | `Message` model usage, renamed functions/constants, `owner_member_id` filters and dict keys, docstrings |
| Output | `output/render.py`, `output/formatters.py`, `output/__init__.py` | `render_message`, full-projection keys |
| Multiplexer | `multiplexer/base.py`, `multiplexer/tmux.py`, `multiplexer/herdr.py` | Inline-preview identifiers referencing the message row |
| CLI | `cli/message.py` | `--message-id` on `ack` / `cancel` / `show`; echo/JSON key `result["message"]["message_id"]` |
| WebUI | `webui/api.py` | `_format_messages` output keys `message_id` / `origin_message_id`; the `POST /api/messages/send` response body's raw `task_id` key (lines 180 and 189, outside `_format_messages`) → `message_id` |
| Admin | `admin/src/types.ts`, `components/Timeline.tsx`, `components/MemberDetail.tsx`, `components/ReactionBar.tsx` | Renamed keys; `mise //admin:build` regenerates the committed `webui/dist` bundle |
| Tests | ~24 files under `cafleet/tests/` | Fixtures, assertions, test names (e.g. `test_list_inbox__filters_broadcast_summary_and_context_id_scope`), and the file rename `tests/output/test_render_task.py` → `test_render_message.py` |

### Documentation surface

Documentation-first order applies (per `.claude/rules/documentation-maintenance.md`): every page below is updated before code.

| Page | What changes |
|---|---|
| `docs/spec/data-model.md` | `### tasks` heading → `### messages`; ER diagram (`members \|\|--o{ messages`, column list); FK rows; `## Task Visibility Rules` → `## Message Visibility Rules`; broadcast grouping by `origin_message_id` |
| `docs/spec/message-envelope.md` | Typed-column names, compact-vs-full table (`owner_member_id` omitted/included), sample full JSON |
| `docs/spec/webui-api.md` | Response keys, including the `POST /api/messages/send` response sample; the `owner_member_id = member_id` inbox-filter prose |
| `docs/spec/cli-options.md` | `--message-id` flag rows, error strings, `<message-id>` placeholders, `permissions.allow` coverage prose |
| `docs/spec/multiplexer-backends.md` | Message-row identifier mentions in the push-notification mechanics |
| `docs/concepts/storage.md` | Inbox-polling-by-`owner_member_id` prose |
| `SPEC.md` | ~30 DB-entity mentions (`task_id`, `origin_task_id`, `context_id`, `tasks` table, index names, column spec, FK-enforcement note, visibility/query semantics); the ~75 generic-English "task" usages stay; smallest drift-removing edits, structure preserved |
| `skills/cafleet/SKILL.md`, `skills/cafleet/reference/cli.md`, `skills/cafleet/roles/director.md` | `--task-id` → `--message-id`, `<task-id>` → `<message-id>`, `--quiet` bare-id prose, "Messages are tasks" reworded to describe the `messages` row directly |
| `skills/cafleet/reference/supervision.md` (L115, L192), `skills/cafleet/reference/exec-routing.md` (L34), `skills/cafleet/reference/director.md` (L146) | `--task-id` / `<task-id>` → `--message-id` / `<message-id>` |
| `skills/cafleet-design-doc/create/create.md` (L181), `skills/cafleet-design-doc/create/roles/director.md` (L51), `skills/cafleet-design-doc/interview/interview.md` (L150) | `--task-id` / `<task-id>` → `--message-id` / `<message-id>` |
| `skills/cafleet-research/report/report.md` (L133, L162, L241), `skills/cafleet-research/presentation/presentation.md` (L132, L186) | `--task-id` / `<task-id>` → `--message-id` / `<message-id>` |
| `[task-id]` capture token in `skills/cafleet-research/report/roles/scout.md` (L31), `skills/cafleet-research/presentation/roles/presentation.md` (L34), `skills/cafleet-research/presentation/roles/visual-reviewer.md` (L34), `skills/cafleet-research/presentation/roles/transcript.md` (L32) | `[task-id]` → `[message-id]` |
| `.claude/skills/skill-author/SKILL.md` (L327, L330) | `--task-id` / `<task-id>` → `--message-id` / `<message-id>` |

`README.md`'s thin surface (pitch, install, section links) carries no `task_id` mention; verify and leave unchanged. Historical `design-docs/` stay untouched. The project `.claude/settings.json` `permissions` block contains no `--task-id` pattern; no settings change. `site/` is gitignored build output; `docs/` pages outside the listed ones are clean.

### Out of scope

- Any change to `fleets`, `members`, `member_placements`, `monitor_config`, `monitor_runtime`, `skill_installs`.
- Data migration or a v4 → v5 upgrade path.
- Backward-compatible aliases for the CLI flag or JSON keys.
- Renaming `from_member_id` / `to_member_id` or altering broadcast/visibility semantics.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (before any code)

- [x] Update `docs/spec/data-model.md` (headings, ER diagram, FK rows, visibility rules, broadcast grouping) <!-- completed: 2026-07-11T14:51 -->
- [x] Update `docs/spec/message-envelope.md` (typed columns, compact/full table, sample JSON) <!-- completed: 2026-07-11T14:52 -->
- [x] Update `docs/spec/webui-api.md` (response keys, inbox-filter prose) <!-- completed: 2026-07-11T14:54 -->
- [x] Update `docs/spec/cli-options.md` (`--message-id` rows, error strings, placeholders, permissions prose) <!-- completed: 2026-07-11T14:56 -->
- [x] Update `docs/spec/multiplexer-backends.md` (message-row identifiers) <!-- completed: 2026-07-11T14:58 -->
- [x] Update `docs/concepts/storage.md` (inbox polling prose; also the `cafleet_v5.db` default-path mentions here and in `docs/get-started/install.md` / `docs/reference/coding-agents/codex.md`) <!-- completed: 2026-07-11T14:58 -->
- [x] Update `SPEC.md` (~30 DB-entity mentions; smallest drift-removing edits) <!-- completed: 2026-07-11T15:09 -->
- [x] Update `skills/cafleet/SKILL.md`, `skills/cafleet/reference/cli.md`, `skills/cafleet/roles/director.md` <!-- completed: 2026-07-11T15:13 -->
- [x] Update the remaining skill pages from the documentation-surface table: `skills/cafleet/reference/{supervision,exec-routing,director}.md`, `skills/cafleet-design-doc/{create/create.md,create/roles/director.md,interview/interview.md}`, `skills/cafleet-research/{report/report.md,presentation/presentation.md}` plus the four `[task-id]` role-file capture tokens, and `.claude/skills/skill-author/SKILL.md` (applied Director-side after a harness `.claude/` write denial) <!-- completed: 2026-07-11T15:22 -->
- [x] Verify `README.md` thin surface is unaffected (no edit expected; only generic-English "task guides" remains) <!-- completed: 2026-07-11T15:18 -->

### Step 2: Schema and model

- [ ] Rename in `db/models.py`: class `Message`, `__tablename__ = "messages"`, columns `message_id` / `owner_member_id` / `origin_message_id`, index names `idx_messages_owner_member_status_ts` / `idx_messages_from_member_status_ts` <!-- completed: -->
- [ ] Bump `config.py` default DB file to `cafleet_v5.db` (line 18 + docstring line 33) and fix the `Task.text` docstring mention <!-- completed: -->
- [ ] Delete `0001_initial_schema.py`, run `cafleet setup db` against the fresh v5 file, regenerate via `mise //cafleet:makemigration "initial schema"`, and review the generated DDL <!-- completed: -->
- [ ] Add the stale-schema guard to `cafleet setup db` (`run_db_init`) — loud exit 1 when the DB is at head on a non-empty chain (`head_rev is not None`) but the `messages` table is absent — with a test that builds a `tasks`-schema database stamped `0001` and asserts the failure <!-- completed: -->
- [ ] Update `tests/db/test_alembic_smoke.py` table/column assertions and renamed test names (chain-guard assertions unchanged); update `tests/db/test_init.py` <!-- completed: -->

### Step 3: Broker, output, multiplexer

- [ ] Rename through `broker/messaging.py` (functions, `owner_member_id` filter + dict keys, `expected_member_field`, docstrings) <!-- completed: -->
- [ ] Rename through `broker/queries.py` and `broker/_shared.py` (`MESSAGE_COLUMNS`, `read_message`, `list_messages_where`, `get_message`) <!-- completed: -->
- [ ] Rename through `broker/members.py`, `broker/monitor.py`, `broker/__init__.py` (joins on `Message.owner_member_id`, re-exports) <!-- completed: -->
- [ ] Rename through `output/render.py` (`render_message`), `output/formatters.py`, `output/__init__.py` <!-- completed: -->
- [ ] Rename through `multiplexer/base.py`, `multiplexer/tmux.py`, `multiplexer/herdr.py` <!-- completed: -->

### Step 4: CLI

- [ ] Update `cli/message.py`: `--message-id` on `ack` / `cancel` / `show`, echo/JSON keys <!-- completed: -->

### Step 5: WebUI and admin frontend

- [ ] Update `webui/api.py`: `_format_messages` output keys and the `POST /api/messages/send` response body's `task_id` key (lines 180, 189) <!-- completed: -->
- [ ] Update `admin/src/types.ts`, `Timeline.tsx`, `MemberDetail.tsx`, `ReactionBar.tsx` <!-- completed: -->
- [ ] Run `mise //admin:build` to regenerate the committed `webui/dist` bundle <!-- completed: -->

### Step 6: Test sweep and verification

- [ ] Sweep the remaining ~24 test files: fixtures, assertions, test names, and the `test_render_task.py` → `test_render_message.py` file rename <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format`, `mise //admin:lint` <!-- completed: -->
- [ ] Final vocabulary check: no `task` / `Task` / `task_id` identifier remains in `cafleet/src/` or `admin/src/`; no `tasks`-table / `--task-id` / `<task-id>` / `[task-id]` mention remains in SPEC.md, `docs/`, any `skills/` tree, or `.claude/skills/skill-author/` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-11 | Initial draft |
| 2026-07-11 | Reviewer round 1: stale-v4-DB loud guard in `cafleet setup db`; send-endpoint `task_id` response key added to the WebUI edit surface; skills documentation inventory completed (all `skills/` trees + `.claude/skills/skill-author/`) |
| 2026-07-11 | Reviewer round 2: stale-schema guard scoped to a non-empty chain (`head_rev is not None`) so the empty-chain regeneration transient passes through |
