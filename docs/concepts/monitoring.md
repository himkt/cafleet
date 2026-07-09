---
icon: lucide/heart-pulse
---

# Monitoring

`cafleet monitor` is a fleet-scoped foreground loop — `scan → wake → sleep` —
that the fleet's dedicated **monitoring member** runs as a **background task** in
its own pane. It supplies the **heartbeat** a Director needs to supervise its
team: a plain loop, not agent reasoning, that scans the **watched set** (the root
Director and every ordinary member, each on its own interval) and, when at least
one watched agent is due, wakes the monitoring member once by keystroking into
its tmux pane. While the loop runs it spends **no model tokens**, and because it
is just a backgrounded command it works identically on **any** backend (`claude`,
`codex`, `opencode`). `cafleet monitor start` runs the loop in-process; the
monitoring member owns its lifetime — there is no detached subprocess and no
`monitor stop` (stop the background task, or delete the monitoring member, to
stop it). One monitor per fleet, and one monitoring member per fleet.

The monitor loop's **wake nudge** keystrokes only the monitoring member's own
pane — which is never parked on a permission prompt — so it does **not** lead
with `Esc`, unlike the message-delivery preview and `cafleet member ping`.
The `Esc`-safeguard mechanics live in [tmux push](tmux-push.md).

## Heartbeat vs facilitation

The monitor decides only the *when* — which watched agents are due and a single
keystroke to wake the monitoring member. It MUST NOT poll, ACK, dispatch work,
health-check, or escalate; those require agent judgment and stay the Director's
job, defined in the `/cafleet` skill's `reference/supervision.md` (the *what*).

| Layer | Owns | Lives in |
|---|---|---|
| Heartbeat (the *when*) | which watched agents are due; the wake keystroke into the monitoring member | the `cafleet monitor` loop |
| Facilitation (the *what*) | poll → ACK → dispatch → health-check → escalate | the Director, per `/cafleet` `reference/supervision.md` |

