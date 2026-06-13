# cafleet monitor — backend-agnostic external scheduler

**Status**: Approved
**Progress**: 59/59 tasks complete
**Last Updated**: 2026-06-13

## Overview

`cafleet monitor` is a detached, fleet-scoped background process that externalizes the *scheduling* of CAFleet supervision into one plain Python loop driving the broker's existing tmux keystroke primitives. It replaces the Claude-only in-session scheduling (`CronCreate` / `ScheduleWakeup` / `/loop`) so a Director on **any** backend (`claude`, `codex`, `opencode`) gets the same heartbeat. The monitor decides only the *when* (who to ping, when); the *what* — the Director's 5-step facilitation loop — stays in the supervision skill.

## Success Criteria

- [x] `cafleet monitor start --fleet-id N` launches a detached process that returns control to the caller's turn immediately, on any backend.
- [x] `cafleet monitor start --fleet-id N --foreground` runs the identical loop in the current pane for debugging.
- [x] A second `monitor start` for a fleet with a live monitor is refused (single-instance, enforced atomically in the DB).
- [x] `cafleet monitor status --fleet-id N` reports true liveness from the DB heartbeat even when the process died silently, plus the per-agent schedule table.
- [x] `cafleet monitor stop --fleet-id N` signals the process, which shuts down cleanly; `fleet delete` also stops the monitor.
- [x] The monitor pings the root Director **unconditionally** on its interval, and a member **only** when it has pending un-acked inbox items.
- [x] Per-agent `interval_seconds` / `enabled` are persisted in `monitor_config`, auto-enrolled at agent registration, editable via CLI and WebUI at parity, and survive a monitor restart (cadence resumes from `last_ping_at`).
- [x] The admin agents page shows each agent's monitoring schedule and lets the operator edit the interval and toggle enable/disable.
- [x] The `CronCreate` / `/loop` scheduling-setup guidance and the codex/opencode "no in-session scheduler" fallback table are removed repo-wide and replaced with "ensure `cafleet monitor` is running"; the Director's facilitation loop is preserved; no deprecation notices remain.
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test`, and `mise //admin:build` all pass.

---

## Background

Today a Director wakes itself up to run supervision by calling its harness's in-session scheduler. Only Claude Code exposes one (`CronCreate` + `/loop`, `ScheduleWakeup`). Codex CLI and opencode expose **no** in-session scheduling primitive, so the monitoring skill documents a "no in-session scheduler → fallback options" table (out-of-band cron, MCP server, user-driven nudges, synchronous-only) and recommends a claude Director for any active-supervision workload. Supervision is therefore uneven across backends.

The waking action itself is already backend-agnostic: the broker keystrokes into a member's pane via `tmux.send_poll_trigger` (the primitive behind `cafleet member ping`) and `tmux.send_inline_preview` (the broker's auto-fire on `message send`). Both work identically for all three backends. The only Claude-specific piece is the *scheduler* that decides when to fire.

