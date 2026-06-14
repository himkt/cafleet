# Monitor heartbeat nudges the monitoring member only

**Status**: Approved
**Progress**: 10/23 tasks complete
**Last Updated**: 2026-06-14

## Overview

Narrow the `cafleet monitor` heartbeat so it wakes **only** the dedicated monitoring member. The root Director is dropped from the loop's nudge targets entirely — the monitoring member's existing capture-classify-reengage routine already re-engages the Director on demand, so a direct heartbeat poll to the Director is redundant. Target topology: `heartbeat → monitoring member ONLY → (on demand) Director`.

## Success Criteria

- [ ] `monitor_tick` wakes only the monitoring member (`send_wake_trigger`); the Director is never pinged by the loop.
- [ ] The root Director is no longer enrolled in `monitor_config`: `create_fleet` stops enrolling it, and a new Alembic data step prunes pre-existing Director `monitor_config` rows.
- [ ] `list_monitor_targets` returns only the monitoring member (plus any stray/legacy enrolled row, which the loop's defensive `else: continue` skips); `monitor status` shows only the monitoring member.
- [ ] `monitor start` prints a startup warning when no monitoring member is enrolled, but still runs the loop.
- [ ] `send_poll_trigger` and `send_wake_trigger` both stay — `cafleet member ping` reuses `send_poll_trigger`; no keystroke helper is removed.
- [ ] Four request-driven orchestration skills (`cafleet-design-doc-create`, `-interview`, `cafleet-research-report`, `-presentation`) drop `cafleet monitor start`; `cafleet-design-doc-execute` instead adopts the monitoring-member model (its Copilot review loop is heartbeat-driven). No "Director runs monitor start" / "start the heartbeat" text or deprecation notes remain anywhere outside this design doc.
- [ ] `mise //cafleet:lint`, `:format`, `:typecheck`, and `:test` all pass; `docs/`, `README.md`, and every affected `SKILL.md` are consistent with the new behavior.

---

## Background

This is the third iteration on the monitor heartbeat:

| Design | What it established |
|---|---|
| `0000087` | The `cafleet monitor` loop: enrolled the root Director **and every member**; pinged the Director with `send_poll_trigger`, members with `send_resume_trigger`. |
| `0000090` | Restricted enrollment to **two** roles — the root Director (`send_poll_trigger`) and one dedicated **monitoring member** (`send_wake_trigger`); removed `send_resume_trigger`; the monitoring member owns `monitor start` and re-engages the Director on idle. |
| `0000091` (this doc) | Drop the **Director** from the heartbeat entirely. The loop wakes **only** the monitoring member; the Director is re-engaged solely on demand. |

The Director's direct heartbeat poll is now redundant. The monitoring member, on every wake, already captures the Director's pane, classifies it active vs idle, and — when idle — re-engages the Director with an `Esc`-safeguarded `cafleet message send` (which fires the broker's inline-preview keystroke into the Director's pane). The Director therefore still gets woken when it actually needs to be — by the monitoring member's on-demand nudge, and by the broker's inline-preview keystroke on every inbound `message send` — without the loop polling it on a blind fixed cadence.

A consequence: a fleet that runs the monitor **without** a monitoring member loses the Director's only heartbeat backstop (the loop would target nobody). Five orchestration skills currently have the **Director** run `monitor start` with no monitoring member. Four of them — `cafleet-design-doc-create`, `-interview`, `cafleet-research-report`, `-presentation` — are genuinely request-driven: every member wakes on the broker's inline-preview keystroke on each `message send`, and the Director is woken by members' replies (also inline previews) and drives the work by active polling. They do not need a heartbeat backstop, so this design removes `monitor start` from them rather than forcing a monitoring member into every run.

The fifth, `cafleet-design-doc-execute`, is **not** request-driven in its Copilot-review phase (Step 7): that loop polls an *external* service (GitHub Copilot) which never fires a broker inline-preview into the Director's pane, so after a fix-push nothing would give the Director a turn to re-poll the PR. Execute therefore **keeps an active heartbeat** — but via the new architecture: it spawns a dedicated `--role monitor` monitoring member (the monitoring member runs `monitor start`, the Director never does), and Step 7's PR-poll cadence + silence timer are driven by the monitoring member's periodic idle-nudges to the Director (each nudge gives the Director a turn to re-poll), not by direct Director heartbeat polls.

---

## Specification

### Locked decisions (confirmed with the user)

| # | Decision |
|---|---|
| Q1 | Drop `cafleet monitor start` from **four** request-driven orchestration skills (`cafleet-design-doc-create`, `-interview`, `cafleet-research-report`, `-presentation`); no heartbeat backstop replaces it there. **Carve out `cafleet-design-doc-execute`** (user decision): its Step-7 Copilot loop is heartbeat-driven, so execute keeps an active heartbeat by adopting the monitoring-member model (the monitoring member runs `monitor start`; the Director never does) and reframing the Step-7 poll cadence + silence timer onto the monitoring member's idle-nudges. The design doc lists each skill edit explicitly and follows the removal rule (no leftover "Director runs monitor start" text, no deprecation notes outside this doc). |
| Q2 | Stop enrolling the root Director in `monitor_config`. `create_fleet` no longer enrolls it; `list_monitor_targets` returns only the monitoring member; a new Alembic data step prunes pre-existing Director rows, mirroring `0000090`'s prune (`0003`). |
| Q3 | `monitor start` in a fleet with **no** enrolled monitoring member: **warn-but-run** — print a startup warning that no monitoring member is enrolled, then run the loop unchanged. |
| Q4 | `monitor status` shows **only** the monitoring member. Keep deriving `is_director` defensively (so a stray/legacy enrolled Director row still labels and the loop still skips it), but no Director row is expected in normal operation. |
| — | `send_poll_trigger` **stays** (`cafleet member ping` reuses it) and `send_wake_trigger` stays. No keystroke helper is removed by this design. |

### Topology — before and after

Before (`0000090`):

```
monitor loop (runs in the monitoring member's pane)
  ├─ Esc + `cafleet … message poll`  → Director pane      (facilitation heartbeat)   ← REMOVED
  └─ Esc + wake nudge                → monitoring member  (self-ping → routine)
```

After (this design):

```
monitor heartbeat → monitoring member ONLY
  └─ on each wake (Esc + wake nudge, self-ping into its own pane):
       capture Director pane → classify active vs idle
         ├─ ACTIVE → do nothing
         └─ IDLE   → re-engage the Director ON DEMAND via `cafleet message send`
                     (fires the broker inline-preview keystroke into the Director's pane)
```

The Director receives **no** direct keystroke from the loop. Its re-engagement paths are: (1) the monitoring member's on-demand idle nudge, and (2) the broker's inline-preview keystroke on every inbound `message send`. The monitoring member's **canonical** routine is **unchanged** from `0000090` (a *conditional* idle-nudge — it re-engages the Director only when it can name un-acked inbox items or stalled members), **except** for the execute carve-out's **extended** Copilot-wait routine, where the idle-nudge becomes unconditional to grant the Director a re-poll turn (§6, A9).

### 1. Loop keystroke selection (`monitor/loop.py::monitor_tick`)

The `is_director` keystroke arm is removed; the branch collapses to the monitoring-member case with the same defensive skip:

```python
# monitor/loop.py — monitor_tick (after)
for target in broker.list_monitor_targets(fleet_id):
    target["pane_alive"] = target["pane_id"] in live_panes
    if not should_ping(target, now):
        continue
    # The loop wakes ONLY the monitoring member, with a wake nudge that drives
    # its capture-and-assess routine. Any other enrolled row — a stray/legacy
    # Director row that survived the prune, the Administrator, an ordinary
    # member — is defensively skipped, never woken.
    if target["is_monitoring_member"]:
        keystroke = mux.send_wake_trigger
    else:
        continue
    keystroke(target_pane_id=target["pane_id"], fleet_id=fleet_id, agent_id=target["agent_id"])
    ...
```

`should_ping` is unchanged (it is role-agnostic — interval / enabled / pane-liveness only). Its docstring drops the "Both enrolled roles — the root Director and the monitoring member" framing in favor of "the dedicated monitoring member (the Director is no longer enrolled; `is_director` is still derived for the loop's defensive skip and `monitor status` labeling)".

`send_poll_trigger` is **not** removed — `cafleet member ping` (`cli/member.py::member_ping`) still calls it. Removing only the loop's call site leaves no dead code.

### 2. Enrollment — the Director is no longer enrolled (`broker/fleets.py::create_fleet`)

`create_fleet` currently enrolls the root Director:

```python
# broker/fleets.py — create_fleet (BEFORE — to be removed)
# Enroll the root Director (pane-bound) in monitoring; the Administrator
# below has no placement and is intentionally not enrolled.
monitor.enroll_agent(session, director_agent_id)
```

This call and its comment are deleted. After this change the **only** enrollment site is `register_agent` when `kind == MONITORING_MEMBER_KIND` (unchanged from `0000090`). A fresh fleet therefore has **zero** enrolled agents until a monitoring member is created; `list_monitor_targets` (which inner-joins `monitor_config`) then returns only the monitoring member.

`broker/monitor.py::enroll_agent` and `list_monitor_targets` docstrings are updated to state that enrollment is restricted to exactly **one** role per fleet — the dedicated monitoring member — and that `is_director` is retained as a derived field purely for the loop's defensive skip and `monitor status` labeling (a Director row is not expected).

### 3. `monitor status` display (`cli/monitor.py::monitor_status`)

**No code change.** The role-label expression (`director` / `monitor` / `member`) stays exactly as-is for defensive correctness — a stray/legacy enrolled Director row would still render as `director`, and the loop skips it. The table's output changes only because the Director row is no longer present (it is no longer enrolled). The doc edits (A2, A3) update the sample `monitor status` output to drop the Director row.

### 4. `monitor start` warns when no monitoring member is enrolled (`cli/monitor.py::monitor_start`, Q3)

After `_require_live_fleet(fleet_id)` and `ensure_tmux_or_die()` (which runs between them at `cli/monitor.py:50`), and before `loop.run_monitor_loop(...)`, check whether any enrolled agent is a monitoring member and warn (to stderr) if not, then proceed:

```python
# cli/monitor.py — monitor_start, after _require_live_fleet / ensure_tmux_or_die
targets = broker.list_monitor_targets(fleet_id)
if not any(t["is_monitoring_member"] for t in targets):
    click.echo(
        f"Warning: fleet {fleet_id} has no enrolled monitoring member; the "
        f"monitor heartbeat will wake no agent. Spawn one first with "
        f"'cafleet member create --role monitor'.",
        err=True,
    )
loop.run_monitor_loop(fleet_id, tick)
```

In the canonical flow the warning never fires: the monitoring member is enrolled at `member create` (which precedes its boot), so by the time it runs `monitor start` in its own pane its `monitor_config` row already exists and appears in `list_monitor_targets`. The warning fires only in the misconfigured/legacy case (e.g. a Director running `monitor start` with no monitoring member). No new broker helper is needed — `list_monitor_targets` already carries `is_monitoring_member`.

### 5. Data migration (`0004`, Q2)

A new Alembic revision (`down_revision = "0003"`) runs a one-shot data step (no schema change) that prunes the root-Director rows `0003` deliberately kept:

```python
# 0004_prune_director_monitor_config.py
revision = "0004"
down_revision = "0003"

def upgrade() -> None:
    op.execute(
        """
        DELETE FROM monitor_config
        WHERE agent_id IN (
            SELECT director_agent_id FROM fleets WHERE director_agent_id IS NOT NULL
        )
        """
    )

def downgrade() -> None:
    pass  # re-enrolling the Director is neither possible nor desirable
```

After `0003` (kept only Director rows) + `0004` (deletes Director rows), `monitor_config` holds only monitoring-member rows. The downgrade is a no-op, consistent with `0003`. The two alembic-smoke assertions that encode the head revision (`test_*_records_head_0003` and the revision-count test) are advanced to head `0004` / four revisions.

### 6. Skill-drift resolution (Q1)

**Four** of the request-driven orchestration skills become **no-monitor** teams: `cafleet-design-doc-create`, `cafleet-design-doc-interview`, `cafleet-research-report`, and `cafleet-research-presentation`. For each, every `cafleet monitor start` launch, every "start the heartbeat / Step-Nb" section, every dependency-table heartbeat row, and every "stop the monitor" teardown step is removed (removal rule: no leftover text, no deprecation notes). Where a skill loads `cafleet-agent-team-supervision` / `cafleet-agent-team-monitoring` (the `design-doc-*` skills load both; the `research-*` skills load `cafleet-agent-team-monitoring` only), the load **stays** — those skills carry governance and facilitation policy (Authorization-Scope Guard, idle semantics, Stall Response, Cleanup) the orchestration Director still needs — but the orchestration skill adds an explicit one-line override so a Director following the procedure does not re-introduce a heartbeat from the loaded skill:

> This team is request-driven: it does **not** run `cafleet monitor` or spawn a `--role monitor` member. Members wake on the broker's inline-preview keystroke on every `message send`; the Director is woken by members' replies and drives work by active polling (and `cafleet member ping` for manual recovery).

The `cafleet-agent-team-monitoring` and `cafleet-agent-team-supervision` skills themselves are **not** rewritten to make the monitoring member optional — they remain the canonical home of the heartbeat for actively-supervised teams. They are edited only for the Director-no-longer-polled change (A5/A6): the loop wakes only the monitoring member, and the supervision skill's "a monitor poll-trigger wake is the Director's cue to run its facilitation loop" cue is removed (the Director receives no poll-trigger from the loop anymore — it facilitates when the monitoring member nudges it on demand or when work arrives via inline preview).

**The execute carve-out (user decision).** `cafleet-design-doc-execute` is the exception: its Step-7 Copilot-review loop is heartbeat-driven (`silence_ticks` counts monitor wakes; 7a adds PR-review polling to each wake; 7e/7f escalate after ~30 silent wakes), and Copilot is an *external* reviewer that never fires a broker inline-preview into the Director's pane — so a no-monitor execute team would have no turn source to re-poll the PR after a fix-push. Execute therefore adopts the **monitoring-member model** (matching the agent-team skills): the **first** `member create` is a `--role monitor` monitoring member that runs `monitor start` (the Director never does), gated by its `ready: monitor live` handshake (first-in) and stopped before its pane is killed at teardown (first-out). Step 7 is reframed so its PR-poll cadence and silence timer ride the **monitoring member's periodic idle-nudges**: each interval the monitoring member finds the Director idle-while-awaiting-Copilot and nudges it, which gives the Director a turn to re-poll the PR. `silence_ticks` is redefined as **consecutive Director turns (driven by the monitoring member's idle nudge) with 0 new Copilot items**; the ~30-wake / ~30-minute escalation maps onto ~30 such nudges.

This works only if a nudge actually fires during a *quiet* Copilot wait — and the canonical `0000090` routine would **not** fire one. That routine nudges an idle Director only when it can name "what needs attention (un-acked inbox items, stalled members)," and during a Copilot wait the inbox is empty and members already reported their fixes, so it has nothing to name and takes the "do nothing" path — `silence_ticks` would never advance. Execute's monitoring member therefore runs an **extended** routine: when it finds the Director idle, it nudges **unconditionally** — it does **not** gate the nudge on nameable inbox/stalled content — purely to grant the Director a re-poll turn. That unconditional idle-nudge is the turn source that advances `silence_ticks`. **No Director→monitoring-member "Step 7 active" handshake is needed** (the cleaner of the two options): the monitoring member stays PR-agnostic and simply grants the idle Director a turn each interval for the whole execute lifetime — harmless outside Step 7 (the Director re-polls, finds nothing new, idles again), and exactly the turn source Step 7 needs. The Director's own Step-7 per-turn checklist is what re-polls the PR; the monitoring member never needs to know a PR exists. This extended routine is a spawn-prompt **delta scoped to execute's monitoring member only** — the canonical `cafleet-agent-team-monitoring` routine (A5) is unchanged. The enumerated edits, including this spawn-prompt delta, are in A9.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first ordering per `.claude/rules/design-doc-numbering.md`: Phase A (docs/README/skills) lands before any code.

### Phase A: Documentation

- [x] A1. Rewrite `docs/concepts/monitoring.md`: the heartbeat wakes **only** the monitoring member; remove the Director-poll wake keystroke, the "two agents / Director + monitoring member" enrollment framing, and the "monitor poll-trigger wake is the Director's cue" facilitation bullet; state the Director is re-engaged only on demand (monitoring member's idle nudge + broker inline preview); update the Lifecycle mermaid diagram to drop the `Esc + poll → Director pane` edge; update the Enrollment/schema section (Director no longer enrolled; note the `0004` prune migration). <!-- completed: 2026-06-14T11:34 -->
- [x] A2. Update `docs/spec/data-model.md`: `monitor_config` is enrolled **only** for the monitoring member (the root Director is no longer enrolled); document the `0004` Alembic data step that prunes pre-existing Director rows; keep the derived `is_director` note (used for defensive skip + status labeling). <!-- completed: 2026-06-14T11:36 -->
- [x] A3. Update `docs/spec/cli-options.md`: add the `monitor start` startup warning (no enrolled monitoring member → warn-but-run); update the `monitor status` sample output to drop the Director row (only the monitoring member appears); refresh the `## cafleet monitor` prose to "wakes only the monitoring member". <!-- completed: 2026-06-14T11:38 -->
- [x] A4. Update `README.md` monitoring summary (line ~85): the loop "wakes only two agents — the root Director and the monitoring member itself" becomes "wakes only the monitoring member"; the Director is re-engaged on demand by the monitoring member (and the broker inline-preview keystroke). <!-- completed: 2026-06-14T11:39 -->
- [x] A5. Update `skills/cafleet-agent-team-monitoring/SKILL.md`: the heartbeat wakes only the monitoring member; remove the Director-poll keystroke from the keystroke list / any "loop pings Director + monitoring member" or "two enrolled agents" text; the monitoring member re-engages the Director on demand (routine unchanged); the Monitor Lifecycle (monitoring member runs `monitor start`; first-in/first-out; `ready: monitor live` gate; stop-before-delete) is unchanged. <!-- completed: 2026-06-14T11:42 -->
- [x] A6. Update `skills/cafleet-agent-team-supervision/SKILL.md`: the Director is no longer polled by the heartbeat — **remove** the "a monitor poll-trigger wake is the cue to run the entire facilitation loop" cue; replace with: the Director runs facilitation when the monitoring member nudges it on demand (idle re-engagement) or when work arrives via the broker inline-preview keystroke; update any "loop wakes the Director and the monitoring member" framing to "the loop wakes only the monitoring member". <!-- completed: 2026-06-14T11:45 -->
- [x] A7. Update `skills/cafleet/SKILL.md`, `skills/cafleet/reference/director.md`, and `skills/cafleet/reference/recovery.md`: change every "loop pings Director + monitoring member" / "a bare `Esc`+poll to the Director, an `Esc`+wake nudge to the monitoring member" description to "the loop wakes only the monitoring member"; state explicitly that `send_poll_trigger` / `cafleet member ping` are unchanged (the Director-poll keystroke survives for manual recovery, only the loop stops calling it). <!-- completed: 2026-06-14T11:49 -->
- [x] A8. Drop `cafleet monitor start` from the **four** no-monitor orchestration skills per §6, following the removal rule. Per file: `skills/cafleet-design-doc-create/SKILL.md` (dependency-table heartbeat row, the Step-1b "run the supervision heartbeat" block, the Step-6 stop-the-monitor teardown) + `roles/director.md` (the "Run the monitor … BEFORE spawning any member" bootstrap + teardown lines); `skills/cafleet-design-doc-interview/SKILL.md` (dependency-table row, the Step heartbeat block, the Step-2f stop); `skills/cafleet-research-report/SKILL.md` + `roles/director.md` (bootstrap monitor-start lines); `skills/cafleet-research-presentation/SKILL.md` + `roles/director.md` (bootstrap monitor-start lines). Add the §6 "request-driven, no monitor" override note where each skill loads the agent-team skills. <!-- completed: 2026-06-14T11:58 -->
- [x] A9. Convert `cafleet-design-doc-execute` to the monitoring-member model and reframe Step 7 (user carve-out, §6). In `skills/cafleet-design-doc-execute/SKILL.md`: replace the "Director runs `cafleet monitor start`" bootstrap (dependency-table heartbeat row + the Step-3b heartbeat block) with the monitoring-member spawn (the **first** `member create` is `--role monitor --model sonnet`; it runs `monitor start`; the `ready: monitor live` handshake gates the first ordinary member — first-in), and rewrite the Step-8 teardown to stop the monitoring member's `monitor start` task before deleting it (first-out). Rewrite Step 7 so the PR-poll cadence + silence timer ride the monitoring member's idle-nudges rather than direct Director heartbeat polls: redefine `silence_ticks` (`:537-538`) as "consecutive Director turns driven by the monitoring member's idle nudge with 0 new Copilot items"; update 7a (`:540`, "add PR-review polling to each monitor wake" → "…to each Director turn the monitoring member's idle-nudge drives"), 7b (`:546-548` per-wake procedure), 7e/7f (`:565`, `:595-597` — "30 consecutive silent monitor wakes" → "~30 consecutive idle-nudge-driven turns"), and the Step-7 per-wake checklist (`:608-628`, retitle "per-wake" → "per idle-nudge turn"). In `roles/director.md`: rewrite the Step-7 bullet (`:28`) and the PR-Review row in its table (`:144`) to the same idle-nudge framing, and replace its "Director runs the monitor" bootstrap with the monitoring-member spawn. Apply the removal rule (no leftover "Director runs monitor start" / bare "monitor wake" framing). **Spawn-prompt delta (the §6 extended routine):** execute's monitoring-member spawn prompt carries an extended routine vs the canonical `cafleet-agent-team-monitoring` prompt — when it finds the Director **idle** it nudges **unconditionally** (granting a re-poll turn), rather than only when it can name un-acked inbox / stalled-member content. State this delta in execute's monitoring-member spawn prompt and note that the canonical `cafleet-agent-team-monitoring` routine (A5) keeps its conditional nudge. No Step-7 enter/exit handshake is required — the monitoring member is PR-agnostic and the Director's Step-7 per-turn checklist consumes the granted turn. <!-- completed: 2026-06-14T12:08 -->
- [x] A10. Update the docs `0000091` additionally invalidates (per review): `docs/how-to/monitor-and-recover.md` (`:31` — "the loop pings only two agents — the Director and the monitoring member" → only the monitoring member); `docs/concepts/overview.md` (`:46-48` mermaid — collapse the two `Monitor -. poll-trigger keystroke .-> PaneA/PaneB` edges to one wake-nudge edge to the monitoring member); `docs/spec/webui-api.md` (`:73` — add the root Director to the "never enrolled" list; `:58-66` — the `GET /api/agents` sample must not present the Director as enrolled); `docs/how-to/mixed-backend-team.md` (`:33-37` — "a Director … gets the same heartbeat" / "Run the monitor once … with `cafleet monitor start`" is invalidated: the Director no longer gets a heartbeat; the monitoring member is the canonical runner). `docs/concepts/tmux-push.md` needs **no** edit — it ties the poll-trigger keystroke to `cafleet member ping`, which is unchanged. (Correction, Director-arbitrated: `docs/concepts/token-reduction.md` DID need an edit — its lead paragraph still described the monitor injecting a per-tick `message poll` keystroke into the Director; rewritten so the monitor wakes only the monitoring member and the Director is re-engaged on demand. Its line-28 `cafleet member ping` poll-trigger reference is unchanged.) <!-- completed: 2026-06-14T12:14 -->

### Phase B: Code

- [ ] B1. `broker/fleets.py::create_fleet`: remove the `monitor.enroll_agent(session, director_agent_id)` call and its comment so the Director is no longer enrolled. <!-- completed: -->
- [ ] B2. `monitor/loop.py::monitor_tick`: remove the `is_director` keystroke arm (collapse to the monitoring-member-only branch with the defensive `else: continue`); update the in-branch comment and `should_ping`'s docstring to drop the "root Director" framing (the Director is no longer enrolled; `is_director` is kept only for the defensive skip + status). <!-- completed: -->
- [ ] B3. `broker/monitor.py`: update the `enroll_agent` and `list_monitor_targets` docstrings — enrollment is restricted to exactly one role (the monitoring member); `is_director` is a derived field retained for the loop's defensive skip and `monitor status` labeling, not because a Director row is expected. <!-- completed: -->
- [ ] B4. `cli/monitor.py::monitor_start`: before `loop.run_monitor_loop`, warn (to stderr) when `list_monitor_targets(fleet_id)` contains no `is_monitoring_member` row, then run the loop unchanged (§4). <!-- completed: -->
- [ ] B5. Add Alembic revision `cafleet/db/alembic/versions/0004_prune_director_monitor_config.py` (`down_revision="0003"`): `upgrade()` deletes `monitor_config` rows whose `agent_id` is a fleet's `director_agent_id`; `downgrade()` is a no-op (§5). <!-- completed: -->

### Phase C: Tests

- [ ] C1. `tests/monitor/test_loop.py`: rewrite `test_monitor_tick__poll_to_director_wake_to_monitor_skips_ordinary` for the new world — the monitoring member receives the wake keystroke, the Director receives **no** keystroke (the `polls` list is empty), the Director is **not** enrolled (`get_monitor_config(sid, director_id) is None`), and only the monitoring member's `last_ping_at` advances. Rename it to reflect "wake-to-monitor-only; Director not pinged, not enrolled". <!-- completed: -->
- [ ] C2. `tests/broker/test_monitor.py`: assert `create_fleet` does **not** enroll the Director (`get_monitor_config(director) is None`); only a `--role monitor` member is enrolled; `list_monitor_targets` returns just the monitoring member. Update the existing Director-enrolled assertions by name: `test_enroll_on_create__director_enrolled_administrator_not` (`:74` — rename + invert to assert the Director is **not** enrolled); the `update_monitor_config` test that relies on "the Director is always enrolled" (`:202` — retarget to the monitoring member); `test_list_monitor_targets__director_and_monitoring_member_shape` (`:280` — rewrite to monitoring-member-only); and the `pending_count` tests (`:313-346`) that index `targets[director_id]` (retarget to the monitoring member). <!-- completed: -->
- [ ] C3. `tests/monitor/test_should_ping.py`: review fixtures — `should_ping` is role-agnostic so it still passes for any target dict, but remove/retarget any case that asserts the Director is enrolled-and-pinged via the loop world (keep pure-policy cases as-is). <!-- completed: -->
- [ ] C4. `tests/cli/test_monitor.py`: `monitor status` shows only the monitoring member (no Director row); `monitor start` prints the no-monitoring-member warning to stderr when none is enrolled but still invokes `run_monitor_loop`; `monitor start` with a monitoring member enrolled emits **no** warning. Update any sample-output assertion that expected a Director row. <!-- completed: -->
- [ ] C5. Migration tests: advance the alembic-smoke head assertions to `0004` / four revisions; add a data-migration test that `0004` deletes the Director's `monitor_config` row and leaves the monitoring member's row intact (mirroring the `0003` prune test). **Also UPDATE the existing** `tests/broker/test_monitor.py::test_prune_migration__deletes_non_director_rows_downgrade_noop` (`:543`): it upgrades to `head` (`:562`) and asserts the root-Director row `{10}` survives (`:563`, and again after downgrade `:567`) — with `0004` as the new head, row 10 is now deleted, so pin its upgrade to `"0003"` (testing `0003` in isolation) or adjust its post-head assertions. <!-- completed: -->
- [ ] C6. `tests/webui/test_monitor_api.py`: retarget the Director-enrolled assertions to the monitoring member — `test_get_agents__monitor_field_folded` (`:139-144`, whose comment reads "the two enrolled roles — the root Director and the dedicated monitoring member" and asserts the Director's folded `monitor` is enabled), `test_get_agent_monitor__200_for_enrolled` (`:154`), and `test_patch_agent_monitor__updates_interval_and_enabled` (`:185`) plus the other patch tests that use `director_id` as THE enrolled agent. After the change the Director's folded `monitor` is `null` and `GET /api/agents/{director}/monitor` 404s; the monitoring member is the enrolled target. <!-- completed: -->

### Phase D: Verification

- [ ] D1. `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test` all pass. Sweep for residual Director-heartbeat / orphaned `monitor start` references across `docs/`, `README.md`, `skills/`, `.claude/`, root `CLAUDE.md`, `cafleet/.claude/` with a regex broad enough to catch non-contiguous phrasings (e.g. `git grep -nIE "pings? the (root )?Director|Director \+ monitoring member|Director and the monitoring member|two (enrolled|agents)|Esc.?\+.?poll|poll-trigger|gets the same heartbeat"`); the only legitimate remaining hits are under `design-docs/` (history). Note: `cafleet member ping` / `send_poll_trigger` references are **not** residuals — that keystroke is unchanged — so do not "fix" them. <!-- completed: -->
- [ ] D2. Manual smoke (operator, optional): with a monitoring member live, confirm the loop wakes the monitoring member but never keystrokes the Director's pane; confirm `monitor start` in a fleet with no monitoring member prints the warning and still runs; confirm `cafleet member ping` still keystrokes the Director's pane (`send_poll_trigger` intact). <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-14 | Initial draft |
| 2026-06-14 | Reviewer pass: carved out `cafleet-design-doc-execute` (keeps a monitoring member; Step-7 Copilot loop reframed onto the monitoring member's idle-nudges) per user decision — only 4 skills drop the monitor (A8/A9 split); added the invalidated-docs task (A10) and the `test_monitor_api.py` task (C6); named the specific `test_monitor.py` tests in C2; C5 now updates the existing `0003` prune test; aligned §4 prose to `ensure_tmux_or_die` ordering; broadened the D1 sweep regex |
| 2026-06-14 | Reviewer pass 2: pinned the execute carve-out's nudge trigger — execute's monitoring member runs an **extended** routine that nudges the idle Director **unconditionally** (no Step-7 handshake) to grant the `silence_ticks` re-poll turn; scoped the "routine unchanged from 0000090" claim to the canonical conditional nudge (Topology) and put the execute spawn-prompt delta in A9 |
