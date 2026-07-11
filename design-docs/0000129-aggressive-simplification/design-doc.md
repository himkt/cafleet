# Aggressive Simplification: Drop the Administrator, Consolidate CLI, Docs, and Skills

**Status**: Approved
**Progress**: 49/52 tasks complete
**Last Updated**: 2026-07-11

## Overview

Shrink the repository along five axes without losing fundamental features: delete the built-in Administrator member, remove or consolidate low-value CLI surface (`member nudge`, the three `member list` variants, the `--tail` alias, the unused `wait_agent_status` capability, the `client_command` indirection), fold thin documentation stubs, merge micro reference pages inside the skills, and slim the test suite accordingly. The admin WebUI **stays** (mid-execution user decision — see Background); the already-landed WebUI doc/spec sweeps are reverted by Step 5. Breaking changes are acceptable; every change is a hard break with no deprecation residue, and all documentation/skills update in the same cycle.

## Success Criteria

- [x] The admin WebUI is fully intact: `admin/`, `cafleet/src/cafleet/webui/`, `cafleet/tests/webui/`, and `cafleet/src/cafleet/cli/server.py` exist; `cafleet server --help` exits 0; `fastapi` and `uvicorn` remain in `cafleet/pyproject.toml` and `uv.lock`; `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` remain as settings; the WebUI docs (`docs/how-to/use-the-webui.md`, `docs/spec/webui-api.md`, SPEC.md §WebUI, README pitch/bullet) are restored
- [x] `cafleet fleet create` seeds exactly one built-in member (the root Director) and its output carries no administrator field; the member `kind` taxonomy is exactly `director` / `monitor` / `member`; the WebUI sends as the root Director (per § B *WebUI sender model*) and its rebuilt bundle carries no administrator branch
- [ ] The default `db_path` is `cafleet_v4.db` (legacy `cafleet_v3` databases are abandoned wholesale — no data migration); `db/alembic/versions/` contains exactly one fresh initial migration `0001` with `down_revision = None`, regenerated via `mise //cafleet:makemigration` after all implementation finished; the chain-guard test in `tests/db/test_alembic_smoke.py` asserts it; no live surface mentions `cafleet_v3`
- [x] `cafleet member nudge` no longer exists; re-engagement is documented as `cafleet message send`
- [x] `cafleet member list` has a single output shape (no `--activity` / `--all` flags, both fail with Click's `No such option`, exit 2): every active registry entry with `kind` and `idle` columns; the WebUI `/members` endpoint still serves roster rows (`description` / `status` / `registered_at`) via the retained `broker.list_roster`
- [x] `cafleet member capture --tail` fails with `No such option` (exit 2); `--lines` works unchanged
- [x] `wait_agent_status` is gone from the `AgentStateAware` protocol and the herdr backend; `agent_status` is untouched
- [x] The `client_command` decorator is gone; all six `message` subcommands are plain functions with unchanged CLI behavior
- [x] No live surface (source, tests, `README.md`, `SPEC.md`, `docs/`, `skills/`, `.claude/`) mentions the Administrator, `member nudge`, `member list --activity` / `--all`, or `capture --tail` — including the restored WebUI pages, which are re-swept for those mentions after restoration
- [x] Skill micro-pages are merged with no content loss: `skills/cafleet/reference/{output-flags,broadcast}.md` → `reference/cli.md`; `skills/cafleet-design-doc/reference/template.md` → `reference/guidelines.md`; `skills/cafleet-research/reference/slidev/techniques/*.md` → `reference/slidev.md`; no dangling relative links remain in `skills/` or `docs/`
- [x] `docs/concepts/multiplexer-backends.md` is folded into `docs/spec/multiplexer-backends.md`; the zensical nav has no removed pages
- [x] SPEC.md, `docs/spec/`, and `docs/api/` all remain as surfaces (per user decision) and are content-accurate after the removals and the WebUI restoration
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //:docs-build` pass

---

## Background

The baseline for this design is the repository state **after design 0000128** (`--json` relocated to a per-subcommand trailing flag) is implemented; nothing here re-plans that work, and code references below name symbols rather than line numbers where the 0000128 diff would shift them.

A repository survey (2026-07-11) measured: documentation ≈ 12,900 Markdown lines vs ≈ 8,850 non-test source lines (Python 6,900 + admin frontend 2,150), with tests at ≈ 17,800 lines (2.6× source). The user confirmed the following scope: drop the built-in Administrator; keep herdr, all three coding-agent backends, and the overlay machinery; keep SPEC.md, `docs/spec/`, and `docs/api/` as separate surfaces; restructure the rest of `docs/` aggressively; flatten skills without losing instruction content ("keep the instruction sufficient even for haiku"); any CLI shape may break with docs/skills updated in lockstep; test slimming is in scope with the bar "current behavior stays covered".

**Scope revision (2026-07-11, mid-execution)**: the original design also removed the admin WebUI subsystem. After Steps 1–4 landed (docs/SPEC/README/skills sweeps) and the Step 5 code deletion was staged, the user decided the WebUI is needed. The staged code deletion was reverted in the working tree (nothing WebUI-code-related was committed); the WebUI doc/spec/README sweeps that landed in the Step 1–4 commits are reverted by the revised Step 5 below. Everything WebUI-adjacent that the original design deleted as "dead once webui/api.py is gone" (broker `queries.py`, `get_member_names`, `list_monitor_configs`, `list_roster`) stays alive: `webui/api.py` remains their consumer.

Approximate removal impact: Administrator ≈ 60 source lines + a dedicated test file; CLI consolidation ≈ 250 source lines; docs/skills consolidation removes 1 doc stub page, 6 skill micro-pages, and every mention of the removed features.

---

## Specification

### A. The admin WebUI stays (scope revision)

The WebUI subsystem (`admin/` SPA, `cafleet/src/cafleet/webui/`, `cafleet server`, `broker_host` / `broker_port` settings, `fastapi` + `uvicorn` deps, `tests/webui/`) is retained unchanged. Its broker dependencies remain live: `broker/queries.py` (`list_inbox` / `list_sent` / `list_timeline` / `get_task`), `broker/members.py` `get_member_names` and `list_roster(include_task_holders=...)`, and `broker/monitor.py` `list_monitor_configs` all keep their current homes and signatures.

The WebUI doc/spec surfaces removed by the already-landed Step 1–4 commits are restored by Step 5:

| Surface | Restoration |
|---|---|
| `docs/how-to/use-the-webui.md`, `docs/spec/webui-api.md` | restore from git history (`git checkout main -- <path>`); restore both `zensical.toml` nav rows |
| WebUI/server mentions in `docs/` (`index.md`, `how-to/index.md`, `how-to/design-doc-development.md`, `concepts/{storage,overview,monitoring}.md`, `get-started/{install,contributing}.md`, `api/broker.md`, `spec/cli-options.md` `server` section, `get-started/configure.md` `CAFLEET_BROKER_HOST/PORT` rows) | restore from the pre-Step-1 state of each file |
| SPEC.md WebUI module spec (§6.8), `server` command surface, `broker_host` / `broker_port` rows, other WebUI mentions | restore from the pre-Step-2 state |
| README pitch ("and an admin WebUI") + Specification bullet ("WebUI API"); CLAUDE.md tech-stack line (FastAPI); `.claude/rules/commands.md` WebUI/admin bullets; `.claude/settings.json` `Bash(mise //admin*)` allow row | restore from the pre-Step-3 state |
| `broadcast_summary` task rows | unchanged (part of broadcast semantics) |

**Re-sweep constraint**: restored content must not reintroduce the *removed* features — after restoring each file, re-apply the Administrator / `member nudge` / `--activity` / `--all` / `--tail` sweeps within the restored text (e.g. if a restored WebUI page mentions the Administrator, that sentence is updated, not restored verbatim). The skills' "broker timeline" phrasing (already landed) stays — it is accurate with or without the WebUI.

### B. Remove the built-in Administrator member

The Administrator is seeded per fleet, has no pane, is excluded from monitoring and broadcasts, cannot be deregistered, and is referenced by no command or documented CLI workflow. It is identified by card kind (`$.cafleet.kind` = `builtin-administrator`), not by a schema column, so removal is code + a data migration; no schema change.

**WebUI sender model (user decision, mid-execution)**: the WebUI frontend previously sent messages AS the Administrator (`admin/src/Dashboard.tsx` selected the `administrator`-kind member as `sender_id` and disabled Send when absent). Post-removal, the WebUI sends as the fleet's **root Director**: Dashboard selects the `director`-kind member as sender (Send disabled only if no active Director exists — a state no real fleet reaches); the recipient dropdown excludes the sender (unchanged semantics, now keyed off the Director); the `administrator` literal disappears from `types.ts`'s kind union and from every component branch (`MessageInput`, `MemberAvatar`, `Sidebar`, `MemberDetail`, `AppHeader` if referenced). Rebuild the bundle via `mise //admin:build` so `webui/dist` matches. `webui/api.py` needs no change (it has no Administrator reference).

| Site | Change |
|---|---|
| `broker/fleets.py` `create_fleet` | delete the administrator card/row block; drop `administrator_member_id` from the return dict and docstring |
| `output/formatters.py` `format_fleet_create` | compact form becomes `<fleet_id> director=<member_id>`; the full block loses the `administrator:` line (6 lines) |
| `broker/members.py` `deregister_member` | delete the `is_administrator` guard and its error string `Administrator cannot be deregistered` |
| `broker/members.py` `register_member` | the heartbeat-enrollment exclusion collapses to `if kind != _shared.MONITORING_MEMBER_KIND` |
| `broker/messaging.py` broadcast recipient query | drop the `CARD_KIND_SQL != ADMINISTRATOR_KIND` condition — a broadcast reaches every active member except the sender |
| `broker/_shared.py` | delete `ADMINISTRATOR_KIND` and `is_administrator`; delete `_card_kind` if it has no remaining caller; `derive_member_kind` collapses to the 3-value taxonomy `director` / `monitor` / `member` |
| `broker/__init__.py` | drop the `ADMINISTRATOR_KIND` export |

**Fresh database + regenerated initial migration (user decision, mid-execution — supersedes the earlier migration-`0002` data migration)**: legacy `builtin-administrator` rows are abandoned wholesale instead of migrated. The default database file name bumps in `config.py` (`db_path`: `cafleet_v3.db` → `cafleet_v4.db`, field + docstring), so every install starts a **fresh, empty** database (no data is copied from v3 — user decision); the old `cafleet_v3.db` file is simply left behind. All existing migration scripts under `cafleet/src/cafleet/db/alembic/versions/` are deleted (including the superseded `0002` generated earlier this cycle), and **after all implementation finishes** (Step 9) a single fresh initial migration is regenerated against the final models: `cafleet setup db` (creates the fresh `cafleet_v4` DB at empty head) → `mise //cafleet:makemigration "initial schema"` (mints `0001`, `down_revision = None`) → `cafleet setup db` (applies it). The chain-guard test keeps asserting the single fresh initial revision `0001`. The filename sweep covers `tests/db/test_init.py` (default-URL test), `docs/concepts/storage.md`, `docs/get-started/install.md`, `docs/reference/coding-agents/codex.md`, `skills/cafleet/reference/cli.md`, and the SPEC.md configuration rows.

**Execution sequencing (live-broker constraint)**: `cafleet` is installed editable from this repo, so the `config.py` bump takes effect for the running implementation fleet the moment it is saved — pointing every member's broker at the empty v4 DB and severing team communication (observed live; the bump was reverted to restore comms). The bump therefore lands **last — after the Reviewer loop and the user approval, applied Director-side once the implementation fleet is torn down** (review-first ordering, user decision): the Reviewer reviews the branch on the current fleet with the bump still pending (the design doc documents the 2-line residue — the `config.py` bump and its `test_init.py` green-up — as the sole post-review change), then teardown → bump → § B regeneration/verification gates → final commit. `tests/db/test_init.py` (expecting v4) stays red until that moment — an accepted extension of the Steps 6–8 red window.

Legacy administrator rows had no placement and no monitor enrollment, so no other table needs touching. The chain-guard test per `.claude/rules/database-migrations.md` keeps asserting the single fresh initial revision `0001` with `down_revision = None` (regenerated at Step 9).

### C. CLI and code consolidation

#### C1. Remove `member nudge`

`member nudge` persists and notifies through `broker.send_message` — the identical persistence + Esc-safeguarded inline-preview path as `message send`. The wrapper-level deltas it adds are dropped as part of this breaking change: the live-multiplexer requirement (`ensure_multiplexer_or_die`, exit 1 without one — the replacement `message send` succeeds even when no multiplexer is running, since the preview is best-effort), the target pre-resolution via `_load_authorized_member` (`send_message`'s own destination check still rejects a cross-fleet or inactive target), the absence of `--full` / `--quiet` and truncation/render, and the bespoke JSON shape `{member_id, pane_id, task_id, notification_sent}` (replaced by `message send`'s task envelope). None of these deltas is load-bearing for re-engagement. Delete the subcommand (`cli/member.py` `member_nudge`). The documented replacement everywhere is:

```bash
cafleet message send --fleet-id <fleet-id> --from-member-id <sender> --to-member-id <target> --text "..."
```

Operator-side migration (recorded here because live docs must not mention the removed command): any `Bash(cafleet member nudge ...)` patterns in user-level `~/.claude/settings.json` become dead and can be deleted; `message send` is already covered by the existing per-subcommand allow pattern.

#### C2. Single `member list` shape

Replace the three CLI variants (bare / `--activity` / `--all`) and their CLI-side broker queries + three formatters with one:

- **Flags**: only `--fleet-id` (and the shared trailing `--json`). `--activity` and `--all` are deleted; the mutual-exclusion error disappears with them.
- **Rows**: every **active** registry entry of the fleet (current `--all` semantics: root Director, monitoring member, ordinary members, placementless rows). Because the row set is active-only, no output shape carries a `status` column — it would be the constant `"active"`.
- **Text columns**: `member_id`, `name`, `kind`, `backend`, `pane_id`, `idle`. Placementless rows render `-` in placement cells; a placed row with no pane renders `(pending)`; `idle` is humanized `Ns`/`Nm`/`Nh` (`-` when no activity), keeping today's `--activity` aggregation semantics (seconds since the member's most recent task activity).
- **JSON**: one dict per row with `member_id`, `name`, `kind`, `placement` (sub-dict or `null`), and the activity fields `last_sent` / `last_recv` / `last_ack` / `idle`. The `description` and `registered_at` fields of today's roster dump are intentionally dropped from the CLI list shape — per-member detail remains available via `member show`.
- **Broker**: one function `list_members(fleet_id)` producing exactly the rows above; delete `list_members_with_activity` and the old placed-members-only `list_members` body it replaces. **`list_roster` stays** — it is the WebUI `/members` provider (scope revision; the WebUI frontend renders `description` / `status` / `registered_at` from roster rows) — and keeps its `include_task_holders` parameter.
- **Output**: one `format_member_list`; delete `format_member_list_activity`. **`format_member_roster` stays only if a live caller remains after the CLI rewrite**; if the roster formatter's sole consumer was the CLI `--all` path (the WebUI serializes roster dicts directly), delete it.

