# Single Director per Fleet

**Status**: Approved
**Progress**: 49/49 tasks complete
**Last Updated**: 2026-06-08

## Overview

Restrict every fleet to exactly one Director — the root Director recorded in `fleets.director_agent_id` — and forbid nested teams: only the root Director may own members, and a member may never be named as another member's `placement.director_agent_id`. This collapses the team model to a single flat tier, which lets several now-redundant scoping parameters, flags, and code paths be simplified away in the same change.

## Removal Rule (binding)

This change **removes** concepts, so `config/claude/rules/removal.md` governs it. When implementing, **DELETE every mention** of each removed concept from **all** surfaces — README, `docs/concepts`, `docs/spec`, every `SKILL.md`, `.claude/rules`, and source code (including code comments). The removed concepts are:

- nested teams / "multiple Directors per fleet" / "second-level Director" / sub-teams
- the within-fleet "cross-Director boundary"
- `--agent-id` on the director-side member subcommands (`capture` / `send-input` / `exec` / `ping` / `delete`) and on `member list`
- `agent list --agent-id`
- `message poll --since` / `--page-size`

There must be **no** "deprecated", "previously supported", or restoration-pointer notices anywhere. After the change the repo must read as if these concepts never existed — only document what exists. This design doc is the **sole** historical record of the removal. The doc may NAME a removed concept only to specify its deletion; it must never describe one as still-existing or with deprecation framing.

The one nuance: the within-fleet "cross-Director boundary" is **reworded to cross-*fleet*** (a real authorization boundary that stays) — the within-fleet, Director-to-Director framing is what goes away, not the cross-fleet boundary itself.

## Success Criteria

- [x] `broker.register_agent` rejects any member placement whose `director_agent_id` is not the fleet's root Director.
- [x] `broker.list_members` / `list_members_with_activity` take only `fleet_id` (no `director_agent_id` param) and never surface the root Director itself.
- [x] `cafleet member list` no longer accepts `--agent-id` (dropped, like `agent list`); it lists every member of the fleet given by `--fleet-id`.
- [x] The director-side member subcommands (`capture`, `send-input`, `exec`, `ping`, `delete`) drop `--agent-id` entirely (like `member list`); they take only `--member-id` (plus command-specific args) and reject only cross-fleet access — a `--member-id` not in `--fleet-id` returns "not found". There is no caller-is-root check.