The loop's only keystroke is a *wake nudge* (the `send_wake_trigger` helper) into
the monitoring member's own pane — a single-line instruction to run its
capture-classify-reengage routine now. The loop fires that wake when **≥ 1
watched agent** (the Director or an ordinary member) has come due, and the nudge
**names** each due agent as `<role> <id> (<name>) [<reasons>]` (role `director` or
`member`; reasons drawn from `interval`, `status:done`, `stall-check`) plus the
Director id as the standing inspect-and-re-engage target, so the monitoring member
inspects exactly those named panes plus the Director this wake. The loop runs
inside the monitoring member's own pane, so this wake lands in that same pane's
foreground (see [The monitoring member](#the-monitoring-member)).
The wake nudge does not lead with `Esc` — `send_wake_trigger` does not pass
`esc_first`, because the monitoring member's pane is never on a permission prompt.

The Director receives **no** keystroke from the loop. It is re-engaged only on
demand: by the monitoring member's nudge — `cafleet member nudge`,
which persists an ACKable broker task carrying the summary **and** fires the
hardened, `Esc`-safeguarded inline preview — and by the broker's inline-preview
keystroke on every inbound `cafleet message send`.

## The watched set

Each tick, the monitor evaluates its enrolled, active agents — the **watched
set** — and flags the ones whose interval has elapsed. Enrollment covers the
**root Director** (default **180 s**) and **every ordinary member** (default
**720 s**), each carrying its own per-agent interval. The dedicated monitoring
member is **not** enrolled: it is the *watcher*, not a watched agent (see
[The monitoring member](#the-monitoring-member)). The write-only Administrator and
placementless agents are never enrolled.

When ≥ 1 watched agent is due, the loop wakes the monitoring member **once** and
stamps `last_ping_at = now` on each agent due by `interval` or `status:done` (a
stall-check-only due agent is excluded from that stamp — its `last_ping_at` is
untouched and its stall baseline is committed instead; see [Cadence and tick
precision](#cadence-and-tick-precision)). That stamp advances the pinged agent's
cadence, so a just-flagged agent is not due again on the next tick — this prevents
a wake-storm while the watcher is still working. `last_ping_at` means "the last
time the monitor dispatched a check for this agent."

The loop **never keystrokes a watched pane** — its only keystroke is the wake
nudge into the watcher's own pane.

`pending_count` (the count of an agent's un-acked inbox items) is still computed
and shown in `monitor status`, but it does not gate anything — it is purely
informational.

A watched agent is flagged only when it is enabled, its pane is alive, and its
interval has elapsed since its last wake-dispatch; otherwise it is skipped.

Each wake-dispatch is logged to the monitor's stdout as
`<iso-ts> due agent <id> (<name>) [<reasons>] -> wake monitor`, one line per due
agent (the `[<reasons>]` suffix lists that agent's joined wake reasons).
Because `cafleet monitor start` runs in the foreground of the monitoring member's
background task, that task's output shows live heartbeat activity. If there is
**no** monitoring member (or its pane is dead), the loop records nothing and
simply continues — there is no one to wake.

### Native agent-state due trigger (herdr only)

On the **herdr** backend, a watched agent is due when its interval elapsed **or**
its native agent status transitions into `done` — the sole wake-on-status state,
`_WAKE_ON_STATUS = ("done",)`. This native trigger **augments, never replaces**,
the interval trigger, and it is isolated to the monitor loop: `monitor_tick` keeps
computing interval-due-ness only, with no knowledge of native status.

herdr natively tracks each agent's lifecycle state
(`working`/`blocked`/`done`/`idle`/`unknown`) — a capability the tmux backend
does not have (see [Multiplexer backends](multiplexer-backends.md)). When the
resolved backend implements the optional `AgentStateAware` capability, each tick
the loop point-reads the native status of every **enabled** watched agent whose
pane is alive, comparing it against an in-memory `dict[agent_id, last_status]` it
owns (a monitor-disabled agent is skipped on the native path too, matching the
interval path).

A **transition into `done`** flags that agent due, tagged with a `status:done`
wake-reason label, and its set is unioned with the interval-due (and stall-check)
set to decide the wake. Because the comparison is against the last-seen status, a
single `done` episode wakes the watcher only once.

A **transition into `blocked` is recorded but never wakes the watcher.** `blocked`
means the agent is awaiting a user answer, and the monitoring member's only
actuation is nudging the Director. Waking the watcher for a `blocked` agent has
exactly one correct outcome — capture the pane, classify it `awaiting_user`, and
do nothing — so the wake is pure token cost plus a nonzero chance the watcher
misjudges and nudges the pane, whose keystroke leads with `Esc` and cancels the
pending prompt (an `AskUserQuestion` box, when the blocked agent is the Director).
The `blocked` status is still written into `last_status` so the episode is tracked
and a later `blocked → working` recovery is detected; it simply never flags a
wake. `done` is therefore the only native state that can emit a wake-reason tag,
and its worst case is a `finished` report the Director then judges — a
non-destructive path.

On the **tmux** backend the capability is absent, so this branch never runs and
agents come due by interval and stall-check only. No DB column backs the native
status; the last-seen state lives only in the running loop's memory.

## The monitoring member

The monitoring member is a single, dedicated coding-agent member — spawned with
`cafleet member create --role monitor` (the Director passes `--model haiku`) —
that owns the heartbeat and applies LLM judgment to the watched agents' state.
When `--coding-agent` is omitted it inherits the spawning Director's backend
(see [Coding agents](coding-agents.md)). It
is identified by `agent_card_json.cafleet.kind == "monitoring-member"` (the same
`kind`-marker pattern the built-in Administrator uses; no new SQL column) and is
located by the broker's `find_monitoring_member` lookup (the kind marker joined to
its placement) — **not** by a `monitor_config` row, since the monitoring member is
never enrolled. It is the **one** process in the fleet that runs
`cafleet monitor start`. There is at most one monitoring member per fleet; a
second `--role monitor` spawn is rejected.

On each wake the loop keystrokes a nudge **naming the freshly-due agents** into
its own pane, and the monitoring member runs its routine — staying within two
read/act commands, read-only `cafleet member capture` and
`cafleet member nudge`:

1. **Read the due agents named in the wake nudge** — each rendered as
   `<role> <id> (<name>) [<reasons>]`, the reasons drawn from `interval`,
   `status:done`, and `stall-check`. Those agents, plus the Director, are who you
   inspect this wake. (`cafleet monitor status` is available as optional context —
   e.g. to read intervals or pending counts — but it is **not** the source of the
   due set; the nudge's named list is authoritative.)
2. **Capture each named due agent's pane** via `cafleet member capture
   --member-id <id>` (read-only; `member capture` accepts any in-fleet agent with a
   placement, the root Director included), always also capturing the Director, and
   **classify each pane from its capture content only** into one of five states,
   applied in precedence order — the first match wins and stops:

   | State | Evidence | Monitor action |
   |---|---|---|
   | `awaiting_user` | The capture shows an unanswered question or permission prompt | **None** — never re-engage |
   | `unknown` | The pane is dead/unreadable, or this is a stall-check wake and no previous stall-check capture of this pane is remembered | **None** — fail-safe |
   | `finished` | A completed turn at an empty input prompt, no pending question | Report to the Director |
   | `stalled` | A stall-check wake whose capture is identical to this pane's previous stall-check capture | Report to the Director |
   | `working` | In-flight work matched by no earlier rule | None |

   Native `agent_status` is **never** classification evidence — the rubric is
   capture-content only, and byte-identical across the tmux and herdr backends.
   When a capture cannot distinguish `awaiting_user` from `finished`, classify
   **`awaiting_user`**: the costs are asymmetric — a missed `finished` delays a
   nudge by one cycle, but a misjudged `awaiting_user` destroys the user's pending
   prompt.
3. **Compare only against the previous stall-check capture.** For an agent tagged
   `stall-check`, compare its capture against the single capture you remember from
   that pane's last stall-check wake; with no such baseline, classify `unknown`.
   Then — unconditionally, whatever you classified, including `awaiting_user` and
   `unknown` — replace that pane's remembered baseline with the capture you just
   took. A capture taken on an `interval` or `status:done` wake is read, classified,
   and discarded; it never becomes a baseline. You remember exactly one baseline
   capture per pane, from its last stall-check wake.
4. **Re-engage the Director** via `cafleet member nudge
   --agent-id <monitoring-member-id> --member-id <director-id> --text "..."` when a
   due agent is `stalled` or `finished`, or the Director itself is `finished` with
   un-ACKed inbox — naming what needs attention. The Director alone judges whether a
   `finished` agent still owes assigned work; the monitoring member cannot see the
   dispatch ledger and never makes that call. **But when the Director's own pane is
   `awaiting_user`, send nothing this wake — no matter how many due agents are
   `stalled` or `finished`.** Re-engaging an `awaiting_user` pane is barred
   outright, and that bar outranks every nudge trigger: `member nudge` fires an
   inline preview whose keystroke leads with `Esc`, and that `Esc` exists to stop
   the trailing `Enter` from blindly *confirming* a prompt — the same keystroke
   would cancel a Director's pending `AskUserQuestion`. The suppressed report is not
   buffered and not lost: the agent stays due on its interval and stall-check
   cadences and re-surfaces, unchanged, on its next wake.

The monitoring member's *observation* spans the Director **and** every
freshly-due member, but its *actuation* is **Director-only**: it never
keystrokes ordinary members with task instructions — all member-driving routes
back through the Director, who owns the whole task.

## Cadence and tick precision

| Knob | Default | Set by |
|---|---|---|
| Root Director ping interval | `180s` | `monitor_config.interval_seconds` (the Director's row) |
| Ordinary member ping interval | `720s` | `monitor_config.interval_seconds` (each member's row) |
| Stall-check interval | `240s` | `monitor_stall_interval` / `CAFLEET_MONITOR_STALL_INTERVAL` (`0` disables) |
| Scan tick | `5s` | `monitor start --tick N` (per run) |

The monitor scans once per **tick** and flags each watched agent whose
**interval** has elapsed since its last wake-dispatch. Because an agent only comes
due at a tick boundary, **the tick is the floor on interval precision**: an
interval that is not a multiple of the tick snaps up to the next tick boundary
(e.g. a 7 s interval under a 5 s tick fires at ~10 s). Set the tick smaller than
the smallest interval you care about. Each interval is editable per agent
(`cafleet monitor config` or the admin WebUI), so the 180 s / 720 s defaults are
just the enrollment seeds.

**Stall detection runs on its own cadence**, independent of the 180 s / 720 s
ping intervals. Each watched agent is additionally **stall-check due** every
`monitor_stall_interval` seconds (default **240 s**, from
`CAFLEET_MONITOR_STALL_INTERVAL`; `0` disables stall detection entirely). A
stall-check wake tags the agent `stall-check`, telling the monitoring member to
compare its capture against that pane's previous stall-check baseline — two
unchanged observations one interval apart classify it `stalled`, calling a hang in
~8 minutes rather than the ~24 minutes it would take if bullet 2 rode the 720 s
member interval. The stall cadence is tracked process-locally (never persisted, no
schema change); an agent not yet seen is stall-check due on the first tick,
mirroring the interval path's `last_ping_at is None → due` convention, which seeds
each pane's baseline one interval early. A stall-check-only wake does **not**
advance the 720 s ping cadence, so the two cadences stay independent.

## Single-instance and liveness

Exactly one monitor may run per fleet. The **`monitor_runtime` DB row is the
single authority** for both single-instance coordination and `status` liveness
— there is no PID file and no state directory. The single-instance claim runs in
one SQLite write transaction, so two concurrent `monitor start` calls cannot
both win.

**Liveness is read from the DB heartbeat**, not from the process table: the
running monitor rewrites `last_tick_at` every tick, so a monitor that died
silently is detected as stale (`now - last_tick_at` exceeds the stale window)
even though nothing cleaned up after it. `os.kill(pid, 0)` is a corroborating
signal, not the authority.

Because a stale heartbeat is treated as dead, a fresh `start` may reclaim the
slot from a momentarily-wedged monitor. To keep two live monitors from both
pinging, the slot has exactly one owner (the pid that claimed it) and both the
per-tick heartbeat and the on-exit clear are **ownership-checked**: a displaced
monitor's next heartbeat matches zero rows, so it self-terminates without
pinging and without wiping the winner's row.

## Lifecycle

```mermaid
%%{init: {'theme': 'default', 'themeVariables': {'fontSize': '16px'}}}%%
flowchart LR
    Start["monitor start<br/>(monitoring member's background task)"] --> Claim["claim runtime row"]
    Claim --> Tick["every tick:<br/>heartbeat (STOP if slot lost) → scan watched set → wake monitor if any due"]
    Tick --> Tick
    Tick --> Stop["stop the task /<br/>delete the monitoring member /<br/>fleet delete"]
    Stop --> Clear["clear runtime row"]
    Tick -. wake nudge .-> PaneMon["monitoring member pane"]
```

- **Spawned first.** The monitoring member is the **first** `member create` in
  the fleet (first-in). After it boots it launches `cafleet monitor start --fleet-id N` as a **background task** in its own pane, confirms with `cafleet monitor
  status`, and reports `ready: monitor live` to the Director. Receipt of that
  handshake is the gate for spawning ordinary members — this is the only
  `monitor start` in the fleet; the Director no longer runs it. The loop inherits
  the monitoring member's pane environment (`$TMUX`, `$CAFLEET_DATABASE_URL`) and
  fails fast on startup if it cannot reach a tmux session.
- **Run**: each tick first writes the ownership-checked heartbeat (a zero-row
  update means the slot was reclaimed → the loop self-terminates with `STOP`),
  then scans the watched set (Director + members), wakes the monitoring member
  with the wake nudge when ≥ 1 watched agent is due, and stamps the due agents'
  `last_ping_at`.
- **Stop (first-out).** Teardown stops the monitor **before** the monitoring
  member's pane is killed: the Director messages the monitoring member to stop
  its `monitor start` background task (the task-stop delivers SIGTERM/SIGINT, so
  the loop runs its `finally` and clears the runtime row), the monitoring member
  confirms, and only then does the Director `cafleet member delete` it — first,
  before the ordinary members. A hard pane-kill instead leaves the heartbeat to
  go stale, after which `status` reports stopped (the accepted degraded path).
  There is no `monitor stop` command. `fleet delete` needs no stop step — a
  running loop's next tick sees the soft-deleted fleet and self-terminates, and
  `delete_fleet` removes the `monitor_runtime` + `monitor_config` rows.

Per-agent schedule (`interval_seconds`, `enabled`) is persisted, so cadence
resumes from `last_ping_at` across a restart. The schedule is editable from both
the CLI (`cafleet monitor config`) and the admin WebUI at parity; launching and
stopping the loop is CLI-only by nature (it is the monitoring member's background
task).

## Enrollment and schema

Two tables back the monitor. Both reuse a parent id as a 1:1 INTEGER primary key
(no fresh sequence), and both are cleaned explicitly on teardown.

- **`monitor_config`** — one row per **enrolled** agent, holding its
  `interval_seconds`, `last_ping_at`, and `enabled` flag. Enrollment covers the
  **root Director** (180 s, enrolled in `create_fleet` after its placement is
  added) and **every ordinary member** (720 s, enrolled in `register_agent` when
  it has a placement and is not the monitoring member). The dedicated monitoring
  member is **not** enrolled (it is the watcher, located by kind via
  `find_monitoring_member`); neither are the write-only Administrator or
  placementless agents. `is_director` is *derived* at scan time
  (`agent_id == fleets.director_agent_id`) for `monitor status` role labeling; it
  is not denormalized.
- **`monitor_runtime`** — one row per fleet, holding the running loop's `pid`,
  `started_at`, `last_tick_at` heartbeat, and `tick_seconds`. "No monitor" is
  modeled cleanly as "no row".

See [Data model](../spec/data-model.md) for the full column definitions and the
[CLI options](../spec/cli-options.md#cafleet-monitor) page for the
`cafleet monitor` command surface.