The supervision workflows keep their signal: the stall heuristic reads the `idle` column from the single output (`skills/cafleet/reference/recovery.md`, `reference/director.md` § Member List, `docs/how-to/monitor-and-recover.md` rewrite their `--activity` invocations to bare `member list` — already landed in Steps 1–4).

#### C3. `member capture` alias

Delete the `--tail` alias; `--lines` remains the only spelling. `--ansi/--no-ansi` is unchanged.

#### C4. Remove `wait_agent_status`

Delete the method from the `AgentStateAware` protocol (`multiplexer/base.py`) and its herdr implementation (`multiplexer/herdr.py`). It has no caller in src; `agent_status` (consumed by `monitor/loop.py`) is untouched.

#### C5. Inline `client_command`

Delete the `client_command` decorator (`cli/_helpers.py`) and its per-command lambda parameters in `cli/message.py`. Each of the six `message` subcommands becomes a plain function — matching the style of every other CLI group — composed from the surviving shared pieces (`fleet_id_option`, member-id options, `verify_member_fleet` guard, `truncate_task_text` / `render_tasks_in_result`, `json_flag` / `full_flag` / `quiet_flag`, the formatters). CLI behavior (flags, output, error strings, exit codes) is byte-identical; `tests/cli/test_client_command.py` is deleted and any unique coverage moves into the message suites.

