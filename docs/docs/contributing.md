# Contributing

CAFleet is developed using its own CAFleet-orchestrated skills — the
repository dogfoods the spec-driven-development flow it ships. This document
covers the project layout, the local development loop, and the contribution
path.

## Project structure

| Top-level entry | Purpose |
|---|---|
| `cafleet/` | The `cafleet` Rust package (clap CLI + axum server + rusqlite persistence); builds the single `cafleet` binary. |
| `admin/` | Admin WebUI SPA (Vite + React + TypeScript + Tailwind CSS); its build output is embedded in the binary. |
| `skills/` | Coding-agent skill files (`cafleet`, `cafleet-design-doc`), installed into the agent homes by `cafleet setup` / `mise //:skill-install`. |
| `admin/package.json`, `docs/package.json` | Per-package pnpm manifests — `admin/` and `docs/` are standalone pnpm packages, each with its own committed `pnpm-lock.yaml`; `node_modules/` is gitignored. |
| `design-docs/` | Numbered design documents (`NNNNNNN-<slug>/design-doc.md`). |
| `docs/` | The rspress documentation-site project (standalone pnpm package `cafleet-docs`): `rspress.config.ts` and the operator-facing pages in its nested `docs/` content root. |

## Rust boundaries and compatibility

When changing the broker, decode SQL rows into typed records once and construct
wire JSON in CLI or HTTP presenters. Move consumers in order: member,
placement, message, then monitor; remove a temporary JSON compatibility adapter
once its consumers use the typed interface. The record fields and nullable
states are defined in [Data model](spec/data-model.md#typed-broker-records).

Keep concrete process execution and notifications in `runtime/`:

```text
CLI / HTTP → runtime adapters → broker notification traits
                             → multiplexer / coding-agent interfaces
CLI / HTTP → broker records → CLI / HTTP presenters
```

`SystemRunner`, `SystemProbe`, and the concrete notifier adapter live below the
CLI. The broker owns notification policy and its trait, while the runtime
adapter performs the transport call. The broker does not start subprocesses
or import HTTP/CLI handlers; HTTP handlers do not import CLI helpers. Continue
using injectable runners, probes, and notification traits in tests.

Treat the internal type migration as an implementation change. Preserve JSON
key order, names, nulls, envelopes, text output, exit codes, and guard order.
Introduce domain error variants only when a caller must branch, and translate
them at each CLI/HTTP boundary using that operation's existing contract. A
missing required message name becomes an HTTP 500 integrity error rather than
a panic or invented name; see [response compatibility](spec/webui-api.md#response-compatibility).

Preserve the distinction between durable delivery and notification. Typed
outcomes retain the stored message id, notification attempt, and raw transport
diagnostic. A failed preview does not roll back or resend the message; keep the
[CLI partial-failure output and recovery instructions](spec/cli-options.md#message-send-partial-failure).

## Shared diagnosis and query refactoring

Shared diagnosis and connection reuse preserve CLI behavior while roster
query separation and batched name lookup avoid unused database work. Idle
calculation clamps only its final result to zero.

Keep `Diagnosis` as facts: `SchemaState` is `Missing`, `Unversioned`, `Behind`,
`Head`, `Ahead`, or `Unreachable`; `AssetState` records each agent's resolved
path/source, matching or stale current install, missing current install, or
path error, with superseded installs kept separately. Preserve versions and
raw causes, and let CLI presenters supply the existing wording. These types
must not add fields or enum names to doctor JSON.

Use the invocation's existing SQLite connection for schema diagnosis, assets
checks, and command work. Collect/apply schema first, then assets only when
the command's policy permits it. Doctor still reports all three sections;
setup still attempts assets after DB failure and refreshes diagnosis after
DB creation/migration on the same connection. Reconnect when the first open
failed rather than preventing recovery. HTTP retains a connection per
blocking handler. The full guard and setup behavior is in
[diagnosis reuse](spec/cli-options.md#diagnosis-reuse).

Keep ownership available for failure cleanup: fleet creation takes the
invocation's `Option<Connection>` slot so it can close the database before
pane compensation on broker failure. The slot then stays empty; successful
creation keeps the same connection. Observers report the real remaining
connection and must not reopen it just to supply an observation.

Keep `list_roster_records` free of message activity aggregates;
`list_member_records` supplies activity for CLI member listing. Batch
`get_member_names` by at most 500 unique bound ids, preserving empty-input,
unknown-id, and deregistered-member behavior. See
[query and activity contracts](spec/data-model.md#query-and-activity-contracts).
Tests should observe query counts and selected fields, connection reuse and
guard order, plus fixed-clock activity results; implementation details such
as SQL whitespace are not the contract.

## Member history query limits

The [planned history limit contract](spec/webui-api.md#member-history-limits)
keeps unbounded broker/HTTP callers compatible and bounds the WebUI's inbox
and sent requests at 201 rows for a 200-row display. Keep limits in bound SQL
parameters after the delivery filter and deterministic status/id ordering;
do not fetch all history and truncate it in Rust. Reuse the existing
`idx_messages_owner_member_status_ts` and `idx_messages_from_member_status_ts`
indexes, inspect `EXPLAIN QUERY PLAN`, and add no index for this change.

Verify 0/200/201/large histories, timestamp ties, foreign-fleet rejection,
deregistered members, invalid and duplicate limits, validation precedence,
and omission compatibility. Observe the real prepared statement and its
bindings to distinguish SQL limiting from result truncation. Test the actual
frontend fetch functions with a mocked `fetch`, and test display slicing and
the overflow flag through the same pure helper the component uses. These
checks need neither a browser nor a running server; preserve the existing
refresh and error behavior.

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
cafleet setup             # migrate the database schema and install the embedded assets (idempotent)
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
`pnpm install --no-frozen-lockfile` in `admin/` to regenerate
`admin/pnpm-lock.yaml` — `mise //admin:install` installs with
`--frozen-lockfile` and cannot update the lockfile.

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

Order matters: run `cafleet setup` first, then `mise //:skill-install` —
`cafleet setup` overwrites the agent-home skills with the binary's
build-time-embedded assets, so re-run `mise //:skill-install` after any
later `cafleet setup` to restore the working-tree skills.

## Building docs locally

The docs site is an [rspress](https://rspress.rs/) project rooted at `docs/`,
a standalone pnpm package with its own lockfile. Build the documentation site
(this site) locally with:

```bash
mise //docs:install
mise //docs:build
```

These tasks are thin wrappers around `pnpm install --frozen-lockfile` and
`pnpm build` in `docs/` (defined in `docs/mise.toml`) and are the same
commands the GitHub Actions workflow runs. For a live-reloading local
preview while editing pages, run `mise //docs:dev`.

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
  procedures, and rationale as prose. Every table has at least two data rows
  and cells of at most two sentences. An enumeration that belongs on more
  than one page gets one owning page carrying the table; every other mention
  is a link plus a one-clause summary.
