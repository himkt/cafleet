# Monitoring

`cafleet monitor` is a fleet-scoped foreground loop — `scan → wake → sleep` —
that the fleet's **Director** runs as a background task in its own pane. It
supplies the **heartbeat** the Director needs to supervise its team: a plain
loop, not agent reasoning, that fires one unconditional fleet-level wake into
the Director's own pane once per wake interval, asking it to health-check its
members and resume its own work if something was still running. While the loop
runs it spends no model tokens, and because it is just a backgrounded command
it works identically on any backend. One monitor per fleet; the Director stops
the background task at teardown.

## Heartbeat vs facilitation

The loop decides only the *when*. Everything downstream of the wake — reading
panes, judging state, re-engaging members — is the Director's facilitation:

| Layer | Owns | Lives in |
|---|---|---|
| Heartbeat (the *when*) | the unconditional fleet-level wake into the Director's pane | the `cafleet monitor` loop |
| Facilitation (the *what next*) | pane capture and classification, health-check judgment, assignment, dispatch, recovery, escalation | the Director, per the cafleet skill's supervision protocol |

The loop's only keystroke is a **wake trigger** into the Director's own pane —
a pure trigger, not a protocol payload. It opens `[cafleet] tick:`, names every
active non-Director member as
`<member-id> (<name>; coding_agent=<agent>; unacked=<pending-count>)` in
ascending member-id order, tells the Director to scan panes with
`cafleet monitor scan`, poll its inbox, ACK, and dispatch, and closes with the
resume clause: `Resume your work if something was still running.` A fleet with
no other members receives the `no members to health-check.` form, still
carrying the scan instruction. The tmux and herdr payloads are byte-identical;
the exact grammar is pinned in
[Multiplexer backends](../spec/multiplexer-backends.md). The loop never
keystrokes any member pane — `cafleet member ping` stays a manual Director
primitive.

The scan the payload instructs is the **facilitation snapshot**:
`cafleet monitor scan <fleet-id>` is a one-shot, read-only command that
captures the Director's own pane and every active member's pane in a single
invocation — Director first, then members in ascending member-id order — so
the Director reads one synchronized fleet snapshot instead of one pane at a
time. A pending placement or a failed pane capture renders an annotated entry
and the scan still completes; the command performs no DB writes and stores no
capture content. The heartbeat/facilitation split is unchanged: the scan is a
facilitation primitive the Director runs on wake, not part of the loop.
`cafleet member capture` remains the targeted deeper-investigation primitive
for a single pane.

The Director's re-engagement is itself **capture-gated**: before the Director
fires a re-engagement keystroke at a member (`cafleet member ping`, a
non-exempt `cafleet message send`, or a `cafleet message broadcast`), it reads
a fresh capture of the target's pane and classifies the content as
`awaiting_user`, `finished`, `working`, or `stall_candidate` using the capture
cues of the target's backend overlay, firing only on `finished` or a confirmed
stall — a pane classified `awaiting_user` or `working` has its round skipped
and the entire send deferred to a later facilitation tick. One fresh
`cafleet monitor scan` satisfies the gate for every member for that
facilitation turn; once the Director keystrokes a pane, that pane's snapshot
is stale, and a further re-engagement of the same member within the turn needs
a fresh capture — a single-member `cafleet member capture` or a new scan. The
full pre-ping capture gate is part of the cafleet skill's supervision
protocol, which the Director follows on each on-tick health check and whenever
it re-engages a member.

## Cadence and tick precision {#cadence-and-tick-precision}

| Knob | Default | Set by |
|---|---|---|
| Director wake interval | `600s` | `CAFLEET_MONITOR_WAKE_INTERVAL` / `cafleet monitor FLEET_ID --interval N` |
| Scan tick | `5s` | `cafleet monitor FLEET_ID --tick N` (per run) |

