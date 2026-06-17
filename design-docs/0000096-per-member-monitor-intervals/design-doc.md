# Per-member monitor intervals

**Status**: Approved
**Progress**: 24/36 tasks complete
**Last Updated**: 2026-06-17

## Overview

Invert where the monitoring interval lives. Today the dedicated monitoring member holds the only `monitor_config` row and the loop wakes just that member on its single interval. This design enrolls the root Director (180 s) and every ordinary member (720 s) with their **own** per-member intervals, removes the monitoring member's interval entirely, and reframes the `cafleet monitor` loop so the monitoring member checks each watched agent on that agent's own cadence — re-engaging through the Director, never with blind keystrokes.

## Success Criteria

- [ ] The root Director is enrolled in `monitor_config` at `create_fleet` with a **180 s** interval; every ordinary member is enrolled at `register_agent` with a **720 s** interval; the Administrator and card-only agents stay unenrolled.
- [ ] The dedicated monitoring member is **no longer enrolled** in `monitor_config` and carries no interval anywhere (DB, CLI, WebUI) — full removal, zero deprecation residue per `~/.claude/rules/removal.md`.
- [ ] `cafleet monitor start` (run by the monitoring member) wakes the monitoring member when ≥ 1 watched agent is due by its own interval, and never blind-keystrokes a Director or member pane.
- [ ] On each wake the monitoring member re-queries `cafleet monitor status`, inspects each freshly-due agent with read-only `cafleet member capture`, and re-engages the Director via `cafleet member nudge`; it never drives ordinary members directly.
- [ ] The admin agent-detail page (`/fleets/:id/agents/:id`) shows an editable interval for every enrolled member (Director + ordinary members) and shows no interval control for the monitoring member.
- [ ] Alembic `0005` deletes the monitoring member's `monitor_config` rows and backfills existing active root Directors (180 s) + ordinary members (720 s); downgrade is a no-op.
- [ ] `mise //cafleet:lint`, `:format`, `:typecheck`, `:test`, and `mise //admin:build` all pass; docs, `README.md`, and every affected `SKILL.md` read as if the monitoring member never had an interval.

---

## Background

The monitor heartbeat has evolved across four prior designs; this one is the fifth iteration:

| Design | What it established |
|---|---|
| `0000087` | The `cafleet monitor` loop (`scan → ping → sleep`): enrolled the root Director **and every member**, each with its own `monitor_config.interval_seconds` (default 60 s); the loop pinged each agent directly on its interval. |
| `0000090` | Restricted enrollment to the Director + one dedicated **monitoring member**; removed the blind member resume-nudge; the monitoring member owns `monitor start` and re-engages the Director on idle. |
| `0000091` | Dropped the **Director** from enrollment too. The loop wakes **only** the monitoring member on the monitoring member's interval; the Director is re-engaged solely on demand. |
| `0000095` | Confined the monitoring member's on-wake routine to exactly two commands — read-only `cafleet member capture` and `cafleet member nudge` (re-engage the idle Director). *(Approved; lands alongside or before this design — see §10.)* |

The net of `0000090`+`0000091` is the **current** state: `monitor_config` holds exactly one row per fleet — the monitoring member — and the loop wakes only that member on a single 60 s interval. Ordinary members and the Director are never enrolled and never pinged by the loop.

This design **inverts** that. The per-member interval was the right primitive in `0000087`; what was missing was the safe wake path (judgment + `Esc` safeguard + Director-mediated re-engagement) that `0000090`/`0000091` built. This design composes the two: every watched agent regains its own interval, but the *checking* is performed by the monitoring member's LLM judgment (capture → judge → re-engage the Director), never by a blind loop keystroke into the watched pane.

---

## Specification

### 1. The inversion at a glance

