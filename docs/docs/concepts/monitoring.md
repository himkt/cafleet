# Monitoring

`cafleet monitor` is a fleet-scoped foreground loop — `scan → wake → sleep`.
The fleet's **monitor member** hosts it as a backend-resolved long-lived execution
in its own pane.
The monitor member is a dedicated watcher spawned before any other member
(by the `cafleet fleet create` bootstrap) on a cheap model — its work is
bounded classification, not generation. The loop supplies the **heartbeat**: a
plain loop, not agent reasoning, that fires one unconditional fleet-level wake
into the monitor member's own pane once per wake interval. On each wake the
monitor member classifies every member pane and contacts the Director only
when something actually needs attention, so the Director is never nudged by a
timer. The monitor member alone owns the execution handle and liveness checks;
the Director reacts only to broker signals and never launches or polls the
execution. Hosting mechanics differ by backend, while the heartbeat semantics
are identical. One monitor loop per fleet; deleting the monitor member kills
its pane and the loop process with it. Separately, the database enforces one
active monitor member per fleet, including concurrent registrations. A
monitor's pane dying does not itself deregister the member: deregister the
old member before re-spawning it. Existing duplicate records block migration
and require [duplicate-monitor recovery](storage.md#duplicate-monitor-recovery).

## Heartbeat, classification, facilitation

The loop decides only the *when*; the monitor member owns the *what changed*;
everything downstream — assignment, dispatch, recovery, escalation — stays the
Director's facilitation:

| Layer | Owns | Lives in |
|---|---|---|
| Heartbeat (the *when*) | the unconditional fleet-level wake into the monitor member's pane | the `cafleet monitor` loop |
| Classification (the *what changed*) | pane capture, per-backend content classification, quiet confirmation, the bounded pings, event messages to the Director | the monitor member, per its role protocol (part of the cafleet skill) |
| Facilitation (the *what next*) | health-check judgment, assignment, dispatch, recovery, escalation | the Director, per the cafleet skill's supervision protocol |

The loop's only keystroke is a **wake trigger** into the monitor member's own
pane — a pure trigger, not a protocol payload. It opens `[cafleet] tick:`,
names every active ordinary member (the Director and the monitor member
excluded) as `<member-id> (<name>; coding_agent=<agent>;
unacked=<pending-count>)` in ascending member-id order, always carries a
`Director:` segment in the same field grammar, then closes with
`Follow your monitor role protocol.` and the resume clause:
`Resume your work if something was still running.` A fleet with no ordinary
members receives the `no members to health-check.` form, still carrying the
`Director:` segment. The tmux and
herdr payloads are byte-identical; the exact grammar is pinned in
[Multiplexer backends](../spec/multiplexer-backends.md).

## The monitor member's wake protocol

The sole normative carrier of the on-wake protocol is the monitor role file,
part of the cafleet skill — the wake payload points at it (`Follow your
monitor role protocol.`) and carries no protocol clauses itself. In outline,
on each wake the monitor member:

1. Captures the whole fleet once with `cafleet monitor scan <fleet-id>`,
   using each entry's emitted `content`, `captured_at`, and `content_sha256`.
2. Classifies **content only**, per the **target member's** backend overlay
   capture cues (`awaiting_user` / `finished` / `working` /
   `stall_candidate`); a dead, garbled, or failed capture is `unknown`. The
   classification universe is the wake payload's members plus the Director —
   the scan also captures the monitor's own pane, which it ignores.
3. Confirms quiet across two consecutive wakes: `stall_candidate` and
   `finished` are both quiet observations, and a member is **confirmed
   quiet** only when its `content_sha256` on this wake is byte-identical to
   the previous wake's. A first quiet capture only seeds the baseline;
   changed content, `working`, or `awaiting_user` ends the quiet period and
   re-arms the member.
4. Pings an ordinary member at most once per quiet period
   (`cafleet member ping`) — the monitor's **fixed-ping exception**.
   Confirmed quiet alone suffices here: a member may have stalled mid-task
   with an empty inbox, and one bounded poll trigger per quiet period is
   cheap.
5. Pings the Director only when it is actually stalled: confirmed quiet
   across two consecutive wakes **and** its wake-payload `unacked` count is
   greater than 0. A quiet Director with an empty inbox is at legitimate
   rest — pinging it on quiet alone would recreate the timer-nudge problem
   the monitor member exists to remove.
6. Messages the Director per event (a plain `cafleet message send`): a member
   still unchanged at the next wake after its ping, a ping delivery failure,
   or an `unknown` capture — each said once per quiet period. With no event,
   it sends nothing.

The monitor member's command surface on wake is exactly three families —
`cafleet monitor scan`, `cafleet member ping`, and `cafleet message send` to
the Director. Never `message broadcast`, never `member prompt`, never a ping
at itself.

The Director's re-engagement channels are the broker's automatic inline
previews, the monitor's event messages, and the monitor's stalled-Director
ping. The Director's own re-engagement remains **capture-gated**: before it
fires a re-engagement keystroke at a member (`cafleet member ping`, a
non-exempt `cafleet message send`, or a `cafleet message broadcast`), it reads
a fresh capture of the target's pane and classifies the content with the
capture cues of the target's backend overlay, firing only on `finished` or a
confirmed stall. One fresh `cafleet monitor scan` satisfies the gate for every
member for that facilitation turn; once the Director keystrokes a pane, that
pane's snapshot is stale and a further re-engagement of the same member needs
a fresh capture. The full pre-ping capture gate is part of the cafleet skill's
supervision protocol. The capture-state taxonomy thus has two consumers: the
monitor member's on-wake classification and the Director's pre-ping gate.

The scan both consumers use is a **read-only snapshot**:
`cafleet monitor scan <fleet-id>` is a one-shot command that captures the
Director's pane and every active member's pane in a single invocation —
Director first, then members in ascending member-id order — so the reader
gets one synchronized fleet snapshot instead of one pane at a time. A pending
placement or a failed pane capture renders an annotated entry and the scan
still completes; the command performs no DB writes and stores no capture
content. `cafleet member capture` remains the targeted deeper-investigation
primitive for a single pane.

## Cadence and tick precision {#cadence-and-tick-precision}

| Knob | Default | Set by |
|---|---|---|
| Wake interval | `600s` | `CAFLEET_MONITOR_WAKE_INTERVAL` / `cafleet monitor FLEET_ID --interval N` |
| Scan tick | `5s` | `cafleet monitor FLEET_ID --tick N` (per run) |

The monitor loop scans once per **tick** and the wake fires at the first tick
boundary on which the wake interval has elapsed, so the tick is the floor on
interval precision. The first wake is measured from the moment the loop
started: it fires only once the interval has elapsed since launch, so a
freshly spawned monitor member gets its startup window undisturbed. Every
later wake is measured from the last delivered wake.
`CAFLEET_MONITOR_WAKE_INTERVAL=0` (or `--interval 0`) disables the wake while
the loop keeps claiming the runtime slot and heartbeating every tick.

The resolved interval is stamped per fleet into the `monitor_runtime` row at
each `cafleet monitor` start and re-read on every tick, so a running loop's
cadence is editable from the admin WebUI's interval editor: an edit takes
effect within one tick and lasts until the next `cafleet monitor` start
re-stamps the interval from the CLI/env resolution. Saving `0` disables the
wake exactly as `--interval 0` does, while the loop keeps heartbeating.

The schedule is not the only wake trigger: the admin WebUI's "Wake now"
control (`POST /api/monitor/wake`) records a durable wake request on the
fleet's runtime row, and the running loop honors it on its next tick — the
wake lands within one scan tick even when the interval is `0` or the
schedule is not yet due, because an explicit operator action bypasses a
disabled or not-yet-due schedule. Repeat requests coalesce into a single
wake, and a wake the loop has to skip (no resolvable monitor pane) leaves
the request pending to retry on the next tick, exactly as a scheduled wake
stays due. A delivered wake — scheduled or forced — stamps the last-wake
timestamp and clears any pending request in the same write, so a forced
wake resets the schedule baseline.

The wake fires whenever the interval has elapsed and the fleet's monitor
member is resolvable to a live pane, **including when the fleet has no
ordinary members** (the `no members to health-check.` payload form). No
active monitor member, or a monitor pane the multiplexer no longer lists,
means no wake and no timestamp stamp — the fleet stays due, so the wake fires
as soon as a monitor pane is back. The last wake timestamp is durable across
loop restarts, so a fleet that has already been woken keeps its remaining
wake cadence across an immediate restart rather than being woken instantly; a
fleet that has never been woken restarts its first-wake timer from the new
launch. A failed wake commits nothing and retries on the next tick.

## Keystroke safety

The wake is typed `Esc`-first: `Escape`, a settle delay, the payload, then
`Enter` — the same safeguard every inline message preview already uses, so a
wake landing on a pending permission prompt clears it instead of answering
it. The resume clause makes the wake self-healing: a monitor member that
stalls mid-turn is re-engaged by its own next wake.

One hazard is documented rather than guarded: if the operator is
mid-composition at the monitor member's pane when a wake lands, the `Esc`
clears any pending prompt box and the payload is appended to whatever text is
already in the composer, then `Enter` submits both together. An operator is
rarely typing in the monitor member's pane, and `--interval 0` is the escape
for hands-on sessions.

## Single-instance and liveness

Exactly one monitor loop may run per fleet. The `monitor_runtime` DB row is
the single authority for both the single-instance claim (one SQLite write
transaction, so two concurrent `cafleet monitor` calls cannot both win) and
liveness: the running loop rewrites `last_tick_at` every tick, so a loop that
died silently reads as stale. Both the per-tick heartbeat and the on-exit
clear are ownership-checked — a displaced loop's next heartbeat matches zero
rows and it self-terminates.

## Runtime cleanup

The [planned resource cleanup](../spec/cli-options.md#monitor-resource-cleanup)
keeps a lease immediately after claim, unregisters every installed signal
handle, and attempts owner-checked clear on startup failure, tick failure,
normal stop, or replacement by another PID. A failed startup write or flush
cannot establish a healthy running loop. Cleanup preserves the primary error
and reports an additional clear failure; a clear failure alone is an error.
A replacement owner's row survives, and crash recovery continues to use stale
reclaim.

## Lifecycle

**Spawn.** `cafleet fleet create` spawns the monitor member as part of the
fleet bootstrap: one command creates the fleet, root Director, and monitor
rows in a DB transaction and takes ownership of the spawned pane, with the
Director-authored monitor prompt passed via `--monitor-file` and the
backend's monitor-default model via `--monitor-model` (see
[CLI options](../spec/cli-options.md#fleet-create)). Failed bootstrap attempts
DB rollback and known-pane cleanup. A cleanup failure or unknown pane id is
reported with the primary error; inspect those diagnostics before retrying. At startup the monitor member sends the standard `ready`
signal, launches `cafleet monitor <fleet-id>` using its backend-resolved
long-lived-execution primitive, and confirms the startup line the loop prints
immediately after claiming the runtime row — `monitor loop started (fleet
<fleet_id>, tick <tick>s, pid <pid>)`. On Codex, it runs the command without
shell `&`, retains the managed execution's session ID, and inspects the initial output.
If the line is absent while the session remains active, it performs one immediate poll.
A missing session ID or an early exit is a failed start; an
active but unconfirmed session is terminated after that poll. Only after the
line is observed does the monitor member send `monitor live` to the Director.
The monitor member is the only party that owns this execution or its handle:
the Director receives only broker status signals, ordinary members never run
the loop, and the session ID is never shared. That gate message unblocks the
Director's first ordinary `cafleet member create`; the CLI enforces the same order (spawning
an ordinary member into a fleet with no active monitor member fails — see
[CLI options](../spec/cli-options.md#member-create)), and a
`member create --role monitor` spawn into a fleet that already has an active
monitor member also fails. Any failed start is reported to the Director without claiming the
monitor is live.

Once `monitor live` arrives, the Director spawns the ordinary members and
**dispatches on ready**: when a member's ready signal arrives, the Director
ACKs it and dispatches that member's first task in the same turn, provided
the task's inputs exist. First-task dispatch is per-member — never held
waiting for other members' ready signals or placements. A member whose
first task genuinely depends on an input that does not yet exist (e.g. a
deliverable another member has not produced) legitimately stays idle until
that input lands — the Director dispatches whatever is dispatchable, to
whoever is ready.

**Teardown**, in order: the Director deletes the **monitor member first**
(first-out — the pane kill takes the loop process down, ending the wake
source before any other member disappears), deletes each remaining member
with `cafleet member delete`, verifies with `cafleet member list` that only
the root Director's row remains, runs `cafleet fleet delete <fleet-id>`, and
confirms with `cafleet fleet list`. `fleet delete` alone also ends a
still-running loop — its next tick sees the soft-deleted fleet and
self-terminates.

**Recovery and standing liveness.** If the monitor member's pane dies without
a graceful stop, the loop process dies with the pane and a stale
`monitor_runtime` row survives
with a non-null `pid`. That row reads as dead on both liveness axes — the
heartbeat goes stale and the process probe reports no such process — so a
fresh `cafleet monitor` run reclaims it and succeeds. The Director re-spawns
the dead monitor with `--role monitor` (the one-per-fleet guard counts only
active members, so a deleted monitor frees the slot), and the fresh loop
reclaims the stale row. Backends with push-style exit notification surface a
loop exit directly. When a broker message reopens a later Codex turn, the
monitor member polls its retained session once before other work. If the
execution exited, the monitor member relaunches `cafleet monitor <fleet-id>`
and repeats the bounded startup confirmation; it reports `monitor restarted`
only after the replacement prints the startup line, and reports a failed
relaunch instead of claiming a restart. The Director remains broker-reactive
and never owns the execution handle or runs a session-poll loop. `cafleet fleet
delete` removes the row unconditionally. No manual cleanup step exists or is
needed.

See [Data model](../spec/data-model.md) for the backing table and
[CLI options](../spec/cli-options.md#cafleet-monitor) for the command surface
of both the loop and the one-shot
[`monitor scan`](../spec/cli-options.md#cafleet-monitor-scan).
