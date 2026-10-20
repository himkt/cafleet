# Monitoring

`cafleet monitor` is a fleet-scoped loop — `deliver → wake → sleep`.
`cafleet fleet create` starts it as a detached process, so it depends on no
member's shell tool. The loop does two jobs. Its **delivery pass** sends the
keystrokes the broker is holding for panes that were busy. Its **heartbeat**
is a plain timer, not agent reasoning, that fires one unconditional
fleet-level wake into the **monitor member**'s pane once per wake interval.

The monitor member is a dedicated watcher spawned before any other member
(by the `cafleet fleet create` bootstrap) on a cheap model — its work is
bounded classification, not generation. On each wake it classifies every
member pane and contacts the Director only when something actually needs
attention, so the Director is never nudged by a timer.

One monitor loop runs per fleet. Separately, the database enforces one
active monitor member per fleet, including concurrent registrations. A
monitor's pane dying does not itself deregister the member: deregister the
old member before re-spawning it. Existing duplicate records block migration
and require [duplicate-monitor recovery](storage.md#duplicate-monitor-recovery).

## Delivery, heartbeat, classification, facilitation

The loop decides only the *when*; the monitor member owns the *what changed*;
everything downstream — assignment, dispatch, recovery, escalation — stays the
Director's facilitation:

| Layer | Owns | Lives in |
|---|---|---|
| Delivery (the *when it lands*) | holding each broker keystroke until its pane is at rest, then sending it | the broker and the `cafleet monitor` loop |
| Heartbeat (the *when to look*) | the unconditional fleet-level wake into the monitor member's pane | the `cafleet monitor` loop |
| Classification (the *what changed*) | pane capture, per-backend content classification, quiet confirmation, the bounded pings, event messages to the Director | the monitor member, per its role protocol (part of the cafleet skill) |
| Facilitation (the *what next*) | health-check judgment, assignment, dispatch, recovery, escalation | the Director, per the cafleet skill's supervision protocol |

The **wake trigger** into the monitor member's own pane is a pure trigger,
not a protocol payload. It opens `[cafleet] tick:`,
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

## The delivery pass {#delivery-pass}

A message, an exec dispatch, and an exec resume each need one keystroke into
a member's pane. The broker sends that keystroke only into a pane at rest. A
pane that shows a permission prompt keeps its prompt, and a member in the
middle of a turn keeps its turn: the keystroke is **held**, and the work
stays owed.

The command that creates the work tries once, so delivery into a pane at rest
is immediate. The loop then retries every held item on every tick — also when
the wake interval is `0` — so held work always has a wake and needs no second
signal from anyone:

1. Read the multiplexer's live pane set. If the root Director's pane is gone,
   the loop stops: a fleet without a Director pane has no one to facilitate
   it.
2. Close the execs whose command was lost.
3. Run the [ready watchdog](#ready-watchdog).
4. For every active member — the Director included — whose pane is live and
   that has owed work, make one delivery attempt. A member with nothing owed
   is neither captured nor keystroked, so an idle fleet is left alone.

The broker judges a pane from its captured text and names one of four states:

| State | Meaning | Delivery |
|---|---|---|
| `finished` | The composer is visible, with no prompt box and no active work | Sent now |
| `awaiting_user` | A permission or selection prompt is waiting for a keypress | Held |
| `working` | A turn, a tool, or a command is running | Held |
| unclassified | None of the cues match | Held |

A hold is bounded. Once it has lasted longer than the hold timeout the
delivery is **forced** into the pane as it is, with a line that tells the
recipient what the broker interrupted. The gate, the timeout, the payloads,
and the per-backend cues are specified in
[Held delivery](../spec/multiplexer-backends.md#held-delivery).

For the Director this means **dispatch is unconditional**: it sends an
assignment the moment it has one, and the broker decides when the keystroke
lands. The Director keeps no deferred sends and reads no capture before a
send.

## Ready watchdog {#ready-watchdog}

`ready` is the only signal that the agent in a pane booted. On every tick the
loop looks for an active non-Director member that owns a pane, was spawned at
least 180 seconds ago, and has never sent a message. It posts one
silent-member [notice](../spec/data-model.md#broker-notices) to the Director
for that member and never repeats it. A member that goes silent after it has
spoken is the monitor member's to report.

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
7. Reports a denial of one of its own commands to the Director once, naming
   the command and the denial text. A monitor member whose `message send` is
   denied from the start is reported by the [ready watchdog](#ready-watchdog).

The monitor member's command surface on wake is exactly three families —
`cafleet monitor scan`, `cafleet member ping`, and `cafleet message send` to
the Director. Never `message broadcast`, never `member prompt`, never a ping
at itself. A claude monitor member is spawned with all three already allowed
(see [Spawn-time allow rules](../spec/coding-agent-backends.md#spawn-time-allow-rules)).

The Director's re-engagement channels are the broker's inline previews, the
broker's [notices](../spec/data-model.md#broker-notices), the monitor's event
messages, and the monitor's stalled-Director ping. `cafleet member ping` is
gated by the broker itself: it keystrokes a pane at rest or a quiet pane the
broker cannot classify, and skips a pane that is working, waiting on a
prompt, or running an exec. A captured prompt is not a relayed question: the
Director answers explicit member questions through the broker.

Pane state therefore has two readers with different jobs. The broker's
four-state classification decides whether a keystroke may be sent. The
monitor member's reading adds `stall_candidate` and `unknown` and decides
whether a quiet pane deserves a ping or a report.

An `unknown` capture calls for investigation, not a ping or an assumption
that the pane died. Use `cafleet doctor` to diagnose the current connection and
`cafleet member list` / `member show` to check the registered placement.
Registry data does not prove physical pane presence or absence; retain
`unknown` when disappearance is unproven. Ask the user for missing pane-state
facts only if that uncertainty blocks the work. The skill's recovery reference
owns the procedure.

The scan is a **read-only batch of captures**:
`cafleet monitor scan <fleet-id>` is a one-shot command that captures the
Director's pane and every active member's pane in a single invocation —
Director first, then members in ascending member-id order — so the reader
gets individually timed captures together. A pending
placement or a failed pane capture renders an annotated entry and the scan
still completes; the command performs no DB writes and stores no capture
content. `cafleet member capture` remains the targeted deeper-investigation
primitive for a single pane.

## Cadence and tick precision {#cadence-and-tick-precision}

| Knob | Default | Set by |
|---|---|---|
| Wake interval | `600s` | `cafleet monitor FLEET_ID --interval N`, the fleet's stored interval, or `CAFLEET_MONITOR_WAKE_INTERVAL` |
| Tick | `5s` | `cafleet monitor FLEET_ID --tick N`, or the fleet's stored tick |

The loop runs its delivery pass once per **tick**, and the wake fires at the
first tick boundary on which the wake interval has elapsed, so the tick is
the floor on interval precision. The first wake is measured from the moment
the loop started: it fires only once the interval has elapsed since launch,
so a freshly spawned monitor member gets its startup window undisturbed.
Every later wake is measured from the last delivered wake.
`CAFLEET_MONITOR_WAKE_INTERVAL=0` (or `--interval 0`) disables the wake while
the loop keeps claiming the runtime slot, heartbeating, and delivering every
tick.

The interval is stored per fleet in the `monitor_runtime` row and re-read on
every tick, so a running loop's cadence is editable from the admin WebUI's
interval editor: an edit takes effect within one tick. A restarted loop keeps
the stored interval and tick unless it is started with a flag; the full
precedence is in
[CLI options](../spec/cli-options.md#monitor-loop-precedence). A new fleet
starts from the environment default. Saving `0` disables the wake exactly as
`--interval 0` does.

The schedule is not the only wake trigger: the admin WebUI's "Wake now"
control (`POST /api/monitor/wake`) records a durable wake request on the
fleet's runtime row, and the running loop honors it on its next tick — the
wake lands within one tick even when the interval is `0` or the
schedule is not yet due, because an explicit operator action bypasses a
disabled or not-yet-due schedule. Repeat requests coalesce into a single
wake, and a wake the loop has to skip (no resolvable monitor pane) leaves
the request pending to retry on the next tick, exactly as a scheduled wake
stays due. A delivered wake — scheduled or requested — stamps the last-wake
timestamp and clears any pending request in the same write, so a requested
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

Every keystroke is typed `Esc`-first: `Escape`, a settle delay, the payload,
then `Enter`. A delivery is sent only after the broker has seen the pane at
rest, and the `Escape` covers the moment between that capture and the
keystroke: a prompt that opens in between is rejected, never confirmed.

The wake is the one loop keystroke that is not held. It re-engages a monitor
member stuck mid-turn, its work is an idempotent re-scan, and the monitor
member's spawn posture shows no user prompt. The resume clause makes the wake
self-healing: a monitor member that stalls mid-turn is re-engaged by its own
next wake.

One hazard is documented rather than guarded: if the operator is
mid-composition at a pane when a keystroke lands, the payload is appended to
whatever text is already in the composer, then `Enter` submits both together.
An operator is rarely typing in the monitor member's pane, and `--interval 0`
is the escape for hands-on sessions.

## Single-instance and liveness

Exactly one monitor loop may run per fleet. The `monitor_runtime` DB row is
the single authority for both the single-instance claim (one SQLite write
transaction, so two concurrent `cafleet monitor` calls cannot both win) and
liveness: the running loop rewrites `last_tick_at` every tick, so a loop that
died silently reads as stale. Both the per-tick heartbeat and the on-exit
clear are ownership-checked — a displaced loop's next heartbeat matches zero
rows and it self-terminates.

A loop that cannot reach the multiplexer must not look live. It probes the
multiplexer before claiming the row and exits without a claim when the probe
fails, and a running loop that fails to list panes on several consecutive
ticks clears its row and exits, so the next command that needs the loop
starts a replacement.

## Runtime cleanup

The [resource cleanup](../spec/cli-options.md#monitor-resource-cleanup)
keeps a lease immediately after claim, unregisters every installed signal
handle, and attempts owner-checked clear on startup failure, tick failure,
normal stop, or replacement by another PID. A failed startup write or flush
cannot establish a healthy running loop. Cleanup preserves the primary error
and reports an additional clear failure; a clear failure alone is an error.
A replacement owner's row survives, and crash recovery continues to use stale
reclaim.

## Lifecycle

**Spawn.** `cafleet fleet create` bootstraps the fleet in one command: it
creates the fleet, root Director, and monitor rows in a DB transaction, takes
ownership of the spawned monitor pane, and then starts the monitor loop as a
detached process. The Director-authored monitor prompt is passed via
`--monitor-file`, with the backend's monitor-default model via
`--monitor-model` and supported effort via `--monitor-effort` (see
[CLI options](../spec/cli-options.md#fleet-create)). The command returns only
after the loop is live. A failed bootstrap attempts DB rollback and
known-pane cleanup, and a loop that does not start is compensated the same
way: the monitor pane is killed and the fleet is soft-deleted. A cleanup
failure or unknown pane id is reported with the primary error; inspect those
diagnostics before retrying.

The monitor member's own startup is one step: it sends the standard `ready`
signal and ends its turn. That `ready` unblocks the Director's first ordinary
`cafleet member create`; the CLI enforces the same order (spawning an
ordinary member into a fleet with no active monitor member fails — see
[CLI options](../spec/cli-options.md#member-create)), and a
`member create --role monitor` spawn into a fleet that already has an active
monitor member also fails.

The Director then spawns the ordinary members and **dispatches on ready**:
when a member's ready signal arrives, the Director ACKs it and dispatches
that member's first task in the same turn, provided the task's inputs exist.
A member that sent `ready` and is still reading receives the assignment when
it goes idle, because the broker holds the keystroke until then. First-task
dispatch is per-member — never held waiting for other members' ready signals
or placements. A member whose first task genuinely depends on an input that
does not yet exist (e.g. a deliverable another member has not produced)
legitimately stays idle until that input lands — the Director dispatches
whatever is dispatchable, to whoever is ready.

**Teardown**, in order: the Director deletes the **monitor member first**,
deletes each remaining member with `cafleet member delete`, verifies with
`cafleet member list` that only the root Director's row remains, runs
`cafleet fleet delete <fleet-id>`, and confirms with `cafleet fleet list`.
`fleet delete` ends the loop: its next tick sees the soft-deleted fleet and
stops.

**Recovery and standing liveness.** The loop and the monitor member fail
independently. If the monitor member's pane dies, the loop keeps delivering;
the Director deletes the dead member and re-spawns it with `--role monitor`
(the one-per-fleet guard counts only active members, so a deleted monitor
frees the slot). If the loop process dies, the next `message send`,
`message broadcast`, or `member exec` that leaves work owed starts a
replacement, and the admin WebUI shows the loop stopped in the meantime. A
loop that died without a graceful stop leaves a `monitor_runtime` row with a
non-null `pid`; that row reads as dead on both liveness axes — the heartbeat
goes stale and the process probe reports no such process — so the
replacement reclaims it. `cafleet fleet delete` removes the row
unconditionally. No manual cleanup step exists or is needed.

See [Data model](../spec/data-model.md) for the backing table and
[CLI options](../spec/cli-options.md#cafleet-monitor) for the command surface
of both the loop and the one-shot
[`monitor scan`](../spec/cli-options.md#cafleet-monitor-scan).