| Axis | Current (post-`0000091`) | This design |
|---|---|---|
| Who is enrolled in `monitor_config` | the monitoring member only | the root Director + every ordinary member |
| Interval owner | the monitoring member (single, default 60 s) | each watched agent (Director **180 s**, member **720 s**) |
| Monitoring member's interval | a `monitor_config` row | **none** (removed entirely) |
| What the loop keystrokes | wakes the monitoring member on its interval | wakes the monitoring member when ≥ 1 **watched** agent is due |
| Who inspects a watched agent | the monitoring member inspects the **Director** only | the monitoring member inspects **each freshly-due watched agent** |
| Re-engagement | via the Director (`cafleet member nudge`) | via the Director (`cafleet member nudge`) — unchanged |

The load-bearing reframe is the split between the **watched set** and the **watcher**:

- **Watched set** = the enrolled agents (Director + ordinary members), each with its own interval. This is what `monitor_config`, `list_monitor_targets`, and `monitor status` represent.
- **Watcher** = the dedicated monitoring member, identified by `agent_card_json.cafleet.kind == "monitoring-member"` (a **kind marker**, *not* a `monitor_config` row). It runs `monitor start` and performs the checking.

### 2. Enrollment matrix & defaults

| Agent kind | Enrolled in `monitor_config`? | Default interval | Insertion site |
|---|---|---|---|
| Root Director | **yes** | **180 s** | `broker.create_fleet` (after its `AgentPlacement` is added) |
| Ordinary member | **yes** | **720 s** | `broker.register_agent` when `placement is not None` AND `kind != MONITORING_MEMBER_KIND` |
| Monitoring member | **no** | — | (not enrolled; located by kind marker, §3) |
| Administrator | no | — | (no placement) |
| Card-only `agent register` | no | — | (no placement) |

Two role-based default constants replace the single `DEFAULT_PING_INTERVAL_SECONDS = 60`, which is **defined in `cafleet/broker/monitor.py`** and re-exported by `cafleet/monitor/__init__.py`:

```python
# cafleet/broker/monitor.py
DIRECTOR_PING_INTERVAL_SECONDS = 180
MEMBER_PING_INTERVAL_SECONDS = 720
```

`cafleet/monitor/__init__.py` re-exports both names — its `from cafleet.broker.monitor import (...)` block, its `__all__`, and its module docstring (which counts the re-exported tunables) are updated accordingly. `enroll_agent`'s `interval` argument **loses its default and becomes required**: both call sites pass it explicitly (`create_fleet` passes 180, `register_agent` passes 720), so no single implicit default is shared across roles.

The `MonitorConfig.interval_seconds` model `server_default` **stays `"60"`** and is left untouched. That value was written into the real schema by the frozen migration `0002` (`cafleet db init` builds schema via Alembic, not `create_all`), so changing only the model default would create model/migration drift without removing the 60 from a migrated DB. The frozen 60 is harmless because it is never read: every enrollment path and the §9 migration write an explicit 180/720.

### 3. Locating the watcher (kind, not enrollment)

Because the monitoring member is no longer in `monitor_config`, the loop locates it by its kind marker. A new broker helper:

```python
def find_monitoring_member(fleet_id: int) -> dict | None:
    """Return the fleet's active monitoring member as {agent_id, name, pane_id}, or None.

    Identified by agent_card_json.cafleet.kind == MONITORING_MEMBER_KIND, joined to
    agent_placements for its pane. There is at most one per fleet (register_agent guard).
    """
```

It selects the single active agent in the fleet whose `json_extract(agent_card_json, '$.cafleet.kind') == MONITORING_MEMBER_KIND`, inner-joined to `agent_placements` for `tmux_pane_id`. Returns `None` when no monitoring member exists (the warn-but-run case, §7).

### 4. The loop (`monitor/loop.py::monitor_tick`)

The loop's keystroke target changes from "every due enrolled agent" to "the single watcher, once, when any watched agent is due."

