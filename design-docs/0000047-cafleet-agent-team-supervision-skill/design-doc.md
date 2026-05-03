# CAFleet Agent Team Supervision & Monitoring Skills

**Status**: Approved
**Progress**: 0/16 tasks complete
**Last Updated**: 2026-05-03

## Overview

Introduce two CAFleet-native skills, `cafleet:agent-team-monitoring` (the mechanism: how to actively facilitate a CAFleet team using cron-like wake-ups) and `cafleet:agent-team-supervision` (the governance layer that depends on monitoring and adds always-applicable obligations including the new Authorization-Scope Guard). Make both skills backend-aware: Claude Code Directors use `CronCreate` / `ScheduleWakeup` for the active loop, codex Directors cannot run an in-session loop at all (codex CLI exposes no scheduling primitive — confirmed via survey, see Background §2). The existing `cafleet-monitoring` skill is renamed to `cafleet:agent-team-monitoring` and reorganized into backend-specific sections; supervision is a new skill that builds on top.

## Success Criteria

- [ ] `skills/agent-team-monitoring/SKILL.md` exists and contains: when-to-monitor rules, the active-loop mechanism described per-backend (Claude Code: `CronCreate` / `ScheduleWakeup` + `/loop` template; codex: documented absence + fallback guidance), the health-check sequence, and team-facilitation instructions (dispatch queued work, nudge stalled members, escalate after N attempts).
- [ ] `skills/agent-team-supervision/SKILL.md` exists and: (a) declares a hard dependency on `agent-team-monitoring` ("Load `Skill(agent-team-monitoring)` first" stated in the SKILL preamble); (b) covers Core Principle, Communication Model, Idle Semantics, **Authorization-Scope Guard**, Spawn Protocol, User Delegation, Stall Response, Cleanup; (c) does not duplicate the `/loop` Prompt Template — it cross-references monitoring.
- [ ] The Authorization-Scope Guard section lives in supervision and explicitly forbids the "Skipping. Holding for go." failure mode and defines what counts as a real stop signal versus absence of confirmation.
- [ ] The monitoring skill's backend-aware section explicitly states: "Codex CLI has no in-session scheduling primitive. A codex root Director cannot run the active `/loop` monitor. Recommendation: use `--coding-agent claude` for the root Director when active supervision is required; codex members are fully supported." Lists the fallback options for codex Directors (out-of-band cron driver, MCP scheduling server, user-driven manual nudges).
- [ ] `skills/cafleet-monitoring/` is deleted; every reference in the repo points at the new skill names.
- [ ] Project `CLAUDE.md` and `.claude/CLAUDE.md` skill listings replace the `/cafleet-monitoring` entry with two entries (`/agent-team-monitoring` and `/agent-team-supervision`) and document the load order (monitoring first, supervision second).
- [ ] Every `Skill(cafleet-monitoring)` mention in other skills (notably `cafleet:design-doc-create`, `cafleet:design-doc-execute`, `cafleet:design-doc-interview`, `cafleet:cafleet`) is updated to load both new skills in the correct order.
- [ ] Plugin manifest / packaging (`cafleet/pyproject.toml`, plugin metadata, anything that enumerates shipped skills) lists both new skills and omits the old one.
- [ ] Regression checks: `grep -rn "cafleet-monitoring" .` outside `.git/` and this design doc returns zero hits; `grep -rn "Skipping\. Holding for go" .` outside `.git/` and this design doc returns zero hits.

---

## Background

### 1. Failure mode that motivated this work

In session `0e66c1c3-9606-4633-b034-723d02b6c26d` (codex coding-agent design doc execution), the Director repeatedly emitted `Skipping. Holding for go.` while the Programmer (`4182be20-…`) and Tester (`253dd701-…`) members sat idle with empty inboxes. The user authorized the workstream when they kicked it off, but the Director treated each scheduled `/loop` tick as a fresh permission gate and refused to dispatch queued work (review-comment routing) without explicit re-confirmation. Result: members starved, wall time wasted, user escalated with profanity.

