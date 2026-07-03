# Simplify Director Spawn Prompts and Reconcile CLI-Name Drift (Issue #155)

**Status**: Approved
**Progress**: 8/35 tasks complete
**Last Updated**: 2026-07-02

## Overview

Rewrite the CAFleet spawn-prompt skeleton so a rendered prompt carries only literal identity values (no dollar-sign env-var references, no wordy command-example blocks) and points the agent at its skills and role file. In the same cycle, fold the CLI's real `str.format` identity-substitution into the skills as the canonical mechanism, and reconcile the entire skill/doc corpus onto the shipped `cafleet member *` / `--member-id` command surface so no spawn prompt ever needs a transitional translation note again.

## Success Criteria

- [ ] No rendered spawn prompt contains a dollar-sign `CAFLEET_*` identity reference; identity appears only as CLI-substituted literals (e.g. `FLEET ID: 24`, `YOUR AGENT ID: 88`).
- [ ] The canonical spawn-prompt skeleton uses the CLI's four `str.format` placeholders (`{fleet_id}`, `{agent_id}`, `{director_agent_id}`, `{coding_agent}`) and documents that literal braces must be doubled (`{{`, `}}`).
- [ ] The `COMMUNICATION PROTOCOL` command-example block and any `INSTALLED CLI NOTE` / translation table are removed from every spawn skeleton and per-role delta.
- [ ] Every skill, role, reference, workflow, doc page, `SPEC.md`, and `README.md` describes the shipped `cafleet member create/delete/list/capture/exec/ping/nudge` surface (with `--member-id`), and the words "delivered verbatim, no placeholder substitution" and "identity via injected `CAFLEET_*` env vars" no longer appear.
- [ ] A static guard test fails if any file in the project-local edit surface (skills, docs, `SPEC.md`, `README.md`, project-local `.claude/`, `CLAUDE.md`) reintroduces `cafleet agent spawn`, the `cafleet pane` group, or a dollar-sign `CAFLEET_*` identity reference; it runs inside `mise //cafleet:test`.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

Issue #155 ("Stop using env var like stuffs") reports three problems with the prompts the Director renders when it spawns a member, all rooted in one stale mental model plus a CLI rename that never propagated into the docs.

**Problem 1 — env-var-like placeholders as literal text.** The spawn skeleton embeds dollar-sign `CAFLEET_*` identity references (`FLEET ID: $CAFLEET_FLEET_ID`, `YOUR AGENT ID: $CAFLEET_AGENT_ID`) and instructs the member to read them at runtime.

**Problem 2 — the self-id line was thought unknowable at render time.** The skills claim the member's own id is allocated during the spawn and therefore cannot be embedded, so it must arrive via an injected env var.

**Problem 3 — the prompts are too wordy.** A full `COMMUNICATION PROTOCOL` block with command examples, plus a hand-added `INSTALLED CLI NOTE` translation table, bloat every spawn.

### Decisive finding: the env-var model is fiction in 0.14.0

Source inspection (`cafleet/src/cafleet/cli/member.py`, `_prompt.py`, `_helpers.py`, and a repo-wide grep of `cafleet/src/`) establishes the actual mechanism:

- **`cafleet member create` runs `str.format` over the spawn prompt.** `resolve_prompt` (`_prompt.py:70-110`) substitutes `{fleet_id}`, `{agent_id}`, `{director_agent_id}`, and `{coding_agent}` into the chosen template (file > positional > default). `{agent_id}` **is** the member's own freshly-allocated id (`new_agent_id`), so the CLI knows and substitutes it at spawn time. Custom prompts must double literal braces to survive `.format()`.
- **No identity env var is injected into a member pane.** The only variable forwarded to the spawned pane is `CAFLEET_DATABASE_URL` (`member.py:260-261`). A grep of `cafleet/src/` for `CAFLEET_FLEET_ID` / `CAFLEET_AGENT_ID` / `CAFLEET_DIRECTOR_AGENT_ID` / `set-environment` returns nothing. There is no `$CAFLEET_AGENT_ID` or `$CAFLEET_DIRECTOR_AGENT_ID` for a member to read — those variable names do not exist at runtime.

Consequently identity reaches a member **exclusively** through the `str.format` prompt substitution. This very Drafter's spawn prompt was rendered that way (its `FLEET ID: 24` / `YOUR AGENT ID: 88` lines are CLI-substituted literals), which validates the mechanism end to end. The mechanism issue #155 asks for already ships; only the skills are stale.

