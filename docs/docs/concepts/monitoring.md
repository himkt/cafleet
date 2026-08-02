# Monitoring

`cafleet monitor` is a fleet-scoped foreground loop — `scan → wake → sleep` —
that the fleet's dedicated **monitoring member** runs as a background task in
its own pane. It supplies the **heartbeat** a Director needs to supervise its
team: a plain loop, not agent reasoning, that scans the **watched set** (every
ordinary member, each on its own interval) and, when at least one watched
member is due, wakes the monitoring member once by keystroking into its pane.
While the loop runs it spends no model tokens, and because it is just a
backgrounded command it works identically on any backend. One monitor and one
monitoring member per fleet; there is no `monitor stop` — deleting the
monitoring member kills its pane and the loop terminates with it.

## Heartbeat vs facilitation

The loop decides only the *when*. The monitoring member judges what it sees
and may perform one fixed action; task judgment stays with the Director:

| Layer | Owns | Lives in |
|---|---|---|
| Heartbeat (the *when*) | which watched members are due; one synchronized wake into the monitoring member | the `cafleet monitor` loop |
| Quiet-member judgment | capture classification, two-wake quiet confirmation, at most one fixed inbox-poll ping per quiet period, plain per-event Director messages | the monitoring member's own in-context notes |
| Facilitation (the *what next*) | assignment judgment, task dispatch, health-check, recovery, escalation | the Director, per the cafleet skill's supervision protocol |

The loop's only keystroke is a **wake trigger** into the monitoring member's
own pane — a pure trigger, not a protocol payload. It names each due member as
`member <id> (<sanitized-name>; coding_agent=<backend>) [<reasons>]`, carries
the Director descriptor (identifying the recipient of the monitoring member's
reports), and closes with a single pointer sentence naming the monitoring
member's role protocol. The tmux and herdr payloads are byte-identical. The
loop never keystrokes a watched pane; the monitoring member may separately
invoke the fixed `cafleet member ping` once per confirmed quiet ordinary
member.

The facilitation layer's re-engagement is itself **capture-gated**: before the
Director fires a re-engagement keystroke at a member (`cafleet member ping`, a
non-exempt `cafleet message send`, or a `cafleet message broadcast`), it takes
a fresh read-only `cafleet monitor capture` of the target and classifies it on
the same five-state rubric the monitoring member uses, firing only on
`finished` or a confirmed stall — a pane classified `awaiting_user` or
`working` has its round skipped and the entire send deferred to a later
facilitation tick. The full pre-ping capture gate is part of the cafleet
skill's supervision protocol, which the Director follows whenever it
re-engages a member.

## The watched set

Enrollment is by member class:

| Member class | Enrolled | `monitor` field on `GET /api/members` |
|---|---|---|
| An ordinary member with a placement | yes, at 720 s | A schedule object |
| The fleet's root Director | no | `null` |
| The dedicated monitoring member | no — it is the watcher | `null` |
| A deregistered member | no — the row is hard-deleted on deregistration | `null` |
| A registry row without a placement | no | `null` |

