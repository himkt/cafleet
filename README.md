# CAFleet

https://github.com/user-attachments/assets/bd2b195a-f3de-4fa3-bcc8-3c6ef9f1016a

Agent Teams reinvented for collaborative coding across multiple coding-agent backends (Claude Code, Codex, and OpenCode), with full code transparency.

## 1. Features

- **Multi-backend teams** — mix `claude`, `codex`, and `opencode` members in a single fleet with no broker-level differences.
- **Persistent, auditable messages** — every inter-agent message is stored in SQLite; deregistered agents and their history remain readable.
- **Single-file broker** — one SQLite database; no separate database daemon to run or monitor. CLI commands write directly through a shared broker module without requiring a running server.
- **Fleet isolation** — fleets are non-overlapping namespaces; cross-fleet agents are invisible to each other.
- **Pluggable multiplexer backends** — host member panes in **tmux** or **herdr** ([herdr.dev](https://herdr.dev)); the active backend is auto-detected from the environment (`HERDR_ENV` / `TMUX`) and overridable with `CAFLEET_MULTIPLEXER`. On herdr, the monitor also reacts to native `blocked`/`done` agent states.
- **Push notifications** — after persisting a message, the broker keystrokes a 2-line inline preview into the recipient's multiplexer pane so coding agents receive messages without invoking `cafleet message poll`.
- **Monitoring member** — a dedicated member runs `cafleet monitor start` as a background task, scheduling periodic heartbeats that keep the Director informed of idle or stalled team members without spending model tokens on scheduling.
- **Design-doc-driven development** — built-in skills for spec-driven development (SDD), used to evolve CAFleet itself.
- **Unified CLI** — a single `cafleet` command covers fleet management, member lifecycle, messaging, monitoring, and the admin WebUI server.

## 2. Architecture

```
CLI (click) ──────────────► broker/ (sync SQLAlchemy)
                                        │
Admin WebUI ──► webui/app.py ──► webui/api.py
                  (FastAPI)              │
monitor loop ────────────────────────────┤
                                         ▼
                              SQLite (fleets / agents / tasks /
                                agent_placements /
                                monitor_config / monitor_runtime /
                                skill_installs)
                                         │
                             ┌──── multiplexer pane ◄──── inline-preview keystroke
                             │
                     coding-agent pane  monitoring member pane
```

**Key design decisions:**

- The `cafleet` CLI and the admin WebUI both access SQLite directly through a shared `broker` module — no HTTP server is needed for agent operations.
- Agents are organized into **fleets** identified by a non-secret integer `fleet_id`. Agents sharing the same fleet can discover and message each other; agents in different fleets are invisible to one another.
- A fleet has exactly one **root Director** (created by `cafleet fleet create`). Only the root Director may own members. Members are spawned by the Director via `cafleet member create` and are bound to individual multiplexer panes (tmux or herdr).
- The `member` group is the single home for the agent lifecycle — spawn, teardown, introspection (`show`, `list --all`), and keystroke interaction.
- A dedicated **monitoring member** runs `cafleet monitor start` as a background task to supply the heartbeat. The monitor decides only *when* agents are due; the monitoring member inspects each due agent and re-engages an idle Director on demand.

Full architecture documentation: <https://himkt.github.io/cafleet/concepts/overview/>

## 3. Quick Start

### Prerequisites

- Python 3.12+
- A terminal multiplexer — **tmux** or **herdr** (required for `fleet create` and all `member *` commands that touch panes). The backend is auto-detected from the environment (`HERDR_ENV` / `TMUX`); set `CAFLEET_MULTIPLEXER` to `tmux` or `herdr` to override.
- At least one of: `claude` (Claude Code), `codex` (OpenAI Codex CLI), or `opencode`

### Install

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet setup               # migrate the database schema + install the skills
```

`cafleet setup` is the one-step onboarding command — bare `cafleet setup` runs two independent halves in order: first migrates the database schema (applies the bundled Alembic migrations up to the head revision, creating the database file on first run), then installs the skills matching your installed `cafleet` version into every detected coding-agent home (`~/.claude`, `~/.codex`, `~/.config/opencode`). Each half fails independently; the command exits non-zero if either fails. Bare `cafleet setup` takes no options.

Each half is also available as its own subcommand: `cafleet setup db` (schema only) and `cafleet setup skill [--agent claude|codex|opencode]...` (skills only; `--agent` is repeatable and scopes the install). Re-run `cafleet setup` or `cafleet setup skill` after upgrading to refresh the skills.

The default database is `~/.local/share/cafleet/cafleet.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

Contributors working from a clone install the skills from the working tree with `mise //:skill-install`. Full details: <https://himkt.github.io/cafleet/get-started/install/>

Per-agent config lives on the Configure page: <https://himkt.github.io/cafleet/get-started/configure/>

### Basic usage

Run inside a tmux or herdr session. Create a fleet (records your current pane as the root Director):

```bash
cafleet fleet create --label "my-project"
# → 1 director=2 admin=3
```

Spawn a member pane (the Director's id is `2`, fleet id is `1`):

```bash
cafleet member create --fleet-id 1 --agent-id 2 \
  --name "worker" --description "Worker member" \
  --text "You are worker. Reply hello when polled."
# → 4 worker backend=claude pane=%7
```

List every active agent in the fleet (includes Director, Administrator, and all members):

```bash
cafleet member list --fleet-id 1 --all
```

Inspect a specific agent's details:

```bash
cafleet member show --fleet-id 1 --member-id 4 --full
```

Send a message from the worker (`4`) to another member (`5`):

```bash
cafleet message send --fleet-id 1 --agent-id 4 --to 5 --text "hello"
```

Tear down when done:

```bash
cafleet member delete --fleet-id 1 --member-id 4
cafleet fleet delete --fleet-id 1
```

The full walkthrough is at <https://himkt.github.io/cafleet/get-started/quickstart/>

### Try it with a coding agent

For the simplest path, ask Claude Code or Codex to demonstrate CAFleet:

```text
I want to see how cafleet works.
Please create a new team with two members using cafleet and let them ping-pong each other.
After the demonstration, please shutdown the team.
```

CAFleet also ships built-in skills for spec-driven development (SDD). **CAFleet is developed using CAFleet itself.** Invoke the `cafleet-design-doc` skill (create workflow) with a one-line request:

```text
I want to create a simple TUI calculator. Please create a design doc using the CAFleet skill.
```

Existing design docs are in [`design-docs/`](design-docs/). See your coding-agent's skill documentation for the literal invocation syntax.

## 4. CLI Usage

The unified `cafleet` CLI is organized as three top-level commands (`setup`, `doctor`, `server`) plus four command groups. Every `member *`, `message *`, and `monitor *` command, plus `fleet show` and `fleet delete`, requires `--fleet-id <int>` immediately after the subcommand name.

### Top-level commands

| Command | Description |
|---|---|
| `cafleet setup` | Migrate the database schema and install the coding-agent skills in one step (two independent halves, run in order) |
| `cafleet setup db` | Migrate the database schema only (idempotent) |
| `cafleet setup skill` | Install the coding-agent skills only and record the installed version (`--agent claude\|codex\|opencode`, repeatable) |
| `cafleet doctor` | Print the resolved multiplexer backend, the calling pane's session/window/pane identifiers, and the skills-install report (recorded version per home, stale/ok status) |
| `cafleet server` | Start the admin WebUI server (default `127.0.0.1:8000`) |

### `fleet` group — fleet lifecycle

| Subcommand | Description |
|---|---|
| `fleet create` | Create a fleet with its root Director and built-in Administrator |
| `fleet list` | List all non-deleted fleets |
| `fleet show` | Show one fleet (soft-deleted rows included) |
| `fleet delete` | Soft-delete a fleet and deregister its agents |

### `member` group — agent lifecycle and pane interaction

| Subcommand | Description |
|---|---|
| `member create` | Register a member and spawn its coding-agent pane (`--agent-id` Director, `--name`, `--description`, `--text`/`--text-file`) |
| `member delete` | Tear down a member's pane (when one exists) and deregister it; `--force` kills immediately |
| `member show` | Show one agent's detail (`--member-id` target; `--full` for the labeled block with kind, skills, and placement) |
| `member list` | List the fleet's members; `--all` lists every active agent with a `kind` column; `--activity` shows message timestamps |
| `member capture` | Capture the tail of a member's multiplexer pane |
| `member exec` | Dispatch a shell command into a member's pane via the `!` shortcut |
| `member ping` | Inject an inbox-poll keystroke into a member's pane |
| `member nudge` | Deliver an ACKable task and inline preview to a member (typically the Director) |

### `message` group — message broker

| Subcommand | Description |
|---|---|
| `message send` | Send a unicast message (`--to`, `--text`/`--text-file`) |
| `message broadcast` | Broadcast a message to all fleet agents (excluding the Administrator) |
| `message poll` | Fetch un-acked incoming messages |
| `message ack` | Acknowledge a received message (`--task-id`) |
| `message cancel` | Retract an un-acked sent message (`--task-id`) |
| `message show` | Show one task (`--task-id`) |

### `monitor` group — supervision scheduler

| Subcommand | Description |
|---|---|
| `monitor start` | Run the per-fleet scheduler loop in-process (the monitoring member launches this as a background task) |
| `monitor status` | Show monitor liveness and the per-agent schedule |
| `monitor config` | Show or edit an agent's monitor schedule (`--interval`, `--enable`/`--disable`) |

Global flags (placed before the subcommand): `--json` (compact JSON output), `--version`. Per-subcommand: `--full` (untruncated output and verbose labeled block for `member show`; full envelope for messages). The `--text`/`--text-file` pair is accepted by `message send`, `message broadcast`, `member nudge`, and `member create` — exactly one of the two is required; `--text-file -` reads from stdin.

Full CLI reference: <https://himkt.github.io/cafleet/spec/cli-options/>

## 5. API Overview

### REST Registry API

The admin WebUI exposes a JSON API under `/api/`. All fleet-scoped routes require the `X-Fleet-Id` header.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/fleets` | List fleets |
| `GET` | `/api/agents` | List agents in the fleet (with folded monitor schedule) |
| `GET` | `/api/monitor` | Fleet monitor runtime liveness |
| `GET` | `/api/agents/{agent_id}/monitor` | Get an agent's monitor config |
| `PATCH` | `/api/agents/{agent_id}/monitor` | Update an agent's monitor schedule |
| `GET` | `/api/agents/{agent_id}/inbox` | Inbox messages for an agent |
| `GET` | `/api/agents/{agent_id}/sent` | Messages sent by an agent |
| `GET` | `/api/timeline` | Unified fleet timeline |
| `POST` | `/api/messages/send` | Send a message (Administrator as implicit sender) |

Full API reference: <https://himkt.github.io/cafleet/spec/webui-api/>

## 6. Tech Stack

- **Language:** Python 3.12+, managed with [uv](https://docs.astral.sh/uv/)
- **Server:** [FastAPI](https://fastapi.tiangolo.com/) (admin WebUI)
- **Database:** [SQLAlchemy](https://www.sqlalchemy.org/) 2.x (sync `pysqlite` driver) + SQLite
- **CLI:** [click](https://click.palletsprojects.com/)
- **Admin frontend:** Vite + Bun (SPA served at `/`)
- **Task runner:** [mise](https://mise.jdx.dev/)

## 7. Project Structure

```
cafleet/              Python package (broker, CLI, WebUI, monitor, coding-agent)
  src/cafleet/        Source tree
  tests/              Test suite
admin/                Admin WebUI frontend (Vite)
docs/                 Documentation source
  concepts/           Architecture concepts
  get-started/        Install, quickstart, configure, contributing
  how-to/             Task-oriented guides
  api/                API reference
  spec/               CLI options, data model, message envelope, WebUI API
skills/               Coding-agent skill files (cafleet, cafleet-design-doc, cafleet-research)
design-docs/          Design documents (committed alongside implementation)
```

## 8. Development

```bash
git clone https://github.com/himkt/cafleet
cd cafleet
mise //:uv-sync          # sync dependencies
mise //cafleet:test      # run the test suite
mise //cafleet:lint      # lint
mise //cafleet:typecheck # type-check
mise //cafleet:dev       # start the WebUI dev server (127.0.0.1:8000)
```

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, are published at <https://himkt.github.io/cafleet/get-started/contributing/>

## 9. License

MIT
