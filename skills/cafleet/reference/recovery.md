# Recovery flows

Director reference for crash / disconnect / idle / wedged-pane recovery. Member-side recovery is rare — members either run cleanly until the Director runs `member delete`, or their pane crashes hard and the Director re-spawns. The Director owns recovery decisions.

## 2-stage health check

Before assuming a member is stalled, run the cheap check first — poll your own inbox, then capture the member's pane — per [`supervision.md`](supervision.md) § Stall Response. Recovery-specific detail: bump `cafleet member capture --lines` to show a member's full decision-prompt frame (the line count needed is a backend delta, see your overlay).

## Routine monitoring via `member list`

Use `member list` (see [`reference/director.md`](director.md) § *Member List*) for routine supervision ticks instead of capturing every member — its `idle` column is the stall signal. Heuristic: capture only members whose `idle > 5m` AND have an unread inbox — capture is the expensive operation, use it sparingly.

## Stalled member shapes

Once you have a capture, classify the shape:

| Shape | Recovery |
|---|---|
| **Decision-prompt paused** (capture shows the member waiting on a user reaction) | The member is relaying a question for the user. Delegate the decision to the user and forward the answer through the Director's decision surface — the concrete relay (and any pane-keystroke primitive) is a backend delta; see your overlay section (`coding-agent-overlays.md#<name>`) and [`reference/director.md`](director.md#answering-a-members-relayed-question). |
| **Bash-denied** (member sent a CAFleet message asking the Director to run a command on its behalf) | Bash-via-Director protocol — see [`reference/prompt-routing.md`](prompt-routing.md). Dispatch via `cafleet member prompt --shell`, then immediately `cafleet member ping`. |
| **Missed inline-preview keystroke** (the recipient's TUI was in a non-input state when the broker keystroked the message preview, so the preview landed elsewhere on the pane and was lost) | `cafleet member ping <member>` re-keystrokes the `cafleet message poll` command into the member's pane. The member runs `cafleet message poll` and drains whatever has accumulated. |
| **REPL idle / mid tool-call** (capture shows a coding-agent prompt or active tool call, not a decision-prompt frame) | Wait. The member is doing work. Re-check `member list` next tick; only escalate if `idle` keeps growing AND there is unread inbox. |
| **Pane crashed** (capture shows a shell prompt without the coding agent, or the pane is missing entirely) | `cafleet member delete` to tear down the registration cleanly, then `cafleet member create` to re-spawn. The previous member's `member_id` is gone — do not re-use it. |
| **Truly wedged** (capture shows no progress over multiple ticks, no decision-prompt frame, no pending work explanation) | Soft escalation first — `cafleet member ping <member>`. If unchanged after 2–3 ticks, hard escalation: `cafleet member delete` then re-spawn. |

## Recovering from a tmux disconnect

If `cafleet member capture` exits with a multiplexer subprocess error (the multiplexer server is unreachable):

1. Run `cafleet doctor` to confirm your own pane's multiplexer state. If it fails to resolve a backend, you are no longer attached to a supported multiplexer session and recovery is impossible from this shell — re-attach (on tmux, `tmux attach -t <session>`) and re-run.
2. If `cafleet doctor` succeeds but `cafleet member capture` still fails, the target pane is gone (the multiplexer server killed it, or the user closed it manually). Treat as "Pane crashed" above — `cafleet member delete` then re-spawn.
3. Never invoke raw tmux directly — cafleet's primitives encapsulate the fleet-isolation boundary that raw tmux bypasses.

## Shutdown Protocol

The teardown runs in this exact order. **Use cafleet primitives only** — every tmux interaction (write, inspect, metadata) is encapsulated by a cafleet command (`cafleet doctor` for pane metadata at startup); never invoke raw tmux from the Director.

1. **Delete the monitor member FIRST** (`cafleet member delete <monitor-member-id>`, first-out). The pane kill takes the loop process down with it, ending the wake source before any other member disappears. The killed loop leaves a stale `monitor_runtime` row that reads as dead on both liveness axes (stale heartbeat + no such process), so a fresh `cafleet monitor` run reclaims it; `cafleet fleet delete` (step 4) removes the row unconditionally (see [`monitoring.md`](../../../docs/docs/concepts/monitoring.md)).
2. **Delete every remaining member** via `cafleet member delete`. This call kills the pane immediately. Do this per member, not via `fleet delete` alone — `fleet delete` deregisters members in the DB but does NOT kill their panes.
3. **Verify every member is gone via cafleet.** Run `cafleet member list`. Only the root Director's own row (`kind` `director`) should remain. Any other member still present means step 2 failed — re-run `cafleet member delete` on that member, capture if needed, and report to the user if it still refuses to leave.
4. **Run `cafleet fleet delete <fleet-id>`.** This deregisters the root Director, sweeps any member rows that survived step 2, and deletes every `member_placements` row. Deleting the root Director via `member delete` is rejected — always use `fleet delete` for the final teardown step.
5. **Confirm the fleet is closed.** Run `cafleet fleet list`; the current fleet should not appear (soft-deleted fleets are hidden). If it still appears with `active` members, repeat steps 2–4 for that fleet. Any cross-conversation orphan fleet surfaced by this final check is also cleaned up via `cafleet fleet delete <its-fleet-id>` — never via tmux.
