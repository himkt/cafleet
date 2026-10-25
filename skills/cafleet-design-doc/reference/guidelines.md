# Design document guidelines

Write a complete, concise specification that an implementing member can follow. State intended behavior directly and reserve history for change records.

## Template

```markdown
# Title

**Status**: Draft

## Overview

What changes and why, in up to three sentences.

## Success Criteria

- [ ] Measurable outcome.

## Specification

Behavior, interfaces, compatibility, failure handling and important decisions.

## Implementation

### Step 1: Concrete outcome

- [ ] Executable task. <!-- completed: -->

## Changelog

| Date | Decision change |
|---|---|
| YYYY-MM-DD | Initial design. |
```

Use `Draft`, `Approved`, `In Progress` or `Complete` as appropriate. The changelog is optional and records specification decisions. Existing documents may retain accurate progress/date metadata.

## File layout

Use `design-docs/NNNNNNN-short-kebab-slug/design-doc.md`. Choose the highest existing numeric directory prefix plus one, padded to seven digits; start at `0000001`. Recheck availability before creating a fresh target. Preserve an existing or explicit target, and supply absolute artifact/scratch paths to members.

## Content and progress

Include overview, measurable success criteria and executable implementation tasks. Add specification detail where decisions require it: field types/constraints, API shapes, integration boundaries, compatibility and explicit failures. Use examples where they remove ambiguity.

Task checkboxes are progress truth. Complete a task with `- [x] Task <!-- completed: YYYY-MM-DDTHH:MM -->` in one edit, before continuing to the next task. Keep retained counters accurate.

Use tables for parallel comparisons, numbered lists for ordered work and prose for rationale. Link an authoritative registry instead of copying its growing membership. A ready specification has resolved consequential questions, coherent decisions and sufficiently precise tasks; preserve existing answers/annotations until incorporated.
