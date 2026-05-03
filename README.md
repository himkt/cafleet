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
| `cafleet session create [--coding-agent {claude,codex}]` | Create a session; declare the root Director's coding-agent backend (default `claude`) |
| `cafleet session *` | List / show / delete sessions |
| `cafleet agent *` | Register / deregister / list / show agents |
| `cafleet message *` | Send / broadcast / poll / ack / cancel / show messages |
| `cafleet member create [--coding-agent {claude,codex}]` | Spawn a member pane running `claude` (default) or `codex` |
| `cafleet member *` | Delete / list / capture / send-input / exec / ping member panes (Director only) |
| `cafleet server` | Start the admin WebUI on `127.0.0.1:8000` |
| `cafleet doctor` | Print the calling pane's tmux identifiers |

> CLI reference (per-command sections for `session`, `member`, `doctor`, `server`; `agent` / `message` / `db init` covered via the option-source table and `cafleet <cmd> --help`): [docs/spec/cli-options.md](docs/spec/cli-options.md).

### Coding agents

cafleet supports two coding-agent binaries for member panes: `claude` (Claude Code) and `codex` (OpenAI Codex CLI). Pass `--coding-agent {claude,codex}` on `cafleet session create` (operator-declared metadata for the root Director) and `cafleet member create` (selects the spawn-command builder and records the placement). The default is `claude`, so existing invocations are unchanged. A single Director may spawn both `claude` and `codex` members in the same session. Operational details for codex members — including the codex CLI version pin and verification recipe — live in [docs/codex-members.md](docs/codex-members.md).

### Message body truncation

`cafleet message {send,poll,ack,cancel,show}` truncate the message `text` body to the first 10 Unicode codepoints with a literal `...` suffix in both text and `--json` output by default. This collapses per-poll token cost for inbox-polling agents whose bodies typically run 200–500 characters. Pass `--full` (per-subcommand option, placed after the subcommand name) to restore the un-truncated body. Empty bodies and bodies of 10 codepoints or fewer pass through unchanged with no `...` marker. `cafleet message broadcast` is different — it returns a `broadcast_summary` task whose text is generated summary text (e.g. `Broadcast sent to N recipients`), not the original body, so its summary always emits in full. The `--full` flag still exists on `message broadcast` for surface consistency but is a no-op. The `/ui/api/*` WebUI responses are not truncated.

## Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Sessions partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design.

## Development

Clone the repo and use mise for all common tasks:

```bash
git clone https://github.com/himkt/cafleet.git
cd cafleet

mise //cafleet:sync       # install dependencies
mise //cafleet:install    # editable uv tool install of the cafleet CLI
cafleet db init           # apply schema migrations (idempotent; rerun after upgrades)

mise //cafleet:lint       # ruff check + ruff format --check
mise //cafleet:format     # ruff format
mise //cafleet:typecheck  # ty
mise //cafleet:test       # pytest

mise //admin:build        # build the WebUI (required before /ui/ is served)
mise //admin:dev          # WebUI dev server (Vite)
```

## License

MIT