That ordinary-member default, and the rest of the cadence knobs, live in the
knob table under [Cadence and tick precision](#cadence-and-tick-precision).

A watched member enters the due set only from a normal trigger. The scan order
is lifecycle reconciliation, `interval`, durable `stall-check`, native herdr
`status:done`, then `unacked` annotation. One synchronized wake carries the
complete union:

| Reason | Fires when | Advances `last_ping_at`? | Availability | Silenced by |
|---|---|---|---|---|
| `interval` | The member is enabled, its pane is alive, and its own interval has elapsed since `last_ping_at` | yes | Both backends | Disabling the member's schedule in the admin WebUI |
| `unacked` | Annotation only: the already-due member's oldest un-acked delivery is at least its own interval old | no | Both backends | It never creates a due row |
| `stall-check` | The stall-check interval has elapsed since that pane's last stall-check wake | no | Both backends | `CAFLEET_MONITOR_STALL_INTERVAL=0`, or disabling the member's schedule in the admin WebUI |
| `status:done` | The member's native agent status transitions into `done` | yes | herdr only | Disabling the member's schedule in the admin WebUI |

A stale un-acked delivery is context, not proof of a stall: it can annotate an
interval-, stall-check-, or native-due row, always last in the reason list, but
cannot add a row or wake the monitoring member. In particular,
`working + unacked` remains `working` and causes no action by itself. There is
no process-local unacked re-fire map.

On the **herdr** backend a watched member is additionally due when its native
agent status transitions into `done`; a transition into `blocked` (awaiting a
user answer) never wakes the watcher. This trigger augments, never replaces,
the interval trigger and does not exist on tmux — see
[Native agent-state](../spec/multiplexer-backends.md#native-agent-state).

## The monitoring member {#the-monitoring-member}

The monitoring member is a single, dedicated coding-agent member — spawned
first-in with `cafleet member create --role monitor` (the Director passes
`--model haiku`) — that launches `cafleet monitor start` as a background task
in its own pane and applies LLM judgment to the watched members' state. It is
identified by `member_card_json.cafleet.kind == "monitoring-member"`; a second
`--role monitor` spawn is rejected.

The wake trigger tells it *who* is due and *who* the Director is — nothing
else. The full on-wake protocol is part of the cafleet skill, which the
monitoring member loads as required reading; its memory between wakes is its
own conversation notes, not broker state. On each wake it:

1. **Captures** every named due **ordinary** pane with
   `cafleet monitor capture --lines 120 --no-ansi --json`. Capture JSON
   includes `captured_at` and `content_sha256` of the exact emitted content.
   Each target's rendered `coding_agent` selects its own overlay cues. The
   payload's `director` descriptor identifies the recipient of the monitoring
   member's reports; the monitoring member takes no Director-directed pane
   action.
2. **Classifies** content as `awaiting_user`, `unknown`, `finished`, `working`,
   or `stall_candidate`. Any affirmative or ambiguous active-work cue is
   `working`; ambiguity between `awaiting_user` and `finished` resolves to
   `awaiting_user`. A failed or unreadable capture is `unknown`: it never
   seeds, advances, or confirms the quiet baseline and is never pinged; the
   monitoring member clears its recorded baseline for that member and messages
   the Director about the capture failure once.
3. **Confirms quiet members across two stall-check wakes.** `stall_candidate`
   and `finished` are both **quiet** observations for an ordinary member. Only
   a capture taken on a wake whose entry carries the `stall-check` reason may
   seed, advance, or confirm the quiet baseline. A quiet member is confirmed
   only when its `content_sha256` is byte-identical to the sha recorded on the
   previous stall-check wake. A first quiet capture only seeds the baseline;
   after a monitoring-member restart the notes are gone, so the first
   post-restart wake re-seeds and never pings.
4. **Pings at most once** per confirmed quiet member with the fixed
   `cafleet member ping` (a no-op success against a pending placement). One
   ping per quiet period — a pane that changed only by reacting to the ping is
   the same quiet period, not a new one. Observed `working` or
   `awaiting_user`, or materially changed quiet content, ends the quiet
   period: the baseline re-seeds and the member is re-armed. It never pings
   the Director or itself, never uses `member prompt` or
   `message broadcast`, and takes no other pane action.
5. **Messages the Director per event**: when an event needs Director
   attention — a member still unchanged at the next stall-check wake after its
   ping, a ping delivery failure, or a capture failure — the monitoring member
   sends a plain `cafleet message send` to the Director about it; with no such
   event, it sends nothing. Each send fires immediately regardless of the
   Director's pane state — the inline preview's `Esc` safeguard makes it safe
   on any pane, and it doubles as the Director's facilitation cue. Each
   member's situation is said once per quiet period, not on every subsequent
   wake.

An idle member (`finished`) and a stalled one (`stall_candidate`) are treated
identically up to the ping; whether assigned work remains is the Director's
judgment, prompted by the monitoring member's plain message.

## Cadence and tick precision {#cadence-and-tick-precision}

| Knob | Default | Set by |
|---|---|---|
| Ordinary member ping interval | `720s` | `monitor_config.interval_seconds` (each member's row) |
| Stall-check interval | `240s` | `monitor_stall_interval` / `CAFLEET_MONITOR_STALL_INTERVAL` |
| Unacked-delivery annotation threshold | the member's own ping interval | `monitor_config.interval_seconds` (no independent trigger) |
| Scan tick | `5s` | `monitor start --tick N` (per run) |

The monitor scans once per **tick** and a member only comes due at a tick
boundary, so the tick is the floor on interval precision. Per-member intervals
and the `enabled` flag are editable via the admin WebUI.
`last_stall_check_at` is persisted separately from `last_ping_at`, so an
immediate monitor-loop restart honors the remaining stall cadence. A failed
watcher wake commits neither dispatch timestamp.
`CAFLEET_MONITOR_STALL_INTERVAL=0` disables stall-check wakes and therefore
monitor pings. Unacked staleness only controls whether the hint is appended to
a normal due row.

The effective wake cadence is the shorter of the two triggers across the
watched set. A fleet holding at least one ordinary member therefore wakes every
240 s under the default stall-check interval, and every 720 s when
`CAFLEET_MONITOR_STALL_INTERVAL=0` leaves only the interval trigger. A fleet
holding just the Director and the monitoring member has an empty watched set:
the loop claims the runtime slot and heartbeats every tick, but no wake fires
until the first ordinary member is spawned.

## Single-instance and liveness

Exactly one monitor may run per fleet. The `monitor_runtime` DB row is the
single authority for both the single-instance claim (one SQLite write
transaction, so two concurrent `monitor start` calls cannot both win) and
liveness: the running loop rewrites `last_tick_at` every tick, so a monitor
that died silently reads as stale. Both the per-tick heartbeat and the on-exit
clear are ownership-checked — a displaced monitor's next heartbeat matches
zero rows and it self-terminates.

## Lifecycle

The monitoring member is spawned **first-in**: it launches the loop as a
background task, confirms the startup line the loop prints immediately after
claiming the runtime row — `monitor loop started (fleet <fleet_id>, tick
<tick>s, pid <pid>)` — and its `ready: monitor live` handshake gates spawning
ordinary members. A loop task that exits instead (runtime-claim conflict, dead
fleet) is reported to the Director as a failed start. Teardown is
**first-out**: the Director deletes the monitoring member before the ordinary
members, and the loop terminates with its pane; a runtime row the pane kill
leaves behind is removed by `fleet delete`. `fleet delete` alone also ends a
still-running loop — its next tick sees the soft-deleted fleet and
self-terminates.

Per-member schedule state (`interval_seconds`, `enabled`, `last_ping_at`,
`last_stall_check_at`) is persisted in `monitor_config`, so the stall-check
cadence survives restarts. Disable/dead/pending-pane cleanup clears
`last_stall_check_at`; soft deregistration explicitly deletes the row. See
[Data model](../spec/data-model.md) for the backing tables and
[CLI options](../spec/cli-options.md#cafleet-monitor) for the command surface.
