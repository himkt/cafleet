# Agent → Member Consolidation, Uniform Identity Flags, and Director Auto-Discovery

**Status**: Complete
**Progress**: 29/29 tasks complete
**Last Updated**: 2026-07-11

## Overview

Resolves GitHub issues #184 ("Consolidate agent and member") and #185 ("Director auto discovery"), plus the CLI option-naming confusion around `--agent-id` / `--member-id`. The registry noun "agent" is retired in favor of "member" everywhere — CLI, SQLite schema, Python internals, HTTP API, docs, and skills — every identity flag gets exactly one meaning (`--member-id` = the member in question; `--from-member-id` / `--to-member-id` on two-party commands), and `cafleet member create` auto-resolves the Director from `--fleet-id` instead of requiring a Director-identity flag.

## Success Criteria

- [x] `cafleet member create --fleet-id <f> --name <n> --description <d> --text <p>` spawns a member with no Director-identity flag; the CLI resolves the Director from `fleets.director_member_id`.
- [x] `cafleet message send --fleet-id <f> --from-member-id <s> --to-member-id <r> --text <t>` is the send shape; the removed spellings (`--agent-id`, `--to`) fail with Click's standard no-such-option error (exit 2).
- [x] The default SQLite file is `~/.local/share/cafleet/cafleet_v3.db`; the migration chain is a single fresh initial revision `0001` creating the members schema (the old chain is deleted, not migrated); the chain-guard test asserts the 1-revision chain. Pre-existing `cafleet_v2.db` files are never opened by the new code.
- [x] Member-ness derives from "active member WITH a placement row AND `member_id != fleets.director_member_id`"; the broker has one list/kind-derivation path (no parallel `list_fleet_agents` vs `list_members` families, no Python-vs-SQL kind split).
- [x] The registry noun "agent" is absent from CLI help, error strings, JSON keys, docs, SPEC.md, and skills — outside the coding-agent boundary (§ Scope boundary) and historical records (git, design docs).
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` pass; `mise //admin:build` regenerates the frontend bundle with the renamed API keys.

---

## Background

Today one registry entity wears two nouns and three flag semantics:

- **Two nouns**: the DB and broker say "agent" (`agents` table, `register_agent`, `--agent-id`); the CLI command group and supervision docs say "member" (`cafleet member create`, `--member-id`). A "member" is not a table — it is an `Agent` row with an `AgentPlacement` whose `director_agent_id` is non-null (`broker/members.py`).
- **Three meanings of `--agent-id`**: "me, the sender" (`message send/broadcast/poll/ack/cancel/show`, `member nudge`), "the Director" (`member create`), and "the target" (`monitor config`). Users mixing the surfaces produce commands like `cafleet message send --fleet-id 13 --agent-id 58 --member-id 61` that parse on no command.
- **A duplicate column**: `agent_placements.director_agent_id` always equals `fleets.director_agent_id` (nested teams are forbidden; `register_agent` rejects any non-root value). Its remaining job is (a) carrying `member create --agent-id` input and (b) discriminating members from the root Director's own placement (`IS NOT NULL`). Issue #185 removes (a); a fleets join replaces (b).
- **Parallel derivation paths**: `list_members` / `list_roster` / `list_members_with_activity` (broker/members.py) vs `list_fleet_agents` (broker/agents.py); Python `derive_agent_kind` vs the ad-hoc SQL kind mapping in `list_fleet_agents`.

---

## Specification

### Scope boundary: "coding agent" is not renamed

The **coding agent** (the backend binary: `claude` / `codex` / `opencode`) is a different concept from a registry member and keeps its noun everywhere:

| Untouched surface | Examples |
|:--|:--|
| CLI flag + column | `--coding-agent`, `member_placements.coding_agent`, `skill_installs.coding_agent` |
| Python package | `cafleet/src/cafleet/coding_agent/`, `CODING_AGENTS` registry |
| Spawn placeholder + prompt line | `{coding_agent}`, `CODING AGENT:` |
| Multiplexer process-state API | `AgentStateAware`, `agent_status`, `wait_agent_status` — these three process-state names only (they describe the backend process, not the registry); the multiplexer's poll-trigger payload and registry-id signature params ARE renamed (§ Python internals) |
| `cafleet setup` targets | `--agent claude-code/codex/opencode` (skill-install targets are backends) |
| Docs & skills homes | `docs/reference/coding-agents/`, `docs/concepts/coding-agents.md`, `docs/api/coding-agent.md`, `skills/cafleet/reference/coding-agent/`, `.claude/rules/coding-agent-overlay.md` |

