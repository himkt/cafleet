---
name: cafleet-design-doc
description: "Standardized design document format with template and guidelines. Load this skill ONLY to consult the template when editing an existing design doc, or transitively from /design-doc-create. To CREATE a new design doc, use /design-doc-create — this skill alone does not author docs. Teammates in agent teams must always load this skill using Skill(design-doc). Do NOT write design documents in a freeform format."
allowed-tools: Read, Write, Edit, Glob, Grep
---

# Design Document Format

This skill provides a standardized format for creating design documents, specifications, and implementation plans.

## Additional resources

- For the document template, see [template.md](template.md)
- For section guidelines, quality standards, formatting rules, and best practices, see [guidelines.md](guidelines.md)
- For the inter-agent coordination protocol — verb + pointer schema for `/design-doc-create` and `/design-doc-execute`, plus the inline `COMMENT(role)` marker convention shared by `/design-doc-interview` (whose Director-Analyzer cafleet messages are exempt from the verb + pointer schema) — see [coordination.md](coordination.md)

