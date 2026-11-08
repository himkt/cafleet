---
name: cafleet-design-doc
description: >-
  Use when the user asks to create a design doc, design document, specification,
  or technical spec (create workflow → Director/Drafter/Reviewer team); or to
  implement or execute a design doc (execute workflow → TDD or Programmer-only
  team). Also the standardized format spec — consult the template and
  guidelines when editing a design doc. Always invoke this skill and route into
  the matching workflow, orchestrated as a CAFleet team. Teammates in member teams
  load the core and assigned references through their backend's supported
  skill-loader without starting another team.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, AskUserQuestion
---

# Design Document Skill (CAFleet Edition)

This skill is the umbrella for the design-document lifecycle: the standardized **format spec** plus the CAFleet-native **orchestration workflows** in Dispatch. Consult a reference page for the format, or route a main-session user request through Dispatch to run a team. **Assigned teammates** continue their existing workflow ([Member startup](#member-startup)); loading this umbrella as a member creates no second team.

## Required reading

Identify your coding agent — your spawn prompt's `CODING AGENT:` line, or your own identity as a standalone / main-session reader. Then, before routing into a workflow or consulting a reference page, read **and resolve** your overlay section [`../cafleet/reference/coding-agents.md#<name>`](../cafleet/reference/coding-agents.md) (see *Resolve your overlay* in the cafleet `SKILL.md`), so no runtime token such as `{decision_surface}` is emitted unresolved.

Consult these directly (no team) only when the task needs them:

| Read | When |
|---|---|
| [reference/guidelines.md](reference/guidelines.md) | Writing or checking a doc against the standard structure (the template opens the page), or needing section guidelines, quality standards or formatting rules |
| [reference/coordination.md](reference/coordination.md) | Needing the inter-member verb + pointer + `COMMENT(role)` schema |

Each workflow body in Dispatch gates BASE resolution, supervision and coordination in its own Required-reading block.

## Dispatch

For a main-session user request matching a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, the moment the request matches, without waiting for the user to say "use cafleet".

**Routing into a workflow means executing its entire orchestration.** The Director registers its current session with `cafleet fleet create`, spawns the role team, receives each ready and drives the workflow’s review/revision rounds through approval, keeping reception under [Director wake](../cafleet/roles/director.md#director-wake) until fleet deletion; the linked workflow body owns its phase gates.

| When the user wants to… | Invoke this skill and run |
|---|---|
| Create a design doc / specification / technical spec | the **create** workflow ([create/create.md](create/create.md)) — Director/Drafter/Reviewer team |
| Implement / execute a design doc | the **execute** workflow ([execute/execute.md](execute/execute.md)) — TDD or Programmer-only team |

## Member startup

Every design-doc team member opens its authoritative role first, follows the CAFleet [reader and ready rule](../cafleet/SKILL.md#required-reading) and, using its `CODING AGENT:` identity, completes these reads in order before the first substantive assignment.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../cafleet/reference/coding-agents.md) | Resolve Runtime bindings, supported skill loader and bound notes. |
| 2 | [CAFleet core](../cafleet/SKILL.md) and [member role](../cafleet/roles/member.md) | Startup identity, ready, broker commands, member authority and shutdown. |
| 3 | [BASE](../cafleet/roles/member.md#base) | Before task work: inherited BASE, guarded writes and missing-line status. |
| 4 | This core and [guidelines](reference/guidelines.md) | Load the assigned workflow's format and role references before document work; retain your role's scope. |
| 5 | [Coordination](reference/coordination.md) | Before payloads, markers and work/status messages. |

Read [command routing](../cafleet/roles/member.md#reconsider-then-route) before routing a command your harness denied. Apply supplied host-rule equivalents where optional files are absent; route an essential unknown prerequisite to the Director before dependent work.
