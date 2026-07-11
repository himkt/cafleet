# Aggressive Simplification: Drop the WebUI and Administrator, Consolidate CLI, Docs, and Skills

**Status**: Approved
**Progress**: 8/50 tasks complete
**Last Updated**: 2026-07-11

## Overview

Shrink the repository along six axes without losing fundamental features: delete the admin WebUI subsystem, delete the built-in Administrator member, remove or consolidate low-value CLI surface (`member nudge`, the three `member list` variants, the `--tail` alias, the unused `wait_agent_status` capability, the `client_command` indirection), fold thin documentation stubs, merge micro reference pages inside the skills, and slim the test suite accordingly. Breaking changes are acceptable; every change is a hard break with no deprecation residue, and all documentation/skills update in the same cycle.

## Success Criteria

- [ ] `cafleet server` no longer exists (`No such command 'server'`, exit 2); `admin/`, `cafleet/src/cafleet/webui/`, and `cafleet/tests/webui/` are deleted; `fastapi` and `uvicorn` are gone from `cafleet/pyproject.toml` and `uv.lock`; the built wheel contains no `webui/` files
- [ ] `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` no longer exist as settings; `cafleet fleet create` seeds exactly one built-in member (the root Director) and its output carries no administrator field; the member `kind` taxonomy is exactly `director` / `monitor` / `member`
- [ ] Alembic migration `0002` deregisters legacy `builtin-administrator` rows; the chain is linear `0001 → 0002` and the chain-guard test in `tests/db/test_alembic_smoke.py` asserts it
- [ ] `cafleet member nudge` no longer exists; re-engagement is documented as `cafleet message send`
- [ ] `cafleet member list` has a single output shape (no `--activity` / `--all` flags, both fail with Click's `No such option`, exit 2): every active registry entry with `kind` and `idle` columns
- [ ] `cafleet member capture --tail` fails with `No such option` (exit 2); `--lines` works unchanged
- [ ] `wait_agent_status` is gone from the `AgentStateAware` protocol and the herdr backend; `agent_status` is untouched
- [ ] The `client_command` decorator is gone; all six `message` subcommands are plain functions with unchanged CLI behavior
- [ ] No live surface (source, tests, `README.md`, `SPEC.md`, `docs/`, `skills/`, `.claude/`) mentions the WebUI, `cafleet server`, the Administrator, `member nudge`, `member list --activity` / `--all`, or `capture --tail`
- [ ] Skill micro-pages are merged with no content loss: `skills/cafleet/reference/{output-flags,broadcast}.md` → `reference/cli.md`; `skills/cafleet-design-doc/reference/template.md` → `reference/guidelines.md`; `skills/cafleet-research/reference/slidev/techniques/*.md` → `reference/slidev.md`; no dangling relative links remain in `skills/` or `docs/`
- [ ] `docs/concepts/multiplexer-backends.md` is folded into `docs/spec/multiplexer-backends.md`; the zensical nav has no removed pages
- [ ] SPEC.md, `docs/spec/`, and `docs/api/` all remain as surfaces (per user decision) and are content-accurate after the removals
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //:docs-build` pass

---

## Background

The baseline for this design is the repository state **after design 0000128** (`--json` relocated to a per-subcommand trailing flag) is implemented; nothing here re-plans that work, and code references below name symbols rather than line numbers where the 0000128 diff would shift them.

A repository survey (2026-07-11) measured: documentation ≈ 12,900 Markdown lines vs ≈ 8,850 non-test source lines (Python 6,900 + admin frontend 2,150), with tests at ≈ 17,800 lines (2.6× source). The user confirmed the following scope: drop the admin WebUI and the built-in Administrator; keep herdr, all three coding-agent backends, and the overlay machinery; keep SPEC.md, `docs/spec/`, and `docs/api/` as separate surfaces; restructure the rest of `docs/` aggressively; flatten skills without losing instruction content ("keep the instruction sufficient even for haiku"); any CLI shape may break with docs/skills updated in lockstep; test slimming is in scope with the bar "current behavior stays covered".

Approximate removal impact: WebUI ≈ 2,900 source lines (admin 2,145 + webui 258 + server/config/broker dead code) + ~500 test lines + the bundled `webui/dist` assets; Administrator ≈ 60 source lines + a dedicated test file; CLI consolidation ≈ 250 source lines; docs/skills consolidation removes 2 doc pages, 6 skill micro-pages, and every mention of the removed features.

---

## Specification

### A. Remove the admin WebUI

Wholesale deletion of the WebUI subsystem. The CLI accesses SQLite directly; no documented member/Director workflow depends on the WebUI.

| Surface | Action |
|---|---|
| `admin/` (React SPA, its `mise.toml`, `bun.lock`, configs) | delete the directory |
| `cafleet/src/cafleet/webui/` (`api.py`, `app.py`, `dist/` bundle) | delete the package |
| `cafleet/src/cafleet/cli/server.py` + `cli.add_command(server)` in `cli/__init__.py` | delete |
| `cafleet/tests/webui/` | delete |
| `config.py` `broker_host` / `broker_port` fields (+ `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`) | delete; prune the class docstring's server references |
| `cafleet/pyproject.toml`: `fastapi`, `uvicorn[standard]` deps; the `src/cafleet/webui/dist/**/*` packaging entries; the `fastapi.*` / `uvicorn` ty overrides | delete; resync `uv.lock` via `mise //:uv-sync` |
| root `mise.toml`: `"admin"` in `[monorepo].config_roots`; the `//admin:build` step in `all-install` | delete |
| `cafleet/mise.toml`: the `dev` task (uvicorn launcher); the `//admin:install` + `//admin:build` steps in `publish` | delete |

**Broker functions that become dead** once `webui/api.py` (their only consumer) is gone:

| Function | Action |
|---|---|
| `broker/queries.py` `list_inbox` / `list_sent` / `list_timeline` | delete; move `get_task` (still used by `message show`) into `broker/messaging.py` and delete `queries.py` |
| `broker/members.py` `get_member_names` | delete |
| `broker/monitor.py` `list_monitor_configs` | delete |
| `broker/members.py` `list_roster(include_task_holders=...)` | the parameter is deleted (the function itself is absorbed by axis C2) |
| `broker/__init__.py` | drop the corresponding re-exports |

`broadcast_summary` task rows are part of broadcast semantics (sender-side record, excluded from poll/pending/activity) and are unchanged.

### B. Remove the built-in Administrator member

The Administrator is seeded per fleet, has no pane, is excluded from monitoring and broadcasts, cannot be deregistered, and is referenced by no command or documented workflow. It is identified by card kind (`$.cafleet.kind` = `builtin-administrator`), not by a schema column, so removal is code + a data migration; no schema change.

| Site | Change |
|---|---|
| `broker/fleets.py` `create_fleet` | delete the administrator card/row block; drop `administrator_member_id` from the return dict and docstring |
| `output/formatters.py` `format_fleet_create` | compact form becomes `<fleet_id> director=<member_id>`; the full block loses the `administrator:` line (6 lines) |
| `broker/members.py` `deregister_member` | delete the `is_administrator` guard and its error string `Administrator cannot be deregistered` |
| `broker/members.py` `register_member` | the heartbeat-enrollment exclusion collapses to `if kind != _shared.MONITORING_MEMBER_KIND` |
| `broker/messaging.py` broadcast recipient query | drop the `CARD_KIND_SQL != ADMINISTRATOR_KIND` condition — a broadcast reaches every active member except the sender |
| `broker/_shared.py` | delete `ADMINISTRATOR_KIND` and `is_administrator`; delete `_card_kind` if it has no remaining caller; `derive_member_kind` collapses to the 3-value taxonomy `director` / `monitor` / `member` |
| `broker/__init__.py` | drop the `ADMINISTRATOR_KIND` export |

**Migration `0002`** (generated via `mise //cafleet:makemigration`, then hand-edited to a pure data migration since there is no schema diff):

```python
def upgrade() -> None:
    now = datetime.now(UTC).isoformat()
    op.execute(
        "UPDATE members SET status='deregistered', deregistered_at='" + now + "' "
        "WHERE status='active' "
        "AND json_extract(member_card_json, '$.cafleet.kind') = 'builtin-administrator'"
    )

def downgrade() -> None:
    """Data cleanup is not reversible; downgrade is a no-op."""
```

Legacy administrator rows had no placement and no monitor enrollment, so no other table needs touching. Update the chain-guard test per `.claude/rules/database-migrations.md`: expected count 2, revision `0002` with `down_revision = "0001"`.

### C. CLI and code consolidation

#### C1. Remove `member nudge`

`member nudge` persists and notifies through `broker.send_message` — the identical persistence + Esc-safeguarded inline-preview path as `message send`. The wrapper-level deltas it adds are dropped as part of this breaking change: the live-multiplexer requirement (`ensure_multiplexer_or_die`, exit 1 without one — the replacement `message send` succeeds even when no multiplexer is running, since the preview is best-effort), the target pre-resolution via `_load_authorized_member` (`send_message`'s own destination check still rejects a cross-fleet or inactive target), the absence of `--full` / `--quiet` and truncation/render, and the bespoke JSON shape `{member_id, pane_id, task_id, notification_sent}` (replaced by `message send`'s task envelope). None of these deltas is load-bearing for re-engagement. Delete the subcommand (`cli/member.py` `member_nudge`). The documented replacement everywhere is:

```bash
cafleet message send --fleet-id <fleet-id> --from-member-id <sender> --to-member-id <target> --text "..."
```

Operator-side migration (recorded here because live docs must not mention the removed command): any `Bash(cafleet member nudge ...)` patterns in user-level `~/.claude/settings.json` become dead and can be deleted; `message send` is already covered by the existing per-subcommand allow pattern.

#### C2. Single `member list` shape

Replace the three variants (bare / `--activity` / `--all`) and their three broker queries + three formatters with one:

- **Flags**: only `--fleet-id` (and the shared trailing `--json`). `--activity` and `--all` are deleted; the mutual-exclusion error disappears with them.
- **Rows**: every **active** registry entry of the fleet (current `--all` semantics: root Director, monitoring member, ordinary members, placementless rows). Because the row set is active-only, no output shape carries a `status` column — it would be the constant `"active"`.
- **Text columns**: `member_id`, `name`, `kind`, `backend`, `pane_id`, `idle`. Placementless rows render `-` in placement cells; a placed row with no pane renders `(pending)`; `idle` is humanized `Ns`/`Nm`/`Nh` (`-` when no activity), keeping today's `--activity` aggregation semantics (seconds since the member's most recent task activity).
- **JSON**: one dict per row with `member_id`, `name`, `kind`, `placement` (sub-dict or `null`), and the activity fields `last_sent` / `last_recv` / `last_ack` / `idle`. The `description` and `registered_at` fields of today's roster dump are intentionally dropped from the list shape — per-member detail remains available via `member show`.
- **Broker**: one function `list_members(fleet_id)` producing exactly the rows above; delete `list_members_with_activity` and `list_roster` (and the old placed-members-only `list_members` body it replaces).
- **Output**: one `format_member_list`; delete `format_member_list_activity` and `format_member_roster`.