The monitor scans once per **tick** and the wake fires at the first tick
boundary on which the wake interval has elapsed, so the tick is the floor on
interval precision. The first wake is measured from the moment the monitor
started: it fires only once the interval has elapsed since launch, so a
freshly created fleet gets its Director's spawning window undisturbed. Every
later wake is measured from the last delivered wake.
`CAFLEET_MONITOR_WAKE_INTERVAL=0` (or `--interval 0`) disables the wake while
the loop keeps claiming the runtime slot and heartbeating every tick.

The resolved interval is stamped per fleet into the `monitor_runtime` row at
each `cafleet monitor` start and re-read on every tick, so a running loop's
cadence is editable from the admin WebUI's interval editor: an edit takes
effect within one tick and lasts until the next `cafleet monitor` start
re-stamps the interval from the CLI/env resolution. Saving `0` disables the
wake exactly as `--interval 0` does, while the loop keeps heartbeating.

The wake is unconditional: it fires whenever the interval has elapsed and the
Director's pane is alive, **including when the fleet has no other members**.
The Director is itself a supervision target — the resume clause is the remedy
for a stalled Director — and a fleet with no members is a transient bootstrap
state, not a steady state (the Director stops the loop at teardown). The last
wake timestamp is durable across loop restarts, so a fleet that has already
been woken keeps its remaining wake cadence across an immediate restart rather
than being woken instantly; a fleet that has never been woken restarts its
first-wake timer from the new launch. A failed wake commits nothing and
retries on the next tick.

## Keystroke safety

The wake is typed `Esc`-first: `Escape`, a settle delay, the payload, then
`Enter` — the same safeguard every inline message preview into the Director's
pane already uses, so a wake landing on a pending permission prompt clears it
instead of answering it.

One hazard is documented rather than guarded: if the operator is
mid-composition at the Director's pane when a wake lands, the `Esc` clears any
pending prompt box and the payload is appended to whatever text is already in
the composer, then `Enter` submits both together. This is exactly the hazard
every inline message preview already carries at that same pane, and it is
accepted for the same reason. Operators running hands-on sessions at the
Director's pane can choose `--interval 0` to silence the wake for the
duration.

## Single-instance and liveness

Exactly one monitor may run per fleet. The `monitor_runtime` DB row is the
single authority for both the single-instance claim (one SQLite write
transaction, so two concurrent `cafleet monitor` calls cannot both win) and
liveness: the running loop rewrites `last_tick_at` every tick, so a monitor
that died silently reads as stale. Both the per-tick heartbeat and the on-exit
clear are ownership-checked — a displaced monitor's next heartbeat matches
zero rows and it self-terminates.

## Lifecycle

**Launch.** Immediately after `cafleet fleet create` and before the first
`cafleet member create`, the Director launches
`cafleet monitor <fleet-id>` as a background task in its own
pane and confirms the startup line the loop prints immediately after claiming
the runtime row — `monitor loop started (fleet <fleet_id>, tick <tick>s, pid
<pid>)` — before spawning any member. A loop task that exits instead
(runtime-claim conflict, dead fleet) is a failed start to resolve before
spawning.

**Teardown**, in order: the Director stops the background task (the loop's
signal handler runs its ownership-checked runtime clear), deletes each member
with `cafleet member delete`, verifies with `cafleet member list` that only
the root Director's row remains, runs
`cafleet fleet delete <fleet-id>`, and confirms with
`cafleet fleet list`. `fleet delete` alone also ends a still-running loop —
its next tick sees the soft-deleted fleet and self-terminates.

**Recovery.** If the Director's pane dies without stopping the loop, the loop
process dies with the pane and a stale `monitor_runtime` row survives with a
non-null `pid`. That row reads as dead on both liveness axes — the heartbeat
goes stale and the process probe reports no such process — so a fresh
`cafleet monitor` run reclaims it and succeeds. `cafleet fleet delete`
removes the row unconditionally. No manual cleanup step exists or is needed.

See [Data model](../spec/data-model.md) for the backing table and
[CLI options](../spec/cli-options.md#cafleet-monitor) for the command surface
of both the loop and the one-shot
[`monitor scan`](../spec/cli-options.md#cafleet-monitor-scan).
