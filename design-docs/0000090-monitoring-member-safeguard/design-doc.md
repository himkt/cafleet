# Safe Director-only Monitoring with a Dedicated Monitoring Member

**Status**: Approved
**Progress**: 0/26 tasks complete
**Last Updated**: 2026-06-14

## Overview

Close the `cafleet monitor` safety hole where a bare `Enter` keystroke confirms a coding agent's pending permission prompt. Replace the blind "ping every member" loop with an `Esc`-safeguarded heartbeat that targets only the Director plus one new, dedicated **monitoring member** — a member that watches the Director with its own LLM judgment and re-engages it when the team stalls.

## Success Criteria

- [ ] Every monitor keystroke ping sends `Esc` before the literal text + `Enter`, so a pending permission prompt can never be confirmed by the trailing `Enter`.
- [ ] The monitor loop pings **only** the root Director and the monitoring member; ordinary members are never enrolled and never pinged by the loop.
- [ ] A monitoring member can be spawned via `cafleet member create --role monitor`, is marked `agent_card_json.cafleet.kind == "monitoring-member"`, and is the one process that runs `cafleet monitor start`.
- [ ] The monitoring member captures the Director's pane, classifies it active vs idle, and on idle re-engages **the Director** (never drives ordinary members directly).
- [ ] `send_resume_trigger` (the blind ordinary-member resume nudge) is removed entirely — the repo reads as if member-pinging never existed.
- [ ] `mise //cafleet:lint`, `:format`, `:typecheck`, and `:test` all pass; docs, README, and every affected SKILL are consistent with the new behavior.

---

## Background

`cafleet monitor` is a per-fleet `scan → ping → sleep` loop. Today it has two problems, both verified in code:

1. **Unsafe keystroke.** `tmux.py::_send_literal_then_enter` does `send-keys -l <text>` → `sleep 0.12s` → `send-keys Enter`, with **no `Esc`** first. If the target pane is a coding agent sitting on a permission-approval prompt, the agent ignores the literal text but the trailing bare **`Enter` confirms the pending permission prompt** — the permission guard is bypassed.
2. **Blind fan-out.** `monitor/loop.py::monitor_tick` pings **every** enrolled agent each tick: the Director via `send_poll_trigger` and **all members** via `send_resume_trigger`. Every pane-bound agent is auto-enrolled at registration (`broker/agents.py::register_agent` → `monitor.enroll_agent`), so the loop keystrokes into every member's pane unconditionally.

The fix combines a mechanical safeguard (the `Esc` prefix) with an architectural shift: a single dedicated **monitoring member** owns the `monitor start` background task and applies LLM judgment to the Director's state, while the dumb loop is narrowed to just the Director and that monitoring member.

---

## Specification

### Locked decisions (confirmed with the user)

| # | Decision |
|---|---|
| A | Spawn interface is `cafleet member create --role monitor` — a `--role {member,monitor}` option (default `member`), **not** a boolean flag and **not** a dedicated subcommand. |
| A | Only the root Director and the monitoring member keep a `monitor_config` row (ordinary-member auto-enroll is dropped). The monitoring member is marked `agent_card_json.cafleet.kind == "monitoring-member"` (no new SQL column). An Alembic data step deletes pre-existing non-Director `monitor_config` rows. |
| B | **Single** `Esc` per ping: `Escape` → ~0.1s settle → literal text → 0.12s → `Enter`, applied to **both** the Director poll and the monitoring-member wake. |
| C | The loop's periodic `Esc`+poll heartbeat to the Director stays **unconditional** — the rare mid-turn interruption is accepted (Directors idle between short facilitation turns; the next tick re-engages), and the ping is **not** gated. |
| D | The monitoring member runs on Sonnet 4.6 — spawned with `--model sonnet` (`claude-sonnet-4-6`). |
| — | Bootstrap is first-in (monitoring member spawned before ordinary members); teardown is first-out (monitoring member deleted first); `monitor start` runs in the monitoring member's pane, not the Director's. |