The supervision workflows keep their signal: the stall heuristic reads the `idle` column from the single output (`skills/cafleet/reference/recovery.md`, `reference/director.md` § Member List, `docs/how-to/monitor-and-recover.md` rewrite their `--activity` invocations to bare `member list`).

#### C3. `member capture` alias

Delete the `--tail` alias; `--lines` remains the only spelling. `--ansi/--no-ansi` is unchanged.

#### C4. Remove `wait_agent_status`

Delete the method from the `AgentStateAware` protocol (`multiplexer/base.py`) and its herdr implementation (`multiplexer/herdr.py`). It has no caller in src; `agent_status` (consumed by `monitor/loop.py`) is untouched.

#### C5. Inline `client_command`

Delete the `client_command` decorator (`cli/_helpers.py`) and its per-command lambda parameters in `cli/message.py`. Each of the six `message` subcommands becomes a plain function — matching the style of every other CLI group — composed from the surviving shared pieces (`fleet_id_option`, member-id options, `verify_member_fleet` guard, `truncate_task_text` / `render_tasks_in_result`, `json_flag` / `full_flag` / `quiet_flag`, the formatters). CLI behavior (flags, output, error strings, exit codes) is byte-identical; `tests/cli/test_client_command.py` is deleted and any unique coverage moves into the message suites.

