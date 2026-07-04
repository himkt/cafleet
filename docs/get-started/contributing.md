---
icon: lucide/heart-handshake
---

# Contributing

CAFleet is developed using its own CAFleet-orchestrated skills — the
repository dogfoods the spec-driven-development flow it ships. This document
covers the project layout, the local development loop, and the contribution
path.

## Project structure

| Top-level entry | Purpose |
|---|---|
| `cafleet/` | The `cafleet` Python package (FastAPI + SQLAlchemy + click). |
| `admin/` | Admin WebUI SPA (Vite + React + TypeScript + Tailwind CSS). |
| `skills/` | Plugin skills shared by the Claude Code and Codex manifests. |
| `package.json` + `bun.lock` (repo root) | Bun toolchain manifests for the Slidev + agent-browser tools used in the repo. Driven via `mise //:bun-install` / `mise //:slidev <deck>`; `node_modules/` is gitignored. |
| `design-docs/` | Numbered design documents (`NNNNNNN-<slug>/design-doc.md`). |
| `docs/` | CLI reference, message envelope, and other operator-facing docs. |

## Development

Clone the repo and use mise for all common tasks:

```bash
git clone https://github.com/himkt/cafleet.git
cd cafleet

mise //:uv-sync
mise //cafleet:install    # editable uv tool install of the cafleet CLI
cafleet setup db          # create the database schema (idempotent)

mise //cafleet:lint       # ruff check + ruff format --check
mise //cafleet:format     # ruff format
mise //cafleet:typecheck  # ty
mise //cafleet:test       # pytest

mise //admin:build        # build the WebUI (required before / is served)
mise //admin:dev          # WebUI dev server (Vite)
mise //admin:install      # reinstall WebUI deps from the committed lockfile
```

To change the WebUI's dependencies, edit `admin/package.json` and run plain
`bun install` inside `admin/` to regenerate `admin/bun.lock`.
`mise //admin:install` runs `bun install --frozen-lockfile`, so it only
reinstalls from the committed lockfile and cannot update it.

## Contributing changes

CAFleet uses its own design-doc-driven development skills to evolve the
codebase. Some tips for new contributors:

1. Invoke the `cafleet-design-doc` skill (create workflow) with a one-line description —
   orchestrates a Director / Drafter / Reviewer team to produce a design doc
   under `design-docs/NNNNNNN-<slug>/`.
2. Invoke the `cafleet-design-doc` skill (interview workflow) with the path
   `design-docs/NNNNNNN-<slug>` — fine-grained Q&A pass that annotates the
   doc with `COMMENT(claude)` markers for the create workflow's resume mode
   to absorb.
3. Invoke the `cafleet-design-doc` skill (execute workflow) with the path
   `design-docs/NNNNNNN-<slug>` — TDD-cycle implementation pass (Director /
   Programmer / Tester / optional Verifier, plus a fresh Reviewer at review
   time).

See your coding-agent's skill documentation for the literal invocation syntax.
Existing design documents under [`design-docs/`](https://github.com/himkt/cafleet/tree/main/design-docs)
are real examples produced by this loop.

## Documentation style

When editing `docs/` or `README.md`, follow these conventions:

- **Audience split**: `docs/` is written for human developers and operators;
  `skills/` is written for coding agents. Do not mix the registers.
- **Voice**: second person ("you"), active voice, present tense. Lead each
  page with what the reader accomplishes, not with architecture.
- **Terms**: link a term's first use on a page to the
  [Core terms](../concepts/overview.md#core-terms) table in the concepts
  overview; do not re-define it.
- **Examples**: every CLI example is a runnable command using the standard
  sample-id cast — fleet `1`, root Director `2`, Administrator `3`, members
  `4`+ — followed by an expected-output block matching the output shapes in
  [CLI options](../spec/cli-options.md). Never use shell variables to hold
  ids.
- **SSOT**: one fact, one home. When another page needs the fact, link;
  when a fact serves no install/configure/use/understand purpose, delete.
