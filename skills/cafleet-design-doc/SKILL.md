---
name: cafleet-design-doc
description: >-
  Use when the user asks to create a design doc, design document, specification,
  or technical spec (create workflow → Director/Drafter/Reviewer team); to
  validate, review, or interview an existing design doc through multi-round Q&A
  (interview workflow); or to implement or execute a design doc (execute workflow
  → TDD team). Also the standardized format spec — consult the template and
  guidelines when editing a design doc. Always invoke this skill and route into
  the matching workflow, orchestrated as a CAFleet team. Teammates in member teams
  load the core and assigned references through their backend's supported
  skill-loader without starting another team.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, AskUserQuestion
---

# Design Document Skill (CAFleet Edition)

This skill is the umbrella for the full design-document lifecycle: the standardized **format spec** plus three CAFleet-native **orchestration workflows** (create, validate/interview, implement/execute). It is a dispatcher — consult a reference page for the format, or route to the matching workflow body to run a team.

**Assigned teammates** load this skill through the executing backend's supported loader. Codex/OpenCode use absolute paths for this core, CAFleet core, their own backend section and required role references. Continue the existing assigned workflow; loading this umbrella as a member creates no second team. Main-session user requests follow Dispatch below.

## Required reading

Before routing into a workflow or consulting a reference page, Read your overlay — it is row #1 below. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; a standalone / main-session reader uses its own identity.

**Load-bearing — Read before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay section [`../cafleet/reference/coding-agents.md#<name>`](../cafleet/reference/coding-agents.md) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — the failure modes *Resolve your overlay* closes, e.g. a literal `{decision_surface}` emitted unresolved |

**On-demand — consult directly (no team), only when the task needs it:**

| Read | When |
|------|------|
| [reference/guidelines.md](reference/guidelines.md) | you are writing or checking a doc against the standard structure (the template opens the page), or need section guidelines, quality standards, or formatting rules |
| [reference/coordination.md](reference/coordination.md) | you need the inter-member verb + pointer + `COMMENT(role)` schema |

Each workflow body (create / interview / execute) carries its own Required-reading block for the team it runs — base-dir, supervision, and coordination are gated there, not on this dispatch surface.

## Dispatch

For a main-session user request matching a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, the moment the request matches, without waiting for the user to say "use cafleet".

**Routing into a workflow means executing its entire orchestration.** The Director registers its current session with `cafleet fleet create`, spawns the role team, receives each ready and drives the workflow’s review/revision rounds through approval. The Director retains native inbox reception while delegated work remains; the linked workflow body owns its phase gates.

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Create a design doc / specification / technical spec | the **create** workflow ([create/create.md](create/create.md)) — Director/Drafter/Reviewer team |
| Validate / review / interview an existing design doc | the **interview** workflow ([interview/interview.md](interview/interview.md)) — multi-round Q&A |
| Implement / execute a design doc | the **execute** workflow ([execute/execute.md](execute/execute.md)) — TDD team |

## Member startup

Every design-doc team member opens its authoritative role first. Use an available non-shell text reader; prerequisite file reads may use shell when it is the only reader. Ready is your first operational broker shell command and precedes task work. Complete these reads in order before the first substantive assignment, using your `CODING AGENT:` identity.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../cafleet/reference/coding-agents.md) | Resolve your Runtime bindings, supported skill loader and bound notes. |
| 2 | [CAFleet core](../cafleet/SKILL.md) and [member role](../cafleet/roles/member.md) | Startup identity, ready, broker commands, member authority and shutdown. |
| 3 | [BASE](../cafleet/reference/base-dir.md) | Before task work: inherited BASE, guarded writes and missing-line status. |
| 4 | This core and [guidelines](reference/guidelines.md) | Load the assigned workflow's format and role references before document work; retain your role's scope. |
| 5 | [Coordination](reference/coordination.md) | Before payloads, markers and work/status messages. |

Load the listed skills through your executing backend's supported loader; Codex/OpenCode load the cores, own backend and required references by absolute path. Continue the existing assigned workflow without creating a second team. Read [command routing](../cafleet/reference/prompt-routing.md) before routing a command your harness denied. Apply supplied host-rule equivalents where optional files are absent; route an essential unknown prerequisite to the Director before dependent work.