```python
def monitor_tick(fleet_id, now):                       # now: tz-aware datetime
    if not broker.heartbeat_monitor_runtime(fleet_id, os.getpid(), now.isoformat()):
        return STOP                                    # slot reclaimed → self-terminate
    fleet = broker.get_fleet(fleet_id)
    if fleet is None or fleet["deleted_at"] is not None:
        return STOP                                    # fleet vanished / soft-deleted

    watcher = broker.find_monitoring_member(fleet_id)
    mux = TmuxMultiplexer()
    live_panes = mux.list_pane_ids()

    due = []
    for t in broker.list_monitor_targets(fleet_id):    # the WATCHED set (Director + members)
        t["pane_alive"] = t["pane_id"] in live_panes
        if should_ping(t, now):
            due.append(t)

    if due and watcher is not None and watcher["pane_id"] in live_panes:
        mux.send_wake_trigger(                          # wake the WATCHER once
            target_pane_id=watcher["pane_id"], fleet_id=fleet_id, agent_id=watcher["agent_id"])
        broker.record_pings([t["agent_id"] for t in due], now.isoformat())
        for t in due:
            click.echo(f"{now.isoformat()} due agent {t['agent_id']} ({t['name']}) -> wake monitor")
    return CONTINUE
```

Key points:

- **`should_ping` is unchanged** (pure: `enabled` AND `pane_alive` AND interval elapsed since `last_ping_at`), but it is now evaluated over the **watched** agents (Director + members), not over the monitoring member.
- The loop **never keystrokes a watched pane.** Its only keystroke is `send_wake_trigger` into the watcher's own pane (the same helper used today). No blind member ping is reintroduced — the safety property `0000090` established is preserved.
- **`record_pings` stamps `last_ping_at = now` for the due agents at the moment of wake-dispatch.** This advances each watched agent's cadence and prevents a wake-storm: a just-flagged agent is not due on the next 5 s tick, so the loop will not re-wake the watcher every tick while the watcher is still working. `last_ping_at`'s meaning is "the last time the monitor dispatched a check for this agent."
- **The watcher learns *which* agents to inspect by re-querying `cafleet monitor status`**: the agents the loop just flagged are the ones with the smallest last-ping age (`monitor status` renders `last_ping` as an age; freshly-flagged agents read as "just now"). Because the Director (180 s) and a member (720 s) rarely come due on the same tick, most wakes flag exactly one agent, so the freshly-pinged set is unambiguous.
- If there is **no** watcher (or its pane is dead), the loop records nothing and simply continues — there is no one to wake.

### 5. The watched-set scan (`broker/monitor.py::list_monitor_targets`)

`list_monitor_targets` already inner-joins `monitor_config`, so once enrollment flips it returns the watched set (Director + members) automatically. Two cleanups:

- **Drop the now-dead `is_monitoring_member` field** and its `json_extract` kind expression from the row dicts: the monitoring member is never enrolled, so this field was always `False` here. (Watcher identity now lives in `find_monitoring_member`.) Removing it satisfies the removal rule.
- Keep `is_director` (derived from `fleets.director_agent_id`) for `monitor status` role labeling.
- The docstring is rewritten: enrollment is the root Director (180 s) + every ordinary member (720 s); the monitoring member is the watcher, located separately by kind.

`enroll_agent`, `record_ping`/`record_pings`, `update_monitor_config`, `get_monitor_config`, `list_monitor_configs`, and the runtime functions are otherwise unchanged.

### 6. The monitoring member's routine (`skills/cafleet-agent-team-monitoring/SKILL.md`)

The canonical spawn-prompt routine broadens from "watch the Director" to "watch the Director **and** each freshly-due member, routing everything through the Director." It stays within the two-command on-wake scope (`0000095`): read-only `cafleet member capture` (now used for the Director **and** due members) and `cafleet member nudge` (still re-engaging the **Director** only). On each wake:

1. `cafleet monitor status --fleet-id {fleet_id}` — read the watched schedule and identify the agents the monitor just flagged (smallest `last_ping` age).
2. Capture the Director's pane (`cafleet member capture --member-id {director_agent_id}`); classify ACTIVE vs IDLE.
3. For each freshly-due **member**, capture its pane (read-only) and judge whether it is progressing or stalled.
4. Re-engage the **Director** via `cafleet member nudge --member-id {director_agent_id}` when the Director is idle with un-acked inbox / stalled members, **or** when any inspected member looks stalled — naming what needs attention (idle Director, stalled member `<id>`). The Director then drives the stalled member. **Never keystroke task instructions into an ordinary member's pane.**

