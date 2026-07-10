---
icon: lucide/activity
---

# Monitor and recover members

Members run unattended in tmux panes, so the Director needs two primitives:
a cheap roster watch and an escalation ladder for a member that stopped
reacting. This guide checks on a running team and recovers a quiet member.

## Ensure the monitor is running

The recovery ladder below is driven by a periodic supervision tick: the tick
comes from the monitoring member's `cafleet monitor` loop — a per-fleet loop the
fleet's dedicated **monitoring member** runs as a background task in its own pane
— see [Monitoring](../concepts/monitoring.md). The monitoring member is spawned
**first**; it starts the loop and reports `ready: monitor live` (its canonical
spawn prompt lives in the `/cafleet` skill's `roles/monitor.md`):

```bash
cafleet member create --fleet-id 1 --agent-id 2 \
  --name monitor --description "Monitoring member: owns the heartbeat" \
  --role monitor --model sonnet \
  --text-file /abs/path/to/monitor-prompt.md   # spawned first; runs monitor start in its own pane
cafleet monitor status --fleet-id 1              # confirm it is running + see the schedule
```

The monitor supplies only the heartbeat; the inspect-and-recover steps below are
the Director's job.

## Prompt

```text
Check on my CAFleet team in fleet 1. List member activity, find any
member that has gone quiet, inspect its pane, and recover it with the
mildest intervention that works — only deregister it as a last resort.
```

Your agent loads the `cafleet` skill and reads its Director-only
`reference/supervision.md` (recovery ladder and idle semantics, plus the monitoring mechanism).

## What to expect

The agent renders the roster with per-agent idle times, captures the pane of
any member that has gone quiet, and climbs the recovery ladder from mildest
to harshest: re-poke the inbox, dispatch a shell
command ([Bash routing](../concepts/bash-routing.md)), and only as a last
resort deregister the member. You see each intervention land as keystrokes in
the member's pane ([tmux push](../concepts/tmux-push.md)).

## Appendix: the CLI underneath

The commands the agent runs, all from the Director's pane, with literal
ids — fleet `1`, members `4`/`5`/`6`; your ids will differ.

??? example "Expand the walkthrough"

    Watch the team — `last_sent` is the agent's most recent outgoing message,
    `last_recv` its most recent delivery, `last_ack` the most recent delivery
    it acknowledged, and `idle` the wall-time since the latest of `last_sent` /
    `last_recv`; a member that receives work but never sends or acks is stalled
    — `alice` (`4`) below has been quiet for 14 minutes (aggregation rules in
    [CLI options](../spec/cli-options.md#member-list-activity-output)):

    ```bash
    cafleet member list --fleet-id 1 --activity
    ```

    ```
    3 members:
      agent_id        name      status  last_sent  last_recv  last_ack   idle
      --------------  --------  ------  ---------  ---------  ---------  -----
      4               alice     active  -          12:20:00   12:20:00   14m
      5               bob       active  12:30:11   12:33:02   12:33:02   2m
      6               carol     active  12:34:56   12:34:50   12:34:50   6s
    ```

    Inspect the quiet member — prints the last 20 lines of the pane buffer with
    ANSI escapes stripped (`--lines N` for a longer tail); a stalled member
    typically shows a pending prompt:

    ```bash
    cafleet member capture --fleet-id 1 --member-id 4
    ```

    ```
     Do you want to proceed?
     ❯ 1. Yes
       2. No
    ```

    Ladder rung 1, `member ping` — injects an `Esc`-safeguarded
    `cafleet message poll` keystroke (the leading `Esc` dismisses any pending
    permission prompt) so the member drains anything it missed; panes need
    re-poking at all because inline previews are best-effort keystrokes
    ([tmux push](../concepts/tmux-push.md)):

    ```bash
    cafleet member ping --fleet-id 1 --member-id 4
    ```

    ```
    Pinged member alice (%7) — poll keystroke dispatched.
    ```

    Ladder rung 2, `member exec` — keystrokes `! git status` into the pane so
    the coding agent runs it natively, the dispatch half of the
    bash-via-Director protocol ([Bash routing](../concepts/bash-routing.md)):

    ```bash
    cafleet member exec --fleet-id 1 --member-id 4 "git status"
    ```

    ```
    Sent bash command 'git status' to member alice (%7).
    ```

    Ladder rung 3, `member delete` (last resort) — sends the backend exit
    keystroke and waits up to 15 s for the pane to close:

    ```bash
    cafleet member delete --fleet-id 1 --member-id 4
    ```

    ```
    Member deleted.
      agent_id:  4
      pane_id:   %7 (closed)
    ```

    A pane that refuses to close makes the command exit 2 with the pane tail
    and a built-in recovery hint; `cafleet member delete --member-id 4 --force`
    skips the wait, kills the pane, and exits 0 even if the pane was already
    gone:

    ```
    Error: pane %7 did not close within 15.0s after /exit.
    --- pane %7 tail (last 80 lines) ---
    <captured terminal buffer>
    ---
    Recovery: inspect with `cafleet member capture`, then re-run `cafleet member delete`. Or re-run with `--force` to skip the wait and kill the pane.
    ```

Every flag, validation rule, and exit code for the `member`
subcommands is documented in [CLI options](../spec/cli-options.md#cafleet-member).
