# CAFleet

https://github.com/user-attachments/assets/bd2b195a-f3de-4fa3-bcc8-3c6ef9f1016a

Agent Teams reinvented for collaborative coding across multiple coding-agent backends (Claude Code, Codex, and OpenCode), with full code transparency.

## 1. Who is CAFleet for

- Developers running multi-agent coding teams in tmux who want every inter-agent message persisted and auditable.
- Teams mixing `claude`, `codex`, and `opencode` members in one fleet.
- Operators who want a single-file SQLite broker with no server to run.

## 2. Install

CAFleet works with three coding agents: `claude` (Claude Code), `codex` (OpenAI Codex CLI), and `opencode`.
Install the plugin in whichever one you use — the broker CLI is shared.

### 2.1. CAFleet skills

Use your favorite tool to install skills (for example, GitHub CLI `gh skill`, vercel `skills`, or the marketplace of each coding agent).

#### (a) GitHub CLI (recommended)

```bash
gh skill install himkt/cafleet --agent claude-code
gh skill install himkt/cafleet --agent codex
gh skill install himkt/cafleet --agent opencode
```

#### (b) Claude Code marketplace (if you prefer)

```text
/plugin marketplace add himkt/cafleet
/plugin install cafleet@cafleet
```

#### (c) Codex (if you prefer)

```bash
codex plugin marketplace add himkt/cafleet
```


### 2.2. CAFleet CLI (required for CAFleet to function)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init             # apply schema migrations (idempotent; rerun after upgrades)
```

The default database is `~/.local/share/cafleet/cafleet.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

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

Invoke the `cafleet-design-doc-create` skill with a one-line request, e.g.:

```text
I want to create a simple TUI calculator. Please create a design doc using the CAFleet skill.
```

See your coding-agent's skill documentation for the literal invocation syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

You can see the existing design docs on [`design-docs/`](design-docs/), which are actually created by the skills.

## 4. Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Fleets partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. Members spawn as tmux panes running any of the three coding-agent backends, optionally pinned to a specific LLM via `cafleet member create --model <m>` (e.g. `sonnet`, `gpt-5.4-mini`, `anthropic/claude-sonnet-4-6`, `opencode/big-pickle`). Full architecture documentation is published at <https://himkt.github.io/cafleet/concepts/overview/>.

A Director supervises its team on a periodic heartbeat supplied by `cafleet monitor` — a per-fleet `scan → ping → sleep` loop that the fleet's dedicated **monitoring member** runs as a background task in its own pane. The loop wakes only two agents — the root Director and the monitoring member itself — with **`Esc`-safeguarded** keystrokes: an `Escape` precedes every ping so a pane sitting on a pending permission prompt can't have the prompt confirmed by the trailing `Enter`. Ordinary members are never pinged by the loop; the monitoring member instead watches the Director's pane and re-engages it when the team stalls. Because the loop is just a backgrounded command, a Director on **any** backend gets the same heartbeat. Spawn the monitoring member first with `cafleet --fleet-id <id> member create --agent-id <director> --name monitor --description "…" --role monitor --model sonnet --prompt-file <…>`; it starts the loop via `cafleet --fleet-id <id> monitor start` (`monitor status` / `monitor config` round out the surface), stops it by stopping that background task, and `fleet delete` makes the loop self-terminate. See <https://himkt.github.io/cafleet/concepts/monitoring/>.

## 5. Contributing

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, are published at <https://himkt.github.io/cafleet/get-started/contributing/>.
