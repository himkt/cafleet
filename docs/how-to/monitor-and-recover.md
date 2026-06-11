---
icon: lucide/activity
---

# Monitor and recover members

Members run unattended in tmux panes, so the Director needs two primitives: a
cheap roster watch and an escalation ladder for a member that stopped
reacting. All commands run from the Director's pane. The walkthrough pastes
literal ids: fleet `1`, members `4`/`5`/`6` — your ids will differ.

## Watch the team

```bash
cafleet --fleet-id 1 member list --activity
```

```
3 members:
  agent_id        name      status  last_sent  last_recv  last_ack   idle
  --------------  --------  ------  ---------  ---------  ---------  -----
  4               alice     active  -          12:20:00   12:20:00   14m
  5               bob       active  12:30:11   12:33:02   12:33:02   2m
  6               carol     active  12:34:56   12:34:50   12:34:50   6s
```

`last_sent` is the member's most recent outgoing message, `last_recv` its
most recent delivery, `last_ack` the most recent delivery it acknowledged,
and `idle` the wall-time since the latest of `last_sent` / `last_recv`.
A member that receives work but never sends or acks is stalled — `alice`
(`4`) above has been quiet for 14 minutes. The exact aggregation rules live
in [CLI options](../spec/cli-options.md#member-list-activity-output).

## Inspect a quiet member

```bash
cafleet --fleet-id 1 member capture --member-id 4
```

Prints the last 30 lines of the member's pane buffer with ANSI escapes
stripped; pass `--lines N` for a longer tail. A stalled member typically
shows a pending prompt, for example:

```
 Do you want to proceed?
 ❯ 1. Yes
   2. No
```

## The recovery ladder

Work from the mildest intervention up. The reason panes need re-poking at
all — inline previews are best-effort keystrokes — is explained in
[tmux push](../concepts/tmux-push.md).

### 1. `member ping` — re-poke a missed preview

```bash
cafleet --fleet-id 1 member ping --member-id 4
```

```
Pinged member alice (%7) — poll keystroke dispatched.
```

Injects a `cafleet message poll` keystroke into the pane so the member
drains anything it missed.

### 2. `member send-input` — answer a pending prompt

```bash
cafleet --fleet-id 1 member send-input --member-id 4 --choice 1
```

```
Sent choice 1 to member alice (%7).
```

`--choice 1..3` answers an AskUserQuestion option; `--freetext "<text>"`
fills the "Type something" field.

### 3. `member exec` — dispatch a shell command

```bash
cafleet --fleet-id 1 member exec --member-id 4 "git status"
```

```
Sent bash command 'git status' to member alice (%7).
```

Keystrokes `! git status` into the pane so the coding agent runs it natively
— the dispatch half of the bash-via-Director protocol
([Bash routing](../concepts/bash-routing.md)).

### 4. `member delete` — last resort

```bash
cafleet --fleet-id 1 member delete --member-id 4
```

```
Member deleted.
  agent_id:  4
  pane_id:   %7 (closed)
```

The default path sends `/exit` and waits up to 15 s for the pane to close.
A pane that refuses to close makes the command exit 2 with the pane tail and
a built-in recovery hint:

```
Error: pane %7 did not close within 15.0s after /exit.
--- pane %7 tail (last 80 lines) ---
<captured terminal buffer>
---
Recovery: inspect with `cafleet member capture`, answer any prompt with `cafleet member send-input`, then re-run `cafleet member delete`. Or re-run with `--force` to skip the wait and kill the pane.
```

`cafleet member delete --member-id 4 --force` skips the wait, kills the
pane, and exits 0 even if the pane was already gone.

Every flag, validation rule, and exit code for the member subcommands is
documented in [CLI options](../spec/cli-options.md#member-commands).