- [x] `cafleet message poll` returns only un-acked (`input_required`) deliveries and no longer accepts `--since` / `--page-size`; the now-dead broker params are removed.
- [x] `cafleet agent list` no longer requires `--agent-id`.
- [x] ~~The unused `httpx` dev dependency is removed.~~ Reverted — CI showed `httpx` is required by `starlette.testclient` (`test_server_routing.py`); it is retained. No dependency change.
- [x] The WebUI HTTP API surface (`webui_api.py`, including `/inbox` and `/sent`) is documented.
- [x] Per the binding Removal Rule, every removed concept (nested teams, "multiple Directors per fleet", the within-fleet "cross-Director boundary", member-subcommand & `member list` `--agent-id`, `agent list --agent-id`, `poll --since`/`--page-size`) is DELETED from all surfaces — README, `docs/concepts`, `docs/spec`, every `SKILL.md`, `.claude/rules`, source + code comments — with no deprecation/restoration notices; the repo reads as if they never existed.
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` all pass.

---

## Background

A fleet is bootstrapped by `cafleet fleet create`, which mints a root Director (recorded in `fleets.director_agent_id`) plus a built-in Administrator. Members are spawned via `cafleet member create`, each gaining an `agent_placements` row whose `director_agent_id` points at the Director that spawned it.

The data model technically permits a *member* to be named as another member's `placement.director_agent_id`, producing nested second-level teams. Nothing in the product relies on this, but it forces several APIs to carry a `director_agent_id` scoping parameter, complicates the authorization story with a within-fleet "cross-Director boundary" that only matters under nesting, and forces `member list` plus the director-side member subcommands to carry a `--agent-id` they no longer need. This design makes the single-Director model an enforced invariant, removes the machinery that only existed to support nesting, and collapses the member-subcommand boundary to plain fleet isolation — the only boundary that remains is cross-fleet (a `--member-id` outside the `--fleet-id` returns "not found"); there is no caller-auth check.

The change is scoped to coexist with several independent simplifications that share the same files and review surface:

- `cafleet agent list --agent-id` is a required flag used only as an auth gate, never read in the handler body.
- `cafleet message poll` accepts hidden `--since` / `--page-size` flags — absent from `docs/spec/cli-options.md`, but documented in `skills/cafleet/SKILL.md` and woven into the `cafleet-agent-team-monitoring` delta-polling health-check recipe — and returns *all* tasks (including already-acked ones) because `poll_tasks` defaults its `status` filter to `None` and the CLI never passes it.
- The WebUI `GET /api/agents/{id}/inbox` and `/sent` endpoints are undocumented.
- `httpx` is declared as a dev dependency and was assumed unused. (This assumption proved **incorrect** — see A4: `httpx` is a required transitive dependency of `starlette.testclient`, so the proposed removal was reverted.)

### Coding-agent backends are explicitly out of scope

The `claude` / `codex` / `opencode` coding-agent backends, the `--coding-agent` flag (on both `fleet create` and `member create`), the `CODING_AGENTS` registry, the opencode preset materialization, the `agent_placements.coding_agent` column, the codex/opencode reference docs and skill sections, the `.codex-plugin/` distribution, and every README mention of codex/opencode **stay fully intact and must not be touched** by this change.

---

## Specification

### The single-Director invariant

| Rule | Enforcement |
|---|---|
| A fleet has exactly one Director: the root recorded in `fleets.director_agent_id`. | Already true at bootstrap; no new code. |
| Only the root Director may own members. | `register_agent` validates the placement `director_agent_id` equals the fleet root (see below). |
| A member may never be another member's `placement.director_agent_id`. | Same validation — a non-root `director_agent_id` is rejected. |
| The root Director itself has no parent. | `agent_placements.director_agent_id IS NULL` for the root (unchanged bootstrap behavior). |

Consequence: for every **member** placement, `agent_placements.director_agent_id` is always exactly `fleets.director_agent_id`. The only `agent_placements` row with a `NULL` `director_agent_id` is the root Director's own row.

### `register_agent` placement validation (D1)

`broker.register_agent` (`cafleet/src/cafleet/broker.py`) already loads the fleet via `get_fleet(fleet_id)` at the top of the function and already validates, when `placement is not None`, that the named director is active and is not the Administrator. Add a root-equality check in the same block:

```python
# inside register_agent, when placement is not None
root_director_id = sess["director_agent_id"]
if placement["director_agent_id"] != root_director_id:
    raise click.UsageError(
        f"nested teams are not supported; placement director_agent_id "
        f"{placement['director_agent_id']} must equal the fleet root "
        f"Director {root_director_id}."
    )