This is the only behavioral expansion of the monitoring member: its *observation* now spans all members, but its *actuation* is still Director-only.

### 7. CLI surface (`cafleet/cli/monitor.py`)

| Command | Change |
|---|---|
| `monitor start` | The "no enrolled monitoring member" warning now checks `broker.find_monitoring_member(fleet_id) is None` (the monitoring member is no longer in `list_monitor_targets`). Warn-but-run is unchanged otherwise. |
| `monitor status` | The agent table lists the **watched** agents (Director + members) with role / interval / `last_ping` age / enabled / pending. The `is_monitoring_member` → `"monitor"` role branch is **removed** (dead: the monitoring member is not a watched row); role labels are `director` / `member`. The runtime line continues to report loop liveness (pid, last-tick age, tick). |
| `monitor config --agent-id <id>` | **No code change** — it is already generic. It now edits any enrolled agent (Director or member). `monitor config --agent-id <monitoring-member-id>` naturally returns "not enrolled in monitoring," since the monitoring member has no row. |

`monitor status` renders `last_ping` as a human age (e.g. `8s ago`, or `—` when never pinged) so the monitoring member's LLM can spot the freshly-flagged agents (§4). The JSON shape keeps `last_ping_at` (ISO or null) and gains a derived `last_ping_age_seconds` (int or null) per agent for parity.

### 8. WebUI / admin console (`admin/src/`, `cafleet/webui/api.py`)

