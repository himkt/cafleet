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
| `cafleet/` | The `cafleet` Python package (FastAPI + SQLAlchemy + Alembic + click). |
| `admin/` | Admin WebUI SPA (Vite + React + TypeScript + Tailwind CSS). |
| `skills/` | Coding-agent skill files (`cafleet`, `cafleet-design-doc`, `cafleet-research`), installed into the agent homes by `cafleet setup` / `mise //:skill-install`. |
| `package.json` + `pnpm-lock.yaml` (repo root) | pnpm toolchain manifests for the Slidev + agent-browser tools used in the repo. Driven via `mise //:pnpm-install` / `mise //:slidev <deck>`; `node_modules/` is gitignored. |
| `design-docs/` | Numbered design documents (`NNNNNNN-<slug>/design-doc.md`). |
| `docs/` | CLI reference, message envelope, and other operator-facing docs. |

## Tech stack

| Concern | Technology | Notes |
|---|---|---|
| Language | Python 3.12+, managed with [uv](https://docs.astral.sh/uv/) | — |
| Server | [FastAPI](https://fastapi.tiangolo.com/) | Admin WebUI only |
| Database | [SQLAlchemy](https://www.sqlalchemy.org/) 2.x + SQLite | Sync `pysqlite` driver |
| CLI | [click](https://click.palletsprojects.com/) | — |
| Admin frontend | Vite + pnpm | SPA served at `/` |
| Task runner | [mise](https://mise.jdx.dev/) | — |

## Development

Clone the repo and run the first-time setup once:

```bash
git clone https://github.com/himkt/cafleet.git
cd cafleet

mise //:uv-sync
mise //cafleet:install    # editable uv tool install of the cafleet CLI
cafleet setup --skip claude --skip codex --skip opencode   # migrate the database schema only (idempotent)
```

After that, pick the task you need by name:

| Task | Runs | When you need it |
|---|---|---|
| `mise //cafleet:lint` | `ruff check` + `ruff format --check` | Checking Python style before a commit |
| `mise //cafleet:format` | `ruff check --fix` + `ruff format` | Applying Python formatting fixes |
| `mise //cafleet:typecheck` | `ty` | Type-checking the Python package |
| `mise //cafleet:test` | `pytest` | Running the test suite |
| `mise //admin:lint` | `pnpm lint` | Checking the WebUI sources |
| `mise //admin:build` | Vite build | Required before `/` is served |
| `mise //admin:dev` | Vite dev server | Working on the WebUI with hot reload |
| `mise //admin:install` | `pnpm install --frozen-lockfile` | Reinstalling WebUI deps from the committed lockfile |

To change the WebUI's dependencies, edit `admin/package.json` and run plain
`pnpm install --no-frozen-lockfile` from the repository root to regenerate `pnpm-lock.yaml` —
`mise //admin:install` installs with `--frozen-lockfile` and cannot update
the lockfile.

### Installing the skills from your checkout

`cafleet setup` installs the assets from a published Release, so it is the
**end-user (installed-CLI)** path. Contributors working from a clone install
the skills from the working tree instead:

```bash
mise //:skill-install
```

This runs `gh skill install ./ --from-local --agent <backend> --force --scope
user` for each of the three backends (`claude-code`, `codex`, `opencode`),
placing the skills from your checkout (not a Release) into the three agent
homes.

## Building docs locally

Build the documentation site (this site) locally with:

```bash
mise //:docs-build
```

That task is a thin wrapper around `uv run zensical build --clean` and is the
same command the GitHub Actions workflow runs.

## Contributing changes

CAFleet uses its own design-doc-driven development skills to evolve the
codebase. Each workflow's prompt, team, and output is in
[Spec Driven Dev § Prompts](how-to/design-doc-development.md#prompts); run them
in that order — create, then interview, then execute.

One detail matters to contributors specifically: the interview pass annotates
the doc with `COMMENT(user-relay)` markers that the create workflow's resume
mode absorbs.

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
  [Core terms](concepts/overview.md#core-terms) table in the concepts
  overview; do not re-define it.
- **Examples**: every CLI example is a runnable command using the standard
  sample-id cast — fleet `1`, root Director `2`, members
  `3`+ — followed by an expected-output block matching the output shapes in
  [CLI options](spec/cli-options.md). Never use shell variables to hold
  ids.
- **SSOT**: one fact, one home. When another page needs the fact, link;
  when a fact serves no install/configure/use/understand purpose, delete.
- **Tables**: state an enumeration of three or more parallel items carrying
  two or more shared attributes as a table; keep single items, ordered
  procedures, and rationale as prose. Give every table at least two data
  rows — a one-row table costs a header row and buys nothing over a
  sentence — and keep every cell to at most two sentences. When an
  enumeration belongs on more than one page, give it one owning page
  carrying the table and make every other mention a link plus a one-clause
  summary.
