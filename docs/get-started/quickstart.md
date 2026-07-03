---
icon: lucide/zap
---

# Quickstart

This page is a one-screen walkthrough that creates a CAFleet fleet, spawns
two member panes, and sends a message between them. It assumes you have
already followed [Install](install.md) and [Configure](configure.md).

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

## Design-doc-driven development

CAFleet ships built-in skills for spec-driven development (SDD). The CAFleet
project itself uses them to evolve — every change lands as a design document
first, executed by a CAFleet-orchestrated Director / Drafter / Reviewer team.

Invoke the `cafleet-design-doc` skill (create workflow) with a one-line request, e.g.:

```text
I want to create a simple TUI calculator.
```

See your coding-agent's skill documentation for the literal invocation syntax
(Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

## Raw CLI walkthrough

If you would rather drive CAFleet from the shell directly, the commands below
mirror what the skill does internally. Run them inside a tmux session — the
`fleet create` and `member create` commands require one.

The walkthrough pastes literal integer ids: fleet `1`, root Director `2`,
members `4` and `5`, task `10`. Your ids will differ — substitute the
integers your own commands print.

Create a fleet. This records your current pane as the root Director's pane:

```bash
cafleet fleet create --label "demo"
```

```
1 director=2 admin=3
```

The line carries the fleet id (`1`), the root Director's agent id (`2`), and
the built-in Administrator's agent id (`3`). If it scrolls away, run
`cafleet fleet list` — it re-prints the fleet id and the Director id (the
`DIRECTOR` column).

Spawn two member panes. Each member's prompt is just a one-line greeting.
Optional: add `--model <m>` (e.g. `--model sonnet`) to pin a member's LLM;
omitted, the backend binary uses its own default model:

```bash
cafleet member create --fleet-id 1 \
  --agent-id 2 \
  --name "demo-member" \
  --description "Demo member" \
  --text "You are demo-member. Reply hello when polled."
```

```
4 demo-member backend=claude pane=%7
```

```bash
cafleet member create --fleet-id 1 \
  --agent-id 2 \
  --name "reviewer" \
  --description "Reviewer member" \
  --text "You are reviewer. Reply hello when polled."
```

```
5 reviewer backend=claude pane=%8
```

List the fleet's agents — the new members' ids (`4` and `5`) appear
alongside the Director and the Administrator:

```bash
cafleet member list --fleet-id 1 --all
```

```
4 agents:
  agent_id  name           status  kind           backend  session  window_id  pane_id  created_at
  --------  -------------  ------  -------------  -------  -------  ---------  -------  --------------------
  2         Director       active  director       claude   main     @3         %0       2026-04-15T10:00:00+00:00
  3         Administrator  active  administrator  -        -        -          -        -
  4         demo-member    active  member         claude   main     @3         %7       2026-04-15T10:01:00+00:00
  5         reviewer       active  member         claude   main     @3         %8       2026-04-15T10:02:00+00:00
```

Send a message between the members — `demo-member` (`4`) messages
`reviewer` (`5`):

```bash
cafleet message send --fleet-id 1 --agent-id 4 --to 5 --text "hi"
```

```
Message sent.
[10 | from:4 | 2026-06-11T09:00:00.123456+00:00]
hi
```

`reviewer` receives the message as a 2-line inline preview pushed into its
tmux pane and the message lands in the broker queue. From here, the typical
flow is `cafleet message poll` from the recipient and `cafleet message ack`
once it has consumed the message.

When you are done, tear the fleet down:

```bash
cafleet member delete --fleet-id 1 --member-id 4
cafleet member delete --fleet-id 1 --member-id 5
cafleet fleet delete --fleet-id 1
```

```
Deleted fleet 1. Deregistered 2 agents.
```

Where to go next:

- [CLI options](../spec/cli-options.md) — every subcommand and flag.
- [How-to guides](../how-to/index.md) — prompt-first task guides.