### Topology

```
monitor loop (runs in the monitoring member's pane)
  ├─ Esc + `cafleet … message poll`  → Director pane      (facilitation heartbeat)
  └─ Esc + wake nudge                → monitoring member  (its own pane's foreground)

monitoring member (on each wake)
  └─ capture Director pane → classify active vs idle
       ├─ ACTIVE → do nothing
       └─ IDLE   → assess inbox + Director task + member panes (read-only),
                   then re-engage the DIRECTOR with an Esc-safeguarded nudge.
                   Never keystrokes ordinary members with task instructions.
```

The monitoring member is itself enrolled, so the loop (running inside its own pane as a background task) keystrokes the wake nudge into that same pane's foreground — a deliberate self-ping that drives the capture-and-assess routine each tick. The leading `Esc` will interrupt the monitoring member's own in-progress turn if a wake lands while it is mid-routine — the same accepted interruption that decision C grants the Director. Because the 60s ping interval is far longer than a routine's duration, the overlap is rare and the self-interrupt is acceptable (this does not re-open the single-`Esc` decision). Ordinary members are **not** enrolled and receive nothing from the loop.

### 1. The `Esc` keystroke safeguard

The `Esc` prefix is opt-in per keystroke helper — it must **not** apply to `send_exit`, `send_inline_preview`, `send_bash_command`, or `send_freetext_and_submit` (an `Esc` before `/exit` or `! <cmd>` would mis-fire). A new `esc_first` parameter threads it through only the ping helpers.

```python
# tmux.py
_ESC_SETTLE_DELAY = 0.1  # let the agent dismiss a pending prompt and settle after Esc

def _send_literal_then_enter(
    *, target_pane_id, payload, timeout=None, ignore_missing=False, esc_first=False,
):
    if esc_first:
        _run_tolerating_pane_gone(
            ["tmux", "send-keys", "-t", target_pane_id, "Escape"],
            ignore_missing=ignore_missing, timeout=timeout,
        )
        time.sleep(_ESC_SETTLE_DELAY)
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "-l", payload],
        ignore_missing=ignore_missing, timeout=timeout,
    )
    time.sleep(_SUBMIT_DELAY)
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "Enter"],
        ignore_missing=ignore_missing, timeout=timeout,
    )
```

