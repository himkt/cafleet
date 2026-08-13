# Run a mixed-backend team

A single Director can spawn `claude`, `codex`, and `opencode` members in the
same fleet — there are no broker-level differences between the backends
([Coding agents](../concepts/coding-agents.md)). This guide creates a fleet
with one member per backend and messages each of them.

## Prerequisites

- The backend binaries you want to mix (`claude`, `codex`, `opencode`) are on
  `PATH` — `member create` exits 1 with `Error: binary <name> not found on
  PATH` otherwise.
- You have followed [Quickstart § Install](../quickstart.md#install) and
  [Quickstart § Configure](../quickstart.md#configure), and you are inside a tmux or herdr
  session (the multiplexer backend is auto-detected — see
  [Multiplexer backends](../spec/multiplexer-backends.md)).

## Prompt

```text
Create a CAFleet team for this repo with three members — one on claude,
one on codex, and one on opencode. Name them alice, bob, and carol.
Once they are up, send each member a message asking it to report its
backend, and confirm all three reply. Then tear the team down.
```

Your agent loads the `cafleet` skill and follows its Director-only
supervision protocol before spawning members.

Supervision is supplied by the **monitor member** the Director spawns first —
`cafleet member create --role monitor`, immediately after
`cafleet fleet create` and before any ordinary member. The monitor member
launches the `cafleet monitor` loop in its own pane and reports `monitor
live` to the Director before the ordinary members spawn; it works the same on
**any** backend (`claude`, `codex`, or `opencode`)
([Monitoring](../concepts/monitoring.md)).

## What to expect

The agent creates a fleet, then opens three multiplexer panes — one per backend —
each running its member's coding agent. Each message lands as a 2-line
inline preview keystroked into the recipient's pane
([Push notifications](../spec/multiplexer-backends.md#push-notifications)),
so you watch every member wake up
and reply. Only the `claude` pane shows the member name in its pane title
([Coding agents](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals)).
When all three replies are confirmed, the agent closes the panes and
deletes the fleet.

## Appendix: the CLI underneath

The commands the agent runs, with literal ids — fleet `1`, root Director
`2`, monitor member `3`, members `4`/`5`/`6`; your ids will differ.

:::details Expand the walkthrough

Create the fleet — the operator declares the binary running in *your* pane
via `--coding-agent` because cafleet cannot auto-detect it
([Coding agents](../concepts/coding-agents.md)):

```bash
cafleet fleet create --name "demo" --coding-agent claude
```

```
1 director=2
```

Spawn the monitor member first — `--coding-agent` is omitted so it inherits
the Director's backend, and the model is the backend's monitor default
([Monitoring](../concepts/monitoring.md)). It launches the wake loop in its
own pane and reports `monitor live`; an ordinary `member create` before it
exists fails with the monitor-first guard
([CLI options](../spec/cli-options.md#member-create)):

```bash
cafleet member create --fleet-id 1 --role monitor \
  --name "monitor" --description "monitor member" \
  --model haiku "You are the monitor member. Follow your monitor role protocol."
```

```
3 monitor backend=claude pane=%7
```

Then spawn one ordinary member per backend:

```bash
cafleet member create --fleet-id 1 \
  --name "alice" --description "claude member" \
  --coding-agent claude "You are alice. Wait for instructions."
```

```
4 alice backend=claude pane=%8
```

```bash
cafleet member create --fleet-id 1 \
  --name "bob" --description "codex member" \
  --coding-agent codex "You are bob. Wait for instructions."
```

```
5 bob backend=codex pane=%9
```

```bash
cafleet member create --fleet-id 1 \
  --name "carol" --description "opencode member" \
  --coding-agent opencode "You are carol. Wait for instructions."
```

```
6 carol backend=opencode pane=%10
```

List the panes — only the `claude` panes title themselves with the member name
([Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals)),
so use the `pane_id` column to locate `bob` and `carol`:

```bash
cafleet member list 1
```

```
5 members:
  member_id  name           kind      backend   pane_id  idle
  ---------  -------------  --------  --------  -------  ----
  2          Director       director  claude    %0       -
  3          monitor        monitor   claude    %7       -
  4          alice          member    claude    %8       -
  5          bob            member    codex     %9       -
  6          carol          member    opencode  %10      -
```

Message each member — repeat with `--to-member-id 5` and `--to-member-id 6`;
the envelope and the 2-line inline preview are identical for every backend
([Push notifications](../spec/multiplexer-backends.md#push-notifications)):

```bash
cafleet message send --from-member-id 2 --to-member-id 4 "alice: report status"
```

```
Message sent.
[10 | from:2 | 2026-06-11T09:05:00.123456+00:00]
alice: report status
```

Tear down — the monitor member goes first, so the pane kill ends the wake
loop before any other member disappears; repeat `member delete` for members
`4`, `5`, and `6`, then delete the fleet:

```bash
cafleet member delete 3
```

```
Member deleted.
  member_id:  3
  pane_id:    %7 (killed)
```

```bash
cafleet fleet delete 1
```

```
Deleted fleet 1. Deregistered 1 members.
```

:::

Every `member create` / `member delete` flag and exit code is documented in
[CLI options](../spec/cli-options.md#member-create).
