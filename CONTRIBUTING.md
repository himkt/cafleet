# Contributing to CAFleet

CAFleet is developed using its own CAFleet-orchestrated skills — the repository dogfoods the spec-driven-development flow it ships. This document covers the project layout, the local development loop, and the contribution path.

## Project structure

| Top-level entry | Purpose |
|---|---|
| `cafleet/` | The `cafleet` Python package (FastAPI + SQLAlchemy + Alembic + click). |
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
cafleet db init           # apply schema migrations (idempotent; rerun after upgrades)

mise //cafleet:lint       # ruff check + ruff format --check
mise //cafleet:format     # ruff format
mise //cafleet:typecheck  # ty
mise //cafleet:test       # pytest

mise //admin:build        # build the WebUI (required before /ui/ is served)
mise //admin:dev          # WebUI dev server (Vite)
```

## Contributing changes

CAFleet uses its own design-doc-driven development skills to evolve the codebase. Some tips for new contributors:

1. `/cafleet:design-doc-create <one-line description>` — orchestrates a Director / Drafter / Reviewer team to produce a design doc under `design-docs/NNNNNNN-<slug>/`.
2. `/cafleet:design-doc-interview design-docs/NNNNNNN-<slug>` — fine-grained Q&A pass that annotates the doc with `COMMENT(claude)` markers for `/cafleet:design-doc-create` resume mode to absorb.
3. `/cafleet:design-doc-execute design-docs/NNNNNNN-<slug>` — TDD-cycle implementation pass (Director / Programmer / Tester / optional Verifier).

The same flow works from Codex with `$cafleet:design-doc-*` prefixes. Existing design documents under [`design-docs/`](design-docs/) are real examples produced by this loop.