```

- `sess` is the `get_fleet(fleet_id)` result already bound near the top of the function; `sess["director_agent_id"]` is the root.
- This fires only for member registrations (placement supplied). The root Director's own placement is written by `create_fleet`'s bootstrap transaction, which does **not** route through `register_agent`, so the root is never rejected by this check.
- By the time any member registers, bootstrap has completed and `fleets.director_agent_id` is non-NULL, so `root_director_id` is always a real id.

### `list_members` / `list_members_with_activity` / `_base_members_select` (B1)

Drop the `director_agent_id` parameter from all three. The new signatures:

```python
def _base_members_select(fleet_id: int): ...
def list_members(fleet_id: int) -> list[dict]: ...
def list_members_with_activity(fleet_id: int) -> list[dict]: ...
```

`_base_members_select` currently filters `AgentPlacement.director_agent_id == director_agent_id`. **Do not simply delete that predicate** — the root Director also owns an `agent_placements` row (with `director_agent_id IS NULL`), and a bare placement-join would surface the root in `member list`. Replace the predicate with an explicit "is a member" filter:

```python
.where(
    Agent.fleet_id == fleet_id,
    Agent.status == "active",
    AgentPlacement.director_agent_id.is_not(None),
)
```

In the single-Director model `director_agent_id IS NOT NULL` is exactly equivalent to "this placement's director is the fleet root", and it cleanly excludes the root Director's own row.

### `member list` — drop `--agent-id` (B2)

`cli.py` `member_list` currently passes `--agent-id` straight into the broker as a scoping selector. With B1 the broker no longer scopes by it, and the CLI has no real auth gate (commands talk to SQLite directly), so `--agent-id` on this read-only command is non-functional. Drop it entirely — exactly like `agent list` (A1). `member list` then lists every member of the fleet identified by the global `--fleet-id`:

```python
rows = broker.list_members_with_activity(fleet_id) if activity else broker.list_members(fleet_id)
```

### `_load_authorized_member` and the director-side member subcommands (C1)

`_load_authorized_member` (`cli.py`) backs `member capture`, `member send-input`, `member exec`, `member ping`, and `member delete`. In the flat model these commands carry no caller identity: each takes only the global `--fleet-id` plus `--member-id` (and its own command-specific args — `exec`'s positional command, `send-input`'s `--choice` / `--freetext`, `capture`'s `--lines` / `--ansi`, `delete`'s `--force`). The **only** boundary is fleet isolation: a `--member-id` that does not belong to `--fleet-id` returns "not found". There is no caller-is-root or ownership check of any kind.

`_load_authorized_member` therefore loses its `director_agent_id` parameter and both former auth checks (the caller-root check and the membership assertion). It simply resolves the fleet-scoped member:

```python
def _load_authorized_member(fleet_id, member_id, *, placement_missing_template=...):
    target = broker.get_agent(member_id, fleet_id)   # fleet-scoped → cross-fleet returns None
    if target is None:
        raise click.ClickException(f"Agent {member_id} not found")
    placement = target["placement"]
    if placement is None:
        raise click.ClickException(placement_missing_template.format(member_id=member_id))
    return target, placement
```

Cross-fleet isolation is enforced entirely by `broker.get_agent(member_id, fleet_id)` returning `None` for a member outside the fleet. The former within-fleet "cross-Director boundary" and the "is not a member of your team" error are gone — there is no caller to compare against.

Any agent in `--fleet-id` — including the root Director itself — is a valid `--member-id` target; the sole rejection is cross-fleet. (`member delete` of the root stays blocked downstream by the deregister root-guard, and B1 keeps the root out of `member list` output.)

**Scope guard**: `member create` is unaffected — it keeps its `--agent-id` (the spawning Director), which D1 validates equals the fleet root.

### `message poll` — un-acked only, drop `--since` / `--page-size` (A2)

`broker.poll_tasks` currently signs `poll_tasks(agent_id, since=None, page_size=None, status=None)` and the CLI never passes `status`, so every poll returns all tasks including acked ones. Reshape:

```python
def poll_tasks(agent_id: int) -> list[dict]:
    """Return un-acked deliveries addressed to ``agent_id``, newest first."""
    return _list_tasks_where(
        Task.context_id == agent_id,
        status="input_required",
    )
