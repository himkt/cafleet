# CAFleet

https://github.com/user-attachments/assets/a66620cb-4a81-4525-95f2-1f1f22765288

Agent Teams reinvented for collaborative coding across multiple coding-agent backends, with full code transparency.

## 1. Who is CAFleet for

- Developers running multi-agent coding teams in tmux who want every inter-agent message persisted and auditable.
- Teams mixing `claude`, `codex`, and `opencode` members in one fleet.
- Operators who want a single-file SQLite broker with no server to run.

## 2. See it work

Create a fleet, spawn a member pane, message it, and tear everything down — five commands inside a tmux session. The ids below are samples — fleet `1`, root Director `2`, built-in Administrator `3`, member `4`, task `10`; substitute the integers your own commands print.

```bash
cafleet fleet create --label "demo"
```

```
1 director=2 admin=3
```

```bash
cafleet --fleet-id 1 member create --agent-id 2 --name "demo-member" \
  --description "Demo member" -- "You are demo-member. Reply hello when polled."
```

```
4 demo-member backend=claude pane=%7
```

```bash
cafleet --fleet-id 1 message send --agent-id 2 --to 4 --text "hi"
```

```
Message sent.
[10 | from:2 | 2026-06-11T09:00:00.123456+00:00]
hi
```

```bash
cafleet --fleet-id 1 member delete --member-id 4
cafleet fleet delete 1
```

```
Deleted fleet 1. Deregistered 2 agents.
```

The full walkthrough with every expected output is the quickstart: <https://himkt.github.io/cafleet/get-started/quickstart/>. Prompt-first task guides live at <https://himkt.github.io/cafleet/how-to/>.

## 3. Install

CAFleet works with three coding agents: `claude` (Claude Code), `codex` (OpenAI Codex CLI), and `opencode`.
Install the plugin in whichever one you use — the broker CLI is shared.

### 3.1. CAFleet skills

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


### 3.2. CAFleet CLI (required for CAFleet to function)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init             # apply schema migrations (idempotent; rerun after upgrades)
```

The default database is `~/.local/share/cafleet/cafleet.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

Per-coding-agent config (Claude `permissions.allow`, Codex `config.toml` + rules, opencode) lives on the Configure page: <https://himkt.github.io/cafleet/get-started/configure/>.

## 4. Examples

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
I want to create a simple TUI calculator.
```

See your coding-agent's skill documentation for the literal invocation syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

You can see the existing design docs on [`design-docs/`](design-docs/), which are actually created by the skills.

## 5. Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Fleets partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. Members spawn as tmux panes running any of the three coding-agent backends, optionally pinned to a specific LLM via `cafleet member create --model <m>` (e.g. `sonnet`, `gpt-5.4-mini`, `anthropic/claude-sonnet-4-6`). Full architecture documentation is published at <https://himkt.github.io/cafleet/concepts/overview/>.

## 6. Contributing

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, are published at <https://himkt.github.io/cafleet/get-started/contributing/>.
