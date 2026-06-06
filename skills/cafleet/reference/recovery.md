# Recovery flows

Director reference for crash / disconnect / idle / wedged-pane recovery. Member-side recovery is rare — members either run cleanly until the Director runs `member delete`, or their pane crashes hard and the Director re-spawns. The Director owns recovery decisions.

## 2-stage health check

Before assuming a member is stalled, run the cheap check first:

1. **Check the Director's inbox via `cafleet message poll`** — the member may have replied and you missed the inline-preview keystroke. The poll output shows pending messages without touching the member's pane.
2. **Capture the member's pane via `cafleet member capture`** — see what the member is actually doing. Default `--lines 30`. If the capture is too short to show the prompt frame, re-run with `--lines 120` or `--lines 200`.

```bash
cafleet --session-id <session-id> message poll --agent-id <director-agent-id>
cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id>
```

## Routine monitoring via `member list --activity`

The `--activity` flag aggregates per-member `last_sent` / `last_recv` / `last_ack` / `idle` columns so a routine `/loop` tick can decide which members need a capture WITHOUT capturing every member every minute. See [`reference/director.md`](director.md#member-list-with-activity).

```
$ cafleet --session-id <s> member list --agent-id <d> --activity
3 members:
  agent_id        name      state   last_sent    last_recv    last_ack     idle
  --------------  --------  ------  -----------  -----------  -----------  -----
  abc12345        alice     active  12:34:56     12:34:50     12:34:50     6s
  def67890        bob       active  12:30:11     12:33:02     12:33:02     2m
  ghi24680        carol     idle    -            12:20:00     12:20:00     14m
```

Heuristic: capture only members whose `idle > 5m` AND who have an unread inbox (the broker just delivered an inline preview that they have not yet acked). Capture is the expensive operation; use it sparingly.

## Stalled member shapes

Once you have a capture, classify the shape:

| Shape | Recovery |
|---|---|
| **AskUserQuestion-paused** (4-option frame `1./2./3./4. Type something`) | Three-beat AskUserQuestion-delegated workflow — see [`reference/director.md`](director.md#answer-a-members-askuserquestion-prompt). Director MUST delegate the decision to the user via `AskUserQuestion`, then `cafleet member send-input` with the resolved choice/freetext. |
| **Bash-denied** (member sent a CAFleet message asking the Director to run a command on its behalf) | Bash-via-Director protocol — see [`reference/exec-routing.md`](exec-routing.md). Dispatch via `cafleet member exec`, then immediately `cafleet member ping`. |
| **Missed inline-preview keystroke** (the recipient's TUI was in a non-input state when the broker keystroked the message preview, so the preview landed elsewhere on the pane and was lost) | `cafleet member ping <member>` re-keystrokes the `cafleet message poll` command into the member's pane. The member runs `cafleet message poll` and drains whatever has accumulated. |
| **REPL idle / mid tool-call** (capture shows a coding-agent prompt or active tool call, no AskUserQuestion frame) | Wait. The member is doing work. Re-check `member list --activity` next tick; only escalate if `idle` keeps growing AND there is unread inbox. |
| **Pane crashed** (capture shows a shell prompt without the coding agent, or the pane is missing entirely) | `cafleet member delete --force` to tear down the registration cleanly, then `cafleet member create` to re-spawn. The previous member's `agent_id` is gone — do not re-use it. |
| **Truly wedged** (capture shows no progress over multiple ticks, no AskUserQuestion frame, no pending work explanation) | Soft escalation first — `cafleet member ping <member>` to nudge. If unchanged after 2–3 ticks, hard escalation: `cafleet member delete --force` then re-spawn. |

## Recovering from a tmux disconnect

If `cafleet member capture` exits with a tmux subprocess error (the tmux server is unreachable):

1. Run `cafleet doctor` to confirm your own pane's tmux state. If `cafleet doctor` exits 1 with `Error: cafleet member commands must be run inside a tmux session`, your `TMUX` env var is unset — you are no longer attached to a tmux session and recovery is impossible from your current shell. Re-attach (`tmux attach -t <session>`) and re-run.
2. If `cafleet doctor` succeeds but `cafleet member capture` still fails, the target pane is gone (the tmux server killed it, or the user closed it manually). Treat as "Pane crashed" above — `cafleet member delete --force` then re-spawn.
3. Never invoke raw `tmux send-keys`, `tmux kill-pane`, `tmux list-panes`, `tmux capture-pane`, or `tmux display-message` directly. Cafleet's primitives encapsulate the cross-Director authorization boundary; raw tmux bypasses it.

## Recovering from a wedged `/exit`

The default `cafleet member delete` path sends `/exit`, polls `tmux list-panes` for the target `pane_id` until it disappears (15 s timeout), then deregisters and rebalances. On timeout the command exits 2 with the pane buffer tail printed on stderr. Recovery decision tree:

1. **Inspect the tail.** What is the member doing?
2. **AskUserQuestion-paused** → answer the prompt with `cafleet member send-input --choice N` or `--freetext`, then re-run `cafleet member delete`.
3. **Mid tool-call / mid command** → `cafleet member ping <member>` to nudge it back to a prompt, wait 1–2 ticks, then re-run `cafleet member delete`.
4. **Truly wedged** → `cafleet member delete --force`, which skips `/exit` and kill-panes immediately. Always exits 0 (idempotent against an already-dead pane).

## Shutdown Protocol

The teardown MUST run in this exact order. Skipping any step leaves crons firing against dead agents, or orphan coding-agent processes lingering in panes.

**Rule: use cafleet primitives only.** All tmux interactions — write, inspect, and metadata — are encapsulated by cafleet commands. For tmux session/window/pane metadata at Director startup, use `cafleet doctor`. Never invoke raw tmux directly from the Director. If a workflow appears to need a raw tmux call, file a gap in `cafleet member *` or `cafleet doctor` — NOT a raw tmux invocation.

1. **Stop every background `/loop` monitor FIRST.** Any `/loop` cron the Director started during the session must be cancelled with `CronDelete <job-id>` **before** members are deleted. A cron that keeps firing after members are gone will issue `cafleet member list` / `poll` against a tearing-down session, spam `Error: session is deleted`, and (worse) race with the member-delete path and nudge agents that are mid-`/exit`. Fixed-cadence `/loop`s (e.g. the team-health monitor from the `cafleet-agent-team-monitoring` skill) and any augmented loops you created (PR review loops, verifier loops, etc.) all fall under this rule. Stop them all.
2. **Delete every member** via `cafleet member delete`. This call blocks until the target pane is actually gone (15 s default timeout). On timeout follow the wedged-`/exit` decision tree above. Do this per member, not via `session delete` alone — `session delete` deregisters agents in the DB but does NOT send `/exit` to panes.
3. **Verify every member is gone via cafleet.** Run `cafleet member list --agent-id <director-agent-id>`. The team's member roster should be empty. Any agent still present means step 2 failed — re-run `cafleet member delete` on that member, capture if needed, and report to the user if it still refuses to leave. Do NOT use raw tmux to "check" or "force" anything.
4. **Run `cafleet session delete <session-id>`** (positional, no `--session-id` flag). This deregisters the root Director, deregisters the Administrator, sweeps any agent rows that survived step 2, and physically deletes every `agent_placements` row. Plain `cafleet --session-id <session-id> agent deregister --agent-id <root-director-id>` is rejected with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` — always use `session delete` for the final teardown step.
5. **Confirm the session is closed.** Run `cafleet session list`; the current session should not appear (soft-deleted sessions are hidden). If it still appears with `active` agents, repeat steps 2–4 for that session. Any cross-conversation orphan session surfaced by this final check is also cleaned up via `cafleet session delete <its-session-id>` — never via tmux.

Skipping step 1 is the single most common failure and the one that visibly leaks into the operator's view (recurring cron output in the Director's terminal). Skipping step 3 means you proceed to `session delete` without knowing whether members actually quit, leaving orphan coding-agent processes behind.
