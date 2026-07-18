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
  load this skill by its name cafleet-design-doc via their backend's
  skill-loader.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, AskUserQuestion
---

# Design Document Skill (CAFleet Edition)

This skill is the umbrella for the full design-document lifecycle: the standardized **format spec** plus three CAFleet-native **orchestration workflows** (create, validate/interview, implement/execute). It is a dispatcher — consult a reference page for the format, or route to the matching workflow body to run a team.

**Teammates in member teams** must always load this skill by its name `cafleet-design-doc` via their backend's skill-loader — never by reading the skill files directly.

## Required reading

Before routing into a workflow or consulting a reference page, Read your overlay — it is row #1 below. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; a standalone / main-session reader uses its own identity.

**Load-bearing — Read before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../cafleet/reference/coding-agent/<name>-overlay.md`](../cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — you emit a literal `{monitor_model}` / `{decision_surface}` in the workflow you route into, **or** guess a wrong/default value, **or** ignore a backend note (codex has no harness task list) |

**On-demand — consult directly (no team), only when the task needs it:**

| Read | When |
|------|------|
| [reference/guidelines.md](reference/guidelines.md) | you are writing or checking a doc against the standard structure (the template opens the page), or need section guidelines, quality standards, or formatting rules |
| [reference/coordination.md](reference/coordination.md) | you need the inter-member verb + pointer + `COMMENT(role)` schema |

Each workflow body (create / interview / execute) carries its own Required-reading block for the team it runs — base-dir, supervision, and coordination are gated there, not on this dispatch surface.

Before acting, resolve every `{token}` you will use to its overlay value (or the documented default); a literal `{token}` in any command or message is a defect.

## Dispatch

When the user's request matches a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, the moment the request matches, without waiting for the user to say "use cafleet".

**Routing into a workflow means executing its entire orchestration.** The dedicated monitoring member is spawned first-in (`cafleet member create --role monitor`, gating the team behind its `ready: monitor live` handshake), then the role team, then the workflow body's review and revision rounds run through to approval. The linked workflow body is the authoritative procedure.

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Create a design doc / specification / technical spec | the **create** workflow ([create/create.md](create/create.md)) — Director/Drafter/Reviewer team |
| Validate / review / interview an existing design doc | the **interview** workflow ([interview/interview.md](interview/interview.md)) — multi-round Q&A |
| Implement / execute a design doc | the **execute** workflow ([execute/execute.md](execute/execute.md)) — TDD team |

For the document template, section guidelines, or the coordination protocol — consult the **On-demand** reference pages in § Required reading above (no team needed).

Always route design-doc work through the workflow bodies above — each runs the full CAFleet team.