The existing `skills/cafleet-monitoring/SKILL.md` already states `If you stop giving instructions, the entire team stops.` — but it has no rule that distinguishes "user told me to stop" from "user has not said 'go' again in the last minute." The Director conflated the two and self-blocked.

### 2. Survey: in-session scheduling primitives by backend

The active `/loop` monitor today relies on Claude Code's `CronCreate` (cron-scheduled in-session prompt enqueue) and `ScheduleWakeup` (self-pacing dynamic-mode wake-up). These are tools the harness exposes to the model; they are NOT model features. With design `0000046` adding `codex` as a second supported coding-agent backend, the equivalent codex primitive needs to be identified.

Survey result (sources: [Codex CLI features](https://developers.openai.com/codex/cli/features), [Codex Automations](https://developers.openai.com/codex/app/automations), [Codex CLI overview](https://developers.openai.com/codex/cli)):

| Backend | In-session scheduling / cron / wakeup primitive | Notes |
|---|---|---|
| Claude Code CLI | `CronCreate` (cron-scheduled in-session prompt enqueue), `ScheduleWakeup` (self-pacing dynamic-mode wake-up) | Built-in harness tools. Power the `/loop` skill. |
| Codex CLI (local terminal binary) | **None.** No scheduler tool, no cron tool, no self-wake tool the model can invoke from inside a running CLI session. | Confirmed via [Codex CLI Features doc](https://developers.openai.com/codex/cli/features). |
| Codex app (web / cloud) | "Automations" with cron-syntax custom schedules | App-only. Not callable from the local codex CLI. Quote from upstream: "For project-scoped automations, the app needs to be running, and the selected project needs to be available on disk." |
| Codex CLI + MCP server | Theoretically possible — codex supports MCP servers; a custom MCP server could expose a scheduling tool | Not built-in, requires user setup. Out of scope for this design. |

**Implication:** A codex root Director cannot run the active `/loop` monitor at all. The mechanism the existing `cafleet-monitoring` skill relies on (claude `CronCreate`) is unavailable on the codex backend. The monitoring skill must be backend-aware, not backend-blind.

### 3. Why the existing single-skill design no longer fits

`skills/cafleet-monitoring/SKILL.md` mixes three concerns into one file:

1. The active-loop mechanism (`CronCreate` + `/loop` template) — backend-specific (claude only).
2. Team-facilitation instructions (health-check sequence, stall ladder, dispatch logic) — backend-neutral but mechanism-coupled.
3. Always-applicable supervision obligations (Core Principle, idle semantics, cleanup, the missing Authorization-Scope Guard) — backend-neutral and mechanism-independent.

Conflating these prevents the Authorization-Scope Guard from having a natural home, and forces backend-aware logic to splinter across sections. Splitting along the supervision/monitoring boundary — and reversing the dependency direction so supervision loads monitoring — gives each skill a coherent scope:

- **monitoring** owns the mechanism + facilitation instructions; backend-aware.
- **supervision** owns the always-applicable obligations + the new Authorization-Scope Guard; loads monitoring as a prerequisite.

This is the inverse of the global `~/.claude/skills/agent-team-supervision` / `~/.claude/skills/agent-team-monitoring` split (where supervision is the always-load and monitoring is the optional add-on). The reversal is intentional for CAFleet: in CAFleet, monitoring is **how supervision is actually performed**, not an optional add-on. There is no useful CAFleet supervision pattern that doesn't go through the monitoring mechanism — even codex Directors, who can't run the loop, get their facilitation guidance from the monitoring skill (its codex section documents the absence and the fallback options).

---

## Specification

### 1. Two-skill design with reversed dependency

| Skill | Role | Loads / depends on | Backend awareness |
|---|---|---|---|
| `cafleet:agent-team-monitoring` | Foundational mechanism + team-facilitation instructions. The "how to drive the team to completion" layer. | None (lowest layer in the cafleet skill stack) | **Backend-aware.** Per-backend sections for Claude Code (with `CronCreate` / `ScheduleWakeup`) and codex (no in-session scheduling). |
| `cafleet:agent-team-supervision` | Governance layer. Always-applicable obligations + the new Authorization-Scope Guard. | Loads `Skill(agent-team-monitoring)` as a hard prerequisite — stated in the SKILL preamble. | Backend-neutral (the obligations apply identically to both backends; the mechanism that fulfills them is in monitoring). |

**Load order** in any consuming skill (e.g. `cafleet:design-doc-execute`):

```text
Skill(agent-team-monitoring)   # mechanism layer — loaded first
Skill(agent-team-supervision)  # governance layer — loaded second, references monitoring
```

The supervision skill's preamble says explicitly: `Load Skill(agent-team-monitoring) before this one — the mechanism it documents is what supervision is performed through.`

#### Why not the alternatives

| Option | Result | Decision |
|---|---|---|
| A. Two skills, supervision depends on monitoring (this design) | Authorization-Scope Guard lives in supervision; monitoring owns the per-backend mechanism details; load order matches the conceptual layering (mechanism → governance) | **Chosen** |
| B. Two skills, monitoring depends on supervision (mirrors global split) | Symmetric with global skills, but in CAFleet supervision can't actually be performed without monitoring (members don't act autonomously and the Director needs the loop or an explicit fallback). The dependency arrow is backwards relative to how the layers are used. | Rejected per user feedback — supervision is performed THROUGH monitoring in CAFleet, not alongside it. |
| C. Single skill (absorb monitoring into supervision) | One file to load, but mixes per-backend mechanism details with backend-neutral obligations and forces the Authorization-Scope Guard to share scope with the `/loop` template | Rejected per user feedback — the global pair is split for a reason and the same reasoning applies here. |
| D. Keep `cafleet-monitoring` (loop) and add new `agent-team-supervision` (fundamentals) without renaming | Adds the new file without removing the old name; misleading "monitoring only" name persists | Rejected — violates `~/.claude/rules/removal.md` (no aliases / "see also" pointers). The old name must go. |

### 2. `cafleet:agent-team-monitoring` content outline

`skills/agent-team-monitoring/SKILL.md` is the foundational layer. Section order and intent:

| § | Section | Source / change |
|---|---|---|
| 1 | Frontmatter (`name: agent-team-monitoring`, `description: …`) | New, mirrors global naming. Description names the cron-like mechanism explicitly so its scope is obvious. |
| 2 | Placeholder convention | Carried verbatim from `cafleet-monitoring` § Placeholder convention |
| 3 | Mechanism by backend (NEW) | New — sub-sections per backend. See §3 below for the verbatim content. |
| 4 | Team-facilitation instructions | New consolidation of the existing health-check sequence + the explicit "dispatch queued work" rule. Tells the Director: when the loop fires (claude) or when you take a turn (codex), check for queued work first, dispatch it, then run health checks. The instruction "facilitate the team to make tasks finished" is encoded as: 5 imperative steps the Director executes on every supervision tick (poll inbox → ACK → dispatch queued work → run health checks → escalate if needed). |
| 5 | Health-check sequence (the 5-row table currently in `cafleet-monitoring`) | Carried from `cafleet-monitoring` § Monitoring Mandate table. |
| 6 | `/loop` Prompt Template (Claude Code only) | Carried verbatim from `cafleet-monitoring` § `/loop` Prompt Template. Marked clearly as "Claude Code-specific — codex Directors cannot use this." |
| 7 | Loop Lifecycle (spawn / run / user-review / cleanup phase rules) | New — adapted from global `agent-team-monitoring` § Loop Lifecycle, expressed in cafleet primitives. Reinforces "stop the loop BEFORE `member delete`" by reference to `Skill(cafleet)` § Shutdown Protocol. |
| 8 | Stall Response (2-stage health check + escalation table) | Carried from `cafleet-monitoring` § Stall Response, with the response-channels table preserved. |

### 3. Mechanism by backend — verbatim section to embed in monitoring

This is the new backend-aware section. Embed in `skills/agent-team-monitoring/SKILL.md` as written (markdown):

```markdown
## Mechanism by backend

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. The mechanism for this differs between coding-agent backends.

### Claude Code Director (default)

Claude Code's harness exposes two in-session scheduling tools the Director can call:

| Tool | Use |
|---|---|
| `CronCreate` | Schedule a recurring prompt to fire at a cron interval. Used to set up the active `/loop` monitor (typically 1-minute cadence). |
| `ScheduleWakeup` | Self-pacing dynamic-mode wake-up — used by `/loop` when the Director picks its own next-tick delay. |

The `/loop` Prompt Template (§6) is the canonical setup — it uses `CronCreate` under the hood. **The loop is mandatory before any `cafleet member create` call** when the Director runs under Claude Code.

### Codex Director

Codex CLI exposes **no equivalent in-session scheduling primitive** (confirmed via survey of <https://developers.openai.com/codex/cli/features> as of 2026-05). There is no `CronCreate`, no `ScheduleWakeup`, no harness-level tool the model can invoke from inside a running session to schedule a future tick. Codex Automations exist but are app-only and cannot be triggered from the local CLI.

This means: **a codex root Director cannot run the active `/loop` monitor.** The mechanism is unavailable.

#### Recommendation

When active supervision of a CAFleet member team is required, **use `cafleet session create --coding-agent claude`** for the root Director. Codex members are fully supported via `cafleet member create --coding-agent codex` — only the Director needs the cron mechanism. A mixed-backend team (claude Director + codex members) is the canonical configuration for active-supervision workloads.

#### Fallback options when codex must be the Director

If a codex root Director is required (e.g. operator preference, codex-specific workflow, no claude binary available), one of the following fallbacks must be in place. Active supervision **without** one of these fallbacks is not supported.

| Fallback | Mechanism | Operational cost |
|---|---|---|
| **Out-of-band cron driver** | An OS-level scheduler (`cron(8)`, systemd timer, `watch -n 60 …`) keystrokes the supervision-tick prompt into the codex Director's tmux pane via `tmux send-keys`. | Operator must set up + tear down the timer; not visible inside the session. |
| **MCP scheduling server** | Codex CLI supports MCP servers. A custom MCP server can expose a scheduling tool the codex Director invokes inline. | Requires writing or installing an MCP server; configuration lives in `~/.codex/config.toml`. |
| **User-driven nudges** | The user types a tick prompt at intervals (e.g. "tick" every minute). | Manual, error-prone, doesn't scale beyond short sessions. |
| **No active monitor — synchronous in-turn facilitation only** | The Director performs all health checks + dispatch within each of its own active turns; no scheduled wake-up. The team only progresses while the Director has an active turn. | Acceptable only for short, fully-Director-driven workflows (no long-running parallel members). The Director's idle window is the team's idle window. |

The fallback in use must be documented in the session's launch instructions. The supervision skill's Authorization-Scope Guard applies regardless of which fallback is in use.
```

### 4. `cafleet:agent-team-supervision` content outline

`skills/agent-team-supervision/SKILL.md` is the governance layer. Its preamble declares the dependency on monitoring. Section order and intent:

| § | Section | Source / change |
|---|---|---|
| 1 | Frontmatter (`name: agent-team-supervision`, `description: …`) | New, mirrors global naming. Description names the dependency: "Load `Skill(agent-team-monitoring)` first." |
| 2 | Preamble — dependency declaration | New — first paragraph after the title: `This skill builds on Skill(agent-team-monitoring). Load monitoring first — it documents the cron-like mechanism that supervision is performed through. Supervision adds the always-applicable obligations and the Authorization-Scope Guard.` |
| 3 | Core Principle | Carried from `cafleet-monitoring` § Core Principle, lightly reworded |
| 4 | Communication Model | New — explains the broker auto-fire + member pane injection chain (Director `message send` → broker persists task → broker keystrokes `cafleet message poll` into recipient pane via `tmux send-keys` → member's next turn picks it up). Mirrors the global skill's `Communication Model` section in shape. |
| 5 | Idle Semantics | New — adapted from global `Idle Semantics`. Members go idle after every turn; idle ≠ stalled; do not nudge solely because a member is idle. |
| 6 | **Authorization-Scope Guard** | **New — see §5 below for full text.** This is the section that closes the failure mode in Background §1. Lives **here**, not in monitoring. |
| 7 | Spawn Protocol | Carried from `cafleet-monitoring` § Spawn Protocol. The "ensure `/loop` is already running" step links to `Skill(agent-team-monitoring)` for the loop setup itself, with the codex caveat: "if the Director runs under codex, ensure one of the fallbacks listed in `Skill(agent-team-monitoring)` § Mechanism by backend is in place." |
| 8 | User Delegation Protocol | New — adapted from global `User Delegation Protocol`. Members never talk to the user; Director relays via `AskUserQuestion`; pass user answers through verbatim via `cafleet message send`; for `AskUserQuestion`-shaped pane prompts, follow the three-beat workflow in `Skill(cafleet)` § *Answer a member's AskUserQuestion prompt*. |
| 9 | Stall Response (cross-reference only) | New short section that defers to `Skill(agent-team-monitoring)` § Stall Response for the response-channels table — supervision does not duplicate it. |
| 10 | Cleanup Protocol | New short section that defers to `Skill(cafleet)` § Shutdown Protocol for the canonical teardown order. Restates only the rule "stop the loop / out-of-band scheduler BEFORE deleting members". |
| 11 | Quick Reference table | New — mirrors global, expressed in cafleet primitives. |

### 5. The Authorization-Scope Guard — verbatim section to embed in supervision

```markdown
## Authorization-Scope Guard (CRITICAL)

**Absence of confirmation is not a stop signal.** When the user authorizes a
workstream ("execute the design doc", "process the review comments",
"continue with step N"), that authorization persists across `/loop` ticks
(claude), out-of-band scheduler firings (codex fallback), idle notifications,
and your own tool-result interpretations until the user issues an explicit
stop signal.

**You MUST dispatch queued work as soon as a teammate is idle and inputs are
available.** Examples of queued work that the Director MUST route immediately:

- Review comments waiting to be split across Programmer / Tester
- Next implementation step in the active design doc
- Reviewer feedback waiting to land at the Drafter
- A teammate's `cafleet message send` reply waiting to be ACK'd and acted on

**Do NOT** emit `Skipping. Holding for go.`, `Waiting for user confirmation
before dispatching`, or any equivalent passive-hold message in response to a
supervision tick or idle notification. The tick is a *health check*, not a
permission renewal request.

### What counts as a real stop signal

Only these gestures revoke prior authorization. Anything else is noise:

| Signal | Action |
|---|---|
| Explicit "stop", "wait", "halt", "pause" from the user | Stop dispatching; acknowledge; wait for the next instruction. |
| Profanity / frustration directed at your last action | Stop; acknowledge briefly; wait. Do not continue scheduled firings. |
| Repeated rejection of your tool calls (≥2 denials of the same operation in a row) | Stop the operation; surface the blocker; wait for guidance. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a new instruction. |

`/loop` cron firings, out-of-band scheduler firings, teammate idle
notifications, broker auto-fire receipts, and the absence of a fresh "go"
message are **not** stop signals. Treat them as inputs to evaluate, not gates
to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made
(choosing between options, approving a risky / remote-visible operation,
disambiguating a teammate's question), use `AskUserQuestion` — do **not**
emit a passive hold and wait. The hold message produces nothing; the question
unblocks you within seconds.

See `~/.claude/rules/skill-discovery.md` § *Authorization scope* and § *Stop
means stop* for the project-wide policy this section enforces.
```

### 6. File operations summary

| File | Operation |
|---|---|
| `skills/agent-team-monitoring/SKILL.md` | Create (full content per §2 + §3) |
| `skills/agent-team-supervision/SKILL.md` | Create (full content per §4 + §5) |
| `skills/cafleet-monitoring/` (whole directory) | Delete |
| `skills/design-doc-create/SKILL.md` | Replace `Skill(cafleet-monitoring)` references: load `Skill(agent-team-monitoring)` first, then `Skill(agent-team-supervision)`. Both are required at every site that previously loaded `cafleet-monitoring`. |
| `skills/design-doc-execute/SKILL.md` | Same pattern. |
| `skills/design-doc-interview/SKILL.md` | Same pattern. |
| `skills/cafleet/SKILL.md` | Same pattern. |
| `CLAUDE.md` | Replace the `/cafleet-monitoring` skill bullet with two bullets: `/agent-team-monitoring` and `/agent-team-supervision`, in that order. Document the load order. |
| `.claude/CLAUDE.md` | Same as above. |
| `cafleet/pyproject.toml` (plugin manifest, if it enumerates skills) | Update skill list. |
| Any other `pyproject.toml` / plugin manifest that ships skills | Update skill list. |

### 7. Skill names and load directives

| Property | `agent-team-monitoring` | `agent-team-supervision` |
|---|---|---|
| Skill `name` (frontmatter) | `agent-team-monitoring` | `agent-team-supervision` |
| Plugin-namespaced load form | `Skill(cafleet:agent-team-monitoring)` for plugin consumers; `Skill(agent-team-monitoring)` works inside this repo's own skills | `Skill(cafleet:agent-team-supervision)` / `Skill(agent-team-supervision)` |
| User-invocable slash form | `/cafleet:agent-team-monitoring` (rarely invoked directly) | `/cafleet:agent-team-supervision` (rarely invoked directly) |
| Description (frontmatter) | `Active monitoring mechanism for CAFleet Directors. Documents the cron-like loop primitive per backend (Claude Code: CronCreate + /loop; codex: no in-session scheduling, fallback options listed) and the team-facilitation instructions (poll, ACK, dispatch queued work, health-check, escalate). Foundation layer — load before agent-team-supervision.` | `Governance layer for CAFleet Directors. Loads agent-team-monitoring as a hard prerequisite. Defines Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, User Delegation, Stall Response (cross-reference), and Cleanup. Load both skills before any 'cafleet member create' call.` |

### 8. Out of scope

- Changes to the cafleet CLI itself (`cafleet member …` / `cafleet message …` / `cafleet session …`) — the new skills describe existing primitives.
- Changes to the `/loop` cron infrastructure — the existing 1-minute interval and prompt-template mechanism are reused as-is for Claude Code Directors.
- Building an out-of-band cron driver or MCP scheduling server for codex Directors — those are documented as fallback options operators implement themselves; this design only specifies that the supervision skill calls them out.
- Changes to the global `agent-team-supervision` / `agent-team-monitoring` skills — the in-process Agent Teams variants are unaffected.
- Migration tooling for in-flight sessions — no state is persisted under the old skill name; the rename is a pure documentation change.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Author the monitoring skill (foundation layer first)

- [ ] Read `skills/cafleet-monitoring/SKILL.md` (current source) and both `~/.claude/skills/agent-team-supervision/SKILL.md` and `~/.claude/skills/agent-team-monitoring/SKILL.md` (global counterparts) end to end. <!-- completed: -->
- [ ] Create `skills/agent-team-monitoring/SKILL.md` per Specification §2 with the section order listed there. The Mechanism by backend section (§3) MUST be embedded verbatim from §3 of this design doc, including the codex survey result and the fallback options table. <!-- completed: -->
- [ ] Verify the file has the frontmatter from §7 (`name`, `description`) and contains both the Claude Code `/loop` Prompt Template AND the codex per-backend section. <!-- completed: -->

### Step 2: Author the supervision skill (governance layer second)

- [ ] Create `skills/agent-team-supervision/SKILL.md` per Specification §4 with the section order listed there. The preamble (§4 row 2) MUST declare the dependency on monitoring explicitly. The Authorization-Scope Guard (§4 row 6) MUST be embedded verbatim from §5 of this design doc. <!-- completed: -->
- [ ] Verify the file cross-references `Skill(agent-team-monitoring)` for the stall-response table and `/loop` template (no duplication). <!-- completed: -->
- [ ] Verify the Spawn Protocol section (§4 row 7) calls out the codex fallback requirement: "if the Director runs under codex, ensure one of the fallbacks listed in `Skill(agent-team-monitoring)` § Mechanism by backend is in place." <!-- completed: -->

### Step 3: Remove the old skill

- [ ] Delete `skills/cafleet-monitoring/` (whole directory). <!-- completed: -->
- [ ] Run `grep -rn "cafleet-monitoring" .` (excluding `.git/`, `design-docs/0000047-…/`) and confirm zero hits. Any hit is a missed reference. <!-- completed: -->

### Step 4: Update cross-references in other skills

- [ ] In `skills/design-doc-create/SKILL.md`, replace every `Skill(cafleet-monitoring)` load directive with the pair `Skill(agent-team-monitoring)` followed by `Skill(agent-team-supervision)` (in that order). Verify by re-grep. <!-- completed: -->
- [ ] Same pattern for `skills/design-doc-execute/SKILL.md`. <!-- completed: -->
- [ ] Same pattern for `skills/design-doc-interview/SKILL.md`. <!-- completed: -->
- [ ] Same for `skills/cafleet/SKILL.md` and any other `skills/*/SKILL.md` containing the string. <!-- completed: -->

### Step 5: Update project CLAUDE.md surfaces

- [ ] In `CLAUDE.md`, remove the `/cafleet-monitoring` bullet under "Skills" and add two bullets in this order: `/agent-team-monitoring — Active monitoring mechanism. Documents the cron-like loop per backend (Claude Code uses CronCreate + /loop; codex has no in-session scheduling and uses fallback options) and the team-facilitation instructions. Foundation layer — load first.` and `/agent-team-supervision — Governance layer that loads agent-team-monitoring as a prerequisite. Defines Core Principle, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, and User Delegation. Load second.` <!-- completed: -->
- [ ] Apply the identical change in `.claude/CLAUDE.md`. <!-- completed: -->

### Step 6: Update packaging / plugin manifest

- [ ] Inspect `cafleet/pyproject.toml` and any plugin metadata file (e.g. anything enumerating shipped skills under `[tool.*]` or a plugin manifest). If a skill list exists, add `agent-team-monitoring` and `agent-team-supervision`, remove `cafleet-monitoring`. If no enumeration exists, document that fact in this checkbox and skip. <!-- completed: -->
- [ ] If the plugin is built / installed via `mise //cafleet:install` or similar, run the install task and confirm both new skills are discoverable (the user can verify via `Skill` listing on their next session). <!-- completed: -->

### Step 7: Regression checks

- [ ] `grep -rn "cafleet-monitoring" .` outside `.git/` and `design-docs/0000047-…/` returns no hits. <!-- completed: -->
- [ ] `grep -rn "Skipping\. Holding for go" .` outside `.git/` and `design-docs/0000047-…/` returns no hits (sanity check that no skill instructs the Director to emit the failure-mode message). <!-- completed: -->
- [ ] Open `skills/agent-team-monitoring/SKILL.md` and confirm: (a) frontmatter is well-formed YAML; (b) section count matches §2; (c) the Mechanism by backend section matches §3 verbatim including the codex survey result; (d) the `/loop` Prompt Template is marked Claude Code-specific. <!-- completed: -->
- [ ] Open `skills/agent-team-supervision/SKILL.md` and confirm: (a) frontmatter is well-formed YAML; (b) section count matches §4; (c) the preamble declares the monitoring dependency; (d) the Authorization-Scope Guard text matches §5 verbatim; (e) no `/loop` Prompt Template appears (it lives in monitoring). <!-- completed: -->
- [ ] Update this design doc's Status to `Complete` and `Last Updated` to today's date. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-03 | Initial draft (single-skill design — absorbed monitoring into supervision). |
| 2026-05-03 | Revised to two-skill split mirroring the global pair (supervision = always-load, monitoring = loop companion). Authorization-Scope Guard moved to supervision. |
| 2026-05-03 | User feedback (1) the monitoring skill is essentially a cron wrapper and codex compat must be addressed since design 0000046 adds codex; (2) supervision depends on monitoring (dependency arrow reversed from the global pair). Added the codex survey (no in-session scheduling primitive in codex CLI; cloud automations are app-only). Reorganized: monitoring = foundation layer with backend-aware mechanism section + team-facilitation instructions; supervision = governance layer that loads monitoring as a prerequisite + holds the Authorization-Scope Guard. Added codex fallback options table (out-of-band cron driver, MCP scheduling server, user-driven nudges, no-active-monitor synchronous mode). Implementation step count grew to 0/16 across 7 steps. |