```

- Drop `since` and `page_size` from `poll_tasks`.
- Hardcode `status="input_required"` so poll returns only un-acked deliveries.
- `_list_tasks_where` keeps a `status` kwarg (still used). Remove its `since` / `page_size` kwargs **only if** `poll_tasks` was the sole caller passing them — verify with a grep across `broker.py` and tests during implementation, then scrub the now-dead branches.
- `cli.py` `message_poll`: remove the `--since` and `--page-size` options and the params from the handler; call `broker.poll_tasks(agent_id)`.

### `agent list` — drop the unused `--agent-id` gate (A1)

`cli.py` `agent_list` requires `--agent-id` purely as an auth gate (`requires_agent_fleet=True`); the handler never reads it. Remove the `--agent-id` option and the `requires_agent_fleet` gate for this command. `agent list` lists all agents in the fleet identified by the global `--fleet-id`.

### WebUI API documentation (A3)

The WebUI HTTP API in `cafleet/src/cafleet/webui_api.py` is undocumented. Add a reference page documenting the full surface — `GET /fleets`, `GET /agents`, `GET /agents/{id}/inbox`, `GET /agents/{id}/sent`, `GET /timeline`, `POST /messages/send`, and any others present — each with path, method, response shape, and fleet-scoping behavior. `/inbox` and `/sent` are **documented, not removed**.

### `httpx` removal (A4) — REVERTED (CI finding)

A4's premise — that `httpx` is declared but never imported — was **incorrect**. `httpx` is a required transitive dependency of `starlette.testclient` (reached via `fastapi.testclient.TestClient` in `tests/test_server_routing.py`); removing it broke test collection in CI's fresh venv with `ModuleNotFoundError: No module named 'httpx'`. The removal passed local testing only because the developer venv retained a stale `httpx`. `httpx` is therefore **retained** in `[dependency-groups].dev` and the lockfile — there is no dependency change in this design.

---

## Implementation

> Documentation-first per `.claude/rules/design-doc-numbering.md`: all docs/concepts, docs/spec, README, SKILL.md, and rules updates land before any code. Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`.

### Step 1: Concepts documentation

- [x] Update the team-model concept page(s) under `docs/concepts/` (e.g. `overview.md`, `member-lifecycle.md`) to state the single-Director-per-fleet invariant and that nested teams are forbidden; delete any "sub-team" / "multiple Directors" / "second-level Director" / nested-hierarchy language. <!-- completed: 2026-06-08T08:52 -->
- [x] In `docs/concepts/bash-routing.md`, reword the "cross-Director boundary" reference (~line 32) so it describes the bash-via-Director protocol and the cross-*fleet* boundary; remove the within-fleet cross-Director framing. <!-- completed: 2026-06-08T08:52 -->
- [x] Grep `docs/concepts/` for "nested", "sub-team", "multiple Director", "second-level", "cross-Director" and scrub each remaining hit. <!-- completed: 2026-06-08T08:52 -->

### Step 2: Spec — data model

