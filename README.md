# CAFleet

A message broker and agent registry for coding agents — a Claude Code plugin plus a local broker CLI.

> **CAFleet is a local-only tool.** It runs on a single developer machine without authentication. Do not expose the broker on a shared network unless you accept that every listener can see and act within every session.

## Install

### Install CAFleet skills

#### Claude Code

Run the following commands on your terminal:

```
/plugin marketplace add himkt/cafleet
/plugin install cafleet@himkt-cafleet
```

> [!IMPORTANT]
>
> Make sure whther the skills are correctly installed. You can see available skills by running `/skills` on Claude Code prompt.
> 
> Typically, the config entries like following would be written in `~/.claude/settings.json`:
>
> ```json
> "enabledPlugins": {
>   "cafleet@cafleet": true
> },
> "extraKnownMarketplaces": {
>   "cafleet": {
>     "source": {
>       "source": "directory",
>       "path": "/home/himkt/work/himkt/cafleet"
>     }
>   }
> },
> ```
>

#### Codex

Run the following command on your terminal

```
codex plugin marketplace add himkt/cafleet
```

> [!IMPORTANT]
>
> Make sure whther the skills are correctly installed. You can see available skills by running `/skills` on Codex prompt.
> 
> Typically, the config entries like following would be written in `~/.codex/config.toml`:
>
> ```toml
> [marketplaces.cafleet]
> last_updated = "2026-05-16T11:56:51Z"
> last_revision = "03a8caa66c8a7981345d74fe3aec9a6e498792a1"
> source_type = "git"
> source = "https://github.com/himkt/cafleet.git"
>
> [plugins."cafleet@cafleet"]
> enabled = true
> ```
>

### Install CAFleet CLI (required for CAFleet to function)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init             # apply schema migrations (idempotent; rerun after upgrades)
```

The default database is `~/.local/share/cafleet/registry.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

## Simple example to use CAFleet on Claude Code or Codex

In any tmux session, paste this into Claude Code:

```
/cafleet:design-doc-create I want to create a simple TUI calculator
```

Codex:

```
$cafleet:design-doc-create I want to create a simple TUI calculator
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
| `cafleet base-dir {resolve,record}` | Resolve / persist the `${BASE}` output-root used by every CAFleet scratch / audit / figure write (filesystem-only; no `--session-id`). `cafleet base-dir resolve [TASK_NAME]` accepts an optional positional `TASK_NAME` (relative path the consuming skill picks, e.g. `researches/<slug>` or `design-docs/<NNNNNNN>-<slug>`; or an absolute path strictly under the repo root); when supplied, returns the auto-created task folder as `${BASE}` and writes a per-task anchor inside it. Consumer-strips contract: the resolver does not strip child filenames; each consuming skill canonicalizes its argument before calling. |

> CLI reference (per-command sections for `session`, `member`, `doctor`, `server`; `agent` / `message` / `db init` covered via the option-source table and `cafleet <cmd> --help`): [docs/spec/cli-options.md](docs/spec/cli-options.md). Message envelope shape (compact rendered + `--full` typed-column): [docs/spec/message-envelope.md](docs/spec/message-envelope.md).

### Notable flags

| Flag / variable | Where | One-line purpose |
|---|---|---|
| `--pretty` | global, before subcommand | Switch JSON output from the compact default to indented (`json.dumps(..., indent=2)`). Pair with `--json`. |
| `--quiet` | `message send` / `message ack` / `member ping` | Emit only the new task id (8-char prefix); silence the rest of the echo. |
| `--full` | `message *` / `agent list` / `agent show` | Disable body / envelope / agent-card truncation; emit the full typed-column shape. |
| `--activity` | `member list` | Add per-member `last_sent` / `last_recv` / `last_ack` / `idle` columns aggregated from `tasks`. |
| `--lines` / `--tail` / `--ansi` / `--no-ansi` | `member capture` | Default `--lines 30` (was 80); `--tail` is an alias for `--lines`; `--no-ansi` (default) strips ANSI escapes. |
| `CAFLEET_MAX_TEXT_LEN` | env var | Body-truncation codepoint limit (default `200`). See "Message body truncation" below. |

### Coding agents

cafleet supports two coding-agent binaries for member panes: `claude` (Claude Code) and `codex` (OpenAI Codex CLI). Pass `--coding-agent {claude,codex}` on `cafleet session create` (operator-declared metadata for the root Director) and `cafleet member create` (selects the spawn-command builder and records the placement). The default is `claude`, so existing invocations are unchanged. A single Director may spawn both `claude` and `codex` members in the same session. Operational details for codex members — including the codex CLI version pin and verification recipe — live in [docs/codex-members.md](docs/codex-members.md).

> [!IMPORTANT]
> Codex members need the cafleet DB directory to be writable from inside the codex sandbox. Add it to `sandbox_workspace_write.writable_roots` in any `config.toml` codex reads (e.g. `~/.codex/config.toml`):
>
> ```toml
> [sandbox_workspace_write]
> writable_roots = ["/home/<you>/.local/share/cafleet"]
> ```
>
> Use the absolute path matching `CAFLEET_DATABASE_URL` or the default XDG location.

### Message body truncation

`cafleet message {send,poll,ack,cancel,show}` truncate the message `text` body to the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints (default `200`) plus a single `…` codepoint suffix in both text and `--json` output by default. This collapses per-poll token cost for inbox-polling agents whose bodies typically run several hundred characters. Pass `--full` (per-subcommand option, placed after the subcommand name) to restore the un-truncated body and the full typed-column envelope. Empty bodies and bodies whose codepoint length is at most `CAFLEET_MAX_TEXT_LEN` pass through unchanged with no marker. `cafleet message broadcast` is different — it returns a `broadcast_summary` task whose text is a generated summary string (e.g. `Broadcast sent to N recipients`), not the original body, so its summary always emits in full. The `--full` flag is preserved on `message broadcast` for surface consistency but is a no-op. The `/ui/api/*` WebUI responses are not truncated. See [docs/spec/cli-options.md](docs/spec/cli-options.md) § Message Body Truncation and [docs/spec/message-envelope.md](docs/spec/message-envelope.md) for the full rendering rules.

## Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Sessions partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design.

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

## License

MIT