Boundary-straddling pages (a coding-agent page embedding a registry `--agent-id` example) update only the registry tokens.

The project pitch "a message broker and agent registry **for coding agents**" becomes "a message broker and member registry for coding agents" — the trailing phrase keeps its noun.

### One noun: rename maps

**Hard break** (per the user's decision): old names are removed entirely in one release; no deprecation aliases; exactly one canonical spelling per option.

#### Database schema (fresh DB, hard break)

| Today | After |
|:--|:--|
| table `agents` | table `members` |
| `agents.agent_id` | `members.member_id` |
| `agents.agent_card_json` | `members.member_card_json` |
| index `idx_agents_fleet_status` | index `idx_members_fleet_status` |
| table `agent_placements` | table `member_placements` |
| `agent_placements.agent_id` | `member_placements.member_id` |
| `agent_placements.director_agent_id` + index `idx_placements_director` | **dropped** (no forensic retention) |
| `fleets.director_agent_id` | `fleets.director_member_id` |
| `tasks.from_agent_id` / `tasks.to_agent_id` | `tasks.from_member_id` / `tasks.to_member_id` |
| index `idx_tasks_from_agent_status_ts` | index `idx_tasks_from_member_status_ts` |
| `monitor_config.agent_id` | `monitor_config.member_id` |

Unchanged: `tasks.context_id` (carries no noun; it remains the inbox owner's member id), `tasks` other columns, `monitor_runtime`, `skill_installs`. Card JSON key names (`name` / `description` / `skills` / `cafleet.kind`) and the kind constants `builtin-administrator` / `monitoring-member` are also unchanged.

#### Fresh DB and regenerated initial migration

The schema changes ship as a **fresh database**, not an in-place migration (per the user's decision):

1. **Bump the default DB filename**: `_default_database_url` in `config.py` changes `cafleet_v2.db` → `cafleet_v3.db` (default factory and both docstrings). Pre-existing `cafleet_v2.db` files are abandoned in place — the new code never opens them, and no data-migration path exists.
2. **Delete the old migration chain**: remove `db/alembic/versions/0001_initial_schema.py` (the entire chain).
3. **Regenerate the initial migration**: after `db/models.py` is renamed, run `mise //cafleet:makemigration "initial schema"` against a fresh (nonexistent) `cafleet_v3.db` — with an empty chain, autogenerate emits the full members schema and the `env.py` hook mints revision `0001`. Review the generated file per `database-migrations.md` (index names, `sqlite_autoincrement`, FK targets) before committing.

Chain guard: `tests/db/test_alembic_smoke.py::test_single_initial_migration_revision_exists` keeps its single-revision semantics over the fresh `0001` (head `0001`, `down_revision` `None`). No upgrade-path tests exist — there is no upgrade path.

#### Python internals

| Today | After |
|:--|:--|
| `db/models.py` `Agent` / `AgentPlacement` | `Member` / `MemberPlacement` (renamed columns per the schema map; `director_agent_id` column and `Index("idx_placements_director", ...)` deleted) |
| `broker/agents.py` (module) | merged into `broker/members.py` — one registry module |
| `register_agent` / `get_agent` / `deregister_agent` / `get_agent_names` / `verify_agent_fleet` | `register_member` / `get_member` / `deregister_member` / `get_member_names` / `verify_member_fleet` |
| `list_fleet_agents` | **removed** — folded into `list_roster` (§ Unified derivation) |
| `_shared.derive_agent_kind` / `agent_is_active_in_fleet` | `derive_member_kind` / `member_is_active_in_fleet` |
| `monitor.enroll_agent` / `delete_agent_monitor_row` | `enroll_member` / `delete_member_monitor_row` |
| `agent_id` params/keys throughout `broker/` (messaging, queries, monitor), `output/`, `webui/` | `member_id` |
| `multiplexer/`: the keystroked poll payload `cafleet message poll --fleet-id {fleet_id} --agent-id {agent_id}` (`tmux.py`, `herdr.py`) and the `send_poll_trigger` / `send_wake_trigger` signatures (`agent_id`, `due_agents` rows, `director_agent_id` — `base.py`) | `--member-id` in the payload; `member_id` / `due_members` / `director_member_id` |
| `monitor/loop.py`: `fleet["director_agent_id"]`, `target["agent_id"]`, `_last_agent_status` | `fleet["director_member_id"]`, `target["member_id"]`, `_last_member_status` |
| `output.format_agent` | `output.format_member_detail` (the existing `format_member` — the `member create` result renderer — keeps its name) |

The 4-value kind vocabulary is unchanged: `director` / `administrator` / `monitor` / `member` (the last meaning "ordinary member").


#### Result-dict / JSON keys (CLI text + `--json`, broker returns)

`agent_id` → `member_id`, `administrator_agent_id` → `administrator_member_id`, `director_agent_id` → `director_member_id` (fleet create/list/show), `member_agent_id` → `member_id` (member capture/exec/ping/nudge outputs), `from_agent_id` / `to_agent_id` → `from_member_id` / `to_member_id` (task envelopes), `agents` → `members` (monitor status). The placement sub-dict loses its `director_agent_id` key entirely. Text-layout labels swap the same way (`agent_id:` → `member_id:`). Output shapes may change where the unification makes it natural; SPEC.md is the byte-exact record.

### Unified member-ness and kind derivation

**Member-ness predicate** (replaces `AgentPlacement.director_agent_id IS NOT NULL`): an active `members` row **with** a `member_placements` row **and** `member_id != fleets.director_member_id`. The root Director keeps its placement row (it is pane-bound); the join with `fleets` excludes it from member lists exactly as the `NULL`-director row did.

**One query family** in `broker/members.py`:

- `_base_members_select(fleet_id)` joins `Member` ⨝ `MemberPlacement` ⨝ `Fleet` and filters on the predicate above. `list_members` and `list_members_with_activity` keep their shapes on top of it.
- `list_roster(fleet_id, *, include_task_holders: bool = False)` absorbs `list_fleet_agents`: active members LEFT-JOINed to placements, plus — when `include_task_holders=True` (the WebUI need) — deregistered members that still own tasks. Every row's `kind` comes from the single derivation below; the old ad-hoc `administrator`/`"user"` mapping is deleted.
- **One kind path**: SQL supplies `is_root` (`member_id == fleets.director_member_id` from the join) and `card_kind` (`CARD_KIND_SQL`, unchanged `json_extract` over `member_card_json`); `derive_member_kind(is_root, card_kind)` is the only Python collapse, used by `get_member`, `list_roster`, and the WebUI.

**`register_member` simplification**: the `placement` dict loses `director_agent_id` (the column is gone). The nested-team guard and the Administrator-as-director guard are deleted — both are impossible by construction once no caller supplies a director id. One invariant guard remains: when `placement` is supplied, the fleet's `director_member_id` must reference an active member; violation raises `click.ClickException` (`fleet <fleet-id>'s root Director (member <id>) is not active.`) — a loud invariant failure, not a usage error, since the value is no longer user input.

### CLI identity flags

**The scheme** (user-confirmed): two-party commands name both parties as `--from-member-id` (sender) + `--to-member-id` (recipient/target); every single-member command takes `--member-id` meaning "the member in question" — exactly one meaning per spelling, everywhere.

| Command | Identity flags after (all still require `--fleet-id`) |
|:--|:--|
| `message send` | `--from-member-id` + `--to-member-id` (replaces `--agent-id` + `--to`) |
| `message broadcast` | `--from-member-id` (replaces `--agent-id`) |
| `message poll` | `--member-id` (replaces `--agent-id`) |
| `message ack` / `cancel` / `show` | `--member-id` + `--task-id` (replaces `--agent-id`) |
| `member create` | *(none — Director auto-resolved, § below; `--agent-id` removed)* |
| `member delete` / `show` / `capture` / `exec` / `ping` | `--member-id` (unchanged spelling, now the uniform meaning) |
| `member nudge` | `--from-member-id` + `--to-member-id` (replaces `--agent-id` + `--member-id`) |
| `member list` | *(unchanged: no identity flag)* |
| `monitor config` | `--member-id` (replaces `--agent-id`) |
| `monitor start` / `status` | *(unchanged: no identity flag)* |

Confirmed shapes:

```bash
cafleet message send --fleet-id 13 --from-member-id 58 --to-member-id 61 --text hi
cafleet message poll --fleet-id 13 --member-id 58
cafleet message ack  --fleet-id 13 --member-id 58 --task-id 12
cafleet member exec  --fleet-id 13 --member-id 61 "ls"
cafleet member ping  --fleet-id 13 --member-id 61
```

Shared declarations in `cli/_helpers.py`: `agent_id_option` is replaced by `member_id_option` (`--member-id`, help `"Member ID (the member in question)"`), plus new `from_member_id_option` (help `"Sender's member ID"`) and `to_member_id_option` (help `"Recipient member ID"`); `director_member_options` is subsumed by `member_id_option`.

Error strings swap the noun and drop the now-redundant phrasing; the two contract-relevant rewrites:

| Site | Today | After |
|:--|:--|:--|
| `client_command` fleet guard | `agent {id} is not a member of fleet {fleet_id}.` | `member {member_id} is not in fleet {fleet_id}.` |
| `_load_authorized_member` | `Agent {id} not found` | `Member {member_id} not found` |

All other error strings (`no placement row`, rollback hints, monitor resolution, …) swap `agent` → `member` verbatim; SPEC.md records each final string.

### Issue #185: Director auto-discovery in `member create`

`member create` resolves the Director itself, first thing, from `broker.get_fleet(fleet_id)` — before `register_member` (the resolved id also feeds `_resolve_coding_agent`'s monitor backend inheritance and the spawn-prompt substitution). Correct by construction: a fleet has exactly one root Director, back-filled by `create_fleet`. No override flag exists — an override could only ever supply the same value or a rejected one.

| Condition | Error | Exit |
|:--|:--|:--|
| fleet missing | `click.UsageError`: `Fleet '<fleet-id>' not found.` | 2 |
| fleet soft-deleted | `click.ClickException`: `fleet <fleet-id> is deleted` | 1 |
| `director_member_id` is `NULL` (mid-bootstrap corruption) | `click.ClickException`: `fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` | 1 |
| root Director not active | `click.ClickException` from the `register_member` invariant guard (§ above) | 1 |

Scope limit (user-confirmed): auto-discovery applies only to surfaces that *require* the Director id as input — `member create` today. Spawn-prompt Director-id injection remains the member-side mechanism; no `--to-director` convenience on `message send`.

### Spawn placeholders and prompt identity lines

`substitute_spawn_placeholders` renders `{fleet_id}` / `{member_id}` / `{director_member_id}` / `{coding_agent}` (renamed from `{agent_id}` / `{director_agent_id}`); the unknown-placeholder `UsageError` lists the new set. The skill-authored literal identity lines become `YOUR MEMBER ID:` / `DIRECTOR MEMBER ID:` (alongside unchanged `FLEET ID:` / `CODING AGENT:`); every spawn-prompt skeleton in `skills/` updates in the same cycle.

### HTTP API and admin frontend

| Today | After |
|:--|:--|
| `GET /agents` | `GET /members` |
| `GET/PATCH /agents/{agent_id}/monitor`, `GET /agents/{agent_id}/inbox` / `/sent` | `/members/{member_id}/...` |
| response wrapper `{"agents": [...]}` | `{"members": [...]}` |
| JSON keys `agent_id`, `from_agent_id`, `from_agent_name`, `to_agent_id`, `to_agent_name` | `member_id`, `from_member_id`, `from_member_name`, `to_member_id`, `to_member_name` |

The `/members` roster now reports the unified 4-value `kind` (replacing the old `administrator`/`"user"` split) via `list_roster(include_task_holders=True)`.

Admin frontend (`admin/src/`): `types.ts` (`Member`, `MembersResponse`), `api.ts` (`getMembers`, `updateMemberMonitor`, renamed paths/body keys), and the components carrying the noun (`AgentAvatar` → `MemberAvatar`, `AgentDetail` → `MemberDetail`, plus key references in `Dashboard` / `Sidebar` / `Timeline` / `TimelineMessage` / `MessageInput` / `ReactionBar` / `FleetPicker` / `App` / `hooks/useRefreshKeyLoad`). The packaged bundle under `webui/dist/` is regenerated by `mise //admin:build`, never hand-edited.

### Removal discipline

Per the removal rule: after this lands, the repository reads as if the registry noun "agent" never existed — no deprecation notices, no "formerly `--agent-id`" callouts. Legitimate absence guards: one regression test asserting the removed spellings no longer parse (Click's default no-such-option error on `message send --agent-id` / `member create --agent-id`), mirroring the existing `test_agent_group_removed.py` pattern (which itself stays valid — the `cafleet agent` group remains absent).

### Documentation ripple (contract surfaces, same cycle)

| Surface | What changes |
|:--|:--|
| `SPEC.md` | schema DDL (§8, §5.2), kind discriminator (§5.4), nullable `to_member_id` (§5.5), broker function inventory (§6.2), CLI identity flags + per-command options + error strings (§6.3), text layouts (§6.4), WebUI routes/keys (§6.8) — smallest edits that remove the drift |
| `README.md` / `CLAUDE.md` / `pyproject.toml` descriptions / `zensical.toml` `site_description` | pitch reworded to "member registry for coding agents" (README via the `/update-readme` skill) |
| `docs/spec/` | `data-model.md`, `cli-options.md`, `webui-api.md`, `message-envelope.md`, `multiplexer-backends.md` |
| `docs/concepts/` | `overview.md`, `storage.md`, `member-lifecycle.md`, `monitoring.md`, `fleet-isolation.md` (incl. the rewritten messaging error strings `Destination member not found: {to_id}` / `Destination member not in fleet: {to_id}`), `multiplexer-backends.md` |
| `docs/get-started/`, `docs/how-to/`, `docs/index.md` | command examples and narrative |
| `docs/reference/coding-agents/*`, `docs/api/coding-agent.md` | registry tokens inside spawn examples only (boundary-straddlers) |
| `skills/cafleet/` | `SKILL.md`, `roles/*` , `reference/director.md` (spawn skeleton), `reference/cli.md`, `exec-routing.md`, `supervision.md`, `broadcast.md`, `recovery.md`, `output-flags.md`, coding-agent overlays' registry tokens |
| `skills/cafleet-design-doc/`, `skills/cafleet-research/`, `skills/skill-author/` | spawn prompts, poll/send/ack command shapes, identity-line conventions |
| `.claude/rules/` | `bash-tool.md` (command shapes, `<my-agent-id>` placeholders → `<my-member-id>`), `commands.md` ("fleet-id / member-id CLI design" phrasing), `database-migrations.md` (re-point its chain-guard reference to the single-revision chain guard over the fresh `0001`) |



---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (contract-first, per documentation-maintenance)

- [x] Update `docs/concepts/` (overview, storage, member-lifecycle, monitoring, fleet-isolation incl. its messaging error strings, multiplexer-backends) to the member noun, the flag scheme, Director auto-discovery, and the dropped placement column <!-- completed: 2026-07-11T02:06 -->
- [x] Update `docs/spec/`: `data-model.md`, `cli-options.md` (per-command flags + error strings), `webui-api.md`, `message-envelope.md`, `multiplexer-backends.md` <!-- completed: 2026-07-11T02:15 -->
- [x] Update `docs/get-started/` and `docs/how-to/` command examples; `docs/index.md`; `zensical.toml` `site_description` <!-- completed: 2026-07-11T02:22 -->
- [x] Update registry tokens in the boundary-straddling coding-agent docs pages <!-- completed: 2026-07-11T02:28 -->
- [x] Update `SPEC.md` contract surfaces (schema, broker inventory, CLI options, error strings, JSON keys, text layouts, HTTP routes) <!-- completed: 2026-07-11T02:38 -->
- [x] Update `README.md` pitch via the `/update-readme` skill; reword `CLAUDE.md` and both `pyproject.toml` descriptions <!-- completed: 2026-07-11T02:38 -->
- [x] Update `.claude/rules/bash-tool.md` and `.claude/rules/commands.md` command shapes and placeholders, and re-point `.claude/rules/database-migrations.md` at the single-revision chain guard over the fresh initial migration <!-- completed: 2026-07-11T02:43 -->
- [x] Update `skills/cafleet/` (SKILL.md, roles, reference incl. the `director.md` spawn skeleton and `cli.md`) <!-- completed: 2026-07-11T02:48 -->
- [x] Update `skills/cafleet-design-doc/` and `skills/cafleet-research/` workflow + role files (spawn prompts, command shapes, identity lines) <!-- completed: 2026-07-11T03:02 -->
- [x] Update `skills/skill-author/` and the `skills/cafleet/reference/coding-agent/` overlays' registry tokens <!-- completed: 2026-07-11T03:05 -->

### Step 2: Schema

- [x] Bump the default DB filename to `cafleet_v3.db` in `config.py` (`_default_database_url` factory and both docstrings) <!-- completed: 2026-07-11T03:17 -->
- [x] Update `db/models.py`: `Member` / `MemberPlacement`, renamed columns and indexes, `director_agent_id` column + `idx_placements_director` deleted <!-- completed: 2026-07-11T03:17 -->
- [x] Delete `db/alembic/versions/0001_initial_schema.py` and regenerate the fresh initial `0001` via `mise //cafleet:makemigration "initial schema"` from the renamed models; review per `database-migrations.md` <!-- completed: 2026-07-11T03:17 -->
- [x] Restore the chain-guard test in `tests/db/test_alembic_smoke.py` to the single-revision assertion over the fresh `0001` and remove the now-obsolete migration-0002 upgrade suite <!-- completed: 2026-07-11T07:12 -->

### Step 3: Broker

- [x] Merge `broker/agents.py` into `broker/members.py` with the renamed function set; update `broker/__init__.py` exports <!-- completed: 2026-07-11T03:27 -->
- [x] Re-point member-ness and kind derivation at the `fleets` join (`member_id != director_member_id`); rename `_shared` helpers (`derive_member_kind`, `member_is_active_in_fleet`, `placement_dict` without the director key) <!-- completed: 2026-07-11T03:27 -->
- [x] Fold `list_fleet_agents` into `list_roster(fleet_id, include_task_holders=False)` with the single kind path <!-- completed: 2026-07-11T03:27 -->
- [x] `register_member`: drop the director input from `placement`, delete the nested-team and Administrator-as-director guards, add the root-Director-active invariant guard <!-- completed: 2026-07-11T03:27 -->
- [x] Rename `agent_id` params across `broker/messaging.py`, `broker/queries.py`, `broker/monitor.py` (`enroll_member`, `delete_member_monitor_row`), `broker/fleets.py` <!-- completed: 2026-07-11T03:27 -->

### Step 4: CLI

- [x] Replace flag declarations in `cli/_helpers.py` (`member_id_option`, `from_member_id_option`, `to_member_id_option`) and apply the per-command table across `cli/message.py`, `cli/member.py`, `cli/monitor.py` <!-- completed: 2026-07-11T03:40 -->
- [x] `member create`: Director auto-resolution with the § #185 error table; placement dict without a director id <!-- completed: 2026-07-11T03:40 -->
- [x] Rename spawn placeholders to `{member_id}` / `{director_member_id}` in `cli/_text_input.py` including the unknown-placeholder error text <!-- completed: 2026-07-11T03:40 -->
- [x] Rename registry ids in `multiplexer/` (poll-trigger payload `--member-id` in `tmux.py` / `herdr.py`; `send_poll_trigger` / `send_wake_trigger` signatures incl. `due_members`) and `monitor/loop.py` (`director_member_id`, `member_id`, `_last_member_status`) <!-- completed: 2026-07-11T03:40 -->
- [x] Update `output/` formatters: `format_member_detail`, renamed labels and result-dict keys (`member_id`, `director_member_id`, `administrator_member_id`, `from_member_id` / `to_member_id`) <!-- completed: 2026-07-11T03:40 -->
- [x] Add the removed-spelling regression guard (`--agent-id` / `--to` no longer parse) alongside the existing `test_agent_group_removed.py` <!-- completed: 2026-07-11T03:40 -->

### Step 5: WebUI and admin frontend

- [x] Rename `webui/api.py` routes to `/members/...`, response wrapper and JSON keys, unified roster kind via `list_roster(include_task_holders=True)` <!-- completed: 2026-07-11T03:45 -->
- [x] Rename admin frontend types, API client, and components (`MemberAvatar`, `MemberDetail`, key references); regenerate the bundle with `mise //admin:build` <!-- completed: 2026-07-11T03:45 -->

### Step 6: Tests and verification

- [x] Sweep `tests/broker/`, `tests/cli/`, `tests/output/`, `tests/webui/`, `tests/db/`, `tests/multiplexer/`, `tests/monitor/`, `tests/coding_agent/` to the renamed flags, functions, keys, and error strings <!-- completed: 2026-07-11T07:12 -->

- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` all pass <!-- completed: 2026-07-11T07:12 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-11 | Initial draft |
| 2026-07-11 | Reviewer round 1: FK-safe batch recreate for the column drop; multiplexer/monitor rename scope; docs, rules, and test-sweep ripple additions |
| 2026-07-11 | User redirect: fresh DB (`cafleet_v3.db`) + regenerated initial `0001` replaces the data-preserving migration `0002`; SC #3, § schema mechanics, and Step 2 tasks rewritten |
| 2026-07-11 | Execution complete: all 29 tasks and 6 Success Criteria verified, Verifier E2E passed, Reviewer approved after 2 rounds, PR #186 opened; status Complete |