- [x] In `docs/spec/data-model.md`, rewrite the `agent_placements.director_agent_id` row (~line 123): it always equals `fleets.director_agent_id` (the fleet root) for members, and is `NULL` only for the root Director itself. State that nested teams are forbidden. <!-- completed: 2026-06-08T08:54 -->
- [x] Reword the `idx_placements_director` index purpose (~line 134) and the `list_placements_for_director` operation-mapping row (~line 167) to reflect the flat model (list the fleet's members; `director_agent_id` is the root). Align the operation-mapping signature with the new `list_members(fleet_id)`. <!-- completed: 2026-06-08T08:54 -->

### Step 3: Spec — CLI options

- [x] In `docs/spec/cli-options.md`, confirm there are no `message poll --since` / `--page-size` rows to remove (those flags are documented in the skill files, not here — scrubbed in Step 6); ensure the `message poll` entry states it returns only un-acked (`input_required`) deliveries. <!-- completed: 2026-06-08T09:05 -->
- [x] Remove the `member list --agent-id` documentation entirely — the flag is dropped (like `agent list`); document `member list` as taking only the global `--fleet-id`. <!-- completed: 2026-06-08T09:05 -->
- [x] Remove the `--agent-id` row from the `agent list` documentation, and scrub the `agent list ... --agent-id ... not a member of --fleet-id` error-table line (`docs/spec/cli-options.md:685`). <!-- completed: 2026-06-08T09:05 -->
- [x] In `docs/spec/cli-options.md`, drop the `--agent-id` rows for all five director-side member subcommands (`capture` / `send-input` / `exec` / `ping` / `delete`); document each as taking only `--fleet-id` + `--member-id` (plus its own args), where `--member-id` may be any agent in the fleet (including the root Director) and a cross-fleet `--member-id` returns "Agent {id} not found". Remove the "across Directors" / "cross-Director" error-table rows for these commands (the only boundary is fleet isolation — there is no caller-auth error). <!-- completed: 2026-06-08T09:05 -->

### Step 4: Spec — WebUI API (A3)

- [x] Add a `docs/spec/webui-api.md` (or equivalent reference page) documenting every endpoint in `webui_api.py` — path, method, response shape, fleet-scoping — including `GET /agents/{id}/inbox` and `GET /agents/{id}/sent`. Link it from the docs index / nav if one exists. <!-- completed: 2026-06-08T09:08 -->

### Step 5: README & get-started guides

- [x] Scrub `README.md` of any multiple-Directors / nested-teams / sub-team / team-hierarchy language so it reflects the single-Director model. <!-- completed: 2026-06-08T09:12 -->
- [x] README documents neither `message poll --since`/`--page-size` nor `agent list --agent-id` (verified — only codex/opencode mentions, which stay) — no change needed there beyond the nested-teams scrub above. <!-- completed: 2026-06-08T09:12 -->
- [x] Update `docs/get-started/quickstart.md` (e.g. line 64, `cafleet ... agent list --agent-id ...`) to the new `agent list` surface (drop `--agent-id`). <!-- completed: 2026-06-08T09:12 -->
- [x] Audit `docs/get-started/configure.md` and the rest of `docs/get-started/` for `agent list --agent-id` and `message poll --since`/`--page-size` usage and scrub each hit. <!-- completed: 2026-06-08T09:12 -->

### Step 6: Skills

- [x] Repo-wide skills sweep: grep all of `skills/` for `member (capture|send-input|exec|ping|delete) .*--agent-id` and drop `--agent-id` from every hit so each shows only `--fleet-id` + `--member-id` (plus its own args) — **preserving every `member create --agent-id`** (unchanged). This MUST cover the core cafleet skill (`skills/cafleet/SKILL.md`, `reference/director.md`, `reference/exec-routing.md`, `reference/recovery.md`, `roles/director.md`, `roles/member.md`) AND every orchestration skill that drives a team: `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, `cafleet-design-doc-create`, `cafleet-design-doc-execute`, `cafleet-design-doc-interview`, `cafleet-research-report`, `cafleet-research-presentation` (each including its `SKILL.md` and any `roles/director.md`). Reword every caller-auth / "cross-Director" boundary note to plain fleet isolation: a `--member-id` outside `--fleet-id` returns "Agent {id} not found", any in-fleet agent (including the root Director) is a valid target, and there is no caller validation. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet/SKILL.md`, update the `agent list` example to drop `--agent-id`; reword the "cross-Director boundary" reference to the cross-fleet framing. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet/SKILL.md`, delete every `--since` / `--page-size` mention — the `message poll` examples, the flag-table rows, and the `--since` ISO-8601 prose (~L139-150) — and update the "Poll (Check Inbox)" section to state poll now returns only un-acked (`input_required`) deliveries. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet-agent-team-monitoring/SKILL.md`, rewrite the delta-polling health-check recipe (~L62, L90, L119, L122): because poll now returns only un-acked deliveries, the monitoring tick no longer tracks a last-tick `--since` timestamp — re-describe the loop as plain `message poll` → ACK consumes, deleting every `--since` reference rather than leaving a broken loop description. <!-- completed: 2026-06-08T09:15 -->
- [x] Delete the `--since` / `--page-size` mentions in `skills/cafleet-design-doc-create/roles/director.md` (~L59), `skills/cafleet-design-doc-execute/roles/director.md` (~L69), and `skills/cafleet-design-doc-execute/SKILL.md` (~L675, L765); update the `cafleet-design-doc-execute/SKILL.md:675` "team-health --since timestamp" cross-reference so it matches the rewritten monitoring recipe. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet/reference/exec-routing.md`, rewrite the "Cross-Director boundary" section: there is no caller-auth boundary anymore — `member exec` reaches any member of the same `--fleet-id`, and a cross-fleet `--member-id` returns "not found". Remove the "another Director's member" framing. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet/reference/director.md`, remove the `--agent-id` "(cross-Director authorization check)" note and the "Cross-Director delete" section entirely; document the member subcommands as `--member-id`-only with cross-fleet→not-found as the sole boundary. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet/reference/recovery.md`, reword "cross-Director authorization boundary" to "cross-fleet authorization boundary" (fleet isolation; no caller check). <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet/roles/director.md` and `roles/member.md`, remove the within-fleet cross-Director-boundary references. <!-- completed: 2026-06-08T09:15 -->
- [x] In `skills/cafleet-agent-team-supervision/SKILL.md` and `skills/cafleet-agent-team-monitoring/SKILL.md`, scrub any "spawn sub-team" / nested-hierarchy / multi-Director language (do **not** touch the by-backend sections). <!-- completed: 2026-06-08T09:15 -->
- [x] Grep all of `skills/` for "nested", "sub-team", "second-level", "cross-Director", "another Director", "--since", "--page-size", and `member (capture|send-input|exec|ping|delete) .*--agent-id` and scrub each remaining hit. <!-- completed: 2026-06-08T09:15 -->

### Step 7: Project rules

- [x] In `.claude/rules/bash-tool.md`, drop `--agent-id` from the `member exec` and `member ping` examples (showing `--member-id`-only) and reword the "cross-Director boundary" reference (~line 85) to plain fleet isolation (no caller check). <!-- completed: 2026-06-08T09:16 (Director-applied: members cannot Edit .claude/ under dontAsk) -->

### Step 8: broker — placement validation (D1)

- [x] In `broker.register_agent`, add the root-equality validation rejecting any member placement whose `director_agent_id != fleets.director_agent_id` (using the already-loaded `get_fleet` result), raising `click.UsageError` with the specified message. <!-- completed: 2026-06-08T09:27 -->
- [x] Update the `register_agent` docstring to state the single-Director invariant. <!-- completed: 2026-06-08T09:27 -->

### Step 9: broker — flat member listing (B1)

- [x] Drop the `director_agent_id` parameter from `_base_members_select`, `list_members`, and `list_members_with_activity`; replace the placement filter with `AgentPlacement.director_agent_id.is_not(None)` so members are listed and the root Director is excluded. <!-- completed: 2026-06-08T09:27 -->
- [x] Update the docstrings of all three to describe `fleet_id`-only scoping and the flat model. <!-- completed: 2026-06-08T09:27 -->

### Step 10: broker — poll reshape (A2)

- [x] Reshape `broker.poll_tasks` to `poll_tasks(agent_id)` returning `_list_tasks_where(Task.context_id == agent_id, status="input_required")`. <!-- completed: 2026-06-08T09:27 -->
- [x] Grep `broker.py` + tests for other callers of `poll_tasks` and `_list_tasks_where`; remove the now-dead `since` / `page_size` kwargs from `_list_tasks_where` if `poll_tasks` was their only consumer. <!-- completed: 2026-06-08T09:27 -->

### Step 11: cli — member list / authorization / poll / agent list

- [x] `member_list`: remove the `--agent-id` option entirely; call `broker.list_members(fleet_id)` / `list_members_with_activity(fleet_id)` scoped only by the global `--fleet-id`. <!-- completed: 2026-06-08T09:27 -->
- [x] `_load_authorized_member`: drop the `director_agent_id` parameter and BOTH former auth checks (caller-root and membership-assertion); resolve only the fleet-scoped member — raise "not found" when `get_agent` returns `None`, the placement-missing error when placement is `None` — then return `(target, placement)`. Update the docstring. <!-- completed: 2026-06-08T09:27 -->
- [x] Drop the `--agent-id` option and the `requires_agent_fleet` gate from `member_capture` / `member_send_input` / `member_exec` / `member_ping` / `member_delete`; each takes only `--member-id` (plus its own args). Update all five call sites to the new `_load_authorized_member(fleet_id, member_id)` signature. (`member_create` is unchanged — it keeps `--agent-id`.) <!-- completed: 2026-06-08T09:27 -->
- [x] `message_poll`: remove `--since` and `--page-size` options and params; call `broker.poll_tasks(agent_id)`. <!-- completed: 2026-06-08T09:27 -->
- [x] `agent_list`: remove the required `--agent-id` option and the `requires_agent_fleet` gate. <!-- completed: 2026-06-08T09:27 -->

### Step 12: Dependencies (A4) — REVERTED

- [x] ~~Remove `httpx` from `[dependency-groups].dev`~~ — reverted after CI revealed `httpx` is required by `starlette.testclient` (`fastapi.testclient.TestClient` in `tests/test_server_routing.py`). `httpx` is retained in the dev group and lockfile; no dependency change. <!-- completed: 2026-06-08T10:57 -->

### Step 13: Tests

- [x] `test_broker_member_activity.py`: invert `test_..._scoping_excludes_other_directors_...` so registering a member under a non-root `director_agent_id` is **rejected** by `register_agent`; add positive coverage that a member under the root succeeds and appears in `list_members`. Update all `list_members` / `list_members_with_activity` call sites and helpers to the new `fleet_id`-only signatures. <!-- completed: 2026-06-08T09:00 -->
- [x] Add a `broker.register_agent` test asserting a non-root placement `director_agent_id` raises `click.UsageError`. <!-- completed: 2026-06-08T09:00 -->
- [x] `poll_tasks` tests: remove the `--since` / `--page-size` (and broker `since` / `page_size`) coverage; add a test that `poll_tasks` returns only `input_required` tasks (acked/completed tasks excluded). <!-- completed: 2026-06-08T09:00 -->
- [x] `message poll` CLI tests: assert `--since` / `--page-size` are no longer accepted and that poll returns only un-acked deliveries. <!-- completed: 2026-06-08T09:00 -->
- [x] `member list` CLI tests: assert `member list --fleet-id <id>` (no `--agent-id`) lists members and excludes the root Director; drop any "non-root `--agent-id` rejected" assertion. <!-- completed: 2026-06-08T09:00 -->
- [x] `member capture` / `exec` / `ping` / `send-input` / `delete` CLI tests: drop `--agent-id` from every invocation; assert a cross-fleet `--member-id` returns "Agent {id} not found"; REMOVE every "non-root caller rejected" / cross-Director-rejection assertion (no caller-auth check remains). <!-- completed: 2026-06-08T09:00 -->
- [x] `agent list` CLI tests: drop `--agent-id` usage; assert the command lists fleet agents without it. <!-- completed: 2026-06-08T09:00 -->

### Step 14: Verify

- [x] `mise //cafleet:lint` passes. <!-- completed: 2026-06-08T09:34 -->
- [x] `mise //cafleet:typecheck` passes. <!-- completed: 2026-06-08T09:34 -->
- [x] `mise //cafleet:test` passes. <!-- completed: 2026-06-08T09:34 -->
- [x] Final grep sweep across `docs/`, `skills/`, `README.md`, `.claude/rules/`, `cafleet/src/`, and `cafleet/tests/` for "nested", "sub-team", "second-level", "cross-Director", "another Director", "multiple Director", "is not a member of your team", "--since", "--page-size", and `member (capture|send-input|exec|ping|delete) .*--agent-id` confirms no stale mentions remain — source code, comments, and docstrings included. The only `nested` mentions are the intentional D1 enforcement (broker rejection message/docstring) and the Tester's regression-guard tests asserting nested teams are rejected — both permitted by the Removal Rule. <!-- completed: 2026-06-08T09:34 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-07 | Initial draft |
| 2026-06-08 | Implemented. A4 (`httpx` removal) reverted: CI revealed `httpx` is a required transitive dependency of `starlette.testclient` (`test_server_routing.py`); the "unused" premise was false (local test passed only on a stale venv). `httpx` retained. Copilot review caught a real bug — `member delete` could `send_exit`/`kill_pane` the root Director's own pane before the deregister guard — fixed with an early root-guard + regression test. |
