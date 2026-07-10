# Documentation Maintenance

Rules for keeping the project's documentation in sync with its code. These apply
to every change that touches a user- or contract-facing surface; the design-doc
numbering/format rules live separately in `design-doc-numbering.md`.

## Implementation Order

When implementing a design document, ALWAYS update documentation FIRST before writing any code.

The first implementation step in every design document must be:
- Update the appropriate `docs/concepts/<page>.md` page (or add a new Concepts page if the feature introduces a new architectural axis)
- Update `docs/` directory with usage and configuration details
- Update `README.md` only when the change touches its thin surface — the pitch, the install commands, or the docs-site section links (use the `/update-readme` skill to sync it)
- Update `SPEC.md` (the reimplementation specification) so its contract surfaces — CLI, configuration, persistence schema, HTTP API, observable semantics — stay accurate (the `/update-readme` skill maintains it alongside `README.md`)
- Update every affected skill under `skills/*/SKILL.md`
- Update project rules if needed

Only after documentation is complete should code implementation begin.

## First-class documentation targets

`docs/` is the primary home for all descriptive content — features, architecture, usage, configuration, and project structure land under `docs/` first. `README.md` is a thin entry point (pitch, install commands, docs-site section links) and must be updated only when that surface itself changes — the pitch drifts from `docs/index.md`, the install commands change, or a docs-site section is added or removed. Drift on the thin surface is a blocker for "documentation complete".

`SPEC.md` is **equally** first-class. It is the single authoritative reimplementation specification, where exact CLI options, `CAFLEET_*` configuration, the SQLite schema, the HTTP API, error strings, JSON key order, and text layouts ARE the contract. Any change to those surfaces MUST be reflected in `SPEC.md` in the same design-doc cycle, with the smallest edit that removes the drift — preserve its section structure and contract-level detail, and keep it descriptive (specification only; no recommendations or implementation advice). SPEC drift is a blocker for "documentation complete".

`SKILL.md` files are **equally** first-class documentation targets. Any change that alters CLI commands, flags, environment variables, required arguments, output formats, or the expected invocation workflow MUST be reflected in every affected `SKILL.md` in the same design-doc cycle. Skill drift — where a `SKILL.md` example no longer matches the actual CLI — is a blocker for "documentation complete", because Claude Code loads these skills as ground truth and will produce broken tool calls when they go stale.