### D. Documentation restructure

Per the user decision, `SPEC.md`, `docs/spec/`, and `docs/api/` all remain as surfaces. This axis folds one thin stub and (post scope-revision) restores the WebUI pages via Step 5:

- Fold `docs/concepts/multiplexer-backends.md` (34 lines, mostly pointers) into `docs/spec/multiplexer-backends.md` as an introductory section; delete the concepts page and its nav row; retarget inbound links (`docs/index.md`, `docs/concepts/overview.md`, any skill links). *(Landed in Step 1.)*
- Sweep every Administrator / nudge / `--activity` / `--all` / `--tail` mention across `docs/`. *(Landed in Step 1.)*
- Restore the WebUI doc surfaces per § A. *(Step 5.)*

### E. Skills consolidation (content-preserving)

Guiding constraint (user): flattening must not lose instructions — every protocol, command shape, and caveat that exists today survives in some loadable page; only true duplication and removed-feature content is dropped. The overlay machinery and all three backend overlays stay (backends are kept).

| Merge | Result |
|---|---|
| `skills/cafleet/reference/output-flags.md` (20) + `reference/broadcast.md` (28) → `reference/cli.md` | one CLI reference page with *Output flags* and *Broadcast* sections; both source files deleted; every inbound link (SKILL.md load-bearing/on-demand tables, role files, other skills) retargeted |
| `skills/cafleet-design-doc/reference/template.md` (49) → `reference/guidelines.md` | guidelines opens with the template block; template.md deleted; inbound links retargeted (`SKILL.md`'s On-demand reference table and the three workflow bodies `create/create.md` / `interview/interview.md` / `execute/execute.md`; the role files name the template only in prose and need no link edit) |
| `skills/cafleet-research/reference/slidev/techniques/{formatting,math-formulas,two-column-layouts}.md` (254) → `reference/slidev.md` | technique content becomes sections of slidev.md, verbatim; the `techniques/` directory is deleted; inbound links retargeted |

Plus sweeps in all skills (including `.claude/skills/skill-author/SKILL.md`): `member nudge` → `message send` in the Director primitive lists and stall ladders; the `member list` flag rewrites; the kind taxonomy; the `coordination.md` "admin WebUI timeline" phrasing → "broker timeline" (kept post-revision — accurate either way). *(All landed in Step 4.)*

### F. Test slimming

| Action | Targets |
|---|---|
| Delete (removed features) | `tests/broker/test_administrator.py`; every `member nudge` test (dedicated files, cases inside `tests/cli/test_member*.py`, and the nudge entry in `tests/cli/test_text_input.py`'s shared text-body command matrix); `tests/cli/test_client_command.py` |
| Delete (removed-surface meta-tests, per user decision) | `tests/cli/test_agent_flags_removed.py`, `tests/cli/test_agent_group_removed.py`, `tests/cli/test_db_group_removed.py`, and the pre-subcommand `--json` guard added by design 0000128 (wherever it landed). No new absence-guard tests are added — the absence is the test |
| Keep (scope revision) | `tests/webui/` and `tests/cli/test_server.py` stay — the WebUI and `cafleet server` remain |
| Consolidate | `tests/cli/test_member_list_activity.py` + `tests/cli/test_member_list_all.py` + the list cases in `test_member.py` → one `tests/cli/test_member_list.py` covering the single output shape (text columns, JSON fields, placementless/pending rendering, idle aggregation) |
| Update | fleet bootstrap / kind / messaging / monitor tests (Administrator removal, broadcast recipients, 3-value kind); capture tests (`--tail` gone); herdr/multiplexer tests and the fake backend's `wait_agent_status` stub in `tests/monitor/test_loop.py` (`wait_agent_status` gone); `tests/db/test_alembic_smoke.py` chain guard (2 revisions); `fleet create` output tests (no admin field). `tests/broker/test_typed_columns.py` keeps its `list_inbox` / `list_sent` / `list_timeline` / `get_task` cases (queries.py stays) |
| Keep | `test_unhidden_flags.py`, `test_help_budget.py` (current-behavior guards) and all remaining behavior suites |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation lands first, per `.claude/rules/documentation-maintenance.md`. Prerequisite: design 0000128 is implemented and merged.

Steps 1–4 landed under the original (pre-revision) scope; their checked tasks are the historical record of that execution, and the revised Step 5 reverts exactly their WebUI-related portions. Steps 6–7 form one atomic implementation block with Step 8: each code deletion in Steps 6–7 breaks tests that are only removed or updated in Step 8, so the suite is expected red between Steps 6 and 8 and is verified green only at Step 9. When committing per step, land each of Steps 6–7 together with its Step 8 test edits in the same commit (Step 8 then only carries whatever test work remains).

### Step 1: `docs/` and site nav

- [x] Delete `docs/how-to/use-the-webui.md` and `docs/spec/webui-api.md`; remove both nav rows from `zensical.toml` <!-- completed: 2026-07-11T10:05 --> *(reverted by Step 5 per scope revision)*
- [x] Fold `docs/concepts/multiplexer-backends.md` into `docs/spec/multiplexer-backends.md` (intro section); delete the page and its nav row; retarget inbound links in `docs/index.md` and `docs/concepts/overview.md` <!-- completed: 2026-07-11T10:05 -->
- [x] Sweep WebUI/server mentions: `docs/index.md`, `docs/how-to/index.md`, `docs/how-to/design-doc-development.md`, `docs/concepts/{storage,overview,monitoring}.md`, `docs/get-started/{install,contributing}.md`, `docs/api/broker.md`, `docs/spec/cli-options.md` (delete the `server` command section) <!-- completed: 2026-07-11T09:58 --> *(reverted by Step 5 per scope revision)*
- [x] Sweep Administrator mentions (3-value kind, broadcast recipients, `fleet create` output): `docs/spec/{data-model,cli-options}.md`, `docs/concepts/{fleet-isolation,overview,member-lifecycle,monitoring}.md`, `docs/how-to/mixed-backend-team.md`, `docs/get-started/{quickstart,contributing}.md` <!-- completed: 2026-07-11T09:58 -->
- [x] Sweep `member nudge` → `message send`: `docs/spec/{cli-options,multiplexer-backends}.md`, `docs/concepts/{overview,member-lifecycle,monitoring}.md`, `docs/get-started/configure.md` <!-- completed: 2026-07-11T09:58 -->
- [x] Rewrite `member list` docs to the single shape (columns, JSON fields, no flags): `docs/spec/cli-options.md` (flag table, output sections, error table row), `docs/concepts/member-lifecycle.md`, `docs/how-to/{mixed-backend-team,monitor-and-recover}.md`, `docs/get-started/quickstart.md` <!-- completed: 2026-07-11T09:58 -->
- [x] `docs/spec/cli-options.md`: delete the `--tail` alias from the `member capture` flag table; delete the `member nudge` section and its `permissions.allow` row <!-- completed: 2026-07-11T09:58 -->
- [x] `docs/get-started/configure.md`: delete the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` rows <!-- completed: 2026-07-11T09:58 --> *(reverted by Step 5 per scope revision)*

### Step 2: `SPEC.md`

- [x] Delete the WebUI module spec (§6.8), the `server` command surface, and the `broker_host` / `broker_port` configuration rows; prune WebUI mentions elsewhere (e.g. `max_text_len` notes, checklists) <!-- completed: 2026-07-11T10:20 --> *(reverted by Step 5 per scope revision)*
- [x] Remove the Administrator: fleet bootstrap (single built-in Director), `fleet create` output shapes, kind taxonomy (3 values), broadcast recipient rule, deregister guard error string <!-- completed: 2026-07-11T10:20 -->
- [x] Remove `member nudge` (command surface, checklist rows, member-lifecycle sentence); rewrite `member list` to the single shape; drop the `--tail` alias; drop `wait_agent_status` from the multiplexer capability spec; update the `message` subcommand spec if it names `client_command` internals <!-- completed: 2026-07-11T10:20 -->
- [x] Update the broker module spec: `queries.py` gone (`get_task` lives in `messaging.py`), `get_member_names` / `list_monitor_configs` / `list_roster` / `list_members_with_activity` gone, single `list_members` documented <!-- completed: 2026-07-11T10:20 --> *(partially reverted by Step 5: queries.py/get_member_names/list_monitor_configs/list_roster stay; only `list_members_with_activity` goes and the single CLI `list_members` is documented)*

### Step 3: README, CLAUDE.md, rules

- [x] Run `/update-readme`: pitch drops "and an admin WebUI"; Specification bullet drops "WebUI API" (SPEC.md edits from Step 2 are its other input) <!-- completed: 2026-07-11T10:25 --> *(reverted by Step 5 per scope revision)*
- [x] `CLAUDE.md`: update the tech-stack line (drop FastAPI/server) and the CLI blurb if it mentions the server <!-- completed: 2026-07-11T10:25 --> *(reverted by Step 5 per scope revision)*
- [x] `.claude/rules/commands.md`: delete the WebUI server bullets (`cafleet server`, `mise //cafleet:dev`), `mise //admin:dev` / `//admin:build` / `//admin:lint` rows, and the admin steps in the publish description <!-- completed: 2026-07-11T10:32 --> *(reverted by Step 5 per scope revision)*
- [x] `.claude/rules/bash-tool.md`: sweep the `member nudge` mention (Director-side primitives) <!-- completed: 2026-07-11T10:25 -->
- [x] `.claude/settings.json`: delete the `Bash(mise //admin*)` `permissions.allow` row (dead once `admin/` and its mise tasks are gone) <!-- completed: 2026-07-11T10:32 --> *(reverted by Step 5 per scope revision)*

### Step 4: skills

- [x] `skills/cafleet`: merge `reference/output-flags.md` + `reference/broadcast.md` into `reference/cli.md`; delete both files; retarget every inbound link (SKILL.md tables, role files, `reference/*.md`, other skills, `docs/` if any) <!-- completed: 2026-07-11T11:00 -->
- [x] `skills/cafleet`: sweep nudge → `message send` in `SKILL.md`, `roles/{director,monitor}.md`, `reference/{supervision,cli,exec-routing,recovery,director}.md`; rewrite `reference/director.md` § Member List and `reference/recovery.md` heuristics to the single `member list` output; sweep Administrator from `reference/cli.md` roster description <!-- completed: 2026-07-11T10:55 -->
- [x] `skills/cafleet-design-doc`: merge `reference/template.md` into `reference/guidelines.md`; delete the file; retarget inbound links (`SKILL.md` On-demand table, `create/create.md`, `interview/interview.md`, `execute/execute.md`); sweep nudge/WebUI/Administrator mentions (`reference/coordination.md` "admin WebUI timeline" → "broker timeline") <!-- completed: 2026-07-11T11:00 -->
- [x] `skills/cafleet-research`: merge `reference/slidev/techniques/*.md` into `reference/slidev.md`; delete the directory; retarget inbound links; sweep nudge mentions in `report/report.md`, `presentation/presentation.md`, both `roles/director.md` <!-- completed: 2026-07-11T11:00 -->
- [x] `.claude/skills/skill-author/SKILL.md`: sweep removed features (nudge, Administrator, member-list flags) <!-- completed: 2026-07-11T11:00 -->
- [x] Verify no dangling relative links remain in `skills/` and `docs/` (grep for the deleted filenames) <!-- completed: 2026-07-11T11:00 -->

### Step 5: restore the WebUI surfaces (scope revision)


The Step 5 code deletion staged under the original scope was fully reverted in the working tree (admin/, webui/, server.py, queries.py, config, pyproject, mise tasks, uv.lock — nothing was committed). This step reverts the WebUI-related **documentation** edits that DID land in the Step 1–3 commits, with the § A re-sweep constraint (restored text must not reintroduce Administrator / nudge / `--activity` / `--all` / `--tail` mentions).


- [x] Restore `docs/how-to/use-the-webui.md` and `docs/spec/webui-api.md` from git history; restore both `zensical.toml` nav rows; re-sweep the restored pages per § A <!-- completed: 2026-07-11T11:35 -->
- [x] Restore the WebUI/server mentions swept from `docs/` (file list in § A); restore the `server` command section in `docs/spec/cli-options.md` and the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` rows in `docs/get-started/configure.md` <!-- completed: 2026-07-11T11:35 -->
- [x] Restore SPEC.md: WebUI module spec (§6.8), `server` command surface, `broker_host` / `broker_port` configuration rows, and the other pruned WebUI mentions; keep the broker-module section consistent with § C2 (queries.py / get_member_names / list_monitor_configs / list_roster stay; `list_members_with_activity` goes; single CLI `list_members` documented) <!-- completed: 2026-07-11T11:35 -->
- [x] Restore README (pitch + Specification bullet via `/update-readme`), CLAUDE.md tech-stack line, `.claude/rules/commands.md` WebUI/admin bullets, and the `.claude/settings.json` `Bash(mise //admin*)` allow row <!-- completed: 2026-07-11T11:45 -->
- [x] `mise //:docs-build` passes with the restored nav; grep confirms the restored files carry no Administrator / nudge / `--activity` / `--all` / `--tail` mentions <!-- completed: 2026-07-11T11:45 -->

### Step 6: code — Administrator removal

- [x] `broker/fleets.py`: delete the administrator seeding block and the `administrator_member_id` return field; update the docstring <!-- completed: 2026-07-11T11:55 -->
- [x] `broker/members.py`: delete the deregister guard; collapse the enrollment exclusion to the monitoring-member check (`list_roster` and its `include_task_holders` parameter stay — WebUI provider) <!-- completed: 2026-07-11T11:55 -->
- [x] `broker/messaging.py`: drop the administrator condition from the broadcast recipient query <!-- completed: 2026-07-11T11:55 -->
- [x] `broker/_shared.py`: delete `ADMINISTRATOR_KIND` / `is_administrator` (and `_card_kind` if uncalled); collapse `derive_member_kind` to 3 values; `broker/__init__.py`: drop the export <!-- completed: 2026-07-11T11:55 -->
- [x] `output/formatters.py` `format_fleet_create`: compact `<fleet_id> director=<member_id>`; full block loses the administrator line <!-- completed: 2026-07-11T11:55 -->
- [x] Rework the WebUI sender model per § B: `admin/src/{Dashboard,MessageInput,MemberAvatar,Sidebar,MemberDetail}.tsx` + `types.ts` switch from the administrator to the director kind; rebuild via `mise //admin:build` <!-- completed: 2026-07-11T11:55 -->
- [x] Sweep the DB filename to `cafleet_v4.db` in `docs/concepts/storage.md`, `docs/get-started/install.md`, `docs/reference/coding-agents/codex.md`, `skills/cafleet/reference/cli.md`, SPEC.md configuration rows (the `config.py` bump itself is deferred to Step 9 per § B *Execution sequencing*) <!-- completed: 2026-07-11T12:15 -->
- [x] Delete every existing migration script under `cafleet/src/cafleet/db/alembic/versions/` (including the superseded `0002` generated earlier this cycle); the fresh initial `0001` is regenerated at Step 9 per § B <!-- completed: 2026-07-11T12:09 -->
- [x] Chain-guard test in `tests/db/test_alembic_smoke.py` keeps asserting the single fresh initial revision `0001` with `down_revision = None`; `tests/db/test_init.py` default-URL test moves to `cafleet_v4.db` <!-- completed: 2026-07-11T12:09 -->

### Step 7: code — CLI consolidation

- [x] Delete `member_nudge` from `cli/member.py` <!-- completed: 2026-07-11T12:35 -->
- [x] Broker: implement the single `list_members(fleet_id)` (all active registry entries + kind + placement + activity fields); delete `list_members_with_activity`; keep `list_roster` (WebUI provider); update `broker/__init__.py` <!-- completed: 2026-07-11T12:35 -->
- [x] `cli/member.py` `member list`: drop `--activity` / `--all` and the mutual-exclusion check; emit the single shape <!-- completed: 2026-07-11T12:35 -->
- [x] `output/formatters.py`: one `format_member_list` (columns per § C2); delete `format_member_list_activity` (and `format_member_roster` only if the CLI was its last caller) <!-- completed: 2026-07-11T12:35 -->
- [x] `cli/member.py` `member capture`: drop the `--tail` alias <!-- completed: 2026-07-11T12:35 -->
- [x] `multiplexer/base.py` + `multiplexer/herdr.py`: delete `wait_agent_status` <!-- completed: 2026-07-11T12:35 -->
- [x] Inline `client_command`: delete the decorator from `cli/_helpers.py`; rewrite the six `message` subcommands as plain functions with byte-identical CLI behavior <!-- completed: 2026-07-11T12:35 -->

### Step 8: tests

- [x] Delete `tests/broker/test_administrator.py`, all nudge tests (including the entry in `tests/cli/test_text_input.py`'s shared command matrix), `tests/cli/test_client_command.py` (unique coverage moves into the message suites); `tests/webui/` and `tests/cli/test_server.py` stay per scope revision <!-- completed: 2026-07-11T12:09 -->
- [x] Delete `tests/cli/test_agent_flags_removed.py`, `test_agent_group_removed.py`, `test_db_group_removed.py`, and the 0000128 pre-subcommand `--json` guard <!-- completed: 2026-07-11T12:09 -->
- [x] Consolidate the member-list suites into `tests/cli/test_member_list.py` covering the single shape (text columns, JSON fields, placementless/pending, idle) <!-- completed: 2026-07-11T12:09 -->
- [x] Update the remaining suites: fleet bootstrap/kind/messaging/monitor (Administrator removal, broadcast recipients, 3-value kind), capture (`--tail`), herdr + the `tests/monitor/test_loop.py` fake-backend stub (`wait_agent_status`), fleet-create output; `tests/broker/test_typed_columns.py` keeps its queries.py cases; `tests/webui/` cases asserting the administrator sender/kind move to the Director sender model <!-- completed: 2026-07-11T12:09 -->

### Step 9: verification

- [ ] Finalize the DB bump per § B *Execution sequencing* (Director-side, after the implementation fleet teardown): apply the `config.py` `db_path` bump to `cafleet_v4.db`, then `cafleet setup db` (fresh v4) → `mise //cafleet:makemigration "initial schema"` (fresh `0001`, `down_revision = None`) → `cafleet setup db` (applies it); chain-guard and `test_init.py` pass <!-- completed: -->
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` pass <!-- completed: -->
- [x] `mise //:docs-build` passes (nav has no dead pages; WebUI pages restored) <!-- completed: 2026-07-11T12:09 -->
- [x] `mise //cafleet:build` succeeds; `cafleet server --help` exits 0 (WebUI intact) <!-- completed: 2026-07-11T12:09 -->
- [ ] Full-repo grep confirms no live mention of: Administrator, `member nudge`, `--activity`, `member list --all`, `--tail`, `wait_agent_status`, `client_command`, `cafleet_v3` (historical `design-docs/**` excluded); WebUI mentions are present again where restored <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-11 | Initial draft |
| 2026-07-11 | Reviewer round 1: accurate `member nudge` delta rationale; `member list` drops the constant `status` column and records the `description`/`registered_at` JSON drop; corrected `template.md` inbound links; test enumeration extended (`test_server.py`, `test_text_input.py`, `test_typed_columns.py`, `test_loop.py` stub); Steps 5–8 declared one atomic block; added `.claude/settings.json` `mise //admin*` cleanup; dropped the no-op `update-readme` sweep |
| 2026-07-11 | **Scope revision (user, mid-execution)**: the admin WebUI stays. Axis A becomes a restoration spec (Step 5 reverts the landed WebUI doc/spec/README sweeps; the staged WebUI code deletion was reverted uncommitted). Broker WebUI dependencies (`queries.py`, `get_member_names`, `list_monitor_configs`, `list_roster`) stay; C2 keeps `list_roster` as the WebUI provider; F keeps `tests/webui/` and `test_server.py`. Success criteria and Steps 5/8/9 rewritten accordingly; task total 50 → 49 |
| 2026-07-11 | **Arbitration (user)**: post-Administrator WebUI sender model = the root Director. § B gains the *WebUI sender model* spec (frontend rework + `mise //admin:build` rebuild as a new Step 6 task, `tests/webui/` sender-model update in Step 8); task total 49 → 50 |
| 2026-07-11 | **Migration approach revision (user, mid-execution)**: fresh database instead of a data migration — `db_path` bumps `cafleet_v3.db` → `cafleet_v4.db`, all existing migration scripts are deleted (incl. the superseded `0002`), and a single fresh initial `0001` is regenerated via `mise //cafleet:makemigration` after all implementation (Step 9). § B, SC #3, Step 6, and Step 9 rewritten; task total 50 → 52 |
| 2026-07-11 | **Sequencing arbitration (user: "create from fresh")**: no v3→v4 data copy. The editable install makes the `config.py` bump instantly fatal to the live implementation fleet's broker, so the bump moves out of Step 6 into Step 9 (Director-side, post-teardown); § B gains *Execution sequencing*; `test_init.py` red window accepted through Step 9; the Reviewer loop runs on a new fleet against the fresh v4 DB |
| 2026-07-11 | **Endgame re-sequencing (user)**: review-first, bump-last — the Reviewer loop runs on the current implementation fleet with the `config.py` bump still pending; the bump + regeneration gates land after Reviewer and user approval, right before finalize |
