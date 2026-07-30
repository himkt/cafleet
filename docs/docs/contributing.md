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
| `Cargo.toml` | The Cargo workspace root. |
| `cafleet/` | The `cafleet` Rust package (clap CLI + axum server + rusqlite persistence); builds the single `cafleet` binary. |
| `admin/` | Admin WebUI SPA (Vite + React + TypeScript + Tailwind CSS); its build output is embedded in the binary. |
| `skills/` | Coding-agent skill files (`cafleet`, `cafleet-design-doc`, `cafleet-research`), installed into the agent homes by `cafleet setup` / `mise //:skill-install`. |
| `package.json` + `pnpm-lock.yaml` (repo root) | pnpm toolchain manifests for the Slidev + agent-browser tools used in the repo. Driven via `mise //:pnpm-install` / `mise //:slidev <deck>`; `node_modules/` is gitignored. |
| `design-docs/` | Numbered design documents (`NNNNNNN-<slug>/design-doc.md`). |
| `docs/` | The rspress documentation-site project (workspace package `cafleet-docs`): `rspress.config.ts`, mermaid sources in `diagrams/`, and the operator-facing pages in its nested `docs/` content root. |

## Tech stack

| Concern | Technology | Notes |
|---|---|---|
| Language | Rust (stable toolchain, managed with [mise](https://mise.jdx.dev/)) | — |
| CLI | [clap](https://docs.rs/clap/) | — |
| Database | [rusqlite](https://docs.rs/rusqlite/) (bundled SQLite) | Migrations via [refinery](https://docs.rs/refinery/) (embedded SQL chain) |
| Server | [axum](https://docs.rs/axum/) | Admin WebUI only |
| Admin frontend | Vite + pnpm | SPA embedded in the binary at build time, served at `/` |
| Task runner | [mise](https://mise.jdx.dev/) | — |

## Development

Clone the repo and run the first-time setup once:

```bash
git clone https://github.com/himkt/cafleet.git
cd cafleet

mise //cafleet:install    # builds the WebUI dist, then cargo-installs the cafleet CLI (re-run after source edits)
cafleet setup --skip claude --skip codex --skip opencode   # migrate the database schema only (idempotent)
```

After that, pick the task you need by name. Every cargo-invoking task first
builds the WebUI dist (the cargo build embeds it and fails without it), so a
fresh clone needs no manual prerequisite:

| Task | Runs | When you need it |
|---|---|---|
| `mise //cafleet:lint` | `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` | Checking Rust style before a commit |
| `mise //cafleet:format` | `cargo fmt` | Applying Rust formatting fixes |
| `mise //cafleet:typecheck` | `cargo check` | Fast type-checking without producing a binary |
| `mise //cafleet:test` | `cargo test` | Running the test suite |
| `mise //cafleet:build` | `cargo build --release` | Building the release binary |
| `mise //admin:lint` | `pnpm lint` | Checking the WebUI sources |
| `mise //admin:build` | Vite build | Producing the WebUI dist the binary embeds |
| `mise //admin:dev` | Vite dev server | Working on the WebUI with hot reload |
| `mise //admin:install` | `pnpm install --frozen-lockfile` | Reinstalling WebUI deps from the committed lockfile |

To change the WebUI's dependencies, edit `admin/package.json` and run plain
`pnpm install --no-frozen-lockfile` from the repository root to regenerate `pnpm-lock.yaml` —
`mise //admin:install` installs with `--frozen-lockfile` and cannot update
the lockfile.

### Installing the skills from your checkout

`cafleet setup` installs the assets embedded in the installed binary at its
build time, so it is the **end-user (installed-CLI)** path. Contributors
working from a clone install the skills from the working tree instead:

```bash
mise //:skill-install
```

This runs `gh skill install ./ --from-local --agent <backend> --force --scope
user` for each of the three backends (`claude-code`, `codex`, `opencode`),
placing the skills from your checkout (not a Release) into the three agent
homes.

## Building docs locally

The docs site is an [rspress](https://rspress.rs/) project rooted at `docs/`,
a package in the repo's pnpm workspace. Build the documentation site (this
site) locally with:

```bash
mise //:docs-build
```

That task is a thin wrapper around `pnpm --dir docs build` and is the same
command the GitHub Actions workflow runs; it installs the pnpm dependencies
first, so a fresh clone needs no manual prerequisite. For a live-reloading
local preview while editing pages, run `pnpm --dir docs dev`.

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
