# CAFleet

https://github.com/user-attachments/assets/a66620cb-4a81-4525-95f2-1f1f22765288

Agent Teams reinvented for collaborative coding across multiple coding-agent backends, with full code transparency.

## 1. Install

CAFleet works with three coding agents: `claude` (Claude Code), `codex` (OpenAI Codex CLI), and `opencode`.
Install the plugin in whichever one you use — the broker CLI is shared.

### 1.1. CAFleet skills

Use your favorite tool to install skills (for example, GitHub CLI `gh skill`, vercel `skills`, or marketplace of each coding agent.

#### (a) GitHub CLI (recommended)

```
gh skill install himkt/cafleet --agent claude-code
gh skill install himkt/cafleet --agent codex
gh skill install himkt/cafleet --agent opencode
```

#### (b) Claude Code marketplace (if you prefer)

```
/plugin marketplace add himkt/cafleet
/plugin install cafleet@cafleet
```

#### (c) Codex (if you prefer)

```
codex plugin marketplace add himkt/cafleet
```


### 1.2. CAFleet CLI (required for CAFleet to function)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init             # apply schema migrations (idempotent; rerun after upgrades)
```

The default database is `~/.local/share/cafleet/registry.db`. Override with `CAFLEET_DATABASE_URL` (use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs).

## 2. Recommended settings

### 2.1. Claude Code

> [!TIP]
>
> Typically, the config entries like following would be written in `~/.claude/settings.json`:
>
> ```json
> {
>   "permissions": {
>     "allow": [
>       "Bash(cafleet *)",
>       "Skill(cafleet:cafleet)",
>       "Skill(cafleet:cafleet-agent-team-monitoring)",
>       "Skill(cafleet:cafleet-agent-team-supervision)",
>       "Skill(cafleet:cafleet-base-dir)",
>       "Skill(cafleet:cafleet-create-figure)",
>       "Skill(cafleet:cafleet-design-doc)",
>       "Skill(cafleet:cafleet-design-doc-create)",
>       "Skill(cafleet:cafleet-design-doc-execute)",
>       "Skill(cafleet:cafleet-design-doc-interview)",
>       "Skill(cafleet:cafleet-my-slidev)",
>       "Skill(cafleet:cafleet-research-presentation)",
>       "Skill(cafleet:cafleet-research-report)"
>     ],
>     "ask": [
>       "Bash(cafleet * member exec *)"
>     ]
>   }
> }
> ```
>

### 2.2. Codex

> [!TIP]
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
>
> [sandbox_workspace_write]
> writable_roots = ["/home/<you>/.local/share/cafleet"]
> ```
> 
> Recommended Codex rules for `cafleet` commands — drop into your codex rules file (e.g. `~/.codex/rules/cafleet.rules`):
> 
> ```
> prefix_rule(
>     pattern = ["cafleet"],
>     decision = "allow",
>     justification = "All cafleet subcommands are allowed by default",
> )
> 
> prefix_rule(
>     pattern = ["cafleet", "member", "exec"],
>     decision = "prompt",
>     justification = "cafleet member exec runs arbitrary commands on a member; require approval",
> )
> ```

## 3. Examples

### 3.1. Simple example to use CAFleet

Provide the following prompt to Claude Code or Codex to see how it works.

```
I want to see how cafleet works.
Please create a fresh team with two teammates using cafleet and let them ping-pong each other.
After the demonstration, please shutdown the team.
```

### 3.2. Real world usage; Design-doc-driven development

CAFleet provides the builtin skills for Spec Driven Development (SDD). **We're using CAFleet to develop CAFleet!**

Invoke the `cafleet-design-doc-create` skill with a one-line request, e.g.:

```
I want to create a simple TUI calculator.
```

See your coding-agent's skill documentation for the literal invocation syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

You can see the existing design docs on [`design-docs/`](design-docs/), which are actually created by the skills.

## 4. CLI cheatsheet

| Command group | One-line purpose |
|---|---|
| `cafleet db init` | Apply schema migrations (idempotent; rerun after upgrades) |
| `cafleet session create [--coding-agent {claude,codex,opencode}]` | Create a session; declare the root Director's coding-agent backend (default `claude`) |
| `cafleet session *` | List / show / delete sessions |
| `cafleet agent *` | Register / deregister / list / show agents |
| `cafleet message *` | Send / broadcast / poll / ack / cancel / show messages |
| `cafleet member create [--coding-agent {claude,codex,opencode}]` | Spawn a member pane running `claude` (default), `codex`, or `opencode` |
| `cafleet member *` | Delete / list / capture / send-input / exec / ping member panes (Director only) |
| `cafleet server` | Start the admin WebUI on `127.0.0.1:8000` |
| `cafleet doctor` | Print the calling pane's tmux identifiers |
| `cafleet base-dir {resolve,record}` | Resolve / persist the `${BASE}` output-root used by every CAFleet scratch / audit / figure write (filesystem-only; no `--session-id`). `cafleet base-dir resolve [TASK_NAME]` accepts an optional positional `TASK_NAME` (relative path the consuming skill picks, e.g. `researches/<slug>` or `design-docs/<NNNNNNN>-<slug>`; or an absolute path strictly under the repo root); when supplied, returns the auto-created task folder as `${BASE}` and writes a per-task anchor inside it. Consumer-strips contract: the resolver does not strip child filenames; each consuming skill canonicalizes its argument before calling. |

> CLI reference (per-command sections for `session`, `member`, `doctor`, `server`; `agent` / `message` / `db init` covered via the option-source table and `cafleet <cmd> --help`): [docs/spec/cli-options.md](docs/spec/cli-options.md). Message envelope shape (compact rendered + `--full` typed-column) and message body truncation rules (`CAFLEET_MAX_TEXT_LEN`, `--full`, `--quiet`): [docs/spec/cli-options.md](docs/spec/cli-options.md) § Message Body Truncation and [docs/spec/message-envelope.md](docs/spec/message-envelope.md).

## 5. Architecture

CAFleet ships a unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite database. Sessions partition agents into isolated namespaces; the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required for agent operations. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design.

## 6. Contributing

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, live in [CONTRIBUTING.md](CONTRIBUTING.md).
