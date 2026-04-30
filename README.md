# CAFleet

A2A-inspired message broker and agent registry for coding agents — a Claude Code plugin plus a local broker CLI.

> **CAFleet is a local-only tool.** It runs on a single developer machine without authentication. Do not expose the broker on a shared network unless you accept that every listener can see and act within every session.

## Install

### Install the plugin in Claude Code

```
/plugin marketplace add himkt/cafleet
/plugin install cafleet@himkt-cafleet
```

This adds 5 skills under the `cafleet` namespace: `cafleet`, `cafleet-monitoring`, `design-doc`, `design-doc-create`, `design-doc-execute`. Run `/help` in Claude Code to see them.

### Install the broker CLI (required for the plugin to function)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init             # apply schema migrations (idempotent; rerun after upgrades)
```

The default database is `~/.local/share/cafleet/registry.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

## Try it

In any tmux session, paste this into Claude Code:

```
/cafleet:design-doc-create my-feature
```

Claude (the Director) bootstraps a CAFleet session, spawns a Drafter and a Reviewer in adjacent tmux panes, drives the clarification → draft → review loop through the message broker, and lands a polished design doc at `design-docs/my-feature/design-doc.md`.

Want more? See [`skills/cafleet/SKILL.md`](skills/cafleet/SKILL.md) for the raw broker primitives and [`skills/design-doc-create/SKILL.md`](skills/design-doc-create/SKILL.md) for the orchestration this example uses.

## CLI cheatsheet

| Command group | One-line purpose |
|---|---|
| `cafleet db init` | Apply schema migrations (idempotent; rerun after upgrades) |
| `cafleet session *` | Create / list / show / delete sessions |
| `cafleet agent *` | Register / deregister / list / show agents |
| `cafleet message *` | Send / broadcast / poll / ack / cancel / show messages |
| `cafleet member *` | Spawn / delete / list / capture / send-input / exec / ping member panes (Director only) |
| `cafleet server` | Start the admin WebUI on `127.0.0.1:8000` |
| `cafleet doctor` | Print the calling pane's tmux identifiers |

> CLI reference (per-command sections for `session`, `member`, `doctor`, `server`; `agent` / `message` / `db init` covered via the option-source table and `cafleet <cmd> --help`): [docs/spec/cli-options.md](docs/spec/cli-options.md).

## Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Sessions partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design.

## Development

Clone the repo and use mise for all common tasks:

```bash
git clone https://github.com/himkt/cafleet.git
cd cafleet

mise //cafleet:sync       # install dependencies
mise //cafleet:install    # editable uv tool install of the cafleet CLI
cafleet db init           # one-time schema setup

mise //cafleet:lint       # ruff check + ruff format --check
mise //cafleet:format     # ruff format
mise //cafleet:typecheck  # ty
mise //cafleet:test       # pytest

mise //admin:build        # build the WebUI (required before /ui/ is served)
mise //admin:dev          # WebUI dev server (Vite)
```

## License

MIT
