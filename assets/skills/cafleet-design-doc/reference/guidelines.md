# Design Document Guidelines

## Document Template

```markdown
# [Title]

**Status**: Draft | Approved | In Progress | Complete
**Progress**: 0/N tasks complete
**Last Updated**: YYYY-MM-DD

## Overview

[1-3 sentences. What and why.]

## Success Criteria

- [ ] [Measurable outcome]

---

## Background

[Only if context is not obvious. Current state and problem.]

---

## Specification

[Design decisions, technical details, code examples as needed.]

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: [Name]

- [ ] [Task] <!-- completed: -->
- [ ] [Task] <!-- completed: -->

---

## Changelog (optional -- spec revisions only, not implementation progress)

| Date | Changes |
|------|---------|
| YYYY-MM-DD | Initial draft |
```

Overview, Success Criteria (measurable and verifiable) and Implementation (named steps with checkbox tasks) are always present; include Specification when there are design decisions to record.

## Writing Standard

Write a scannable document from which an agent can implement the feature alone; completeness wins over brevity. Record each design decision with brief rationale, data-model field names, types and constraints, API request/response shapes, enumerated error cases and exact integration interfaces.

| Content | Format |
|---|---|
| Enumerated options or states | Table |
| Sequential steps | Numbered list |
| Requirements or constraints | Bulleted list |
| Data structures or schemas | Code block |
| Decision rationale | Prose, 2-3 sentences |

State each fact once, omit what the code or context makes obvious, and add a section only when its content is needed. For simple changes, fold testing, error handling and future considerations into Specification or Implementation; record future considerations only when the current design must accommodate them.

## File Layout

Design documents live at `design-docs/NNNNNNN-<slug>/design-doc.md`, with a short kebab-case feature description as `<slug>`. Normalize a design-doc argument to its task folder by stripping a trailing `/design-doc.md`; a relative argument also drops a leading `design-docs/` and gets `design-docs/` prepended once, and an absolute argument's folder is used directly. [Task-scope resolution](#task-scope-resolution) owns containment, the Director's [scratch and audit procedure](../../cafleet/roles/director.md#member-create--scratch-and-audit-files) owns guarded writes and disabled audits, and the invoking workflow owns document discovery and its explicit output target.

### Task-scope resolution

A task-folder consumer resolves BASE thus, writing no files; members inherit the supplied absolute BASE or the [disabled-audit state](../../cafleet/roles/member.md#base).

1. Run `git rev-parse --show-toplevel` from CWD. On a non-zero exit or empty output, stop and tell the user to change to the repository root and retry.
2. Normalize the argument to its task folder: design-doc workflows use [File Layout](#file-layout), and other consumers own their bucket conventions.
3. Resolve a relative folder against the repository root, rejecting one that equals the root (`the repo root is not a task folder`) or escapes it (`refusing to create a task folder outside the repo`). For an absolute folder that equals the root or is not strictly below it, return `<unset>` and create nothing.
4. Return the absolute path of a folder strictly below the root as BASE. The folder, with audits inside it, is created on the first consumer write.

### Numbering

A new folder takes the highest numeric prefix among the repository's `design-docs/` directories plus 1, zero-padded to seven digits (`0000001` when none exist); unfinished documents still reserve their numbers. A topic or unnumbered slug such as `my-feature` takes the next number, e.g. `0000001-my-feature`, resolved through BASE before any document or audit write; recheck availability before creating the folder and take the next free number if it is taken.

The create and execute workflows also accept an existing directory name or document path and preserve that target. An explicit user-supplied output path overrides numbering.

## Completeness Check

Ready for implementation when:
- No `[TBD]` placeholders remain
- No `COMMENT(` markers remain
- Implementation steps are specific enough to execute without guessing
- All implementation tasks use the timestamp format: `- [ ] Task <!-- completed: -->`
- A `**Progress**: 0/N tasks complete` line exists in the header
