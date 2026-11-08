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

## Core Principle

**Comprehensive with structured readability.** Write a document complete enough that an agent can implement the feature from it alone, kept scannable with tables, lists and code blocks. Every sentence must earn its place, but completeness wins over brevity when it prevents implementation guesswork.

## Formatting Guidelines

| Content Type | Preferred Format |
|---|---|
| Enumerated options or states | Table |
| Sequential steps | Numbered list |
| Requirements or constraints | Bulleted list |
| Data structures or schemas | Code block |
| Decision rationale | Prose (keep to 2-3 sentences max) |

## Section Rules

- **Overview**: Always include. Max 3 sentences.
- **Success Criteria**: Always include. Measurable and verifiable.
- **Background**: Only when the reader lacks context. Skip for obvious changes.
- **Specification**: Include when there are design decisions to record. Use code examples over prose when possible.
- **Implementation**: Always include. Each step has a clear name and checkbox tasks.
- **Changelog**: Optional. Use only for spec-level revisions, not implementation progress.

## File Layout

Normalize the design-doc argument to the task folder before [task-scope resolution](#task-scope-resolution): strip a trailing `/design-doc.md`; for a relative argument, also strip a leading `design-docs/` if present and prepend `design-docs/` once; an absolute argument loses only the trailing filename and its folder is used directly. Task-scope resolution owns containment; the Director's [scratch and audit procedure](../../cafleet/roles/director.md#member-create--scratch-and-audit-files) owns guarded writes and disabled-audit behavior. The invoking workflow owns document discovery and its explicit output target.

Design documents live at `design-docs/NNNNNNN-<slug>/design-doc.md` in every repository using CAFleet, with a short kebab-case feature description as `<slug>`.

### Task-scope resolution

Use this resolver when a consumer operates on a task folder. The Director or consumer resolves BASE; members inherit the supplied absolute BASE or follow the [missing-line disabled-audit state](../../cafleet/roles/member.md#base).

1. Run `git rev-parse --show-toplevel` from CWD. A non-zero exit or empty output means no Git ancestor: stop and tell the user to change to the repository root and retry.
2. Normalize the consumer's argument to its actual task folder — a relative or absolute folder, not a child filename — before containment resolution: design-doc workflows use the normalization above, and other consumers own their bucket conventions.
3. Resolve the folder against the repository root when relative. Reject a relative path that equals the root (`the repo root is not a task folder`) or escapes it (`refusing to create a task folder outside the repo`). Resolve an absolute path directly: if it equals the root or is not strictly below it, return `<unset>` and create nothing.
4. For a folder strictly below the repository root, return its absolute path as BASE. Create the selected folder lazily on the first consumer write, with audits under the task folder.

The resolver applies containment to the normalized folder without writing files.

### Numbering

For a new document's folder, inspect the target repository's `design-docs/` directories, take the highest existing numeric prefix, add 1 and zero-pad it to seven digits, starting at `0000001` when there are no numbered directories. Count every numbered directory, even one whose document is unfinished, so its number stays reserved.

Fresh creation from a topic or an unnumbered slug such as `my-feature` uses the next number in a folder such as `0000001-my-feature`, resolved through BASE before writing any document or audit artifacts. Recheck availability before creating the folder, and select the next available number if another task has taken it.

The create and execute workflows accept an existing directory name such as `0000001-my-feature` or a document path, and preserve existing targets when resuming or executing. Honor an explicit user-supplied output path; numbering supplies the default location when the workflow chooses a new folder.

## What "Comprehensive" Means

- All design decisions are recorded with brief rationale
- Data models include field names, types, and constraints
- API contracts include request/response shapes
- Error cases are enumerated, not left as "handle errors appropriately"
- Integration points specify exact interfaces

## Anti-patterns to Avoid

- Restating what the code or context makes obvious, or repeating information across Overview, Background and Specification.
- Adding sections only because the template has them, or long prose where a table or list suffices.
- Separate "Testing Strategy" / "Error Handling" / "Future Considerations" sections for simple changes — fold them into Specification or Implementation; include speculative "Future Considerations" only when the current design must accommodate them.

## Completeness Check

Ready for implementation when:
- No `[TBD]` placeholders remain
- No `COMMENT(` markers remain
- Implementation steps are specific enough to execute without guessing
- All implementation tasks use the timestamp format: `- [ ] Task <!-- completed: -->`
- A `**Progress**: 0/N tasks complete` line exists in the header