The agent-detail requirement (per-member intervals editable for **all** members; the monitoring member's control removed) is satisfied **by the enrollment flip alone** — the SPA is already generic:

- `GET /api/agents` folds a `monitor` field per **enrolled** agent. After the flip the Director + every member get a non-null `monitor`; the monitoring member gets `null`.
- `AgentDetail.tsx`'s `MonitoringSection` renders only when `agent.monitor !== null` and already exposes the interval input + Save + enable/disable toggle, calling `PATCH /api/agents/{id}/monitor`. So the edit control appears for the Director + members and is **absent** for the monitoring member — no component change required.
- `Sidebar.tsx`'s per-agent schedule badge (if present) likewise surfaces for every enrolled agent automatically.

No WebUI API or SPA code change is required for the edit surface. The work here is **verification** (`mise //admin:build` passes; the detail page shows edits for the Director + members and none for the monitoring member) plus retargeting the existing WebUI tests (§ Step 6).

### 9. Migration (`0005_per_member_monitor_intervals.py`)

`down_revision = "0004"` (current head). One-shot data step, idempotent via `INSERT OR IGNORE`:

```python
def upgrade() -> None:
    # 1. Drop the monitoring member's monitor_config rows (interval removed).
    op.execute("""
        DELETE FROM monitor_config WHERE agent_id IN (
            SELECT agent_id FROM agents
            WHERE json_extract(agent_card_json, '$.cafleet.kind') = 'monitoring-member')
    """)
    # 2. Backfill existing active root Directors @180.
    op.execute("""
        INSERT OR IGNORE INTO monitor_config (agent_id, interval_seconds, enabled)
        SELECT f.director_agent_id, 180, 1 FROM fleets f
        WHERE f.director_agent_id IS NOT NULL AND f.deleted_at IS NULL
    """)
    # 3. Backfill existing active, pane-bound ordinary members @720.
    op.execute("""
        INSERT OR IGNORE INTO monitor_config (agent_id, interval_seconds, enabled)
        SELECT a.agent_id, 720, 1 FROM agents a
        JOIN agent_placements p ON p.agent_id = a.agent_id
        WHERE a.status = 'active'
          AND a.agent_id NOT IN
              (SELECT director_agent_id FROM fleets WHERE director_agent_id IS NOT NULL)
          AND json_extract(a.agent_card_json, '$.cafleet.kind') IS NOT 'monitoring-member'
          AND json_extract(a.agent_card_json, '$.cafleet.kind') IS NOT 'builtin-administrator'
    """)

def downgrade() -> None:
    pass  # re-deriving the pre-inversion enrollment is neither possible nor desirable
```

After `0004` (which left only monitoring-member rows enrolled), step 1 clears them and steps 2–3 enroll the Director + members fresh. A live fleet therefore picks up the new watched set on `cafleet db init` without losing its heartbeat. The alembic-smoke head assertions advance from `0004` → `0005` (five revisions).

### 10. Interplay with design `0000095`

`0000095` (Approved, not yet implemented at draft time) confines the monitoring member's on-wake routine to exactly two commands (`cafleet member capture` + `cafleet member nudge`) and affirms "re-engage the idle Director." This design is **compatible and additive**: it keeps that two-command scope and Director-only actuation, and only broadens what `cafleet member capture` inspects (the Director **plus** freshly-due members). The skill edits in Step 1 are written against the **current** `SKILL.md` text; whichever of the two designs lands second must preserve both the two-command scope (`0000095`) and the all-members observation (this design).

### 11. Removal — zero residue (`~/.claude/rules/removal.md`)

The monitoring member's interval is removed everywhere, in this cycle. After it lands the repo reads as if the monitoring member never had an interval:

- **Schema/data:** no `monitor_config` row for the monitoring member (the `0005` migration + the `register_agent` enrollment gate). The `MonitorConfig.interval_seconds` `server_default` keeps its frozen `"60"` (written by migration `0002`); it is never read, since every enrollment writes an explicit 180/720.
- **Source:** `register_agent` (docstring + enrollment comment), `enroll_agent`, and `list_monitor_targets` docstrings describe the Director + ordinary members as the enrolled set; the dead `is_monitoring_member` field and the `"monitor"` role branch in `monitor_status` are deleted; the single `DEFAULT_PING_INTERVAL_SECONDS = 60` is replaced by the two role constants.
- **Docs / README / skills:** every "the monitoring member is pinged on its interval (default 60 s)" / "the monitoring member is the only enrolled agent" statement is rewritten to the watched-set model (Director 180 s, members 720 s; the monitoring member is the unenrolled watcher).

The design doc and git history are the only record of the prior single-interval model.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Run commands via mise full-path tasks from the repo root: `mise //cafleet:test`, `:lint`, `:typecheck`, `:format`, `mise //admin:build`. Package-relative test paths (`tests/...`).

### Step 1: Documentation & skills first (no code)

Per `.claude/rules/design-doc-numbering.md`, documentation — including every affected `SKILL.md` and `README.md` — lands **before** any code.

**1a — Feature docs**

- [x] `docs/concepts/monitoring.md`: rewrite to the watched-set model — enrolled = root Director (180 s) + every ordinary member (720 s); the monitoring member is the unenrolled **watcher** located by kind; the loop wakes the watcher when ≥ 1 watched agent is due; the watcher captures each freshly-due agent and re-engages the Director. Update the lifecycle/topology diagram. Remove the "monitoring member pinged on its 60 s interval / only enrolled agent" framing. <!-- completed: 2026-06-17T08:13 -->
- [x] `docs/spec/data-model.md`: update the `monitor_config` section — enrolled for the root Director (180 s) + ordinary members (720 s); the monitoring member is not enrolled; note the `0005` backfill/prune migration and the two role-based defaults. <!-- completed: 2026-06-17T08:13 -->
- [x] `docs/spec/cli-options.md`: update the `cafleet monitor` section — `monitor status` lists the watched agents (Director + members) with role/interval/last-ping age (no `monitor` role row); `monitor start`'s no-monitoring-member warning is now keyed on `find_monitoring_member`; `monitor config` edits any enrolled agent. <!-- completed: 2026-06-17T08:13 -->
- [x] `docs/spec/webui-api.md`: update the folded-`monitor` description and samples — the Director + members are enrolled (non-null `monitor`), the monitoring member is `null`; refresh the "never enrolled" list. <!-- completed: 2026-06-17T08:13 -->
- [x] `docs/concepts/overview.md`: update the Monitoring subsection + architecture diagram to the watched-set model. <!-- completed: 2026-06-17T08:13 -->
- [x] `docs/how-to/monitor-and-recover.md` and `docs/how-to/mixed-backend-team.md`: update any "loop wakes only the monitoring member" / interval-default phrasing to the watched-set model. <!-- completed: 2026-06-17T08:13 -->
- [x] `docs/concepts/token-reduction.md`: update any monitor cadence/interval reference (default cadence is now Director 180 s / member 720 s, not 60 s). <!-- completed: 2026-06-17T08:13 -->
- [x] `README.md` (monitoring summary, ~line 85): rewrite via the `/update-readme` skill — the loop watches the Director (180 s) + members (720 s) on their own intervals and wakes the monitoring member to inspect due agents; the monitoring member has no interval. Ensure no `60 s` / "only the monitoring member is enrolled" residue remains. <!-- completed: 2026-06-17T08:13 -->

**1b — Skill rewrites**

- [x] `skills/cafleet-agent-team-monitoring/SKILL.md`: rewrite the heartbeat description and the canonical monitoring-member spawn-prompt routine per §6 — the loop watches the Director + members on per-member intervals and wakes the monitoring member; on each wake the monitoring member reads `monitor status`, captures the Director **and** each freshly-due member, and re-engages the Director (never drives members directly). Drop the "pinged on its 60 s interval / only enrolled agent" framing. Preserve the two-command on-wake scope and Director-only actuation. <!-- completed: 2026-06-17T08:13 -->
- [x] `skills/cafleet-agent-team-supervision/SKILL.md`: update any monitor-cadence / enrollment phrasing to the watched-set model; the Director is re-engaged on demand by the monitoring member's nudge (unchanged) — only the cadence framing changes. <!-- completed: 2026-06-17T08:13 -->
- [x] `skills/cafleet/SKILL.md`, `skills/cafleet/reference/director.md`, `skills/cafleet/reference/recovery.md`: update any "loop wakes only the monitoring member" / monitor-enrollment description to the watched-set model; keep `cafleet member ping` / `send_poll_trigger` references intact (unchanged). <!-- completed: 2026-06-17T08:13 -->
- [x] `skills/cafleet-design-doc-execute/SKILL.md` + `roles/director.md`: verify the Step-7 Copilot-loop framing still holds — the monitoring member's wake cadence is now driven by the watched agents' due-ness (Director default 180 s), not a 60 s monitoring-member interval. Update `silence_ticks` / idle-nudge cadence references to the new default where they cite a concrete interval. <!-- completed: 2026-06-17T08:13 -->

**1c — Removal sweep (docs/skills)**

- [x] Sweep `docs/`, `README.md`, `skills/`, `.claude/`, root `CLAUDE.md`, `cafleet/.claude/` for residual single-interval phrasing: `git grep -nIE "only (the )?monitoring member.*enrolled|monitoring member.*interval|default.*60 ?s|interval.*default.*60"`. Every hit outside `design-docs/` is a removal-rule blocker; legitimate history lives only under `design-docs/`. <!-- completed: 2026-06-17T08:13 -->

### Step 2: Schema, constants & migration

- [x] `cafleet/broker/monitor.py`: replace `DEFAULT_PING_INTERVAL_SECONDS = 60` with `DIRECTOR_PING_INTERVAL_SECONDS = 180` and `MEMBER_PING_INTERVAL_SECONDS = 720`; make `enroll_agent`'s `interval` a **required** argument (drop its default). Update `cafleet/monitor/__init__.py`'s `from cafleet.broker.monitor import (...)` block, `__all__`, and module docstring to re-export both new names (the model `server_default` stays `"60"` and is not touched — §2). <!-- completed: 2026-06-17T08:43 -->
- [x] Add `cafleet/db/alembic/versions/0005_per_member_monitor_intervals.py` (`down_revision="0004"`) per §9: delete monitoring-member rows; backfill active Directors @180 + ordinary members @720; downgrade no-op. <!-- completed: 2026-06-17T08:43 -->
- [x] Update `cafleet/tests/db/test_alembic_smoke.py`: rename `test_alembic_version_table_records_head_0004` → `test_alembic_version_table_records_head_0005` (assert `[("0005",)]`); rename `test_four_migration_revisions_exist` → `test_five_migration_revisions_exist` (assert `len == 5`, the `{0001..0005}` set, `by_revision["0005"].down_revision == "0004"`, `script.get_current_head() == "0005"`, and update its docstring). `test_monitor_config_table_created_by_migration`'s `interval_seconds == 60` assert reads the migrated schema (frozen `0002` default) and stays correct. <!-- completed: 2026-06-17T08:43 -->
- [x] Add a `0005` data-migration test: after `upgrade head`, a pre-existing monitoring-member `monitor_config` row is gone, a pre-existing active Director row is present @180, a pre-existing active ordinary member is present @720, and the Administrator stays unenrolled. <!-- completed: 2026-06-17T08:43 -->
- [x] Update `cafleet/tests/monitor/test_constants.py`: retarget `test_monitor_package_exposes_all_constants` to assert `DIRECTOR_PING_INTERVAL_SECONDS == 180` and `MEMBER_PING_INTERVAL_SECONDS == 720`, keep the `DEFAULT_TICK_SECONDS` / `MONITOR_STALE_FACTOR` / `MONITOR_STALE_FLOOR_SECONDS` asserts, drop the `DEFAULT_PING_INTERVAL_SECONDS` assert, and fix the module docstring tunable count. <!-- completed: 2026-06-17T08:43 -->

### Step 3: Broker — enrollment, cleanup & watcher lookup

- [x] `cafleet/broker/fleets.py::create_fleet`: enroll the root Director via `monitor.enroll_agent(session, director_agent_id, interval=DIRECTOR_PING_INTERVAL_SECONDS)` after its `AgentPlacement` is added (re-adds the Director enrollment `0000091` removed). <!-- completed: 2026-06-17T08:59 -->
- [x] `cafleet/broker/agents.py::register_agent`: when `placement is not None` AND `kind != MONITORING_MEMBER_KIND`, call `monitor.enroll_agent(session, agent_id, interval=MEMBER_PING_INTERVAL_SECONDS)`; when `kind == MONITORING_MEMBER_KIND`, do **not** enroll. Keep the one-per-fleet + pane-bound monitoring-member guards. <!-- completed: 2026-06-17T08:59 -->
- [x] `cafleet/broker/monitor.py`: add `find_monitoring_member(fleet_id) -> dict | None` (§3, kind-filtered join to `agent_placements`); update `enroll_agent` + `list_monitor_targets` docstrings to the watched-set model; **remove** the `is_monitoring_member` field and its `json_extract` kind expression from `list_monitor_targets` rows (keep `is_director`). <!-- completed: 2026-06-17T08:59 -->
- [x] Export `find_monitoring_member` and the two interval constants from `cafleet/broker/__init__.py` as needed. <!-- completed: 2026-06-17T08:59 -->
- [x] `cafleet/broker/agents.py::register_agent`: rewrite the docstring `kind` paragraph and the inline enrollment comment to the watched-set model — an ordinary member (with placement) is enrolled @720; the monitoring member is **not** enrolled. The 1c sweep does not scan `cafleet/src`, so this source residue is fixed here. <!-- completed: 2026-06-17T08:59 -->
- [x] Tests (`tests/broker/test_monitor.py`): `create_fleet` enrolls the Director @180; an ordinary `member create` enrolls @720; a `--role monitor` member is **not** enrolled; `list_monitor_targets` returns the Director + members (no monitoring member, no `is_monitoring_member` key); `find_monitoring_member` returns the monitoring member's pane (and `None` when absent); `deregister_agent` / `delete_fleet` still drop the relevant `monitor_config` rows. <!-- completed: 2026-06-17T08:59 -->

### Step 4: Loop rewrite

- [ ] `cafleet/monitor/loop.py::monitor_tick`: rewrite per §4 — locate the watcher via `broker.find_monitoring_member`; compute the due set over the watched agents (`should_ping` unchanged); when the due set is non-empty and the watcher pane is live, `send_wake_trigger` into the watcher's pane once, `record_pings` the due agent ids, and log each due agent. Update `should_ping`'s docstring (it now ranges over the watched Director + members; the monitoring member is the unenrolled watcher). <!-- completed: -->
- [ ] Tests (`tests/monitor/test_loop.py`): a due Director wakes the watcher (one `send_wake_trigger`, Director's `last_ping_at` advanced, no keystroke into the Director's pane); a due member wakes the watcher and advances that member's `last_ping_at`; nothing due → no wake, no `record_pings`; no monitoring member present → no wake; `STOP` on soft-deleted/missing fleet. <!-- completed: -->
- [ ] Tests (`tests/monitor/test_should_ping.py`): keep the pure-policy cases; ensure fixtures reflect watched Director/member targets (interval/enabled/pane-liveness), not a monitoring-member target. <!-- completed: -->

### Step 5: CLI

- [ ] `cafleet/cli/monitor.py::monitor_start`: replace the `any(t["is_monitoring_member"] …)` warning check with `broker.find_monitoring_member(fleet_id) is None`; warn-but-run unchanged. <!-- completed: -->
- [ ] `cafleet/cli/monitor.py::monitor_status`: remove the `is_monitoring_member` → `"monitor"` role branch (role labels become `director` / `member`); add the `last_ping_age_seconds` derived field and render `last_ping` as a human age in the text formatter. <!-- completed: -->
- [ ] `cafleet/output.py`: update `format_monitor_status` for the last-ping-age column (and drop any `monitor` role rendering). <!-- completed: -->
- [ ] Tests (`tests/cli/test_monitor.py`): `monitor status` lists the Director + members with their intervals (180 / 720) and last-ping age, and shows no `monitor`-role row; `monitor start` warns when no monitoring member exists (now via `find_monitoring_member`) and is silent when one exists; `monitor config --agent-id <director>` and `--agent-id <member>` both edit; `--agent-id <monitoring-member>` reports not-enrolled. <!-- completed: -->

### Step 6: WebUI — verification & test retargeting

- [ ] `cafleet/tests/webui/test_monitor_api.py`: retarget the enrolled-agent assertions to the Director + members — `GET /api/agents` folds a non-null `monitor` for the Director (180) and members (720) and `null` for the monitoring member; `GET /api/agents/{monitoring-member}/monitor` 404s; `PATCH /api/agents/{member}/monitor` updates a member's interval. <!-- completed: -->
- [ ] Confirm `admin/src/` needs no code change for the edit surface (the `MonitoringSection` renders for any `agent.monitor !== null`); run `mise //admin:build`. <!-- completed: -->

### Step 7: Verification

- [ ] `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test`, `mise //admin:build` all pass. <!-- completed: -->
- [ ] Final removal sweep across the whole tree confirms no `60 s` monitor-interval / "only the monitoring member is enrolled" residue survives outside `design-docs/`. <!-- completed: -->
- [ ] Manual smoke (operator, optional): spawn a fleet + monitoring member; confirm `monitor status` shows the Director @180 and members @720, the monitoring member as runtime owner (no interval row); confirm the loop wakes the monitoring member when an agent is due and never keystrokes a watched pane; confirm the admin detail page edits a member's interval and shows no interval control for the monitoring member. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-17 | Initial draft |
| 2026-06-17 | Reviewer pass: corrected the constant location (defined in `broker/monitor.py`, re-exported by `monitor/__init__.py`); dropped the model `server_default` change (frozen `0002` default left untouched); made `enroll_agent`'s `interval` required; added the `test_constants.py` + `register_agent`-prose tasks and spelled out the alembic-smoke renames. Approved by user. |
