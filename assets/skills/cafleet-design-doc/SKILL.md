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

This skill holds the design-document format spec and the CAFleet workflows in [Dispatch](#dispatch). Assigned teammates continue their existing workflow through [Member startup](#member-startup) without starting a second team.

## Required reading

Before routing or consulting a reference, resolve your section of [coding-agents.md](../cafleet/reference/coding-agents.md) per the cafleet core's *Resolve your overlay*, identified by your spawn prompt's `CODING AGENT:` line or, as a main-session reader, your own identity.

Consult directly, without a team, when the task needs it:

- [reference/guidelines.md](reference/guidelines.md): the template, section guidelines, quality standards and formatting rules.
- [reference/coordination.md](reference/coordination.md): the inter-member verb + pointer + `COMMENT(role)` schema.

## Dispatch

When a main-session request matches a row below, invoke this skill and run the linked workflow as a full CAFleet team at once, without waiting for the user to say "use cafleet". The Director registers its session with `cafleet fleet create`, spawns the team, drives review rounds through approval and keeps [Director wake](../cafleet/roles/director.md#director-wake) reception until fleet deletion; the workflow body owns its phase gates.

| When the user wants to… | Run |
|---|---|
| Create a design doc / specification / technical spec | the **create** workflow ([create/create.md](create/create.md)) — Director/Drafter/Reviewer team |
| Implement / execute a design doc | the **execute** workflow ([execute/execute.md](execute/execute.md)) — TDD or Programmer-only team |

## Member startup

Every team member opens its role file first, follows the CAFleet [reader and ready rule](../cafleet/SKILL.md#required-reading), then completes these reads in order before its first substantive assignment:

1. Your `CODING AGENT:` section in [coding-agents.md](../cafleet/reference/coding-agents.md): Runtime bindings, skill loader and bound notes.
2. [CAFleet core](../cafleet/SKILL.md) and [member role](../cafleet/roles/member.md): identity, ready, broker commands, authority and shutdown.
3. [BASE](../cafleet/roles/member.md#base), before task work.
4. This core, [guidelines](reference/guidelines.md) and the assigned workflow's role references, before document work.
5. [Coordination](reference/coordination.md), before payloads, markers and status messages.
