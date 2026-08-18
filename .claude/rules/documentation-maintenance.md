# Documentation Maintenance

Rules for keeping the project's documentation in sync with its code. These apply
to every change that touches a user- or contract-facing surface; the design-doc
numbering/format rules live separately in `design-doc-numbering.md`.

## Implementation Order

When implementing a design document, ALWAYS update documentation FIRST before writing any code: update every affected first-class documentation target per the table below — `docs/` pages first, then `README.md`, `SPEC.md`, the affected `skills/*/SKILL.md`, and project rules if needed. Only after documentation is complete should code implementation begin.

## First-class documentation targets

`docs/` is the primary home for all descriptive content — features, architecture, usage, configuration, and project structure land under `docs/` first. Every other target below is updated in the same design-doc cycle as the change that touches its surface, with the smallest edit that removes the drift. **Drift on any target below is a blocker for "documentation complete".**

| Target | Update trigger | Drift consequence |
|---|---|---|
| `docs/` | Any change to features, architecture, usage, or configuration (a new architectural axis gets its own Concepts page) | The primary documentation home goes stale |
| `README.md` | The thin surface itself changes — the pitch drifts from `docs/docs/index.md`, the install commands change, or a docs-site section is added or removed (sync via the `/update-readme` skill) | The entry point misleads new users |
| `SPEC.md` | Any change to its contract surfaces — CLI options, `CAFLEET_*` configuration, the SQLite schema, the HTTP API, error strings, JSON key order, text layouts (maintained via the `/update-readme` skill) | The single authoritative reimplementation specification no longer matches the shipped contract |
| `skills/*/SKILL.md` | Any change to CLI commands, flags, environment variables, required arguments, output formats, or the expected invocation workflow | Claude Code loads stale skills as ground truth and produces broken tool calls |
| `.claude/rules/` | The project convention the rule captures changes | Agents follow outdated conventions |

`SPEC.md` edits preserve its section structure and contract-level detail, and stay descriptive — specification only, no recommendations or implementation advice.

How `docs/` pages reference — or avoid referencing — repository file paths is governed by `user-facing-docs.md`.