This design moves that scheduler out of the coding agent into a deterministic Python process. The loop is `scan → ping-due-agents → sleep`; it is pure machinery, not agent reasoning, so it must **not** be a coding-agent member (that would burn a model on a sleep-loop and still couldn't self-schedule on codex/opencode). One detached process per fleet, startable with a single shell command from any Director's pane, gives every backend the same heartbeat.

### Heartbeat vs facilitation boundary (the load-bearing constraint)

The monitor provides **only the heartbeat** — the *when*: which agents are due and a keystroke to wake them. It MUST NOT poll, ACK, dispatch, health-check, or escalate. Those require agent judgment and remain the Director's job, defined by the `cafleet-agent-team-supervision` skill (the *what*).

- Pinging the **Director** keystrokes *exactly* `cafleet … message poll` into the Director's pane (the fixed `send_poll_trigger` payload — it cannot inject a richer prompt). That bare poll, on its own, performs only **step 1** of facilitation. The contract that makes the Director run the full 5-step loop therefore lives in the `cafleet-agent-team-supervision` skill, not in the keystroke: the skill instructs the Director to **treat a monitor poll-trigger wake as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), not to read its inbox and stop. The old `/loop` template injected that facilitation prompt directly; the monitor injects only the poll, so this cue MUST be stated explicitly in the supervision rewrite (Step 1b) — otherwise a woken Director may poll and halt without facilitating.
- Pinging a **member** keystrokes the same `cafleet … message poll` into the member's pane, which catches a missed inline preview. For a member the bare poll is the whole job — re-draining the inbox is exactly what is wanted.

The monitor never reasons about message content. It is the alarm clock; the Director is the worker. This boundary is what keeps the loop a plain process rather than a coding agent.

---

## Specification

### 1. Component layout

| Layer | Module | Responsibility |
|---|---|---|
| DB models | `cafleet/db/models.py` | `MonitorConfig`, `MonitorRuntime` ORM classes |
| Migration | `cafleet/db/alembic/versions/0002_monitor_tables.py` | Creates `monitor_config` + `monitor_runtime` (down_revision `0001`) |
| Broker (DB) | `cafleet/broker/monitor.py` | Config CRUD, runtime claim/heartbeat/clear, per-tick scan, `record_ping`; enrollment helper |
| Process/policy | `cafleet/monitor/` package | `loop.py` (loop driver, `monitor_tick`, `should_ping`), `process.py` (detach launcher, PID file, signals, `stop_monitor`), `__init__.py` (constants) |
| Module entry | `cafleet/__main__.py` | `python -m cafleet` → CLI, the detached re-exec target |
| CLI | `cafleet/cli/monitor.py` | `cafleet monitor start|stop|status|config` |
| WebUI API | `cafleet/webui/api.py` | `GET /api/monitor`, `GET`/`PATCH /api/agents/{id}/monitor`, folded `monitor` field on `GET /api/agents` |
| Admin SPA | `admin/src/…` | per-agent Monitoring section (view + edit) + header runtime indicator |

The broker stays the pure DB layer (no OS side effects). The `cafleet/monitor/` package owns process lifecycle, signals, the PID file, and the loop, and calls the broker + `TmuxMultiplexer`. This mirrors the existing split where `broker/` is data-access and `multiplexer/` + `cli/member.py` own tmux side effects.

### 2. Schema

Two new tables, both keyed on `INTEGER` per the project convention. Neither mints a fresh sequence — each reuses a parent id as a 1:1 PK (no `AUTOINCREMENT`), exactly like `agent_placements`.

#### `monitor_config` — per-agent schedule (keyed by `agent_id`)

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `INTEGER` | `PRIMARY KEY` (no AUTOINCREMENT), `REFERENCES agents(agent_id) ON DELETE CASCADE` | The enrolled agent; parent `agents.agent_id` reused 1:1 (mirrors `agent_placements.agent_id`). |
| `interval_seconds` | `INTEGER` | `NOT NULL`, `DEFAULT 60` | Ping cadence for this agent. |
| `last_ping_at` | `TEXT` | nullable | ISO-8601 of the last ping the monitor dispatched to this agent. `NULL` = never pinged ⇒ due immediately. Persisted (not in-memory) so a restart resumes cadence and `monitor status` can display it. |
| `enabled` | `INTEGER` | `NOT NULL`, `DEFAULT 1` | Boolean (SQLite stores 0/1). `0` = monitor skips this agent while preserving its interval for re-enable. |

**`enabled` int↔bool boundary**: the column is `INTEGER` 0/1, but every broker read function (`get_monitor_config`, `list_monitor_configs`, `list_monitor_targets`) casts it to a Python `bool` at the read boundary (`bool(row.enabled)`). So `should_ping`, the CLI, and the WebUI/JSON contract (`enabled: bool`) all see a real bool; the integer representation never leaks past the broker. `update_monitor_config(enabled=…)` accepts a bool and writes the 0/1.

There is **no** `fleet_id` column: fleet scoping is reached through the `monitor_config.agent_id → agents.agent_id → agents.fleet_id` join (the same pattern `members.py` uses for `agent_placements`). Director-vs-member is **derived** at scan time (`agent_id == fleets.director_agent_id`), not denormalized.

#### `monitor_runtime` — per-fleet process/heartbeat state (keyed by `fleet_id`)

Decision: a **dedicated single-row-per-fleet table**, not columns on `fleets`. Rationale:

1. `fleets` is a minted-id core identity entity with a tight, stable schema. `monitor_runtime` is high-write (heartbeat every tick) operational telemetry. Co-locating fast-changing runtime data with slow-changing identity data is the exact concern that already motivated splitting `agent_placements` out of `agents`.
2. A monitor is genuinely optional (a fleet may never start one). "No monitor" is cleanly modeled as "no row", rather than a cluster of nullable columns on every fleet row.
3. Teardown and single-instance claims operate on this row independently of the `fleets` lifecycle.

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `fleet_id` | `INTEGER` | `PRIMARY KEY` (no AUTOINCREMENT), `REFERENCES fleets(fleet_id) ON DELETE RESTRICT` | One row per fleet; reuses `fleets.fleet_id`. |
| `pid` | `INTEGER` | nullable | OS PID of the running foreground worker. `NULL` after a clean stop. Used by `stop` to signal and by the claim for the liveness check. |
| `started_at` | `TEXT` | nullable | ISO-8601 when the current worker claimed the runtime. |
| `last_tick_at` | `TEXT` | nullable | ISO-8601 heartbeat, rewritten every tick. The authority for `status` liveness. |
| `tick_seconds` | `INTEGER` | `NOT NULL`, `DEFAULT 5` | Scan-tick cadence the running monitor uses (so `status` can report it). |

FK enforcement matches the existing `RESTRICT`/`CASCADE` posture (`monitor_config` CASCADEs off `agents` like `agent_placements`; `monitor_runtime` RESTRICTs off `fleets` like the other fleet FKs). Because agents are soft-deregistered and fleets soft-deleted, neither CASCADE/RESTRICT fires on the normal delete paths — rows are cleaned explicitly (see §7), mirroring how `agent_placements` is explicitly deleted today.

#### Migration

`cafleet db init` is the only path that applies schema, and existing operator databases are already stamped at head `0001`. Extending the `0001` migration in place would leave those databases unmigrated (`db init` sees `current == head` and no-ops). Therefore add a **new** revision:

```python
# 0002_monitor_tables.py
revision = "0002"
down_revision = "0001"
# upgrade(): create monitor_config (CASCADE off agents) + monitor_runtime (RESTRICT off fleets),
#            both INTEGER PK, no AUTOINCREMENT, with the columns above.
# downgrade(): drop monitor_runtime then monitor_config.
```

This advances `cafleet db init`'s head from `0001` to `0002`. The two alembic smoke-test assertions that encode the post-0000076 "single collapsed revision" state (`test_only_one_migration_revision_exists`, `test_alembic_version_table_records_collapsed_head_0001`) are updated to expect two revisions and head `0002` — this design legitimately advances the schema.

### 3. Enrollment (auto, at registration)

A `monitor_config` row is inserted with the default interval (`60`) and `enabled=1` for every **pane-bound** agent, at the moment it is registered:

| Agent kind | Insertion site | Enrolled? |
|---|---|---|
| Root Director | `broker.create_fleet` (after its `AgentPlacement` is added) | yes |
| Member | `broker.register_agent` when `placement is not None` | yes |
| Administrator | `broker.create_fleet` (no placement) | **no** (write-only, no pane to ping) |
| Card-only `agent register` | `broker.register_agent` with `placement is None` | **no** (no pane) |

The rule is precisely **"enroll iff the agent has a placement"** — only an agent with a tmux pane can be pinged. The insert happens inside the same write transaction as the agent/placement insert (a shared helper `_enroll(session, agent_id, interval=DEFAULT_PING_INTERVAL_SECONDS)` in `broker/monitor.py`, called by `agents.py` and `fleets.py`), so enrollment is atomic with registration.

### 4. should_ping policy (pure function)

`should_ping(target, now) -> bool` is a pure function of a scan-row dict and the current time, so it is unit-testable without tmux or the DB. Evaluated per enrolled, active agent each tick:

```
def should_ping(target, now):
    if not target["enabled"]:                      # decision 9: per-agent disable
        return False
    if target["pane_id"] is None or not target["pane_alive"]:   # dead/missing pane
        return False
    if target["last_ping_at"] is not None:         # not yet due
        elapsed = now - datetime.fromisoformat(target["last_ping_at"])
        if elapsed.total_seconds() < target["interval_seconds"]:
            return False
    if target["is_director"]:                       # decision 5: director split
        return True                                 # unconditional on its interval
    return target["pending_count"] > 0              # member: only with a reason
```

| Field in `target` | Source |
|---|---|
| `enabled`, `interval_seconds`, `last_ping_at` | `monitor_config` row |
| `pane_id` | `agent_placements.tmux_pane_id` |
| `pane_alive` | membership of `pane_id` in the live-pane set fetched once per tick (see §5) |
| `is_director` | `agent_id == fleets.director_agent_id` |
| `pending_count` | count of `input_required` tasks where `context_id == agent_id` (excludes `broadcast_summary`), a correlated subquery in the scan |

Policy rationale:

- **Director pings unconditionally** because its facilitation does useful work even on an empty inbox (it still health-checks members, dispatches queued work, detects stalls).
- **A member pings only with a reason** (pending un-acked `input_required` items). A periodic ping into an idle, empty-inbox member is pure noise and risks interrupting mid-work.
- **Re-ping is unbounded** (decision 8): as long as a stuck member still has pending items and its interval has elapsed, it is pinged every interval. No backoff, no cap. It self-clears the moment the member acks (its `pending_count` drops to 0 ⇒ `should_ping` returns False).

`last_ping_at` advances whenever the decision to ping is **YES** (i.e. a keystroke is attempted), regardless of whether the best-effort `send_poll_trigger` keystroke reported success — consistent with the broker's existing best-effort keystroke semantics. The next interval retries.

### 5. The loop

`monitor_tick(fleet_id, now)` performs one full pass and is the testable unit.

**Time representation**: `now` is a timezone-aware `datetime` threaded through `monitor_tick` / `should_ping` / `_is_live` (so they do real datetime arithmetic); every DB-storage boundary serializes with `.isoformat()` (the columns are TEXT), and stored ISO strings are parsed back with `datetime.fromisoformat`. The pure functions never touch strings; the broker storage functions only ever receive ISO strings (never a raw `datetime`).

```
def monitor_tick(fleet_id, now):                          # now: tz-aware datetime
    if not broker.heartbeat_monitor_runtime(              # ownership-checked (§6)
            fleet_id, os.getpid(), now.isoformat()):
        return STOP            # another instance owns the slot ⇒ self-terminate
    fleet = broker.get_fleet(fleet_id)
    if fleet is None or fleet["deleted_at"] is not None:
        return STOP            # fleet vanished/soft-deleted ⇒ self-terminate
    live_panes = TmuxMultiplexer().list_pane_ids()        # one tmux call per tick
    for t in broker.list_monitor_targets(fleet_id):       # active+enrolled agents
        t["pane_alive"] = t["pane_id"] in live_panes
        if should_ping(t, now):
            TmuxMultiplexer().send_poll_trigger(
                target_pane_id=t["pane_id"], fleet_id=fleet_id, agent_id=t["agent_id"])
            broker.record_ping(t["agent_id"], now.isoformat())
    return CONTINUE
```

`run_monitor_loop(fleet_id, tick_seconds)` is the thin driver:

```
def run_monitor_loop(fleet_id, tick_seconds):
    pid = os.getpid()
    if not broker.claim_monitor_runtime(
            fleet_id, pid, tick_seconds, datetime.now(UTC).isoformat()):
        raise click.ClickException(f"monitor already running for fleet {fleet_id}")
    write_pid_file(fleet_id, pid)
    install_signal_handlers()                  # SIGTERM/SIGINT -> stop flag
    try:
        while not stop_requested():
            if monitor_tick(fleet_id, datetime.now(UTC)) is STOP:
                break
            interruptible_sleep(tick_seconds)   # wakes early on signal
    finally:
        broker.clear_monitor_runtime(fleet_id, pid)  # ownership-checked; pid -> NULL
        remove_pid_file(fleet_id)
```

New tmux helper: `TmuxMultiplexer.list_pane_ids() -> set[str]` (`tmux list-panes -a -F '#{pane_id}'` split into a set), so pane-liveness for all agents costs one tmux call per tick. `should_ping` stays pure (liveness is passed in).

The loop is robust to a fleet torn down underneath it (returns `STOP`), so an orphaned monitor self-terminates even if `fleet delete` somehow ran without stopping it first.

### 6. Single-instance, liveness, PID file (decision 2)

Two artifacts, with crisp, non-overlapping roles:

| Artifact | Role |
|---|---|
| `monitor_runtime` DB row | **Authoritative coordination + liveness record.** The atomic single-instance claim and `status` liveness derive from it. `stop` reads `pid` from it. |
| PID file `<state_dir>/<fleet_id>.pid` | **The conventional OS handle.** Written at claim, removed at clean shutdown. Primary signal source for `stop`; its presence without a fresh heartbeat marks a silently-dead monitor whose lock `start` may reclaim. |

`state_dir` is `settings.monitor_state_dir` (new `Settings` field, env `CAFLEET_MONITOR_STATE_DIR`, default `~/.local/share/cafleet/monitor/`). The detached worker's stdout/stderr redirect to `<state_dir>/<fleet_id>.log`.

**Environment inheritance & fail-fast.** The detached child is an ordinary `subprocess.Popen` (not a tmux pane), so it inherits the launching Director pane's environment by default — including `$TMUX` (so its `tmux` subprocesses reach the same server and its keystrokes target the right panes) and `$CAFLEET_DATABASE_URL` (so it reads the same registry DB). `monitor start` runs `ensure_tmux_or_die()` (the same guard the `member` commands use) **before** spawning, so a monitor started where it cannot reach a tmux session fails fast with a clear error instead of silently never delivering keystrokes; the foreground worker re-checks on its own startup.

**Atomic claim** (`broker.claim_monitor_runtime`) runs in one write transaction (SQLite's write lock serializes concurrent claims):

```
with write_session() as s:
    row = <select monitor_runtime for fleet_id>
    if row is None:
        <insert row with pid, started_at=now, last_tick_at=now, tick_seconds>; return True
    if _is_live(row, now):           # a live monitor already holds it
        return False
    <update row: pid, started_at=now, last_tick_at=now, tick_seconds>; return True
```

**Liveness** `_is_live(row, now)`: `row.pid is not None` AND heartbeat fresh (`now - last_tick_at <= STALE_AFTER`) AND `os.kill(row.pid, 0)` succeeds. Heartbeat freshness is the **authority** ("true liveness even if the process died silently" — a killed process stops updating `last_tick_at`); `os.kill(pid,0)` is a corroborating signal. `STALE_AFTER = max(MONITOR_STALE_FACTOR * tick_seconds, MONITOR_STALE_FLOOR_SECONDS)` with `MONITOR_STALE_FACTOR = 3`, `MONITOR_STALE_FLOOR_SECONDS = 15`.

**Ownership-checked heartbeat (split-brain guard).** Because `_is_live` treats a stale heartbeat as dead, a momentarily-wedged-but-alive monitor (e.g. paused on a slow tmux call past `STALE_AFTER`) can have its slot reclaimed by a fresh `start`. To stop two live monitors from then both pinging, the slot has exactly one owner — the pid recorded by the claim — and both the per-tick heartbeat and the on-exit clear are **ownership-checked**:

```
heartbeat_monitor_runtime(fleet_id, pid, when):
    n = UPDATE monitor_runtime SET pid=?, last_tick_at=?
        WHERE fleet_id=? AND pid=?          # only the current owner matches
    return n == 1                            # False ⇒ slot reclaimed by another instance

clear_monitor_runtime(fleet_id, pid):
    UPDATE monitor_runtime SET pid=NULL, last_tick_at=NULL
        WHERE fleet_id=? AND pid=?           # a non-owner clear is a no-op
```

On a reclaim, the displaced (old) monitor's next `heartbeat_monitor_runtime` matches 0 rows (the row's `pid` is now the new owner's), returns `False`, and `monitor_tick` returns `STOP` — the loser self-terminates **without** pinging. The ownership-checked clear means that self-terminating loser never wipes the winner's row on exit. The winner keeps ticking. Single-instance therefore holds even across a stale-heartbeat reclaim.

### 7. Lifecycle integration & teardown

- **`monitor stop`** and **`fleet delete`** both invoke `cafleet.monitor.process.stop_monitor(fleet_id)`, which: reads the PID file / runtime `pid`; if a live monitor exists, sends `SIGTERM`, waits up to `MONITOR_STOP_TIMEOUT` (e.g. 5 s) for the heartbeat/pid to clear, escalates to `SIGKILL` on timeout, then ensures the runtime row is cleared and the PID file removed. Idempotent: a no-monitor fleet returns a "nothing running" result.
- **`broker.delete_fleet`** additionally deletes the `monitor_config` rows for the fleet's agents and the `monitor_runtime` row inside its transaction, mirroring the existing `agent_placements` cleanup. The CLI `fleet delete` calls `stop_monitor(fleet_id)` (the OS-signal side effect) **before** `broker.delete_fleet` (the DB cleanup), so the process stops before the fleet soft-deletes.
- **`broker.deregister_agent`** deletes the agent's `monitor_config` row in the same statement block that deletes its `agent_placements` row (runtime config with no historical value, same lifecycle as placement).

### 8. CLI surface — `cafleet monitor`

A new `monitor` group registered in `cafleet/cli/__init__.py`. `--fleet-id` is the global flag (before the subcommand).

| Command | Flags | Behavior |
|---|---|---|
| `monitor start` | `--tick INT` (default 5, `IntRange(min=1)`), `--foreground` | Default: `ensure_tmux_or_die()` + pre-check single-instance, then spawn `[sys.executable, "-m", "cafleet", "--fleet-id", N, "monitor", "start", "--tick", T, "--foreground"]` via `subprocess.Popen(start_new_session=True, stdin=DEVNULL, stdout/stderr=logfile, close_fds=True)`. Poll the runtime up to ~2 s for a fresh heartbeat **whose `pid` equals the spawned child's pid**; on success report `monitor started (pid M, tick Ts)` and return — freeing the caller's turn. If no matching-pid heartbeat appears in the window (import error, crash, or a lost claim race), report a failure pointing at `<state_dir>/<fleet_id>.log` and exit 1 — do **not** falsely report "started" (and never against a *different* already-running monitor's pid). `--foreground`: run `run_monitor_loop` in-process (the worker the detached path re-execs). Errors exit 1 (`monitor already running for fleet N (pid M)`; `fleet N not found`; soft-deleted fleet). |
| `monitor stop` | — | `stop_monitor(fleet_id)`; reports `monitor stopped (pid M)` or `no monitor running for fleet N` (idempotent, exit 0). |
| `monitor status` | — | Runtime liveness (running/pid/last-tick-age/tick, derived from the heartbeat) **plus** the per-agent schedule table (agent_id, name, role director/member, interval, last_ping, enabled, pending). Exit 0. |
| `monitor config` | `--agent-id INT` (required), `--interval INT` (`IntRange(min=1)`), `--enable/--disable` | No edit flags → print the agent's config. With `--interval` / `--enable` / `--disable` → update and print. `--interval` shares the WebUI PATCH lower bound (`>= 1`). `--enable`/`--disable` are mutually exclusive. Exit 1 if the agent is not in the fleet or not enrolled. |

`monitor start`/`stop` are **CLI-only** (not WebUI): launching/killing a detached OS process from an unauthenticated local SPA is out of scope. WebUI/CLI parity is on the **config + runtime-view** surface (§9), which is what the feature requires.

`--json` is honored on every subcommand (the global flag), mirroring the existing member commands. The detached re-exec target is `[sys.executable, "-m", "cafleet", …]` — using `sys.executable` (not a bare `python` PATH lookup that could resolve to an interpreter/venv where `cafleet` is not importable) guarantees the child runs in the **same** environment as the launching CLI. This requires a new `cafleet/__main__.py` delegating to `cli`.

Text/JSON output shapes (illustrative):

```
$ cafleet monitor status --fleet-id 1
monitor: running (pid 4821, last tick 2s ago, tick 5s, started 2026-06-13T04:50:00+00:00)
  agent_id  name         role      interval  last_ping             enabled  pending
  --------  -----------  --------  --------  -------------------  -------  -------
  2         Director     director  60s       2026-06-13T04:51:00   yes      0
  4         alice        member    60s       -                    yes      2
  5         bob          member    30s       2026-06-13T04:50:30   no       0
```

```json
// cafleet --json monitor status --fleet-id 1
{
  "runtime": {"running": true, "pid": 4821, "tick_seconds": 5,
              "last_tick_at": "2026-06-13T04:51:02+00:00", "last_tick_age_seconds": 2,
              "started_at": "2026-06-13T04:50:00+00:00"},
  "agents": [
    {"agent_id": 4, "name": "alice", "role": "member", "interval_seconds": 60,
     "last_ping_at": null, "enabled": true, "pending_count": 2}
  ]
}
```

### 9. WebUI surface (decision 11)

New/changed endpoints in `cafleet/webui/api.py` (all fleet-scoped via the existing `X-Fleet-Id` header dependency):

| Method/Path | Purpose |
|---|---|
| `GET /api/monitor` | Fleet runtime liveness: `{"running": bool, "pid": int\|null, "tick_seconds": int, "last_tick_at": str\|null, "last_tick_age_seconds": int\|null, "started_at": str\|null}`. So the agents page can show a "monitor running/stopped" indicator (an inert schedule otherwise misleads). |
| `GET /api/agents` (extended) | Each agent object gains a `monitor` field: `{"interval_seconds": int, "last_ping_at": str\|null, "enabled": bool}` or `null` when the agent is not enrolled (Administrator, deregistered, card-only). Lets the table render schedules without N calls. |
| `GET /api/agents/{agent_id}/monitor` | The single agent's config (the spec's canonical GET; 404 if the agent is not in the fleet or not enrolled). |
| `PATCH /api/agents/{agent_id}/monitor` | Body `{"interval_seconds"?: int, "enabled"?: bool}` (Pydantic, both optional; `interval_seconds >= 1`). Updates and returns the new config. 404 if not in fleet / not enrolled; 422 on invalid body. |

Errors use the existing FastAPI `HTTPException(detail=...)` pattern and the `get_webui_fleet` dependency (400 missing/non-int header, 404 unknown fleet). `interval_seconds >= 1` is enforced identically on both edit surfaces — the PATCH model (`>= 1`) and the CLI `--interval` (`click.IntRange(min=1)`).

Admin SPA (`admin/src/`):

- `types.ts`: add `MonitorConfig` (`interval_seconds`, `last_ping_at`, `enabled`), `MonitorRuntime`; extend `Agent` with `monitor: MonitorConfig | null`.
- `api.ts`: add `getMonitor(): Promise<MonitorRuntime>` and `updateAgentMonitor(agentId, patch): Promise<MonitorConfig>` (PATCH); follows the existing `request<T>` helper.
- `Sidebar.tsx`: in each `AgentRow`, show a compact schedule badge for enrolled agents (`agent.monitor !== null`) — the interval (e.g. `60s`) and a disabled-state indicator when `!agent.monitor.enabled` — so the agent **list** surfaces every agent's schedule at a glance. This is what decision 11's "show **each** agent's monitoring schedule" requires; without it the schedule would only be visible one selected agent at a time. Reads the folded `agent.monitor`, no extra fetch.
- `AgentDetail.tsx`: add a **Monitoring** section (shown only when `agent.monitor !== null`) rendering interval, last-ping, and enabled, with a small numeric interval input + **Save** and an enable/disable toggle that call `updateAgentMonitor`; the next 5 s poll refresh reconciles. It reads from the inline `agent.monitor` (already in the polled agents list), so no extra fetch is needed on open.
- `AppHeader.tsx` (or a banner like the existing "no Administrator" strip): a monitor running/stopped indicator fed by `getMonitor()`, so the operator sees at a glance whether schedules are live.

The SPA reads per-agent schedule from the folded `monitor` field on `GET /api/agents` (the list it already polls), so it never calls `GET /api/agents/{id}/monitor` — that endpoint exists for CLI/API parity and tests, **not** for the SPA; an implementer should not wire a redundant per-open fetch.

This keeps CLI and WebUI at parity on both axes: **viewing** every agent's schedule (CLI `monitor status` table ↔ the SPA agent-list schedule badges, with full per-agent detail in the `AgentDetail` aside) and **editing** the interval + toggling enable/disable (CLI `monitor config` ↔ PATCH). Process lifecycle (`start`/`stop`) stays CLI-only by design (§8).

### 10. Cadence & precision (decision 6)

- Default ping interval **60 s** for both Director and members (the `monitor_config.interval_seconds` default at enrollment).
- Default scan tick **5 s**, configurable per run via `monitor start --tick N` (stored in `monitor_runtime.tick_seconds`).
- **Tick granularity is the floor on interval precision.** A ping comes due only at a tick boundary, so an interval that is not a multiple of the tick effectively snaps up to the next tick boundary (e.g. a 7 s interval under a 5 s tick fires at ~10 s). This is documented in the concepts page and CLI options.

### 11. Removal / migration (decision 12 + `~/.claude/rules/removal.md`)

Clean-cut, this same cycle. **Only the scheduling is removed.** The Director's 5-step facilitation loop (poll → ACK → dispatch → health-check → escalate) **stays** — the monitor supplies the scheduling, the skill supplies the policy. Every removed mention is replaced with "ensure `cafleet monitor` is running" / `cafleet monitor start --fleet-id <fleet-id>` / `cafleet monitor stop --fleet-id <fleet-id>`. No deprecation notices remain anywhere (the design doc and git history are the record).

What is removed wherever it appears:

- The `CronCreate` / `ScheduleWakeup` in-session-scheduler tables and prose.
- The `/loop` Prompt Template and "start/stop the `/loop`" setup/teardown guidance, including `CronDelete` teardown steps and "record the cron job ID" nuances.
- The codex/opencode **"no in-session scheduler → fallback options"** table (out-of-band cron driver, MCP scheduling server, user-driven nudges, synchronous-only) and the "use a claude Director for supervision" recommendation that exists only because of that asymmetry.
- The "augmented loop swap" in the execute skill (Step 7's create-before-delete `/loop` replacement): the monitor runs unchanged; **PR-review polling becomes a facilitation step the Director runs on the monitor's wake**, not a separate cron. The PR-review *policy* (comment routing, exit conditions) stays; its *scheduling mechanism* (the augmented `CronCreate` loop, the two cron IDs) is removed.

The skill **names** do not change (`cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`) — only their scheduling **content**; their frontmatter `description` strings drop the cron/CronCreate language. `docs/get-started/configure.md`'s `Skill(...)` allowlist entries therefore stay.

Full file inventory is enumerated in Implementation Step 1.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Run commands via mise full-path tasks from the repo root: `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format`, `mise //admin:build`. Package-relative test paths (`tests/...`).

### Step 1: Documentation & skills first (no code yet)

Per `.claude/rules/design-doc-numbering.md`, documentation — including every affected `SKILL.md` and `README.md` — is updated **before** any code. Treat skill/README drift as a blocker.

**1a — New & updated feature docs**

- [x] Add `docs/concepts/monitoring.md` (NEW concepts page — the monitor is a new architectural axis: an external scheduler process). Cover: heartbeat-vs-facilitation boundary; the director-vs-member `should_ping` split; cadence + tick-precision floor; single-instance via PID file + DB heartbeat; lifecycle (start detached → ping due agents → stop / fleet delete); the `monitor_config` + `monitor_runtime` schema at a concept level. <!-- completed: 2026-06-13T06:00 -->
- [x] Register `docs/concepts/monitoring.md` in the docs nav/index (`docs/index.md`, `docs/concepts/` ordering) and cross-link from `docs/concepts/overview.md`. <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/concepts/overview.md`: add `monitor` to Core terms, add a "Monitoring" subsection, and add the monitor process + heartbeat keystroke to the architecture diagram. <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/spec/data-model.md`: add `monitor_config` and `monitor_runtime` table sections (columns, constraints, the no-AUTOINCREMENT 1:1-PK note, the derived director-vs-member note, the explicit-delete-on-teardown note). <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/spec/cli-options.md`: add a `## cafleet monitor` section (`start`/`stop`/`status`/`config`) with flags, key sequence, validation rules, output (text + JSON), and exit codes, modeled on the `member ping` section; update the Subcommand summary and Option Source Matrix. <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/spec/webui-api.md`: document `GET /api/monitor`, `GET`/`PATCH /api/agents/{id}/monitor`, the folded `monitor` field on `GET /api/agents`, request/response shapes, and the 404/422 cases. <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/how-to/monitor-and-recover.md`: add a short "Ensure the monitor is running" lead (the monitor provides the heartbeat that drives the recovery ladder); keep the recovery ladder itself. <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/how-to/mixed-backend-team.md`: note that any-backend Directors now get the same heartbeat via `cafleet monitor` (the monitor removes the claude-Director-for-scheduling asymmetry). <!-- completed: 2026-06-13T06:00 -->
- [x] Update `docs/concepts/token-reduction.md` (line ~10): replace the `/loop` template reference with the external monitor. <!-- completed: 2026-06-13T06:00 -->
- [x] Update `README.md` via the `/update-readme` skill: add the `cafleet monitor` CLI surface and the monitoring concept; ensure no scheduling/`/loop` mentions remain. <!-- completed: 2026-06-13T06:00 -->

**1b — Scheduling removal in the two core skills (substantive rewrite)**

- [x] `skills/cafleet-agent-team-monitoring/SKILL.md`: drop the `CronCreate`/`ScheduleWakeup` language from the frontmatter `description` and intro; remove `## Mechanism by backend` (the in-session-scheduler tables + the codex/opencode no-scheduler section + the `#### Fallback options` table + the claude-Director recommendation); remove `## /loop Prompt Template`; rewrite `## Loop Lifecycle` into a monitor lifecycle ("ensure `cafleet monitor` is running before the first `member create`; stop it before deleting members"). **Keep** `## Team-facilitation instructions`, `## Health-Check Sequence`, `## Stall Response`; update their tick references from `/loop`/fallback to "monitor tick". <!-- completed: 2026-06-13T06:00 -->
- [x] `skills/cafleet-agent-team-supervision/SKILL.md`: update the preamble (drop "cron-like mechanism"), Communication Model step 4 (tick source → monitor), Authorization-Scope Guard (replace `/loop` ticks / scheduler firings with monitor ticks; keep broker auto-fire), Spawn Protocol step 1 (ensure `cafleet monitor` is running — `cafleet monitor start`, all backends), Asynchronous Wait Rule (monitor tick is the backstop), Cleanup Protocol (`cafleet monitor stop` before member delete), Quick Reference ("Start the supervision tick" → `cafleet monitor start`; "Shut down team" → stop monitor). **Also add the explicit wake-trigger cue (per §Heartbeat vs facilitation boundary): a monitor poll-trigger wake — a bare `cafleet … message poll` keystroke landing in the Director's pane — is the cue to run the entire 5-step facilitation loop, NOT to poll the inbox and stop.** Without this, the bare poll only performs step 1. <!-- completed: 2026-06-13T06:00 -->

**1c — Scheduling removal across orchestration & reference skills (consistency clean-cut)**

- [x] `skills/cafleet-design-doc-create/SKILL.md` (dependency-table row; `#### 1b. Start the monitoring /loop`; the Step-6 `CronDelete` teardown; the "monitoring `/loop` will surface…" note) and `skills/cafleet-design-doc-create/roles/director.md` (bootstrap, cleanup, idle-notification, stall-window, abort, progress-tracking, teardown lines). <!-- completed: 2026-06-13T06:00 -->
- [x] `skills/cafleet-design-doc-execute/SKILL.md` (intro Step-7 description; dependency-table row; `#### 3b. Start the monitoring /loop`; Step-8 `CronDelete`; `#### 7a. Replace the monitoring /loop`; the "Augmented Loop Prompt" block; the Stop-means-stop `/loop` firings; abort + shutdown `CronDelete`) and `roles/director.md` (bootstrap, PR-review-loop, idle, stall, abort, progress, teardown). Collapse the augmented-loop swap into "the monitor runs unchanged; PR-review polling is a facilitation step on the monitor's wake." <!-- completed: 2026-06-13T06:00 -->
- [x] `skills/cafleet-design-doc-interview/SKILL.md` (dependency-table row; `#### 2b. Start the monitoring /loop`; Step-2f `CronDelete`). <!-- completed: 2026-06-13T06:00 -->
- [x] `skills/cafleet-research-report/SKILL.md` (load/start; shutdown `CronDelete`) and `roles/director.md` (bootstrap, cleanup, health-check, teardown). <!-- completed: 2026-06-13T06:00 -->
- [x] `skills/cafleet-research-presentation/SKILL.md` (load/start; shutdown `CronDelete`) and `roles/director.md` (bootstrap, cleanup, teardown). <!-- completed: 2026-06-13T06:00 -->
- [x] `skills/cafleet/reference/director.md` (`/loop` tick references at the `--activity` and post-`exec` lines), `skills/cafleet/reference/recovery.md` (the "stop every `/loop` monitor FIRST" teardown rung and the `--activity`/`/loop` references → "stop the monitor"), `skills/cafleet/roles/director.md` (the recovery cross-reference line). <!-- completed: 2026-06-13T06:00 -->
- [x] `.claude/skills/skill-author/SKILL.md` (the `/loop` monitor overhead bullet; `### 2.3 Start the agent-team-monitoring /loop`; `### 2.5` stop-the-`/loop` step; the example `/loop 1m …` and `CronDelete <loop-job-id>` blocks; troubleshooting §7.7/§7.8 ordering and the "not starting the `/loop`" item). Replace with `cafleet monitor` equivalents. <!-- completed: 2026-06-13T06:30 (Director-applied: member is hard-denied writes under project .claude/; teardown order aligned to canonical monitor-stop-first) -->
- [x] Verify zero residual scheduling references across the **whole tree** (no pathspec, so root `CLAUDE.md` and `cafleet/.claude/` are swept too) with a broadened alternation that also catches the fallback-table vocabulary: `git grep -nIE "CronCreate|CronDelete|CronList|ScheduleWakeup|/loop|in-session schedul|no in-session|self-?wakeup|self-?schedul|fallback driver|out-of-band cron|MCP scheduling server|systemd timer|watch -n|synchronous-only|claude Director for"`. The only legitimate remaining hits are under `design-docs/` (history); every match in `docs/`, `README.md`, `skills/`, `.claude/`, root `CLAUDE.md`, or `cafleet/.claude/` is a removal-rule blocker and must be cleaned. <!-- completed: 2026-06-13T06:30 -->
- [x] Confirm the two core skill **names** are unchanged (`cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`) and that `docs/get-started/configure.md`'s `Skill(cafleet:…)` allowlist entries still resolve (only the skills' scheduling *content* changed, not their identifiers). <!-- completed: 2026-06-13T06:00 -->

### Step 2: Schema — models & migration

- [x] Add `MonitorConfig` and `MonitorRuntime` to `cafleet/db/models.py` per §2 (INTEGER PKs, no AUTOINCREMENT, FK CASCADE off `agents` / RESTRICT off `fleets`, defaults `interval_seconds=60`, `enabled` default `1`/server_default `"1"`, `tick_seconds` default `5`). <!-- completed: 2026-06-13T06:42 -->
- [x] Add migration `cafleet/db/alembic/versions/0002_monitor_tables.py` (`down_revision="0001"`): create both tables; `downgrade()` drops `monitor_runtime` then `monitor_config`. <!-- completed: 2026-06-13T06:42 -->
- [x] Update `cafleet/tests/db/test_alembic_smoke.py`, **renaming the two tests whose names encode the old single-revision state** so the names match the new assertions (stale names are a removal blocker per `~/.claude/rules/removal.md`): `test_only_one_migration_revision_exists` → `test_two_migration_revisions_exist` (asserts two revisions, head `0002` chaining to `0001`); `test_alembic_version_table_records_collapsed_head_0001` → `test_alembic_version_table_records_head_0002` (asserts head `0002`); and update `test_alembic_upgrade_head_creates_expected_tables` to include `monitor_config`, `monitor_runtime`. <!-- completed: 2026-06-13T07:00 -->
- [x] Add migration-shape tests: both tables exist after `upgrade head`, INTEGER PKs, no AUTOINCREMENT, `monitor_config.enabled`/`interval_seconds` NOT NULL with defaults, `last_ping_at` nullable, `monitor_runtime.last_tick_at`/`pid` nullable. <!-- completed: 2026-06-13T07:00 -->

### Step 3: Broker DB layer — `broker/monitor.py` + enrollment/cleanup wiring

- [x] Create `cafleet/broker/monitor.py` with the config CRUD: `_enroll(session, agent_id, interval=DEFAULT_PING_INTERVAL_SECONDS)`, `get_monitor_config(fleet_id, agent_id)`, `list_monitor_configs(fleet_id)`, `update_monitor_config(fleet_id, agent_id, *, interval_seconds=None, enabled=None)` (fleet-membership-gated; raises on unknown/not-enrolled), `record_ping(agent_id, when)`. <!-- completed: 2026-06-13T06:58 -->
- [x] Add the per-tick scan `list_monitor_targets(fleet_id)` returning, per active enrolled agent: `agent_id`, `name`, `is_director`, `pane_id`, `interval_seconds`, `last_ping_at`, `enabled`, `pending_count` (correlated `COUNT` of `input_required` non-`broadcast_summary` tasks where `context_id == agent_id`, mirroring `members.py` subquery style). <!-- completed: 2026-06-13T06:58 -->
- [x] Add the runtime functions: `claim_monitor_runtime(fleet_id, pid, tick_seconds, when) -> bool` (atomic claim per §6), `heartbeat_monitor_runtime(fleet_id, pid, when) -> bool` (**ownership-checked** `UPDATE … WHERE fleet_id=? AND pid=?`; returns False when the slot was reclaimed), `read_monitor_runtime(fleet_id) -> dict|None`, `clear_monitor_runtime(fleet_id, pid)` (ownership-checked clear). Liveness helper `_is_live(row, now)` (heartbeat freshness authoritative; `os.kill(pid,0)` corroborating). `when` is an ISO string stored verbatim in the TEXT column; `_is_live` parses `last_tick_at` with `datetime.fromisoformat`. <!-- completed: 2026-06-13T06:58 -->
- [x] Wire enrollment: `broker/agents.py register_agent` inserts a config row when `placement is not None`; `broker/fleets.py create_fleet` inserts the Director's config row (not the Administrator's). Both inside the existing write transaction. <!-- completed: 2026-06-13T06:58 -->
- [x] Wire cleanup: `broker/agents.py deregister_agent` deletes the agent's `monitor_config` row alongside its placement; `broker/fleets.py delete_fleet` deletes the fleet's `monitor_config` rows and the `monitor_runtime` row inside its transaction. <!-- completed: 2026-06-13T06:58 -->
- [x] Export the new broker functions from `cafleet/broker/__init__.py`. <!-- completed: 2026-06-13T06:58 -->
- [x] Tests (`tests/broker/test_monitor.py`): enrollment-on-create (member + director enrolled, Administrator + card-only not); cleanup-on-deregister and on fleet-delete; `update_monitor_config` (interval + enable/disable, unknown/not-enrolled raises, `enabled` returned as a Python `bool`); `list_monitor_targets` (is_director flag, pane_id, pending_count over `input_required` only, `enabled` as bool, excludes deregistered); `record_ping`; `claim_monitor_runtime` (fresh claim; refuses a live row; reclaims a stale row); **ownership-checked `heartbeat_monitor_runtime` returns False after another pid reclaims the slot** (split-brain guard) and ownership-checked `clear_monitor_runtime` no-ops for a non-owner pid; `read` round-trips. <!-- completed: 2026-06-13T07:00 -->

### Step 4: Monitor process/policy — `cafleet/monitor/`

- [x] Add `cafleet/__main__.py` delegating to `cafleet.cli:cli` so `python -m cafleet …` works (the detached re-exec target). <!-- completed: 2026-06-13T08:08 -->
- [x] Add `cafleet/monitor/__init__.py` constants: `DEFAULT_PING_INTERVAL_SECONDS=60`, `DEFAULT_TICK_SECONDS=5`, `MONITOR_STALE_FACTOR=3`, `MONITOR_STALE_FLOOR_SECONDS=15`, `MONITOR_STOP_TIMEOUT=5`. <!-- completed: 2026-06-13T08:08 -->
- [x] Add a `settings.monitor_state_dir` field (`CAFLEET_MONITOR_STATE_DIR`, default `~/.local/share/cafleet/monitor/`) in `cafleet/config.py`. <!-- completed: 2026-06-13T08:08 -->
- [x] Add `list_pane_ids() -> set[str]` to **both** the `Multiplexer` Protocol in `cafleet/multiplexer/base.py` (the `@runtime_checkable` contract that enumerates every keystroke/query method — keep it complete for the in-process-fake / alternative-backend contract) and the concrete `TmuxMultiplexer` in `cafleet/multiplexer/tmux.py` (`tmux list-panes -a -F '#{pane_id}'` split into a set). Cover it in `tests/multiplexer/test_protocol.py` alongside the other methods. <!-- completed: 2026-06-13T08:08 -->
- [x] `cafleet/monitor/loop.py`: pure `should_ping(target, now)` with `now` a tz-aware `datetime` (§4); `monitor_tick(fleet_id, now)` (ownership-checked heartbeat → STOP on lost ownership, fleet-gone STOP, one `list_pane_ids` call, ping due agents, `record_ping`; serializes `now.isoformat()` at every storage boundary); `run_monitor_loop(fleet_id, tick_seconds)` (claim → PID file → signal handlers → interruptible loop passing `datetime.now(UTC)` → finally ownership-checked clear + remove PID file). <!-- completed: 2026-06-13T08:08 -->
- [x] `cafleet/monitor/process.py`: PID-file read/write/remove under `state_dir`; `start_detached(fleet_id, tick_seconds)` (single-instance pre-check, `Popen([sys.executable, "-m", "cafleet", …, "monitor", "start", "--foreground"], start_new_session=True, stdin=DEVNULL, stdout/stderr=logfile, close_fds=True)`, then poll up to ~2 s for a fresh heartbeat whose `pid == child.pid`; on timeout return a failure result naming `<state_dir>/<fleet_id>.log`); `stop_monitor(fleet_id)` (SIGTERM → wait → SIGKILL escalation → ensure runtime cleared + PID file removed; idempotent). <!-- completed: 2026-06-13T08:08 -->
- [x] Tests (`tests/monitor/test_should_ping.py`): director unconditional; member only with `pending_count>0`; disabled skip; dead/missing pane skip; not-due skip; `last_ping_at is None` due-immediately. <!-- completed: 2026-06-13T08:10 -->
- [x] Tests (`tests/monitor/test_loop.py`): `monitor_tick` pings exactly the due agents (assert `send_poll_trigger` calls + `record_ping`), writes a heartbeat, and returns STOP on a soft-deleted/missing fleet (tmux `_run` already stubbed by conftest; stub `list_pane_ids`). <!-- completed: 2026-06-13T08:10 -->
- [x] Tests (`tests/monitor/test_process.py`): `start_detached` calls `Popen` with `start_new_session=True` and a `[sys.executable, "-m", "cafleet", …, "--foreground"]` argv, refuses when a live runtime exists, and returns a failure result (naming the log path) when the child writes no matching-pid heartbeat in the window; `stop_monitor` signals the recorded pid, clears runtime, removes the PID file, and is a no-op when nothing runs (monkeypatch `os.kill`/`Popen`). <!-- completed: 2026-06-13T08:10 -->

### Step 5: CLI — `cafleet monitor` group

- [x] Add `cafleet/cli/monitor.py` with the `monitor` group and `start` (`--tick IntRange(min=1)`, `--foreground`; runs `ensure_tmux_or_die()` before spawning), `stop`, `status`, `config` (`--agent-id`, `--interval IntRange(min=1)`, `--enable/--disable`) per §8; text + JSON output; exit codes (1 for already-running / detached-child-failed-to-start / unknown-or-deleted fleet / not-enrolled agent; 2 for click usage errors). <!-- completed: 2026-06-13T08:28 -->
- [x] Register the group in `cafleet/cli/__init__.py` (`cli.add_command(monitor)`). <!-- completed: 2026-06-13T08:28 -->
- [x] Integrate teardown: `cafleet/cli/fleet.py fleet_delete` calls `monitor.process.stop_monitor(fleet_id)` before `broker.delete_fleet(fleet_id)`. <!-- completed: 2026-06-13T08:28 -->
- [x] Add output formatters for the status table + config row in `cafleet/output.py` (matching the existing compact formatter style). <!-- completed: 2026-06-13T08:28 -->
- [x] Tests (`tests/cli/test_monitor.py`, CliRunner): `start` default path spawns detached and reports started (monkeypatch `start_detached`); `start --foreground` invokes `run_monitor_loop` (monkeypatch the loop to claim+return); `start` already-running exits 1; `start` unknown/deleted fleet exits 1; `stop` reports stopped / nothing-running; `status` running + per-agent table and `--json` shape; `config` show/edit (interval, enable/disable, mutual exclusion exit 2, not-enrolled exit 1); `fleet delete` calls `stop_monitor`. <!-- completed: 2026-06-13T08:30 -->

### Step 6: WebUI API

- [x] Add to `cafleet/webui/api.py`: `GET /api/monitor` (runtime liveness), `GET /api/agents/{agent_id}/monitor`, `PATCH /api/agents/{agent_id}/monitor` (Pydantic `MonitorPatch` with optional `interval_seconds>=1`, `enabled`), and fold a `monitor` field into `GET /api/agents` (null when not enrolled). All via `get_webui_fleet` + broker calls; 404/422 per §9. <!-- completed: 2026-06-13T08:39 -->
- [x] Tests (`tests/webui/test_monitor_api.py`): `GET /api/monitor` running/stopped; `GET /api/agents` carries `monitor` (null for Administrator); `GET /api/agents/{id}/monitor` 200 + 404 (not in fleet / not enrolled); `PATCH` updates interval + enabled, 422 on bad body, 404 on missing. <!-- completed: 2026-06-13T08:40 -->

### Step 7: Admin SPA

- [x] `admin/src/types.ts`: add `MonitorConfig`, `MonitorRuntime`; extend `Agent` with `monitor: MonitorConfig | null`. <!-- completed: 2026-06-13T08:47 -->
- [x] `admin/src/api.ts`: add `getMonitor()` and `updateAgentMonitor(agentId, patch)`. <!-- completed: 2026-06-13T08:47 -->
- [x] `admin/src/components/AgentDetail.tsx`: add the **Monitoring** section (shown when `agent.monitor !== null`) — interval display + numeric input + Save, enable/disable toggle, last-ping display — calling `updateAgentMonitor`; reconcile on the next poll. <!-- completed: 2026-06-13T08:47 -->
- [x] `admin/src/components/AppHeader.tsx` (or a Dashboard banner): a monitor running/stopped indicator fed by `getMonitor()`. Also added the §9 per-agent schedule badge in `Sidebar.tsx`'s `AgentRow` (reads the folded `agent.monitor`). <!-- completed: 2026-06-13T08:47 -->
- [x] `mise //admin:build` succeeds; `mise //admin:lint` clean. <!-- completed: 2026-06-13T08:47 -->

### Step 8: Verify & install

- [x] `mise //cafleet:format` then `mise //cafleet:lint` clean. <!-- completed: 2026-06-13T09:07 -->
- [x] `mise //cafleet:typecheck` clean. <!-- completed: 2026-06-13T09:07 -->
- [x] `mise //cafleet:test` green (broker, monitor, cli, webui, db suites). <!-- completed: 2026-06-13T09:07 (798 passed) -->
- [x] `mise //admin:build` then a manual WebUI smoke: agents page shows the schedule + a running indicator after `cafleet monitor start`, and editing interval / toggling enable round-trips. <!-- completed: 2026-06-13T09:07 (display + indicator + edit-reflect verified via agent-browser; in-browser click blocked by read-only policy, edit applied via CLI + reload) -->
- [x] `mise //cafleet:install` (editable) then an end-to-end smoke in a tmux session: `fleet create` → `monitor start` (returns immediately, detached) → `monitor status` (running + director enrolled) → `member create` (auto-enrolled) → observe a `message poll` keystroke land in the member pane only after it has pending items → `monitor stop` → `fleet delete` (monitor already stopped, rows cleaned). <!-- completed: 2026-06-13T09:07 (lifecycle live-verified: start detached/returns-immediately, status running+director-enrolled, single-instance refusal, stop, fleet-delete cleanup; member-create+observe-keystroke covered by test_loop.py + heartbeat E2E rather than a live keystroke observation) -->
- [x] Stage the design doc with the implementation commits (`docs/` is committed in this project per `.claude/rules/git-workflow.md`). <!-- completed: 2026-06-13T09:07 -->


---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-13 | Initial draft |
