# Absorb the `agent` CLI group into `member`

**Status**: Approved
**Progress**: 31/31 tasks complete
**Last Updated**: 2026-07-03

## Overview

Remove the `cafleet agent` CLI group (`register | list | show | deregister`) and make `cafleet member` the single agent-lifecycle surface: `member delete` gains the registry soft-delete for placementless agents, and the introspection that was exclusive to `agent list` / `agent show` (root Director + Administrator visibility, kind, skills, placement detail) moves to `member list --all` and a new `member show`. The user-creatable paneless external-agent concept is removed with it; after this design the corpus reads as if the `agent` group never existed.

## Success Criteria

- [x] `cafleet agent …` exits 2 with Click's `Error: No such command 'agent'.`; `cafleet/src/cafleet/cli/agent.py` no longer exists.
- [x] `cafleet member show --fleet-id <f> --member-id <id>` renders detail (description, status, kind, skills, placement block) for any active in-fleet agent, placed or placementless, without requiring tmux.
- [x] `cafleet member list --fleet-id <f> --all` lists every active agent of the fleet — root Director, Administrator, monitoring member, ordinary members, placementless rows — with a `kind` column; the default (no `--all`) output is byte-identical to today's.
- [x] `cafleet member delete` on an active placementless agent soft-deletes it and exits 0 (the `use \`cafleet agent deregister\` instead` error path is gone); the root-Director and Administrator guards still reject with the existing broker error strings.
- [x] No `cafleet agent ` invocation and no two-word subcommand mention (`agent register` / `agent list` / `agent show` / `agent deregister`) remains in `docs/`, `SPEC.md`, `README.md`, `CLAUDE.md`, `skills/`, `.claude/`, `admin/src/`, or `cafleet/` — excluding `design-docs/`, the pre-existing `"cafleet agent spawn"` entry in `spawn_prompt_guard.py` `FORBIDDEN_PATTERNS` and its tests (a 0000113 guard string, deliberately retained), and the regression-guard test `cafleet/tests/cli/test_agent_group_removed.py` (an absence-test the removal rule permits; it embeds the legacy invocation it asserts is rejected).
- [x] `FORBIDDEN_PATTERNS` in `spawn_prompt_guard.py` is unchanged — no new anti-`cafleet agent` pattern or assertion is added anywhere (user decision).
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:lint-overlay`, and `mise //cafleet:lint-spawn-guard` pass.

---

## Background

Today the CLI has two lifecycle surfaces. `cafleet member create` already registers the agent, creates the pane, and delivers identity via `str.format` substitution; `cafleet member delete` already deregisters and closes the pane. The `agent` group (`cafleet/src/cafleet/cli/agent.py`) is a registry-only remainder: `agent register` has no live workflow consumer (`fleet create` bootstraps the root Director and Administrator via direct row inserts in `broker/fleets.py:create_fleet`, not via the CLI), and `agent deregister` exists mainly as the teardown named by `member delete`'s "no placement" error path (`cli/member.py:328`).

Design 0000114 (splitting `member` into `agent` + `pane`) was rejected and aborted; the `member` surface is canonical. This design goes the other way: fold the remaining `agent` capabilities into `member` and delete the group.

### Decisions (from user clarification)

| # | Question | Decision |
|---|----------|----------|
| 1 | Fate of the user-creatable paneless external-agent concept | **Dead.** No user-facing way to create a paneless agent remains. `broker.register_agent(placement=None)` stays as internal machinery only (fleet bootstrap invariants, tests). The self-registration bootstrap workflow in `skills/cafleet/reference/cli.md` is deleted outright. "Messages to paneless agents" degenerates to the built-in Administrator — no broker change. The restoration plan for a future A2A/HTTP registration lives only in this design doc (§ Restoration plan). |
| 2 | Introspection surface | **Both**: `member list --all` plus a new `member show`. |
| 3 | `member show` flag shape | Bare `--member-id <target>` only; the `agent show` requester flag (`--agent-id`) and its fleet-membership gate are dropped — consistent with the rest of the `member` group. |
| 4 | Drift guard | **No new guard patterns.** A guard pattern string is itself a standing mention of the dead command. `FORBIDDEN_PATTERNS` is unchanged; no anti-`cafleet agent` assertion is added anywhere. One-time migration only. |
| 5 | WebUI scope | Minimal: only the two empty-state strings naming `cafleet agent register` change (point at `cafleet member create`); the HTTP API and everything else is untouched. |

