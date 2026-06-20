---
name: cafleet-design-doc
description: "Standardized design document format with template and guidelines. Load this skill ONLY to consult the template when editing an existing design doc, or transitively from the cafleet-design-doc-create skill. To CREATE a new design doc, use the cafleet-design-doc-create skill — this skill alone does not author docs. Teammates in agent teams must always load this skill by its name cafleet-design-doc via their backend's skill-loader. Do NOT write design documents in a freeform format."
allowed-tools: Read, Write, Edit, Glob, Grep
---

# Design Document Format

This skill provides a standardized format for creating design documents, specifications, and implementation plans.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

## Additional resources

- For the document template, see [template.md](template.md)
- For section guidelines, quality standards, formatting rules, and best practices, see [guidelines.md](guidelines.md)
- For the inter-agent coordination protocol (the verb + pointer schema + the `COMMENT(role)` marker convention; scope and the create/interview exemptions are in its § Scope), see [coordination.md](coordination.md)

