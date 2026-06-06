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

Invoke the `cafleet-design-doc-create` skill with a one-line request, e.g.:

```text
I want to create a simple TUI calculator.
```

See your coding-agent's skill documentation for the literal invocation syntax
(Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery).

## Raw CLI walkthrough

If you would rather drive CAFleet from the shell directly, the commands below
mirror what the skill does internally. Run them inside a tmux session — the
`fleet create` and `member create` commands require one.

```bash
# 1. Create a fleet. Records this pane as the root Director's pane.
cafleet fleet create --label "demo" --full

# The output prints the fleet id and the root Director's agent id on the
# first two lines. Export them as shell vars for the snippets below.
export FLEET_ID="<paste from the first output line>"
export DIRECTOR_ID="<paste from the second output line>"

# 2. Spawn a member pane. The member's prompt is just a one-line greeting.
cafleet --fleet-id "$FLEET_ID" member create \
  --agent-id "$DIRECTOR_ID" \
  --name "demo-member" \
  --description "Demo member" \
  -- "You are demo-member. Reply hello when polled."

# 3. The Director sends a message to the new member.
cafleet --fleet-id "$FLEET_ID" agent list --agent-id "$DIRECTOR_ID"
# Pick the demo-member agent_id from the output and replace <member-id> below.
cafleet --fleet-id "$FLEET_ID" message send --agent-id "$DIRECTOR_ID" \
  --to "<member-id>" --text "hi"
```

The member receives the message as a 2-line inline preview pushed into its
tmux pane and the message lands in the broker queue. From here, the typical
flow is `cafleet message poll` from the recipient and `cafleet message ack`
once it has consumed the message.

When you are done, tear the fleet down:

```bash
cafleet --fleet-id "$FLEET_ID" member delete --agent-id "$DIRECTOR_ID" \
  --member-id "<member-id>"
cafleet fleet delete "$FLEET_ID"
```

See the [CLI options](../spec/cli-options.md) reference for every subcommand
and flag.
