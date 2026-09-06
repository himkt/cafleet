# Recovery flows

Director reference for crash / disconnect / idle / wedged-pane recovery. Member-side recovery is rare — members either run cleanly until the Director runs `member delete`, or their pane crashes hard and the Director re-spawns. The Director owns recovery decisions.

## 2-stage health check

Before assuming a member is stalled, run the cheap check first — poll your own inbox, then capture the member's pane — per [`supervision.md`](supervision.md) § Stall Response. Recovery-specific detail: bump `cafleet member capture --lines` to show a member's full decision-prompt frame (the line count needed is a backend delta, see your overlay).

## Recovery entry conditions

[`supervision.md`](supervision.md#the-pre-ping-capture-gate) owns the Director's
capture → action decision, including quiet confirmation, deferred sends and
escalation. `member list` supplies registration and idle context; idle duration
and unread counts do not establish a stall. A suspected missed inline preview
still needs that fresh-capture gate before `cafleet member ping`.

A captured decision prompt is `awaiting_user`, not an instruction to answer
it. Relay only a question explicitly sent by the member, per
[`Answering a member's relayed question`](director.md#answering-a-members-relayed-question).
For an explicit Bash-denied request, use the existing exception in
[`prompt-routing.md`](prompt-routing.md): `cafleet member prompt --shell`,
then immediately `cafleet member ping` after successful dispatch.

Once inspection confirms the coding agent exited or its pane disappeared,
use `cafleet member delete` to cleanly deregister it, then `cafleet member create`
to re-spawn. The new registration has a new `member_id`. For an unresponsive
but existing agent, use supervision's escalation procedure before deciding
to re-spawn; elapsed ticks alone do not authorize deleting it.

## Recovering from a tmux disconnect

If `cafleet member capture` exits with a multiplexer subprocess error (the multiplexer server is unreachable):

1. Run `cafleet doctor` to confirm your own pane's multiplexer state. If it fails to resolve a backend, you are no longer attached to a supported multiplexer session and recovery is impossible from this shell — re-attach (on tmux, `tmux attach -t <session>`) and re-run.
2. A successful `cafleet doctor` with a failed capture does not prove the target pane is gone. Use `cafleet member list <fleet-id> --json` or `cafleet member show <member-id> --json` to identify the registered backend and pane, then investigate the capture/connection error. The registry alone does not establish physical pane presence or absence: retain `unknown` and do not ping or delete the member on that evidence alone. Ask the user for missing facts about the target pane only when that uncertainty actually blocks the work, not after every failed capture.
3. Never invoke raw tmux directly — cafleet's primitives encapsulate the fleet-isolation boundary that raw tmux bypasses.

## Shutdown Protocol

The teardown runs in this exact order. **Use cafleet primitives only** — every tmux interaction (write, inspect, metadata) is encapsulated by a cafleet command (`cafleet doctor` for pane metadata at startup); never invoke raw tmux from the Director.

1. **Delete the monitor member FIRST** (`cafleet member delete <monitor-member-id>`, first-out). The pane kill takes the loop process down with it, ending the wake source before any other member disappears. The killed loop leaves a stale `monitor_runtime` row that reads as dead on both liveness axes (stale heartbeat + no such process), so a fresh `cafleet monitor` run reclaims it; `cafleet fleet delete` (step 4) removes the row unconditionally (see [`monitoring.md`](runtime/concepts/monitoring.md)).
2. **Delete every remaining member** via `cafleet member delete`. This call kills the pane immediately. Do this per member, not via `fleet delete` alone — `fleet delete` deregisters members in the DB but does NOT kill their panes.
3. **Verify every member is gone via cafleet.** Run `cafleet member list`. Only the root Director's own row (`kind` `director`) should remain. Any other member still present means step 2 failed — re-run `cafleet member delete` on that member, capture if needed, and report to the user if it still refuses to leave.
4. **Run `cafleet fleet delete <fleet-id>`.** This deregisters the root Director, sweeps any member rows that survived step 2, and deletes every `member_placements` row. Deleting the root Director via `member delete` is rejected — always use `fleet delete` for the final teardown step.
5. **Confirm the fleet is closed.** Run `cafleet fleet list`; the current fleet should not appear (soft-deleted fleets are hidden). If it still appears with `active` members, repeat steps 2–4 for that fleet. Any cross-conversation orphan fleet surfaced by this final check is also cleaned up via `cafleet fleet delete <its-fleet-id>` — never via tmux.
