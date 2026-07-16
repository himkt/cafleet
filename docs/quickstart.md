---
icon: lucide/zap
---

# Quickstart

This page is a one-screen walkthrough that creates a CAFleet fleet, spawns
two member panes, and sends a message between them. It starts from a clean
machine: install the CLI, configure your coding agent, then run the
walkthrough.

## Install

Prerequisites:

- Python 3.12+
- A terminal multiplexer — tmux or herdr (see
  [Multiplexer backends](spec/multiplexer-backends.md))
- At least one of: `claude` (Claude Code), `codex` (OpenAI Codex CLI), or
  `opencode`

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet setup               # migrate the database schema + install the skills
```

The default database lives at `~/.local/share/cafleet/cafleet_v5.db`;
override with the `CAFLEET_DATABASE_URL` environment variable — use an
absolute path, since SQLAlchemy does not expand `~` in SQLite URLs.

## Configure

CAFleet is designed to run inside a coding agent without per-command
permission prompts — each backend has a different config file and permission
system, and the snippets below are the recommended starting points.

### Claude Code

!!! tip "Where this lives"

    Typically the config entries below go in `~/.claude/settings.json`.

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet:cafleet)",
      "Skill(cafleet:cafleet-design-doc)",
      "Skill(cafleet:cafleet-research)"
    ],
    "ask": [
      "Bash(cafleet * member exec *)"
    ]
  }
}
```

The `Bash(cafleet *)` pattern is the single allow-everything entry that the
literal `--fleet-id <int>` / `--member-id <int>` flag convention enables —
one pattern covers every subcommand for every fleet. `cafleet member exec *`
is moved to the `ask` list because it dispatches arbitrary shell commands on
behalf of a member; the operator should confirm each invocation.

### Codex

!!! tip "Where this lives"

    Typically the config entries below go in `~/.codex/config.toml`.

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

`network_access = true` is required because cafleet's multiplexer backends
(tmux and herdr) communicate over a local socket, which the Codex sandbox
classifies as network access — without it cafleet commands fail with
`Operation not permitted`. `writable_roots` grants write access to cafleet's
default SQLite DB directory. Use the absolute path matching
`CAFLEET_DATABASE_URL` or the default XDG location.

!!! tip "Where this lives"

    The Codex rules for `cafleet` commands live at `~/.codex/rules/cafleet.rules`.

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

The more specific prefix rule takes precedence: `["cafleet", "member", "exec"]`
wins over the broad `["cafleet"]` allow, so `cafleet member exec` keeps
prompting while every other subcommand is allowed — for every fleet, since
`--fleet-id` is a trailing flag past the matched prefix.

### Opencode

!!! tip "Where this lives"

    Opencode's `cafleet` agent definition lives at `~/.opencode/agents/cafleet.md`.

No manual configuration is required. The first `cafleet member create
--coding-agent opencode` writes the agent preset to
`~/.opencode/agents/cafleet.md` if it does not already exist. See
[Opencode members](reference/coding-agents/opencode.md) for the preset's
ruleset, the refresh recipe after upgrades, and the MCP-server rule.

### Trust the working directory

Coding agents ask for a trust confirmation the first time they start in a
directory they have not seen before. Trust the workspace in advance: launch
your coding agent once in the working directory the member panes will run in
and accept its first-run prompt, or add a trust entry to the agent's
configuration file (see your agent's reference page). Trust is granted per
directory, so each git worktree needs its own approval.

This prevents a spawn-time stall: in an untrusted directory, the agent's
first-run trust prompt stalls a freshly spawned member — the member ignores
every incoming message until the prompt is cleared.

## Simple example — invoke from a coding agent

For the simplest path, ask Claude Code or Codex to demonstrate CAFleet for
you. Inside an agent, send the following prompt:

```text
I want to see how cafleet works.
Please create a new team with two members using cafleet and let them ping-pong each other.
After the demonstration, please shutdown the team.
```

The coding agent will invoke the `cafleet` skill, which guides it through the
fleet-create / member-create / message-send loop and finally tears the team
down.

## Raw CLI walkthrough

If you would rather drive CAFleet from the shell directly, the commands below
mirror what the skill does internally. Run them inside a tmux or herdr session —
the `fleet create` and `member create` commands require one.

??? example "Expand the walkthrough"

    The walkthrough pastes literal integer ids: fleet `1`, root Director `2`,
    members `3` and `4`, message `10`. Your ids will differ — substitute the
    integers your own commands print.

    Create a fleet. This records your current pane as the root Director's
    pane; `--coding-agent` is required — pass the backend the Director is
    actually running on (here, Claude Code):

    ```bash
    cafleet fleet create --name "demo" --coding-agent claude
    ```

    ```
    1 director=2
    ```

    The line carries the fleet id (`1`) and the root Director's member id
    (`2`). If it scrolls away, run
    `cafleet fleet list` — it re-prints the fleet id and the Director id (the
    `DIRECTOR` column).

    Spawn two member panes. Each member's prompt is just a one-line greeting.
    Optional: add `--model <m>` (e.g. `--model sonnet`) to pin a member's LLM;
    omitted, the backend binary uses its own default model:

    ```bash
    cafleet member create --fleet-id 1 \
      --name "demo-member" \
      --description "Demo member" \
      --text "You are demo-member. Reply hello when polled."
    ```

    ```
    3 demo-member backend=claude pane=%7
    ```

    ```bash
    cafleet member create --fleet-id 1 \
      --name "reviewer" \
      --description "Reviewer member" \
      --text "You are reviewer. Reply hello when polled."
    ```

    ```
    4 reviewer backend=claude pane=%8
    ```

    List the fleet's roster — the new members' ids (`3` and `4`) appear
    alongside the Director:

    ```bash
    cafleet member list --fleet-id 1
    ```

    ```
    3 members:
      member_id  name           kind      backend   pane_id  idle
      ---------  -------------  --------  --------  -------  ----
      2          Director       director  claude    %0       -
      3          demo-member    member    claude    %7       -
      4          reviewer       member    claude    %8       -
    ```

    Send a message between the members — `demo-member` (`3`) messages
    `reviewer` (`4`):

    ```bash
    cafleet message send --fleet-id 1 --from-member-id 3 --to-member-id 4 --text "hi"
    ```

    ```
    Message sent.
    [10 | from:3 | 2026-06-11T09:00:00.123456+00:00]
    hi
    ```

    `reviewer` receives the message as a 2-line inline preview pushed into its
    tmux pane and the message lands in the broker queue. From here, the typical
    flow is `cafleet message poll` from the recipient and `cafleet message ack`
    once it has consumed the message.

    When you are done, tear the fleet down:

    ```bash
    cafleet member delete --fleet-id 1 --member-id 3
    cafleet member delete --fleet-id 1 --member-id 4
    cafleet fleet delete --fleet-id 1
    ```

    ```
    Deleted fleet 1. Deregistered 1 members.
    ```

Where to go next:

- [CLI options](spec/cli-options.md) — every subcommand and flag.
- [How-to guides](how-to/mixed-backend-team.md) — prompt-first task guides.
