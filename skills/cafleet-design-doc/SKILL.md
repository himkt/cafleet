---
name: cafleet-design-doc
description: >-
  Use when the user asks to create a design doc, design document, specification,
  or technical spec (create workflow → Director/Drafter/Reviewer team); to
  validate, review, or interview an existing design doc through multi-round Q&A
  (interview workflow); or to implement or execute a design doc (execute workflow
  → TDD team). Also the standardized format spec — consult the template and
  guidelines when editing a design doc. Always invoke this skill and route into
  the matching workflow, orchestrated as a CAFleet team. Teammates in agent teams
  load this skill by its name cafleet-design-doc via their backend's
  skill-loader.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, AskUserQuestion
---

# Design Document Skill (CAFleet Edition)

This skill is the umbrella for the full design-document lifecycle: the standardized **format spec** plus three CAFleet-native **orchestration workflows** (create, validate/interview, implement/execute). It is a dispatcher — consult a reference page for the format, or route to the matching workflow body to run a team.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

**Teammates in agent teams** must always load this skill by its name `cafleet-design-doc` via their backend's skill-loader — never by reading the skill files directly.

## Dispatch

When the user's request matches a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, the moment the request matches, without waiting for the user to say "use cafleet".

**Routing into a workflow means executing its entire orchestration.** The dedicated monitoring member is spawned first-in (`cafleet member create --role monitor`, gating the team behind its `ready: monitor live` handshake), then the role team, then the workflow body's review and revision rounds run through to approval. The linked workflow body is the authoritative procedure.

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Create a design doc / specification / technical spec | the **create** workflow ([create/create.md](create/create.md)) — Director/Drafter/Reviewer team |
| Validate / review / interview an existing design doc | the **interview** workflow ([interview/interview.md](interview/interview.md)) — multi-round Q&A |
| Implement / execute a design doc | the **execute** workflow ([execute/execute.md](execute/execute.md)) — TDD team |

Consult these reference pages directly (no team):

| When the user wants to… | Consult |
|:--|:--|
| The document template | [reference/template.md](reference/template.md) |
| Section guidelines, quality standards, and formatting rules | [reference/guidelines.md](reference/guidelines.md) |
| The inter-agent coordination protocol (verb + pointer schema, `COMMENT(role)` markers) | [reference/coordination.md](reference/coordination.md) |

Always route design-doc work through the workflow bodies above — each runs the full CAFleet team.
