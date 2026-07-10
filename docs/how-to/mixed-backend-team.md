---
icon: lucide/users
---

# Run a mixed-backend team

A single Director can spawn `claude`, `codex`, and `opencode` members in the
same fleet — there are no broker-level differences between the backends
([Coding agents](../concepts/coding-agents.md)). This guide creates a fleet
with one member per backend and messages each of them.

## Prerequisites

- The backend binaries you want to mix (`claude`, `codex`, `opencode`) are on
  `PATH` — `member create` exits 1 with `Error: binary <name> not found on
  PATH` otherwise.
- You have followed [Install](../get-started/install.md) and
  [Configure](../get-started/configure.md), and you are inside a tmux or herdr
  session (the multiplexer backend is auto-detected — see
  [Multiplexer backends](../concepts/multiplexer-backends.md)).

## Prompt

```text
Create a CAFleet team for this repo with three members — one on claude,
one on codex, and one on opencode. Name them alice, bob, and carol.
Once they are up, send each member a message asking it to report its
backend, and confirm all three reply. Then tear the team down.
```

Your agent loads the `cafleet` skill and reads its Director-only
`reference/supervision.md` before spawning members.

The supervision tick is supplied by the monitoring member's `cafleet monitor`
loop and works the same on **any** backend (`claude`, `codex`, or `opencode`);
the monitoring member is the first `cafleet member create` (`--role monitor`),
and the Director never runs the monitor itself
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
`2`, members `4`/`5`/`6`; your ids will differ.

??? example "Expand the walkthrough"

    Create the fleet — the operator declares the binary running in *your* pane
    via `--coding-agent` because cafleet cannot auto-detect it
    ([Coding agents](../concepts/coding-agents.md)):

    ```bash
    cafleet fleet create --name "demo" --coding-agent claude
    ```

    ```
    1 director=2 admin=3
    ```

    Spawn one member per backend:

    ```bash
    cafleet member create --fleet-id 1 --agent-id 2 \
      --name "alice" --description "claude member" \
      --coding-agent claude --text "You are alice. Wait for instructions."
    ```

    ```
    4 alice backend=claude pane=%7
    ```

    ```bash
    cafleet member create --fleet-id 1 --agent-id 2 \
      --name "bob" --description "codex member" \
      --coding-agent codex --text "You are bob. Wait for instructions."
    ```

    ```
    5 bob backend=codex pane=%8
    ```

    ```bash
    cafleet member create --fleet-id 1 --agent-id 2 \
      --name "carol" --description "opencode member" \
      --coding-agent opencode --text "You are carol. Wait for instructions."
    ```

    ```
    6 carol backend=opencode pane=%9
    ```

    List the panes — only the `claude` pane titles itself with the member name
    ([Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals)),
    so use the `pane_id` column to locate `bob` and `carol`:

    ```bash
    cafleet member list --fleet-id 1 --all
    ```

    ```
    5 agents:
      agent_id  name           status  kind           backend   session  window_id  pane_id  created_at
      --------  -------------  ------  -------------  --------  -------  ---------  -------  --------------------
      2         Director       active  director       claude    main     @3         %0       2026-04-15T10:00:00+00:00
      3         Administrator  active  administrator  -         -        -          -        -
      4         alice          active  member         claude    main     @3         %7       2026-04-15T10:01:00+00:00
      5         bob            active  member         codex     main     @3         %8       2026-04-15T10:02:00+00:00
      6         carol          active  member         opencode  main     @3         %9       2026-04-15T10:03:00+00:00
    ```

    Message each member — repeat with `--to 5` and `--to 6`; the envelope and
    the 2-line inline preview are identical for every backend
    ([Push notifications](../spec/multiplexer-backends.md#push-notifications)):

    ```bash
    cafleet message send --fleet-id 1 --agent-id 2 --to 4 --text "alice: report status"
    ```

    ```
    Message sent.
    [10 | from:2 | 2026-06-11T09:05:00.123456+00:00]
    alice: report status
    ```

    Tear down — repeat `member delete` for members `5` and `6`, then delete the
    fleet:

    ```bash
    cafleet member delete --fleet-id 1 --member-id 4
    ```

    ```
    Member deleted.
      agent_id:  4
      pane_id:   %7 (closed)
    ```

    ```bash
    cafleet fleet delete --fleet-id 1
    ```

    ```
    Deleted fleet 1. Deregistered 2 agents.
    ```

Every `member create` / `member delete` flag and exit code is documented in
[CLI options](../spec/cli-options.md#member-create).