---

## Specification

### Command mapping

| Current | Target | Change kind |
|---|---|---|
| `agent register` | *(removed, no replacement)* | `member create` is the only registration path; `fleet create` bootstraps the built-ins internally. |
| `agent list` | `member list --all` | New flag on the existing command; default output unchanged. |
| `agent show` | `member show` | New subcommand; `--member-id` target, no requester gate. |
| `agent deregister` | `member delete` | Existing command extended: placementless targets become a registry soft-delete instead of an error. |

After removal, `cafleet agent …` fails with Click's standard `Error: No such command 'agent'.` (exit 2).

### `member show` (new)

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The target agent. Any **active in-fleet** agent is valid — placed or placementless (root Director and Administrator included). |
| `--full` | no | Documented flag; affects **text mode only**. |

- No tmux requirement (registry read, like `member list`).
- Cross-fleet / unknown / inactive target → `Error: Agent <member-id> not found` (exit 1) — the same string the other `--member-id` verbs use.
- Data source: `broker.get_agent(member_id, fleet_id)`, extended to also return `skills` (the `skills` list from `agent_card_json`, usually `[]` — `member create` does **not** gain a `--skills` flag; the field stays in the card format for the built-ins and pre-existing rows).
- **Default text**: the compact one-line row `<agent_id> <name> <status>` (same shape as today's `agent show` default).
- **`--full` text**: labeled block. `description` keeps the 60-codepoint truncation; `skills` renders as a compact JSON array or `-` when empty; the placement sub-block appears only when a placement row exists, `placement:   none` otherwise; a `None` field inside the placement (root Director's `director_agent_id`, a pending `pane_id`) renders `-`:

```
  agent_id:    3
  name:        Administrator
  description: <description, 60 cp + …>
  status:      active
  kind:        administrator
  skills:      -
  placement:   none
```

```
  agent_id:    2
  name:        Director
  description: Root Director for this fleet
  status:      active
  kind:        director
  skills:      -
  placement:
    director_agent_id: -
    backend:           claude
    session:           main
    window_id:         @3
    pane_id:           %0
    created_at:        2026-04-15T10:00:00+00:00
```

- **`--json`**: the broker `get_agent` dict unchanged (`agent_id`, `name`, `description`, `status`, `registered_at`, `kind`, `skills`, `placement` or `null`) regardless of `--full` — consistent with the `member` group's unprojected-JSON convention. The old `agent list`/`agent show` default-JSON projection (`id`/`name`/`description`/`status` + conditional `coding_agent`) is retired with the group.
- `kind` values: `director` (derived: `agent_id == fleets.director_agent_id`), `administrator` (`agent_card_json.cafleet.kind == "builtin-administrator"`), `monitor` (`"monitoring-member"`), `member` (everything else). This supersedes `get_agent`'s current two-value `kind` (`builtin-administrator` / `"user"`) — the WebUI consumers of `get_agent` are checked for compatibility in Step 5.

### `member list --all` (extension)

| Flag | Required | Notes |
|---|---|---|
| `--all` | no | List every **active agent** of the fleet, not just members: root Director, Administrator, monitoring member, ordinary members, and any placementless rows. Mutually exclusive with `--activity`. |

- Default behavior (no `--all`) is unchanged: members only (placement row with non-`NULL` `director_agent_id`), root Director excluded, header `N members:` / `0 members.`.
- `--all` header is `N agents:`; the table gains a `kind` column (values as in `member show`) and renders `-` in every placement column (`backend`, `session`, `window_id`, `pane_id`, `created_at`) for placementless rows:

```
4 agents:
  agent_id  name           status  kind           backend  session  window_id  pane_id  created_at
  --------  -------------  ------  -------------  -------  -------  ---------  -------  --------------------
  2         Director       active  director       claude   main     @3         %0       2026-04-15T10:00:00+00:00
  3         Administrator  active  administrator  -        -        -          -        -
  4         monitor        active  monitor        claude   main     @3         %5       2026-04-15T10:01:00+00:00
  5         alice          active  member         claude   main     @3         %7       2026-04-15T10:02:00+00:00
```

- `--json --all` returns the rows unprojected: the `list_members` row shape (`agent_id`, `name`, `description`, `status`, `registered_at`, `placement`) plus `kind`, with `placement: null` for placementless rows.
- `--all --activity` → `Error: --all and --activity are mutually exclusive.` (exit 2). Activity aggregation stays a members-only view; a combined view is out of scope.
- Broker: new `list_roster(fleet_id)` in `broker/members.py` — active agents `LEFT OUTER JOIN agent_placements`, joined against `fleets` for the `director` kind derivation, card `kind` derived in SQL via `json_extract` (same technique as `list_fleet_agents`). `list_members` / `list_members_with_activity` are untouched.

### `member delete` (extension)

The no-placement error path (`cli/member.py:328`: ``agent {member_id} has no placement; use `cafleet agent deregister` instead``) is replaced by a success path:

- **No placement row** — registry soft-delete via `broker.deregister_agent`. Header `Member deleted.`, `pane_status` `(no placement)`, exit 0. JSON `{agent_id, pane_status}` as today.
- The broker guards surface verbatim and unchanged: root Director → `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (also still guarded early in the CLI); Administrator → `Error: Administrator cannot be deregistered` (both exit 1).
- **tmux guard relaxation**: `ensure_tmux_or_die()` moves after target resolution and fires only on the pane-teardown paths (live `pane_id`). A placementless or pending-placement delete succeeds outside tmux — matching the pure-registry operation it now is.
- Exit-codes table updates accordingly: no-placement moves from `1` to `0`.
- `_load_authorized_member` loses its `placement_missing_template` parameter (delete was the only caller needing a custom message; delete now tolerates `placement is None` via its own resolution — e.g. an `allow_missing_placement` flag or a delete-local lookup); the helper's docstring reference to `cafleet agent deregister` goes with it.
- The `member create` rollback warning (`cli/member.py:89`) is re-pointed: ``Run `cafleet member delete --fleet-id <f> --member-id <id>` manually to clean up.``

### Code removal

| Removed | Detail |
|---|---|
| `cafleet/src/cafleet/cli/agent.py` | Whole module; unwire the group from `cli/main.py`. |
| `client_command(renders_agent_card=…)` branch | The `agent list` / `agent show` render path in `cli/_helpers.py`; `output.render_agents_in_result`, its only-purpose helpers `render_agent` / `_render_agent_item` (`output/render.py:118-156`), their `output/__init__.py` exports, and `output.format_register` go with it — none has a production caller outside the retired branch (the WebUI implements its own projections). The `message` group's `requires_agent_fleet` / `truncates_task_text` branches are untouched. |
| The `agent <id> not found or already deregistered.` error string | Raised only by `agent deregister`; unreachable from `member delete` (the `get_agent` active-only gate filters inactive targets first). |

Kept: `broker.register_agent` (including the `placement=None` path — internal machinery per Decision 1), `broker.list_agents` / `list_fleet_agents` / `get_agent` / `get_agent_names` (WebUI API consumers), `output.format_agent` (repurposed/extended as the `member show` formatter), `output.format_indexed_list` (message poll). The `agents` / `agent_placements` schema is unchanged — no data-model or migration work.

### Restoration plan (paneless A2A/HTTP registration)

This section is the only place the removed concept is recorded (per the removal rule). If external paneless agents return, they come back as an A2A/HTTP registration endpoint over `broker.register_agent(placement=None)` — the broker function, the paneless-tolerant message-send path, and the `agents` schema all remain capable of it. Nothing in this design needs reverting beyond re-adding a registration surface and its docs.

### Documentation-migration surface (exhaustive)

Documentation is edited **first**, per `.claude/rules/documentation-maintenance.md`. Every known edit site:

**`docs/`**

| File | Edit |
|---|---|
| `docs/spec/cli-options.md` | Subcommand-summary rows 22–25 (drop the four `agent` rows; add a `member show` row; update the `member delete` / `member list` purpose cells) · `--full` semantics table row `agent list / agent show` → `member show` (text-mode-only; JSON unprojected) · Fleet ID §: "printed by `cafleet fleet create` and `cafleet agent register`" (line 92) → `cafleet member create` · Agent ID § polarity paragraph (line 98): drop the `agent show` requester and `agent deregister` target clauses · Member ID § (line 102): add `member show`; note placement is not required for `show` / `delete` · delete the whole `## cafleet agent` section (lines 396–465) · `member` group intro (line 545): verbs become `create | delete | show | list | capture | exec | ping | nudge`; tmux requirement scoped to the pane-touching verbs · Member-resolution rule 2 (line 552): `member show` and `member delete` tolerate a missing placement · `member delete` § (646–679): replace the no-placement bullet + exit-codes row; add the Administrator guard · `member list` § (681–711): document `--all` (+ exclusivity) · new `member show` § · Error Messages table (902–942): drop the two `agent deregister`-only rows and the not-found-or-already-deregistered row; re-key `agent register` into a soft-deleted fleet → `member create`; replace the `member delete` no-placement row; add the `--all`/`--activity` exclusivity row; extend the generic not-found row with `member show` |
| `docs/concepts/member-lifecycle.md` | Line 11–12: "The Director itself is NOT a member — it registers with plain `cafleet agent register`" → bootstrapped internally by `cafleet fleet create` · Delete-ordering § (66–68): placementless agents are soft-deleted by `member delete` · Commands § (80–91): add `member show`, `member list --all` |
| `docs/spec/data-model.md` | Line 128: "card-only `agent register` calls (no placement) are likewise not enrolled" → "agents without a placement are likewise not enrolled" (column behavior stays accurate; the removed command is not named) |
| `docs/get-started/quickstart.md` (line 98) | `cafleet agent list --fleet-id 1` → `cafleet member list --fleet-id 1 --all` |
| `docs/how-to/mixed-backend-team.md` (line 108) | same replacement |
| `docs/reference/coding-agents/claude.md` (57) / `codex.md` (83) / `opencode.md` (116) | same replacement |
| sweep | grep the rest of `docs/` (incl. `docs/spec/webui-api.md`, `docs/spec/message-envelope.md`, `docs/concepts/*`) for `cafleet agent` / `agent register|list|show|deregister` / "registry-only" / "paneless … deregister" phrasing |

**`SPEC.md`** (the reimplementation contract — smallest edits that remove the drift, section structure preserved)

| Site | Edit |
|---|---|
| §6.3 shared flags (lines 845–855) | `--full` list: drop `agent list`/`show`, add `member show` · `--agent-id` polarity: drop the `agent show` requester / `agent deregister` target clauses · `--member-id` list gains `member show` (help text unchanged) |
| §6.3 `client_command` wrapper (890–913) | "(agent + message groups)" → message group; drop the `renders_agent_card` branch from the wrapper contract |
| §6.3 `agent` group (915–935) | Delete; specify `member show`, `member list --all`, and the `member delete` no-placement success path in the `member` group section |
| Line ~595 | Error-model cross-reference to the `cafleet agent deregister` CLI side → re-point at `member delete` |
| Line ~1086 | `member delete` no-placement error contract → the soft-delete success contract |
| §10 checklist (2375–2389) | Remove the four-item **`agent`** block; update `member delete`; add `member list --all` and `member show` entries |
| sweep | grep `SPEC.md` for remaining `agent register|list|show|deregister` phrasing (e.g. line 850–851, 925) |

**`README.md`** — no direct `cafleet agent` invocation known, but the CLI-surface overview must reflect the removed group and the new `member show` / `--all`; run the `/update-readme` skill (it maintains `SPEC.md` alongside).

**`skills/`**

| File | Edit |
|---|---|
| `skills/cafleet/SKILL.md` | Required Flags §: drop the `agent *` clauses (`--agent-id` requester on `agent show`, target on `agent deregister`, "`agent register` … instead returns the new agent_id") and re-describe · on-demand table row for `reference/cli.md` (line 39): drop "self-registration", "`agent list` / `show`", "`agent deregister`", "the bootstrap workflow" · Placeholder convention: `<my-agent-id>` no longer "returned by your own `cafleet agent register` call" — a member's id comes from its spawn prompt · "substitute the literal ids printed by `cafleet fleet create` / `cafleet agent register`" → `cafleet fleet create` / `cafleet member create` |
| `skills/cafleet/reference/cli.md` | Intro line 3: drop "the self-registration recipe" and "deregister" · delete § *Self-registration recipe* outright (Decision 1) · move the reserved-name `Administrator` caution to `member create --name` guidance · § *List Agents* → `member list --all` / `member show` (new flags, output contract) · § *Deregister* → `member delete` (guards unchanged) · Typical Workflow step 2: drop "Register," |
| `skills/cafleet/reference/output-flags.md` | Line 6: replace the `agent list` / `agent show` surface with `member show` (text-mode-only `--full`) |
| `skills/cafleet/reference/director.md` | "a paneless registry-only agent is torn down with `cafleet agent deregister` instead" → `member delete` handles it |
| `skills/cafleet-design-doc/create/create.md`, `execute/execute.md`, `interview/interview.md`, `create/roles/director.md`, `execute/roles/director.md` | Delete the "no separate `cafleet agent register` call" / "do not register a second Director with `cafleet agent register`" remarks entirely — the command no longer exists, so the warnings are mentions of a dead command; the load-bearing fact ("`fleet create` bootstraps the Director") stays where present |
| sweep | grep all of `skills/` for the four subcommand names |

**Project-local `.claude/` + `CLAUDE.md`** — no known mentions; sweep to confirm (they are inside the drift-guard scan surface, so any miss would also be caught by criterion 5's grep).

**`admin/src/`** — `Dashboard.tsx:131` and `Sidebar.tsx:115` empty-state strings: `cafleet agent register` → `cafleet member create` (Decision 5; no other WebUI change).

**In-code strings (`cafleet/src/`)** — `cli/member.py:61` (helper docstring), `:89` (rollback warning), `:328` (error template): covered by the `member delete` / helper changes above.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: docs/ (documentation first)

- [x] `docs/spec/cli-options.md`: subcommand summary, `--full` semantics, Fleet ID / Agent ID / Member ID sections, member-resolution rules <!-- completed: 2026-07-03T17:16 -->
- [x] `docs/spec/cli-options.md`: delete § `cafleet agent`; add § `member show`; extend § `member delete` and § `member list`; rewrite the Error Messages table rows per the spec <!-- completed: 2026-07-03T17:16 -->
- [x] `docs/concepts/member-lifecycle.md`: Director-bootstrap sentence, delete-ordering paragraph, Commands section <!-- completed: 2026-07-03T17:20 -->
- [x] `docs/spec/data-model.md` line 128 reword <!-- completed: 2026-07-03T17:20 -->
- [x] Replace the five roster examples: `docs/get-started/quickstart.md`, `docs/how-to/mixed-backend-team.md`, `docs/reference/coding-agents/{claude,codex,opencode}.md` <!-- completed: 2026-07-03T17:20 -->
- [x] Sweep the rest of `docs/` for legacy phrasing (`cafleet agent`, the four subcommands, "registry-only"/"paneless" teardown pointers) <!-- completed: 2026-07-03T17:24 -->

### Step 2: SPEC.md + README.md

- [x] `SPEC.md` §6.3: shared flags / polarity / `client_command` wrapper contract <!-- completed: 2026-07-03T17:33 -->
- [x] `SPEC.md`: delete the `agent` group section; specify `member show`, `member list --all`, `member delete` no-placement success in the `member` group section <!-- completed: 2026-07-03T17:33 -->
- [x] `SPEC.md`: error-model cross-references (~595, ~1086) + §10 checklist <!-- completed: 2026-07-03T17:33 -->
- [x] `README.md` CLI-surface update + sweep (via `/update-readme`) <!-- completed: 2026-07-03T17:42 -->

### Step 3: skills/

- [x] `skills/cafleet/SKILL.md` <!-- completed: 2026-07-03T22:29 -->
- [x] `skills/cafleet/reference/cli.md` (delete the self-registration recipe; migrate List Agents / Deregister; relocate the Administrator reserved-name caution) <!-- completed: 2026-07-03T22:29 -->
- [x] `skills/cafleet/reference/output-flags.md` <!-- completed: 2026-07-03T22:29 -->
- [x] `skills/cafleet/reference/director.md` <!-- completed: 2026-07-03T22:29 -->
- [x] `skills/cafleet-design-doc` workflow bodies + Director role files (remove the dead-command remarks) <!-- completed: 2026-07-03T22:29 -->
- [x] Sweep all of `skills/` for the four subcommand names <!-- completed: 2026-07-03T22:29 -->

### Step 4: project rules sweep

- [x] Confirm `.claude/rules/`, `.claude/skills/skill-author/`, and `CLAUDE.md` carry no `cafleet agent` mentions (none expected) <!-- completed: 2026-07-03T22:31 -->

### Step 5: broker + CLI code

- [x] `broker/members.py`: `list_roster(fleet_id)` (active agents, LEFT OUTER placement join, SQL-derived 4-value `kind`) <!-- completed: 2026-07-03T22:44 -->
- [x] `broker/agents.py`: `get_agent` returns `skills` and the 4-value `kind`; verify WebUI `get_agent` consumers tolerate both <!-- completed: 2026-07-03T22:44 -->
- [x] `cli/member.py`: `member show` (`--member-id`, `--full`, no tmux, no requester gate) + formatter (`format_agent` extension: kind / skills / placement block) <!-- completed: 2026-07-03T22:44 -->
- [x] `cli/member.py`: `member list --all` (+ `--activity` exclusivity, `N agents:` table with `kind` column and `-` placement cells) <!-- completed: 2026-07-03T22:44 -->
- [x] `cli/member.py`: `member delete` placementless soft-delete, tmux-guard relaxation, `_load_authorized_member` simplification, rollback-warning re-point (lines 61 / 89 / 328) <!-- completed: 2026-07-03T22:44 -->
- [x] Delete `cli/agent.py`; unwire from `cli/main.py`; retire `renders_agent_card` + `render_agents_in_result` + `render_agent` / `_render_agent_item` (+ their `output/__init__.py` exports) + `format_register` <!-- completed: 2026-07-03T22:44 -->

### Step 6: WebUI strings

- [x] `admin/src/components/Dashboard.tsx:131` and `Sidebar.tsx:115` → `cafleet member create` <!-- completed: 2026-07-03T22:44 -->

### Step 7: tests

- [x] Delete `cafleet/tests/cli/test_agent.py`; add the regression guard at `cafleet/tests/cli/test_agent_group_removed.py`: `cafleet agent list --fleet-id 1` exits 2 with `No such command 'agent'` (this file is the named carve-out in criterion 5 and the Step 8 grep) <!-- completed: 2026-07-03T22:38 -->
- [x] `member show` tests: found / not-found / placementless / `--full` block / `--json` shape / no tmux required <!-- completed: 2026-07-03T22:38 -->
- [x] `member list --all` tests: kind derivation (director / administrator / monitor / member), `-` placement cells, JSON shape, `--activity` exclusivity, default output unchanged <!-- completed: 2026-07-03T22:38 -->
- [x] `member delete` tests: placementless soft-delete success, Administrator guard, tmux not required off the pane paths; update `tests/cli/test_member_delete.py:498` (old error-string assertion) <!-- completed: 2026-07-03T22:38 -->
- [x] Delete `tests/output/test_render_agent.py` (it exercises the retired `render_agent` path directly); add coverage for the extended `member show` formatter; reword the `tests/cli/test_message.py:4` docstring mention <!-- completed: 2026-07-03T22:38 -->

### Step 8: verification

- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:lint-overlay`, `mise //cafleet:lint-spawn-guard` all pass <!-- completed: 2026-07-03T22:48 -->
- [x] Full-corpus grep for the exact patterns `cafleet agent `, `agent register`, `agent list`, `agent show`, `agent deregister` over `docs/ SPEC.md README.md CLAUDE.md skills/ .claude/ admin/src/ cafleet/` (excluding `design-docs/`, the retained `"cafleet agent spawn"` guard string in `spawn_prompt_guard.py` + `tests/test_spawn_prompt_guard.py`, and `cafleet/tests/cli/test_agent_group_removed.py`) returns nothing <!-- completed: 2026-07-03T22:48 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-03 | Initial draft |
| 2026-07-03 | Review round 1: named the regression-guard test (`test_agent_group_removed.py`) as a grep-gate carve-out; retired `render_agent` / `_render_agent_item` + exports; made the grep patterns exact |
| 2026-07-03 | User approved — Status: Approved |