### D. Documentation restructure

Per the user decision, `SPEC.md`, `docs/spec/`, and `docs/api/` all remain as surfaces; this axis only removes pages made dead by A–C and folds one thin stub:

- Delete `docs/how-to/use-the-webui.md` and `docs/spec/webui-api.md`; remove their `zensical.toml` nav rows.
- Fold `docs/concepts/multiplexer-backends.md` (34 lines, mostly pointers) into `docs/spec/multiplexer-backends.md` as an introductory section; delete the concepts page and its nav row; retarget inbound links (`docs/index.md`, `docs/concepts/overview.md`, any skill links).
- Sweep every WebUI / server / Administrator / nudge / `--activity` / `--all` / `--tail` mention across `docs/` (files enumerated in Implementation).
- `README.md` via the `/update-readme` skill: the pitch drops "and an admin WebUI"; the Specification bullet drops "WebUI API".

### E. Skills consolidation (content-preserving)

Guiding constraint (user): flattening must not lose instructions — every protocol, command shape, and caveat that exists today survives in some loadable page; only true duplication and removed-feature content is dropped. The overlay machinery and all three backend overlays stay (backends are kept).

| Merge | Result |
|---|---|
| `skills/cafleet/reference/output-flags.md` (20) + `reference/broadcast.md` (28) → `reference/cli.md` | one CLI reference page with *Output flags* and *Broadcast* sections; both source files deleted; every inbound link (SKILL.md load-bearing/on-demand tables, role files, other skills) retargeted |
| `skills/cafleet-design-doc/reference/template.md` (49) → `reference/guidelines.md` | guidelines opens with the template block; template.md deleted; inbound links retargeted (`SKILL.md`'s On-demand reference table and the three workflow bodies `create/create.md` / `interview/interview.md` / `execute/execute.md`; the role files name the template only in prose and need no link edit) |
| `skills/cafleet-research/reference/slidev/techniques/{formatting,math-formulas,two-column-layouts}.md` (254) → `reference/slidev.md` | technique content becomes sections of slidev.md, verbatim; the `techniques/` directory is deleted; inbound links retargeted |

Plus sweeps in all skills (including `.claude/skills/skill-author/SKILL.md`, which mentions nudge and the Administrator): `member nudge` → `message send` in the Director primitive lists and stall ladders; the `member list` flag rewrites; the kind taxonomy; the `coordination.md` "admin WebUI timeline" phrasing → "broker timeline".

### F. Test slimming

| Action | Targets |
|---|---|
| Delete (removed features) | `tests/webui/` (3 files); `tests/cli/test_server.py` (the deleted `server` command; also the only test consumer of `broker_host` / `broker_port`); `tests/broker/test_administrator.py`; every `member nudge` test (dedicated files, cases inside `tests/cli/test_member*.py`, and the nudge entry in `tests/cli/test_text_input.py`'s shared text-body command matrix); `tests/cli/test_client_command.py` |
| Delete (removed-surface meta-tests, per user decision) | `tests/cli/test_agent_flags_removed.py`, `tests/cli/test_agent_group_removed.py`, `tests/cli/test_db_group_removed.py`, and the pre-subcommand `--json` guard added by design 0000128 (wherever it landed). No new absence-guard tests are added — the absence is the test |
| Consolidate | `tests/cli/test_member_list_activity.py` + `tests/cli/test_member_list_all.py` + the list cases in `test_member.py` → one `tests/cli/test_member_list.py` covering the single output shape (text columns, JSON fields, placementless/pending rendering, idle aggregation) |
| Update | fleet bootstrap / kind / messaging / monitor / queries tests (Administrator removal, broadcast recipients, 3-value kind); `tests/broker/test_typed_columns.py` (drops its `list_inbox` / `list_sent` / `list_timeline` cases; `get_task` cases follow the move to `messaging.py`); capture tests (`--tail` gone); herdr/multiplexer tests and the fake backend's `wait_agent_status` stub in `tests/monitor/test_loop.py` (`wait_agent_status` gone); `tests/db/test_alembic_smoke.py` chain guard (2 revisions); `fleet create` output tests (no admin field) |
| Keep | `test_unhidden_flags.py`, `test_help_budget.py` (current-behavior guards) and all remaining behavior suites |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation lands first, per `.claude/rules/documentation-maintenance.md`. Prerequisite: design 0000128 is implemented and merged.

Steps 5–8 form one atomic implementation block: each code deletion in Steps 5–7 breaks tests that are only removed or updated in Step 8, so the suite is expected red between Steps 5 and 8 and is verified green only at Step 9. When committing per step, land each of Steps 5–7 together with its Step 8 test edits in the same commit (Step 8 then only carries whatever test work remains).

### Step 1: `docs/` and site nav

- [x] Delete `docs/how-to/use-the-webui.md` and `docs/spec/webui-api.md`; remove both nav rows from `zensical.toml` <!-- completed: 2026-07-11T10:05 -->
- [x] Fold `docs/concepts/multiplexer-backends.md` into `docs/spec/multiplexer-backends.md` (intro section); delete the page and its nav row; retarget inbound links in `docs/index.md` and `docs/concepts/overview.md` <!-- completed: 2026-07-11T10:05 -->
- [x] Sweep WebUI/server mentions: `docs/index.md`, `docs/how-to/index.md`, `docs/how-to/design-doc-development.md`, `docs/concepts/{storage,overview,monitoring}.md`, `docs/get-started/{install,contributing}.md`, `docs/api/broker.md`, `docs/spec/cli-options.md` (delete the `server` command section) <!-- completed: 2026-07-11T09:58 -->
- [x] Sweep Administrator mentions (3-value kind, broadcast recipients, `fleet create` output): `docs/spec/{data-model,cli-options}.md`, `docs/concepts/{fleet-isolation,overview,member-lifecycle,monitoring}.md`, `docs/how-to/mixed-backend-team.md`, `docs/get-started/{quickstart,contributing}.md` <!-- completed: 2026-07-11T09:58 -->
- [x] Sweep `member nudge` → `message send`: `docs/spec/{cli-options,multiplexer-backends}.md`, `docs/concepts/{overview,member-lifecycle,monitoring}.md`, `docs/get-started/configure.md` <!-- completed: 2026-07-11T09:58 -->
- [x] Rewrite `member list` docs to the single shape (columns, JSON fields, no flags): `docs/spec/cli-options.md` (flag table, output sections, error table row), `docs/concepts/member-lifecycle.md`, `docs/how-to/{mixed-backend-team,monitor-and-recover}.md`, `docs/get-started/quickstart.md` <!-- completed: 2026-07-11T09:58 -->
- [x] `docs/spec/cli-options.md`: delete the `--tail` alias from the `member capture` flag table; delete the `member nudge` section and its `permissions.allow` row <!-- completed: 2026-07-11T09:58 -->
- [x] `docs/get-started/configure.md`: delete the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` rows <!-- completed: 2026-07-11T09:58 -->

### Step 2: `SPEC.md`

- [ ] Delete the WebUI module spec (§6.8), the `server` command surface, and the `broker_host` / `broker_port` configuration rows; prune WebUI mentions elsewhere (e.g. `max_text_len` notes, checklists) <!-- completed: -->
- [ ] Remove the Administrator: fleet bootstrap (single built-in Director), `fleet create` output shapes, kind taxonomy (3 values), broadcast recipient rule, deregister guard error string <!-- completed: -->
- [ ] Remove `member nudge` (command surface, checklist rows, member-lifecycle sentence); rewrite `member list` to the single shape; drop the `--tail` alias; drop `wait_agent_status` from the multiplexer capability spec; update the `message` subcommand spec if it names `client_command` internals <!-- completed: -->
- [ ] Update the broker module spec: `queries.py` gone (`get_task` lives in `messaging.py`), `get_member_names` / `list_monitor_configs` / `list_roster` / `list_members_with_activity` gone, single `list_members` documented <!-- completed: -->

### Step 3: README, CLAUDE.md, rules

- [ ] Run `/update-readme`: pitch drops "and an admin WebUI"; Specification bullet drops "WebUI API" (SPEC.md edits from Step 2 are its other input) <!-- completed: -->
- [ ] `CLAUDE.md`: update the tech-stack line (drop FastAPI/server) and the CLI blurb if it mentions the server <!-- completed: -->
- [ ] `.claude/rules/commands.md`: delete the WebUI server bullets (`cafleet server`, `mise //cafleet:dev`), `mise //admin:dev` / `//admin:build` / `//admin:lint` rows, and the admin steps in the publish description <!-- completed: -->
- [ ] `.claude/rules/bash-tool.md`: sweep the `member nudge` mention (Director-side primitives) <!-- completed: -->
- [ ] `.claude/settings.json`: delete the `Bash(mise //admin*)` `permissions.allow` row (dead once `admin/` and its mise tasks are gone) <!-- completed: -->

### Step 4: skills

- [ ] `skills/cafleet`: merge `reference/output-flags.md` + `reference/broadcast.md` into `reference/cli.md`; delete both files; retarget every inbound link (SKILL.md tables, role files, `reference/*.md`, other skills, `docs/` if any) <!-- completed: -->
- [ ] `skills/cafleet`: sweep nudge → `message send` in `SKILL.md`, `roles/{director,monitor}.md`, `reference/{supervision,cli,exec-routing,recovery,director}.md`; rewrite `reference/director.md` § Member List and `reference/recovery.md` heuristics to the single `member list` output; sweep Administrator from `reference/cli.md` roster description <!-- completed: -->
- [ ] `skills/cafleet-design-doc`: merge `reference/template.md` into `reference/guidelines.md`; delete the file; retarget inbound links (`SKILL.md` On-demand table, `create/create.md`, `interview/interview.md`, `execute/execute.md`); sweep nudge/WebUI/Administrator mentions (`reference/coordination.md` "admin WebUI timeline" → "broker timeline") <!-- completed: -->
- [ ] `skills/cafleet-research`: merge `reference/slidev/techniques/*.md` into `reference/slidev.md`; delete the directory; retarget inbound links; sweep nudge mentions in `report/report.md`, `presentation/presentation.md`, both `roles/director.md` <!-- completed: -->
- [ ] `.claude/skills/skill-author/SKILL.md`: sweep removed features (nudge, Administrator, member-list flags) <!-- completed: -->
- [ ] Verify no dangling relative links remain in `skills/` and `docs/` (grep for the deleted filenames) <!-- completed: -->

### Step 5: code — WebUI removal

- [ ] Delete `admin/`, `cafleet/src/cafleet/webui/`, `cafleet/src/cafleet/cli/server.py`; drop the `server` import + `add_command` from `cli/__init__.py` <!-- completed: -->
- [ ] `config.py`: delete `broker_host` / `broker_port` and their docstring entries <!-- completed: -->
- [ ] `cafleet/pyproject.toml`: drop `fastapi` + `uvicorn[standard]`, the `webui/dist` packaging entries, and the `fastapi.*` / `uvicorn` ty overrides; run `mise //:uv-sync` <!-- completed: -->
- [ ] Root `mise.toml`: drop `"admin"` from `config_roots` and `//admin:build` from `all-install`; `cafleet/mise.toml`: delete the `dev` task and the admin steps in `publish` <!-- completed: -->
- [ ] Broker dead code: delete `list_inbox` / `list_sent` / `list_timeline`, move `get_task` into `broker/messaging.py`, delete `broker/queries.py`; delete `get_member_names` and `list_monitor_configs`; update `broker/__init__.py` exports <!-- completed: -->

### Step 6: code — Administrator removal

- [ ] `broker/fleets.py`: delete the administrator seeding block and the `administrator_member_id` return field; update the docstring <!-- completed: -->
- [ ] `broker/members.py`: delete the deregister guard; collapse the enrollment exclusion to the monitoring-member check; drop the `include_task_holders` parameter (function absorbed in Step 7) <!-- completed: -->
- [ ] `broker/messaging.py`: drop the administrator condition from the broadcast recipient query <!-- completed: -->
- [ ] `broker/_shared.py`: delete `ADMINISTRATOR_KIND` / `is_administrator` (and `_card_kind` if uncalled); collapse `derive_member_kind` to 3 values; `broker/__init__.py`: drop the export <!-- completed: -->
- [ ] `output/formatters.py` `format_fleet_create`: compact `<fleet_id> director=<member_id>`; full block loses the administrator line <!-- completed: -->
- [ ] Generate migration `0002` (`mise //cafleet:makemigration "deregister builtin administrator members"`), hand-edit to the pure data migration in § B (no-op downgrade) <!-- completed: -->
- [ ] Update the chain-guard test in `tests/db/test_alembic_smoke.py`: 2 revisions, `0002.down_revision == "0001"`, rename the test <!-- completed: -->

### Step 7: code — CLI consolidation

- [ ] Delete `member_nudge` from `cli/member.py` <!-- completed: -->
- [ ] Broker: implement the single `list_members(fleet_id)` (all active registry entries + kind + placement + activity fields); delete `list_members_with_activity` and `list_roster`; update `broker/__init__.py` <!-- completed: -->
- [ ] `cli/member.py` `member list`: drop `--activity` / `--all` and the mutual-exclusion check; emit the single shape <!-- completed: -->
- [ ] `output/formatters.py`: one `format_member_list` (columns per § C2); delete `format_member_list_activity` and `format_member_roster` <!-- completed: -->
- [ ] `cli/member.py` `member capture`: drop the `--tail` alias <!-- completed: -->
- [ ] `multiplexer/base.py` + `multiplexer/herdr.py`: delete `wait_agent_status` <!-- completed: -->
- [ ] Inline `client_command`: delete the decorator from `cli/_helpers.py`; rewrite the six `message` subcommands as plain functions with byte-identical CLI behavior <!-- completed: -->

### Step 8: tests

- [ ] Delete `tests/webui/`, `tests/cli/test_server.py`, `tests/broker/test_administrator.py`, all nudge tests (including the entry in `tests/cli/test_text_input.py`'s shared command matrix), `tests/cli/test_client_command.py` (unique coverage moves into the message suites) <!-- completed: -->
- [ ] Delete `tests/cli/test_agent_flags_removed.py`, `test_agent_group_removed.py`, `test_db_group_removed.py`, and the 0000128 pre-subcommand `--json` guard <!-- completed: -->
- [ ] Consolidate the member-list suites into `tests/cli/test_member_list.py` covering the single shape (text columns, JSON fields, placementless/pending, idle) <!-- completed: -->
- [ ] Update the remaining suites: fleet bootstrap/kind/messaging/monitor/queries (Administrator + broker deletions), `tests/broker/test_typed_columns.py` (dead query callers; `get_task` import move), capture (`--tail`), herdr + the `tests/monitor/test_loop.py` fake-backend stub (`wait_agent_status`), fleet-create output <!-- completed: -->

### Step 9: verification

- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` pass <!-- completed: -->
- [ ] `mise //:docs-build` passes (nav has no dead pages) <!-- completed: -->
- [ ] `mise //cafleet:build` succeeds and the wheel contains no `webui/` files <!-- completed: -->
- [ ] Full-repo grep confirms no live mention of: WebUI, `cafleet server`, `CAFLEET_BROKER_HOST/PORT`, Administrator, `member nudge`, `--activity`, `member list --all`, `--tail`, `wait_agent_status`, `client_command` (historical `design-docs/**` excluded) <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-11 | Initial draft |
| 2026-07-11 | Reviewer round 1: accurate `member nudge` delta rationale; `member list` drops the constant `status` column and records the `description`/`registered_at` JSON drop; corrected `template.md` inbound links; test enumeration extended (`test_server.py`, `test_text_input.py`, `test_typed_columns.py`, `test_loop.py` stub); Steps 5–8 declared one atomic block; added `.claude/settings.json` `mise //admin*` cleanup; dropped the no-op `update-readme` sweep |
