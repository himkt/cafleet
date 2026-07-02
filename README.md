# CAFleet

https://github.com/user-attachments/assets/bd2b195a-f3de-4fa3-bcc8-3c6ef9f1016a

Agent Teams reinvented for collaborative coding across multiple coding-agent backends (Claude Code, Codex, and OpenCode), with full code transparency.

## 1. Who is CAFleet for

- Developers running multi-agent coding teams in tmux who want every inter-agent message persisted and auditable.
- Teams mixing `claude`, `codex`, and `opencode` members in one fleet.
- Operators who want a single-file SQLite broker with no server to run.

## 2. Install

CAFleet works with three coding agents: `claude` (Claude Code), `codex` (OpenAI Codex CLI), and `opencode`. The broker CLI is shared by all three; the skills are installed per backend you use.

The recommended end-user path is two commands:

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet setup               # install the skills + create the database schema
```

`cafleet setup` installs the skills matching your installed `cafleet` version into every detected coding-agent home (`~/.claude`, `~/.codex`, `~/.config/opencode`) and creates the database schema. Scope the skills install to specific agents with `--agent claude|codex|opencode` (repeatable). Re-run it after upgrading the package to refresh the skills.

The default database is `~/.local/share/cafleet/cafleet.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

Contributors working from a clone install the skills from the working tree with `mise //:skill-install`. Full install details are on the Install page: <https://himkt.github.io/cafleet/get-started/install/>.

Per-coding-agent config (Claude `permissions.allow`, Codex `config.toml` + rules, opencode) lives on the Configure page: <https://himkt.github.io/cafleet/get-started/configure/>.

## 3. Examples

### 3.1. Simple example to use CAFleet

Provide the following prompt to Claude Code or Codex to see how it works.

```text
I want to see how cafleet works.
Please create a new team with two members using cafleet and let them ping-pong each other.
After the demonstration, please shutdown the team.
```

### 3.2. Real world usage; Design-doc-driven development

CAFleet provides the builtin skills for Spec Driven Development (SDD). **We're using CAFleet to develop CAFleet!**

Invoke the `cafleet-design-doc` skill (create workflow) with a one-line request, e.g.:

```text
I want to create a simple TUI calculator. Please create a design doc using the CAFleet skill.
```

See your coding-agent's skill documentation for the literal invocation syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

You can see the existing design docs on [`design-docs/`](design-docs/), which are actually created by the skills.

## 4. Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Fleets partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. Members spawn as tmux panes running any of the three coding-agent backends, optionally pinned to a specific LLM via `cafleet agent spawn --model <m>` (e.g. `sonnet`, `gpt-5.4-mini`, `anthropic/claude-sonnet-4-6`, `opencode/big-pickle`). Every command that carries a text body — `message send`, `message broadcast`, member nudges, and the member spawn prompt — takes a unified `--text <str>` / `--text-file <path>` pair (`--text-file -` reads the body from stdin), so long or multi-line bodies bypass the shell's `ARG_MAX` limit. Full architecture documentation is published at <https://himkt.github.io/cafleet/concepts/overview/>.

A Director supervises its team on a periodic heartbeat supplied by `cafleet monitor` — a per-fleet loop the fleet's dedicated **monitoring member** runs as a background task in its own pane. It spends no model tokens and works on any backend: the monitor schedules the heartbeat, and the monitoring member inspects each due agent and re-engages an idle Director on demand. Spawn the monitoring member first (`cafleet agent spawn … --role monitor --model haiku`); it launches `cafleet monitor start`, and stopping that background task or `fleet delete` ends the loop. See <https://himkt.github.io/cafleet/concepts/monitoring/>.

## 5. Contributing

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, are published at <https://himkt.github.io/cafleet/get-started/contributing/>.