The full ping sequence is therefore: **`Escape` → 0.1s → `send-keys -l <text>` → 0.12s → `Enter`** (tmux's key name for the escape key is `Escape`).

`send_poll_trigger` and the new `send_wake_trigger` both pass `esc_first=True`. Because the Director's manual nudge `cafleet member ping` reuses `send_poll_trigger`, it inherits the safeguard for free — a desirable side effect (manual nudges hit the same permission-prompt hole).

### 2. Keystroke helpers

| Helper | Used by | Change |
|---|---|---|
| `send_poll_trigger` | loop (Director), `member ping` | `esc_first=True`. Payload unchanged (`cafleet … message poll`). |
| `send_wake_trigger` (**new**) | loop (monitoring member) | `esc_first=True`. Single-line nudge instructing the monitoring member to run its capture-and-assess routine (no shell-special chars, sane whether it lands in the agent input or a shell prompt). |
| `send_resume_trigger` | loop (ordinary members) | **Removed entirely** — no other caller exists. Apply the project removal rule: delete the method, its docstring, and every doc/skill mention. |

Proposed `send_wake_trigger` payload:

```
[monitor] wake: run your monitoring routine now — capture the Director pane,
judge it active vs idle, and if idle assess the inbox and members and re-engage
the Director with an Esc-safeguarded nudge.
```

### 3. Identification and enrollment

`is_director` is already *derived* (`agent_id == fleets.director_agent_id`). The monitoring member is identified the same way the Administrator is — a `kind` marker in `agent_card_json`:

```python
# broker/_shared.py
MONITORING_MEMBER_KIND = "monitoring-member"

def is_monitoring_member(agent_card_json: str | None) -> bool:
    if not agent_card_json:
        return False
    try:
        kind = json.loads(agent_card_json).get("cafleet", {}).get("kind")
    except ValueError:
        return False
    return kind == MONITORING_MEMBER_KIND
```

Enrollment becomes selective. The root Director is still enrolled in `create_fleet` (unchanged). In `register_agent`, the unconditional `monitor.enroll_agent` call is gated on the new `kind`:

```python
# broker/agents.py — register_agent gains `kind: str | None = None`
agent_card = {"name": name, "description": description, "skills": skills or []}
if kind is not None:
    agent_card["cafleet"] = {"kind": kind}
...
if placement is not None:
    session.add(AgentPlacement(...))
    if kind == _shared.MONITORING_MEMBER_KIND:
        monitor.enroll_agent(session, agent_id)   # ordinary members: no row
```

Because `list_monitor_targets` joins on `monitor_config`, restricting enrollment to `{Director, monitoring member}` makes the per-tick scan return exactly those two rows — no extra filter in the loop. `list_monitor_targets` additionally surfaces `is_monitoring_member` (derived from the card) so the loop selects the keystroke explicitly and `monitor status` can label the role.

**One monitoring member per fleet.** The guard lives in **`register_agent`** — the single enforcement site; the `member create` CLI passes `kind` straight through and does **not** re-check. When `kind == MONITORING_MEMBER_KIND`, `register_agent` queries for an existing active monitoring member in the same fleet before inserting, reusing the `func.json_extract(Agent.agent_card_json, "$.cafleet.kind")` pattern already used in `agents.py`:

```python
# broker/agents.py — inside register_agent, when kind == MONITORING_MEMBER_KIND
existing = session.execute(
    select(Agent.agent_id).where(
        Agent.fleet_id == fleet_id,
        Agent.status == "active",
        func.json_extract(Agent.agent_card_json, "$.cafleet.kind")
        == _shared.MONITORING_MEMBER_KIND,
    )
).first()
if existing is not None:
    raise click.ClickException(
        f"fleet {fleet_id} already has an active monitoring member "
        f"(agent {existing.agent_id}); only one is allowed."
    )
```

### 4. Loop keystroke selection

```python
# monitor/loop.py — monitor_tick
for target in broker.list_monitor_targets(fleet_id):
    target["pane_alive"] = target["pane_id"] in live_panes
    if not should_ping(target, now):
        continue
    if target["is_director"]:
        keystroke = mux.send_poll_trigger
    elif target["is_monitoring_member"]:
        keystroke = mux.send_wake_trigger
    else:
        continue  # defensive: a stray/legacy enrolled row is skipped, never woken
    keystroke(target_pane_id=target["pane_id"], fleet_id=fleet_id, agent_id=target["agent_id"])
    ...
```

The branch is explicit on `is_director` / `is_monitoring_member`, realizing §3's "select the keystroke explicitly" purpose. The defensive `else: continue` means a stray or legacy `monitor_config` row (e.g. the §8 migration racing a live fleet, or any future enrollment broadening) is **skipped**, never mis-keystroked with a wake nudge into an ordinary member's pane. `should_ping`'s policy is unchanged (interval/enabled/pane-liveness); only its docstring/comments are updated to drop the "every member alike" framing.

### 5. The `--role` option (CLI)

`cli/member.py::member_create` gains:

```python
@click.option(
    "--role", "role",
    type=click.Choice(["member", "monitor"]),
    default="member", show_default=True,
    help="Member role. 'monitor' spawns the dedicated monitoring member.",
)
```

When `role == "monitor"`, the CLI passes `kind="monitoring-member"` into `broker.register_agent`. The model is selected by the existing `--model` flag (the Director passes `--model sonnet`); `--role` controls only the kind marker and enrollment.

### 6. The monitoring member's role and bootstrap

The monitoring member is a Sonnet coding-agent member with a monitoring-only spawn prompt. Its lifecycle:

1. **Spawned first.** The Director runs `cafleet doctor` (env check), then `cafleet member create --role monitor --model sonnet --prompt-file <monitor-role-prompt>` **before** any ordinary `member create`. The Director's `agent_id` is baked into the spawn prompt exactly as for any member (`resolve_prompt(ctx, director_agent_id, new_member_id, …)`).
2. **Boots and reports.** First Bash call sends the ready signal to the Director.
3. **Launches the heartbeat.** It runs `cafleet --fleet-id <fleet> monitor start` as a **background task in its own pane** (the loop blocks, so it is backgrounded), confirms via `cafleet monitor status`, then reports `ready: monitor live` to the Director via `cafleet message send`. That report is the gate for spawning ordinary members (§7). This is the only `monitor start` in the fleet — the Director no longer runs it.
4. **On each wake** (the loop's `Esc`+wake keystroke into its own pane): `cafleet member capture --member-id <director-id>` (resolves the Director pane internally — `member capture` accepts any in-fleet agent with a placement, the root Director included), classify the Director **active** vs **idle** with its own judgment.
   - **ACTIVE** → do nothing.
   - **IDLE** → assess the full picture (Director inbox state, the Director's current task, ordinary members' panes via read-only `cafleet member capture`), then **re-engage the Director** with a concise nudge via `cafleet message send --to <director-id>` summarizing what needs attention (un-ACKed inbox items, stalled members). It never issues task instructions to ordinary members directly — all member-driving routes back through the Director, who owns the whole task.

**How ordinary members are woken now.** With `send_resume_trigger` gone, the loop never nudges an ordinary member; re-engagement is always Director-mediated. The wake paths are: (1) **primary** — the broker's inline-preview keystroke fired on every `cafleet message send` (`tmux.send_inline_preview`), landing the instant the Director or a teammate sends work; (2) **manual recovery** — the Director's `Esc`-safeguarded `cafleet member ping` (reuses `send_poll_trigger`), for a member that missed its inline preview or looks stalled. A member that has gone quiet is surfaced to the Director by the monitoring member's idle assessment; the Director then re-pings via `member ping` or re-sends the instruction. There is no automatic, unconditional member nudge anymore. (A5/A6 carry this into the monitoring and supervision skills.)

The canonical monitoring-member spawn prompt lives in the `cafleet-agent-team-monitoring` skill (the monitoring layer).

### 7. Lifecycle migration and teardown

- **Spawn order (first-in):** monitoring member → (monitor-live handshake) → ordinary members. The supervision Spawn Protocol's "ensure the monitor is running before the first `member create`" step is rewritten: the **first** `member create` *is* the monitoring member. **The monitor-live gate is the monitoring member's report**, not an independent Director check: after launching `monitor start` and confirming `cafleet monitor status` in its own pane (§6.3), the monitoring member sends `ready: monitor live` to the Director; receipt of that message gates the first ordinary `member create`. The Director MAY run `cafleet --fleet-id <f> monitor status` itself as optional corroboration, but it waits on the handshake message rather than block-polling status (consistent with the async wait rule).
- **Teardown order (first-out):** stop the monitor **before** the monitoring member's pane is killed. `run_monitor_loop` traps only SIGTERM/SIGINT — its `finally` clears `monitor_runtime` — and there is no `cafleet monitor stop` command, so the clean stop is: the Director messages the monitoring member to **stop its `monitor start` background task** (the coding agent's task-stop delivers SIGTERM/SIGINT, so the loop runs `finally` and clears the runtime row); the monitoring member confirms; only then does the Director `cafleet member delete` it. Then delete ordinary members, then `cafleet fleet delete`. **Degraded fallback:** if the background task cannot be stopped cleanly and the pane is killed directly, the resulting SIGHUP terminates the loop **without** running `finally`, leaving a stale `monitor_runtime` row — tolerated, because `status` reports stopped once the heartbeat goes stale and `fleet delete` removes the row outright. This is the accepted degraded path, not the default. Because the loop no longer pings ordinary members, the old "stop the monitor before deleting members" race **disappears for ordinary members** and now applies only to the monitoring member itself.

### 8. Data migration

A new Alembic revision runs a one-shot data step (no schema change):

```sql
DELETE FROM monitor_config
WHERE agent_id NOT IN (
    SELECT director_agent_id FROM fleets WHERE director_agent_id IS NOT NULL
);
```

Pre-upgrade there are no monitoring members, so this leaves exactly the root-Director rows enrolled. New monitoring members enroll going forward via the gated `register_agent` path. The downgrade is a no-op (re-enrolling every pane-bound agent is neither possible nor desirable).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first ordering per `.claude/rules/design-doc-numbering.md`: Phase A (docs/README/skills) lands before any code.

### Phase A: Documentation

- [ ] A1. Rewrite `docs/concepts/monitoring.md`: who-gets-pinged = Director + monitoring member only; the two `Esc`-safeguarded keystrokes (Director poll, monitoring-member wake); the monitoring member's capture-classify-reengage role; enrollment restricted to those two; lifecycle now runs `monitor start` in the monitoring member's pane. Delete all ordinary-member resume-nudge content. <!-- completed: -->
- [ ] A2. Update `docs/spec/data-model.md`: `monitor_config` is enrolled only for the root Director + the monitoring member; document `agent_card_json.cafleet.kind == "monitoring-member"`; note the Alembic data migration that prunes legacy non-Director rows. <!-- completed: -->
- [ ] A3. Update `docs/spec/cli-options.md`: add `cafleet member create --role {member,monitor}`; note the `Esc` safeguard on the monitor pings and `member ping`; refresh the `cafleet monitor` section. <!-- completed: -->
- [ ] A4. Update `README.md` monitoring summary: dedicated monitoring member + `Esc` safeguard + Director-and-monitoring-member-only heartbeat. <!-- completed: -->
- [ ] A5. Update `skills/cafleet-agent-team-monitoring/SKILL.md`: document the dedicated monitoring member, the canonical monitoring-member spawn prompt and its capture-classify-reengage routine, the two `Esc`-safeguarded keystrokes, and the replacement member-wake paths (inline preview primary, Director `member ping` manual recovery — §6); remove the member resume-nudge; update Monitor Lifecycle (start runs inside the monitoring member; first-in/first-out; the `ready: monitor live` handshake gate; stop the monitor background task before deleting the monitoring member). <!-- completed: -->
- [ ] A6. Update `skills/cafleet-agent-team-supervision/SKILL.md`: Spawn Protocol reorder (monitoring member is the first `member create` and starts the monitor; the monitoring member's `ready: monitor live` message gates the first ordinary `member create`); Stall Response notes that a quiet member is surfaced by the monitoring member and re-woken via the Director's `member ping`; Cleanup reorder (stop the monitor's background task, then delete the monitoring member first). <!-- completed: -->
- [ ] A7. Update `skills/cafleet/SKILL.md` + `skills/cafleet/reference/director.md`: `member create --role monitor`; the `Esc`-first `member ping` mechanism; the revised Shutdown Protocol ordering (stop the monitor's background task in the monitoring member → delete the monitoring member → delete ordinary members → `fleet delete`). <!-- completed: -->
- [ ] A8. Update `.claude/rules/bash-tool.md`: the `member ping` description becomes an `Esc` + `cafleet … message poll` + `Enter` keystroke (the leading `Esc` is the safeguard). <!-- completed: -->

### Phase B: Code

- [ ] B1. `cafleet/src/cafleet/multiplexer/tmux.py`: add `_ESC_SETTLE_DELAY = 0.1` and an `esc_first: bool = False` parameter to `_send_literal_then_enter` (Escape → settle → text → submit-delay → Enter). <!-- completed: -->
- [ ] B2. `tmux.py`: pass `esc_first=True` from `send_poll_trigger`; add `send_wake_trigger` (`esc_first=True`, the single-line monitoring-member wake nudge); **remove `send_resume_trigger` entirely**. <!-- completed: -->
- [ ] B3. `cafleet/src/cafleet/broker/_shared.py`: add `MONITORING_MEMBER_KIND = "monitoring-member"` and `is_monitoring_member(agent_card_json)` (mirrors `is_administrator`). <!-- completed: -->
- [ ] B4. `cafleet/src/cafleet/broker/agents.py::register_agent`: add `kind: str | None = None`; write `agent_card["cafleet"] = {"kind": kind}` when set; gate `monitor.enroll_agent` on `kind == MONITORING_MEMBER_KIND` (ordinary members no longer enrolled); reject a second active monitoring member per fleet. <!-- completed: -->
- [ ] B5. `cafleet/src/cafleet/broker/monitor.py::list_monitor_targets`: add `is_monitoring_member` to each row (derived from `agent_card_json` kind) for explicit keystroke selection and `status` labeling. <!-- completed: -->
- [ ] B6. `cafleet/src/cafleet/monitor/loop.py::monitor_tick`: select `send_poll_trigger` for the Director and `send_wake_trigger` for the monitoring member; update `should_ping` docstring/comments to drop the "every member alike" framing. <!-- completed: -->
- [ ] B7. `cafleet/src/cafleet/cli/member.py::member_create`: add `--role {member,monitor}` (default `member`); pass `kind="monitoring-member"` to `register_agent` when `--role monitor`. <!-- completed: -->
- [ ] B8. `cafleet/src/cafleet/cli/monitor.py::monitor_status`: label the monitoring member's role (e.g. `monitor`) in the agents table, derived from `is_monitoring_member`. <!-- completed: -->
- [ ] B9. New Alembic revision: data-only step deleting `monitor_config` rows for non-Director agents (§8); downgrade is a no-op. <!-- completed: -->

### Phase C: Tests

- [ ] C1. tmux multiplexer tests: assert the `Esc`-first sequence (`Escape` → text → `Enter`) for `send_poll_trigger` and `send_wake_trigger`; assert `esc_first=False` helpers (`send_exit`, `send_inline_preview`, `send_bash_command`) send **no** `Esc`; assert `send_resume_trigger` no longer exists. <!-- completed: -->
- [ ] C2. `tests/monitor/test_should_ping.py`: update fixtures for the Director + monitoring-member enrollment world. <!-- completed: -->
- [ ] C3. `tests/monitor/test_loop.py`: Director receives the poll keystroke, the monitoring member receives the wake keystroke, and a never-enrolled ordinary member is never pinged. <!-- completed: -->
- [ ] C4. `tests/broker/test_monitor.py`: enrollment is restricted to Director + monitoring member; `is_monitoring_member` / kind marker; `list_monitor_targets` surfaces the new field; the Alembic data migration prunes legacy rows. <!-- completed: -->
- [ ] C5. `tests/cli/test_monitor.py`: `monitor status` reflects the monitoring member's role. <!-- completed: -->
- [ ] C6. `tests/cli/test_member.py` (create path): `--role monitor` sets the kind marker and enrolls in `monitor_config`; an ordinary `--role member` create does **not** enroll; a second `--role monitor` spawn is rejected. <!-- completed: -->
- [ ] C7. `tests/cli/test_member_ping.py`: `member ping` now keystrokes `Esc` first (inherited from `send_poll_trigger`). <!-- completed: -->

### Phase D: Verification

- [ ] D1. `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test` all pass. <!-- completed: -->
- [ ] D2. Manual smoke: spawn a monitoring member (`--role monitor --model sonnet`), confirm `monitor status` is live, confirm an ordinary member receives no loop keystroke, and confirm the `Esc` prefix dismisses a pending permission prompt instead of confirming it. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-14 | Initial draft |
| 2026-06-14 | Reviewer pass: split Overview; documented the self-ping interrupt, member-wake recovery paths, the `monitor live` handshake gate, and the clean teardown stop (SIGTERM before pane-kill); pinned the monitor-uniqueness guard to `register_agent` with detection + error text; made the loop keystroke branch explicit |
