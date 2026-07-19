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

---

For the documentation-first implementation order and the first-class documentation targets (README, SPEC, docs, skills), see `documentation-maintenance.md`.
