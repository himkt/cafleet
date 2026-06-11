---
icon: lucide/users
---

# Run a mixed-backend team

A single Director can spawn `claude`, `codex`, and `opencode` members in the
same fleet — there are no broker-level differences between the backends
([Coding agents](../concepts/coding-agents.md)). This guide creates a fleet
with one member per backend and messages each of them.

The walkthrough pastes literal ids: fleet `1`, root Director `2`, members
`4`/`5`/`6`. Your ids will differ — substitute the integers your own
commands print.

## Prerequisites

- The backend binaries you want to mix (`claude`, `codex`, `opencode`) are on
  `PATH` — `member create` exits 1 with `Error: binary <name> not found on
  PATH` otherwise.
- You have followed [Install](../get-started/install.md) and
  [Configure](../get-started/configure.md), and you are inside a tmux
  session.

## Create the fleet

```bash
cafleet fleet create --label "demo" --coding-agent claude
```

```
1 director=2 admin=3
```

`--coding-agent` here declares which binary is running in *your* pane — the
operator declares it because cafleet cannot auto-detect it
([Coding agents](../concepts/coding-agents.md)).

## Spawn one member per backend

```bash
cafleet --fleet-id 1 member create --agent-id 2 \
  --name "alice" --description "claude member" \
  --coding-agent claude -- "You are alice. Wait for instructions."
```

```
4 alice backend=claude pane=%7
```

```bash
cafleet --fleet-id 1 member create --agent-id 2 \
  --name "bob" --description "codex member" \
  --coding-agent codex -- "You are bob. Wait for instructions."
```

```
5 bob backend=codex pane=%8
```

```bash
cafleet --fleet-id 1 member create --agent-id 2 \
  --name "carol" --description "opencode member" \
  --coding-agent opencode -- "You are carol. Wait for instructions."
```

```
6 carol backend=opencode pane=%9
```

## Find the panes

```bash
cafleet --fleet-id 1 member list
```

```
3 members:
  agent_id        name      status  backend   session  window_id  pane_id  created_at
  --------------  --------  ------  --------  -------  ---------  -------  --------------------
  4               alice     active  claude    main     @1         %7       2026-06-11T09:01:00.000000+00:00
  5               bob       active  codex     main     @1         %8       2026-06-11T09:02:00.000000+00:00
  6               carol     active  opencode  main     @1         %9       2026-06-11T09:03:00.000000+00:00
```

Only the `claude` pane shows the member name in its pane title — see
[Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals)
— so use the `pane_id` column to locate `bob` and `carol`.

## Message round-trip across backends

```bash
cafleet --fleet-id 1 message send --agent-id 2 --to 4 --text "alice: report status"
```

```
Message sent.
[10 | from:2 | 2026-06-11T09:05:00.123456+00:00]
alice: report status
```

Repeat with `--to 5` and `--to 6` — the envelope is identical for every
backend, and each member receives the same 2-line inline preview in its pane
([tmux push](../concepts/tmux-push.md)).

## Tear down

```bash
cafleet --fleet-id 1 member delete --member-id 4
```

```
Member deleted.
  agent_id:  4
  pane_id:   %7 (closed)
```

Repeat for members `5` and `6`, then delete the fleet:

```bash
cafleet fleet delete 1
```

```
Deleted fleet 1. Deregistered 2 agents.
```

Every `member create` / `member delete` flag and exit code is documented in
[CLI options](../spec/cli-options.md#member-commands).
