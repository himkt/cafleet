# CAFleet

https://github.com/user-attachments/assets/a66620cb-4a81-4525-95f2-1f1f22765288

Agent Teams reinvented for collaborative coding across multiple coding-agent backends, with full code transparency.

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

### 4.1. Simple example to use CAFleet

Provide the following prompt to Claude Code or Codex to see how it works.

```text
I want to see how cafleet works.
Please create a new team with two members using cafleet and let them ping-pong each other.
After the demonstration, please shutdown the team.
```

### 4.2. Real world usage; Design-doc-driven development

CAFleet provides the builtin skills for Spec Driven Development (SDD). **We're using CAFleet to develop CAFleet!**

Invoke the `cafleet-design-doc-create` skill with a one-line request, e.g.:

```text
I want to create a simple TUI calculator. Please create design doc using cafleet skill.
```

See your coding-agent's skill documentation for the literal invocation syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

You can see the existing design docs on [`design-docs/`](design-docs/), which are actually created by the skills.

## 4. Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Fleets partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. Members spawn as tmux panes running any of the three coding-agent backends, optionally pinned to a specific LLM via `cafleet member create --model <m>` (e.g. `sonnet`, `gpt-5.4-mini`, `anthropic/claude-sonnet-4-6`, `opencode/big-pickle`). Full architecture documentation is published at <https://himkt.github.io/cafleet/concepts/overview/>.

## 5. Contributing

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, are published at <https://himkt.github.io/cafleet/get-started/contributing/>.
