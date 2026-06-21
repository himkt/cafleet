---
name: cafleet-design-doc
description: >-
  Design document format spec plus CAFleet-native orchestration to create,
  validate, and implement design docs. Consult the template/guidelines when
  editing an existing design doc; or create a new design doc / specification /
  technical spec (Director/Drafter/Reviewer), validate/interview an existing one
  through multi-round Q&A, or implement/execute one with a TDD team. Teammates in
  agent teams must always load this skill by its name cafleet-design-doc via
  their backend's skill-loader. Do NOT write or implement design docs freeform —
  route through this skill's workflows.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, AskUserQuestion
---

# Design Document Skill (CAFleet Edition)

This skill is the umbrella for the full design-document lifecycle: the standardized **format spec** plus three CAFleet-native **orchestration workflows** (create, validate/interview, implement/execute). It is a dispatcher — consult a reference page for the format, or route to the matching workflow body to run a team.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

**Teammates in agent teams** must always load this skill by its name `cafleet-design-doc` via their backend's skill-loader — never by reading the skill files directly.

## When to use

| You want to… | Go to |
|:--|:--|
| Consult the document template | [reference/template.md](reference/template.md) |
| Consult section guidelines, quality standards, and formatting rules | [reference/guidelines.md](reference/guidelines.md) |
| Consult the inter-agent coordination protocol (verb + pointer schema, `COMMENT(role)` markers) | [reference/coordination.md](reference/coordination.md) |
| Create a new design doc / specification / technical spec (Director/Drafter/Reviewer team) | [create/create.md](create/create.md) |
| Validate / interview an existing design doc through multi-round Q&A | [interview/interview.md](interview/interview.md) |
| Implement / execute a design doc with a TDD team | [execute/execute.md](execute/execute.md) |

Do NOT write or implement design documents freeform — always route through the workflow bodies above.
