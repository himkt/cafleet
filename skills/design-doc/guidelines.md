# Design Document Guidelines

## Core Principle

**Comprehensive with structured readability.** Write a document complete enough that Claude can implement the feature by reading only this document. Use structured formatting (tables, lists, code blocks) to keep it scannable. Every sentence must earn its place, but prefer completeness over brevity when it prevents implementation guesswork.

## Formatting Guidelines

| Content Type | Preferred Format |
|-------------|-----------------|
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

Design documents use a directory-based structure:

- Path: `design-docs/{slug}/design-doc.md`
- The directory may contain related artifacts (e.g., `question.md` from interviews)
- Design-doc commands that operate on a single document (e.g., `/design-doc-create`, `/design-doc-interview`, `/design-doc-execute`) accept a slug name (e.g., `my-feature`) as argument. The `design-docs/` prefix is auto-prepended via `base-dir` integration

## What "Comprehensive" Means

- All design decisions are recorded with brief rationale
- Data models include field names, types, and constraints
- API contracts include request/response shapes
- Error cases are enumerated, not left as "handle errors appropriately"
- Integration points specify exact interfaces

## What "Comprehensive" Does NOT Mean

- No restating obvious things
- No separate sections for trivial content (fold into Specification or Implementation)
- No speculative "Future Considerations" unless the current design must accommodate them

## Anti-patterns to Avoid

- Restating what is already obvious from the code or context
- Adding sections just because the template has them
- Writing long prose when a table or list would suffice
- Separate "Testing Strategy", "Error Handling", "Future Considerations" sections for simple changes — fold into Specification or Implementation if needed
- Repeating the same information in Overview, Background, and Specification

## Completeness Check

Ready for implementation when:
- No `[TBD]` placeholders remain
- No `COMMENT(` markers remain
- Implementation steps are specific enough to execute without guessing
- All implementation tasks use the timestamp format: `- [ ] Task <!-- completed: -->`
- A `**Progress**: 0/N tasks complete` line exists in the header
