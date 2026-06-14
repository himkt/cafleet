# Recovery flows

Director reference for crash / disconnect / idle / wedged-pane recovery. Member-side recovery is rare — members either run cleanly until the Director runs `member delete`, or their pane crashes hard and the Director re-spawns. The Director owns recovery decisions.

## 2-stage health check

Before assuming a member is stalled, run the cheap check first:

1. **Check the Director's inbox via `cafleet message poll`** — the member may have replied and you missed the inline-preview keystroke. The poll output shows pending messages without touching the member's pane.
2. **Capture the member's pane via `cafleet member capture`** — see what the member is actually doing. Default `--lines 30`. If the capture is too short to show the prompt frame, re-run with `--lines 120` or `--lines 200`.

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>
cafleet --fleet-id <fleet-id> member capture \
  --member-id <member-agent-id>
```

## Routine monitoring via `member list --activity`

The `--activity` flag aggregates per-member `last_sent` / `last_recv` / `last_ack` / `idle` columns so a routine monitor tick can decide which members need a capture WITHOUT capturing every member every wake. See [`reference/director.md`](director.md) § *Member List (with `--activity`)*.

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
3. Never invoke raw `tmux send-keys`, `tmux kill-pane`, `tmux list-panes`, `tmux capture-pane`, or `tmux display-message` directly. Cafleet's primitives encapsulate the cross-fleet authorization boundary (fleet isolation); raw tmux bypasses it.

## Recovering from a wedged `/exit`

The default `cafleet member delete` path sends `/exit`, polls `tmux list-panes` for the target `pane_id` until it disappears (15 s timeout), then deregisters and rebalances. On timeout the command exits 2 with the pane buffer tail printed on stderr. Recovery decision tree:

1. **Inspect the tail.** What is the member doing?
2. **AskUserQuestion-paused** → answer the prompt with `cafleet member send-input --choice N` or `--freetext`, then re-run `cafleet member delete`.
3. **Mid tool-call / mid command** → `cafleet member ping <member>` to nudge it back to a prompt, wait 1–2 ticks, then re-run `cafleet member delete`.
4. **Truly wedged** → `cafleet member delete --force`, which skips `/exit` and kill-panes immediately. Always exits 0 (idempotent against an already-dead pane).

## Shutdown Protocol

The teardown MUST run in this exact order. Skipping any step leaves the monitor keystroking polls against dead agents, or orphan coding-agent processes lingering in panes.

**Rule: use cafleet primitives only.** All tmux interactions — write, inspect, and metadata — are encapsulated by cafleet commands. For tmux session/window/pane metadata at Director startup, use `cafleet doctor`. Never invoke raw tmux directly from the Director. If a workflow appears to need a raw tmux call, file a gap in `cafleet member *` or `cafleet doctor` — NOT a raw tmux invocation.

1. **Stop the monitor FIRST.** The `cafleet monitor` loop runs as a background task **in the monitoring member's pane** (the Director no longer runs it). The clean stop is: the Director messages the monitoring member to **stop its `monitor start` background task** — the coding agent's task-stop delivers SIGTERM/SIGINT, so the loop runs its `finally` and clears the `monitor_runtime` row — and the monitoring member confirms. Do this **before** the monitoring member's pane is killed (teardown is first-out — the mirror of the first-in spawn order). There is no `cafleet monitor stop` command. A monitor that keeps ticking after teardown begins keystrokes ping commands (a bare `Esc`+poll to the Director, an `Esc`+wake nudge to the monitoring member) into tearing-down panes, races with the delete path, and nudges agents that are mid-`/exit`. There is exactly one monitor loop per fleet — stopping it ends every supervision tick at once (team-health and the Step-7 PR-review poll alike, since PR-review polling is a facilitation step the Director runs on the same heartbeat, not a separate scheduler). Because the loop no longer pings ordinary members, this race now applies only to the monitoring member itself. **Degraded fallback:** if the background task cannot be stopped cleanly and the pane is killed directly, the resulting SIGHUP terminates the loop **without** running `finally`, leaving a stale `monitor_runtime` row — tolerated, because `status` reports stopped once the heartbeat goes stale and `fleet delete` (step 4) removes the row outright. `cafleet fleet delete` (step 4) also makes any still-running loop self-terminate on its next tick, so this rung is belt-and-suspenders — but stop the task explicitly first so no tick races the deletions.
2. **Delete the monitoring member first, then every ordinary member** via `cafleet member delete` (first-out: the monitoring member, whose `monitor start` task you stopped in step 1, is deleted before the ordinary members it was watching). This call blocks until the target pane is actually gone (15 s default timeout). On timeout follow the wedged-`/exit` decision tree above. Do this per member, not via `fleet delete` alone — `fleet delete` deregisters agents in the DB but does NOT send `/exit` to panes.
3. **Verify every member is gone via cafleet.** Run `cafleet member list`. The team's member roster should be empty. Any agent still present means step 2 failed — re-run `cafleet member delete` on that member, capture if needed, and report to the user if it still refuses to leave. Do NOT use raw tmux to "check" or "force" anything.
4. **Run `cafleet fleet delete <fleet-id>`** (positional, no `--fleet-id` flag). This deregisters the root Director, deregisters the Administrator, sweeps any agent rows that survived step 2, and physically deletes every `agent_placements` row. Plain `cafleet --fleet-id <fleet-id> agent deregister --agent-id <root-director-id>` is rejected with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead.` — always use `fleet delete` for the final teardown step.
5. **Confirm the fleet is closed.** Run `cafleet fleet list`; the current fleet should not appear (soft-deleted fleets are hidden). If it still appears with `active` agents, repeat steps 2–4 for that fleet. Any cross-conversation orphan fleet surfaced by this final check is also cleaned up via `cafleet fleet delete <its-fleet-id>` — never via tmux.

Skipping step 1 is the single most common failure and the one that visibly leaks into the operator's view (recurring cron output in the Director's terminal). Skipping step 3 means you proceed to `fleet delete` without knowing whether members actually quit, leaving orphan coding-agent processes behind.
