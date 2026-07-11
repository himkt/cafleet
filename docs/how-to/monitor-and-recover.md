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
cafleet member create --fleet-id 1 \
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

The agent renders the roster with per-member idle times, captures the pane of
any member that has gone quiet, and climbs the recovery ladder from mildest
to harshest: re-poke the inbox, dispatch a shell
command ([`member exec`](../spec/cli-options.md#member-exec)), and only as a
last resort deregister the member. You see each intervention land as
keystrokes in the member's pane
([Push notifications](../spec/multiplexer-backends.md#push-notifications)).

## Appendix: the CLI underneath

The commands the agent runs, all from the Director's pane, with literal
ids — fleet `1`, monitoring member `3`, members `4`/`5`/`6`; your ids will
differ.

??? example "Expand the walkthrough"

    Watch the team — `idle` is the wall-time since the member's most recent
    task activity (the latest of its most recent outgoing message and its most
    recent delivery); `--json` adds the underlying `last_sent` / `last_recv` /
    `last_ack` timestamps. A member whose `idle` keeps growing while the rest
    of the team moves is stalled — `alice` (`4`) below has been quiet for 14
    minutes (aggregation rules in
    [CLI options](../spec/cli-options.md#member-list)):

    ```bash
    cafleet member list --fleet-id 1
    ```

    ```
    5 members:
      member_id  name           kind      backend   pane_id  idle
      ---------  -------------  --------  --------  -------  ----
      2          Director       director  claude    %0       6s
      3          monitor        monitor   claude    %6       -
      4          alice          member    claude    %7       14m
      5          bob            member    claude    %8       2m
      6          carol          member    claude    %9       6s
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
    ([Push notifications](../spec/multiplexer-backends.md#push-notifications)):

    ```bash
    cafleet member ping --fleet-id 1 --member-id 4
    ```

    ```
    Pinged member alice (%7) — poll keystroke dispatched.
    ```

    Ladder rung 2, `member exec` — keystrokes `! git status` into the pane so
    the coding agent runs it natively, the dispatch half of the
    bash-via-Director protocol ([`member exec`](../spec/cli-options.md#member-exec)):

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
      member_id:  4
      pane_id:    %7 (closed)
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