### The CLI-name drift

The shipped 0.14.0 binary (and the repo source under `cafleet/src/cafleet/cli/`, which has `agent.py` + `member.py` and **no** `pane.py`) exposes the tmux-backed lifecycle under a `member` group. The entire skill/role/reference/doc corpus still documents the old `cafleet agent spawn` + `cafleet pane *` surface with `--agent-id`. This is fallout from the merged design 0000111 (cli-simplification) whose rename was never propagated. The transitional translation notes agents have been hand-adding to spawn prompts (Problem 3's wordiness) are a direct symptom.

### Shipped command surface (source of truth: `cli/member.py`, `cli/agent.py`)

| Old (still documented) | Shipped 0.14.0 | Target flag(s) |
|---|---|---|
| `cafleet agent spawn` | `cafleet member create` | `--agent-id` (the Director / actor); prompt via `str.format` |
| `cafleet agent deregister <pane-bound member>` | `cafleet member delete` | `--member-id` |
| `cafleet pane capture` | `cafleet member capture` | `--member-id` (`--lines`/`--tail`) |
| `cafleet pane exec` | `cafleet member exec` | `--member-id` (positional `COMMAND`) |
| `cafleet pane wake --poll-only` | `cafleet member ping` | `--member-id` |
| `cafleet pane wake --message` | `cafleet member nudge` | `--agent-id` (sender) + `--member-id` (target) + `--text` |
| `cafleet agent list --activity` (supervision) | `cafleet member list --activity` | fleet members with placement |

Flag polarity notes: `member create` takes `--agent-id` for the acting **Director** (not the target); every other lifecycle verb targets its member by `--member-id`; `member nudge` uniquely takes **both** `--agent-id` (sender) and `--member-id` (target). The registry-level `agent register/list/show/deregister` commands **remain** — `agent deregister` is still the correct call for a paneless registry-only agent (a member with a live pane is torn down by `cafleet member delete`).

---

## Specification

### S1 — Canonical identity mechanism: CLI `str.format` substitution

Replace the "identity via injected `CAFLEET_*` env vars" model and every "delivered verbatim — no `{placeholder}` substitution" claim with the actual contract:

- `cafleet member create` runs `str.format` over the resolved prompt (whether from `--prompt-file`, a positional argument, or the default template). It substitutes exactly four placeholders: `{fleet_id}`, `{agent_id}` (the member's own newly-allocated id), `{director_agent_id}`, `{coding_agent}`.
- An author writes a spawn prompt using those brace placeholders; the CLI renders each to a literal at spawn. **Any literal brace in prompt text must be doubled** (`{{` / `}}`) to survive `.format()`.
- An unknown placeholder raises a `UsageError` listing the four supported names (`_prompt.py:99-105`); a malformed brace raises the "double literal braces" `UsageError`.
- Two-stage rendering stays intact: the **Director** substitutes the values it already knows as literals before the call (`BASE`, the absolute role-file path, the cafleet-load purpose phrase), then the **CLI** substitutes the four identity placeholders. The Director must leave no stray single braces other than the four identity placeholders.

The `--prompt-file` audit artifact at `<BASE>/prompts/<role>-<UTC>.md` therefore carries the four `{...}` placeholders pre-substitution — that is expected and is the authoritative record of what was spawned.

### S2 — Simplified spawn-prompt skeleton

The new canonical skeleton (in `skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton*). Identity lines are CLI placeholders; `[INSERT …]` slots are Director-rendered literals; `‹…›` slots come from each skill's per-role delta:

```text
You are ‹ROLE TITLE› in a ‹TEAM NAME› team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/‹role›.md] with the Read tool BEFORE any other action. That file is your authoritative role definition.‹ROLE-DEF SUFFIX› Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the cafleet skill — ‹cafleet-load purpose›
‹EXTRA SKILL LOADS›

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path]
CODING AGENT: {coding_agent}
‹CONTEXT LINES›

‹IMPORTANT / ROLE-CONSTRAINT LINES›

‹START CUE›
```

Changes from the current skeleton:

- **Self-id line kept.** `YOUR AGENT ID: {agent_id}` stays as an explicit labeled line, filled by the CLI at spawn (`{agent_id}` = the member's own `new_agent_id`, `_prompt.py:93-97`). It is required: `cafleet message poll`/`send` both need `--agent-id`, and no env var supplies it. The rejected alternative — drop the self-id line and have the member read its own id at runtime — is not viable precisely because no such env var exists (see the Background finding); the id must be embedded, so the labeled line stays.
- **Identity lines use CLI placeholders**, not `$CAFLEET_*`. After spawn the member sees `FLEET ID: 24`, `DIRECTOR AGENT ID: 84`, `YOUR AGENT ID: 88`, `CODING AGENT: claude`.
- **`COMMUNICATION PROTOCOL` block removed.** The member learns poll/send/ack command shapes from the `cafleet` skill and its role file — not from inline prompt examples. The prose that previously explained `$CAFLEET_*` runtime resolution is deleted.
- **No `INSTALLED CLI NOTE` / translation table.** With the corpus reconciled onto `member *`, there is nothing to translate.

The per-role delta table, the lossless rule (every `IMPORTANT:` line, hard role-constraint, and start cue survives verbatim), the `--prompt-file` audit-write protocol, the `${BASE} == <unset>` guarded-skip, the spawn-size limit, and the backtick caveat are all retained — only the identity/communication framing changes.

### S3 — CLI-name reconciliation

Sweep every occurrence of the old surface to the shipped surface across the repo, using the mapping table in Background. Rules:

- `cafleet agent spawn` → `cafleet member create`; `cafleet pane <verb>` → the matching `member` verb; pane-teardown `agent deregister` → `member delete`; supervision `agent list --activity` → `member list --activity`. `--agent-id` in a **pane/member-lifecycle target** position → `--member-id` (leave `--agent-id` untouched in `message *`, `agent show`, and the `member create`/`member nudge` actor/sender positions).
- Rename the reference page `skills/cafleet/reference/director.md`'s title/framing from "`cafleet agent *` / `cafleet pane *`" to the `member` group; the file itself may keep its `director.md` filename.
- Follow the removal rule (`~/.claude/rules/removal.md`): after the sweep the corpus reads as if the old names never existed. The only place the old names legitimately remain is this design doc and git history.
- **Scope boundary (per user answer Q3b):** edit the **project-local** tree — the repo `skills/` tree, `docs/`, `SPEC.md`, `README.md`, the project-local `.claude/` (its `rules/` and the project-local `.claude/skills/skill-author/` skill), and `CLAUDE.md`. Do **not** edit the promoted `~/.claude/skills/` copies — those are re-synced by `cafleet setup` (design 0000109). The project-local files are the source of truth; the promoted copies are downstream.
- **Source (per user answer Q3a):** prefer **no** source change — `_prompt.py` and `member.py` already use the correct placeholders and command names. Touch source only if a concrete drift strictly requires it (none is currently identified).
- **Sibling design 0000112 (per user answer Q4):** 0000112 landed before this design was executed (merged PR #156); the installed CLI's `member` group no longer exposes `send-input`. Document the CLI as it now ships: the reconciled corpus describes `member create/delete/list/capture/exec/ping/nudge` and does **not** document `send-input`.

### S4 — Static drift-guard test

Add a static checker modeled on the existing overlay-coverage guard (`cafleet/tests/coding_agent/test_overlay_coverage.py` + `mise //cafleet:lint-overlay`): a pure `check_spawn_prompt_drift()` function plus a pytest wrapper and a `mise` lint task wired into `mise //cafleet:test`.

The checker scans the full project-local edit surface — the repo `skills/` tree, `docs/`, `SPEC.md`, `README.md`, the project-local `.claude/` (`rules/` + `skills/skill-author/`), and `CLAUDE.md` — excluding `design-docs/` (the historical record) and the promoted `~/.claude/` copies (not part of this repo), and returns a violation for any occurrence of:

| Forbidden pattern | Why |
|---|---|
| `$CAFLEET_FLEET_ID`, `$CAFLEET_AGENT_ID`, `$CAFLEET_DIRECTOR_AGENT_ID` | dollar-sign identity env refs — the fiction #155 removes; the bare (dollar-less) `CAFLEET_FLEET_ID` name in the CLI env-var catalog is **not** matched |
| `cafleet agent spawn` | removed command; now `cafleet member create` |
| `cafleet pane ` (with trailing space / subcommand) | removed command group; now `cafleet member *` |

Each violation names the file, line, and matched pattern. The pytest module exercises the pure function with crafted passing/failing inputs (so a no-op implementation cannot pass) and asserts the live tree is clean. The scan scope is exactly the S3 project-local edit surface, so every file the reconciliation touches — including the Step-5 targets `.claude/rules/` and the project-local `.claude/skills/skill-author/` — is guarded against regression.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first order per `.claude/rules/documentation-maintenance.md`. Use a repo-wide grep for `cafleet agent spawn`, `cafleet pane`, `$CAFLEET_`, and "delivered verbatim" as the authority on remaining occurrences at each step — the file lists below are the known surface, not a substitute for the grep.

### Step 1: Concept & how-to docs

- [x] Reconcile `docs/concepts/member-lifecycle.md`, `monitoring.md`, `tmux-push.md`, `bash-routing.md`, `token-reduction.md`, `coding-agents.md` onto the `member *` surface <!-- completed: 2026-07-03T11:01 -->
- [x] Reconcile `docs/how-to/monitor-and-recover.md` (pane capture/exec/wake, agent deregister → member verbs) <!-- completed: 2026-07-03T11:04 -->
- [x] Reconcile `docs/get-started/quickstart.md` and `get-started/configure.md` examples <!-- completed: 2026-07-03T11:07 -->
- [x] Reconcile `docs/reference/coding-agents/{claude,codex,opencode}.md` (agent spawn / pane exec / agent deregister) <!-- completed: 2026-07-03T11:12 -->
- [x] Reconcile `docs/concepts/overview.md` (`cafleet agent spawn` → `member create`) <!-- completed: 2026-07-03T11:15 -->
- [x] Reconcile `docs/how-to/mixed-backend-team.md` (four `cafleet agent spawn` command examples → `member create`) <!-- completed: 2026-07-03T11:15 -->
- [x] Reconcile `docs/spec/data-model.md` (`cafleet agent spawn --role monitor` → `member create --role monitor`; any `pane *` / `--agent-id` lifecycle references) <!-- completed: 2026-07-03T11:17 -->
- [x] Reconcile `docs/spec/cli-options.md`: rename `agent spawn`/`pane *` sections to `member *`, fix `--agent-id`→`--member-id` in lifecycle contexts, remove the injected-`CAFLEET_*` identity paragraph, document `member create` `str.format` substitution + double-brace rule <!-- completed: 2026-07-03T11:33 -->

### Step 2: README and SPEC

- [ ] Update `README.md`: `member *` command surface, monitoring-member spawn via `member create`, remove injected-env-var identity narrative <!-- completed: -->
- [ ] Update `SPEC.md`: rewrite the spawn/pane contract sections to `member create/delete/list/capture/exec/ping/nudge`; replace the "delivered verbatim, no substitution" default-prompt text with the `str.format` contract (four placeholders, double-brace rule); fix `--agent-id`→`--member-id` polarity in lifecycle contexts <!-- completed: -->

### Step 3: cafleet skill (SKILL.md, reference, roles)

- [ ] Rewrite `skills/cafleet/SKILL.md` § *Spawned-member identity*: retitle away from "via `CAFLEET_*` env vars", document the `str.format` mechanism (four placeholders, `{agent_id}` = the member's own id, double-brace rule), and correct the fleet-id/agent-id examples <!-- completed: -->
- [ ] Rewrite `skills/cafleet/reference/director.md`: retitle to the `member *` group; replace `agent spawn`/`agent deregister`/`pane *` sections with `member create/delete/capture/exec/ping/nudge/list`; install the S2 simplified skeleton; delete every "verbatim / no `{placeholder}` substitution" statement; document the `str.format` + double-brace contract <!-- completed: -->
- [ ] Reconcile `skills/cafleet/reference/supervision.md` (spawn protocol, monitor lifecycle, `member list --activity`, teardown verbs) <!-- completed: -->
- [ ] Reconcile `skills/cafleet/reference/recovery.md` (teardown/shutdown → `member delete`) <!-- completed: -->
- [ ] Reconcile `skills/cafleet/reference/exec-routing.md` (`pane exec`/`pane wake --poll-only` → `member exec`/`member ping`) <!-- completed: -->
- [ ] Reconcile `skills/cafleet/reference/cli.md` (command catalog; keep the dollar-less `CAFLEET_*` CLI env-var catalog accurate, remove any injected-identity claim) <!-- completed: -->
- [ ] Reconcile `skills/cafleet/reference/broadcast.md` and `reference/output-flags.md` if they name old commands <!-- completed: -->
- [ ] Reconcile `skills/cafleet/roles/monitor.md`: identity block, spawn/teardown verbs, remove `$CAFLEET_*` references <!-- completed: -->
- [ ] Reconcile `skills/cafleet/roles/member.md`: identity block, poll/send shapes read from skill (not `$CAFLEET_*`) <!-- completed: -->
- [ ] Reconcile `skills/cafleet/roles/director.md` (if present) <!-- completed: -->
- [ ] Reconcile the coding-agent overlays `reference/coding-agent/{claude,codex,opencode,_template}.md` if they name old commands or `$CAFLEET_*` identity vars <!-- completed: -->

### Step 4: cafleet-design-doc and cafleet-research skills

- [ ] Reconcile `skills/cafleet-design-doc/SKILL.md` and the `create`/`execute`/`interview` workflow spawn sections + their `roles/director.md` (spawn verbs, skeleton reference, `$CAFLEET_*` removal) <!-- completed: -->
- [ ] Reconcile the create-workflow role files under `skills/cafleet-design-doc/create/roles/` (drafter/reviewer identity blocks, COMMUNICATION PROTOCOL references) <!-- completed: -->
- [ ] Reconcile `skills/cafleet-research/SKILL.md` and the `report`/`presentation` workflow spawn sections + their `roles/director.md` <!-- completed: -->
- [ ] Reconcile `skills/cafleet-design-doc/reference/coordination.md` and any `cafleet-research` reference pages that name old commands <!-- completed: -->

### Step 5: project rules

- [ ] Reconcile `.claude/rules/bash-tool.md`: `pane exec`/`pane wake --poll-only` → `member exec`/`member ping`, `agent spawn`/`agent deregister` → `member create`/`member delete`, and the Director-side dispatch primitives <!-- completed: -->
- [ ] Scan `CLAUDE.md`, `.claude/rules/commands.md`, and `skill-author` skill for old-surface references and reconcile <!-- completed: -->

### Step 6: static drift-guard test

- [ ] Add `cafleet/src/cafleet/spawn_prompt_guard.py` (or equivalent module) with a pure `check_spawn_prompt_drift()` returning file/line/pattern violations for the three forbidden patterns over the project-local edit surface (`skills/`, `docs/`, `SPEC.md`, `README.md`, project-local `.claude/rules/` + `.claude/skills/skill-author/`, `CLAUDE.md`), excluding `design-docs/` and the promoted `~/.claude/` copies <!-- completed: -->
- [ ] Add `cafleet/tests/.../test_spawn_prompt_guard.py`: crafted passing/failing inputs for the pure function + a live-tree "clean" assertion <!-- completed: -->
- [ ] Add a `mise //cafleet:lint-spawn-guard` task and wire it into `mise //cafleet:test`; document both in `.claude/rules/commands.md` <!-- completed: -->

### Step 7: source (only if strictly required)

- [ ] Verify `_prompt.py` default `MEMBER_PROMPT_TEMPLATE` and `member.py` user-facing strings need no change; make the minimal edit only if a concrete drift is found (expected: none) <!-- completed: -->

### Step 8: verification and finalize

- [ ] Run repo-wide grep for `cafleet agent spawn`, `cafleet pane`, `$CAFLEET_FLEET_ID`/`$CAFLEET_AGENT_ID`/`$CAFLEET_DIRECTOR_AGENT_ID`, and "delivered verbatim" — zero hits outside `design-docs/` and git history <!-- completed: -->
- [ ] `mise //cafleet:test` (includes the new guard + overlay coverage) passes <!-- completed: -->
- [ ] `mise //cafleet:lint` and `mise //cafleet:typecheck` pass <!-- completed: -->
- [ ] Commit the design doc on the feature branch per `.claude/rules/git-workflow.md` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-02 | Initial draft |
| 2026-07-03 | Director validation: sibling design 0000112 landed first (PR #156) — `member send-input` no longer ships; removed it from the target surface (Success Criteria, Background table, S3, Step 2) per the doc's own Q4 contingency |
