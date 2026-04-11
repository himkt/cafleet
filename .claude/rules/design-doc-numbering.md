# Design Documents

Design documents are stored in `design-docs/` with a 7-digit zero-padded sequential number prefix.

## Format

```
design-docs/{NNNNNNN}-{slug}/design-doc.md
```

Example: `design-docs/0000001-a2a-registry-broker/design-doc.md`

## Rules

- Always check the latest number before creating a new design document
- Increment by 1 from the highest existing number
- Use 7-digit zero-padding (e.g., `0000001`, `0000002`, `0000003`)
- The slug should be a kebab-case short description of the feature

## How to find the next number

Look at existing directories in `design-docs/` and use the next sequential number.

## Implementation Order

When implementing a design document, ALWAYS update documentation FIRST before writing any code.

The first implementation step in every design document must be:
- Update `ARCHITECTURE.md` with the new feature's architecture
- Update `docs/` directory with usage and configuration details
- Update `README.md` so it stays consistent with `ARCHITECTURE.md` and `docs/` (use the `/update-readme` skill when the change surface is large)
- Update relevant skill documentation (including `.claude/skills/*/SKILL.md`)
- Update project rules if needed

`README.md` is a first-class documentation target on par with `ARCHITECTURE.md` and `docs/`. Any change that affects architecture, CLI surface, API surface, configuration, or project structure MUST be reflected in `README.md` in the same design-doc cycle. Treat README drift as a blocker for "documentation complete".

Only after documentation is complete should code implementation begin.
