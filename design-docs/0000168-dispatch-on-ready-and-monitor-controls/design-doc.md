# Dispatch-on-Ready and Monitor Controls

**Status**: Approved
**Progress**: 0/21 tasks complete
**Last Updated**: 2026-08-16

## Overview

Fix two fleet-operability gaps (GitHub issues #312 and #313) with three changes: the Director dispatches each member's first task the moment that member reports ready instead of holding dispatch behind an all-members barrier; the skill instructions unambiguously make the monitor member the party that starts the `cafleet monitor` wake loop; and the admin WebUI gains a "Wake now" control that force-triggers a monitor wake outside the schedule. Changes 1 and 2 are documentation/skills-only; change 3 is the sole code change (migration V7, loop support, `POST /api/monitor/wake`, WebUI button).

## Success Criteria

- [ ] The canonical supervision protocol states the dispatch-on-ready rule in one place, and all four team workflow bodies (design-doc create, design-doc execute, research report, research presentation) plus their Director role files are consistent with it — no workflow instructs the Director to hold a ready member's first dispatch until other members are ready.
- [ ] The monitor member's role file and the Director-side supervision governance both state that the monitor member — and only the monitor member — launches `cafleet monitor <fleet-id>`.
- [ ] `POST /api/monitor/wake` against a fleet with a live monitor loop returns 200 and the loop delivers a wake within one tick (default 5 s), even when `wake_interval_seconds = 0`.
- [ ] `POST /api/monitor/wake` against a fleet whose monitor loop is not running returns 404.
- [ ] The WebUI monitor popover shows a "Wake now" button that triggers the endpoint and is disabled while the monitor is stopped.
- [ ] Migration chain is contiguous 1..7 with head V7; `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //admin:lint` pass.

---

## Background

Issue #312 ("got stuck right after members initialization"): after spawning a team, the Director sat idle without dispatching any first task. The canonical supervision protocol already prescribes per-member handling ("ACK, dispatch first task" on each ready arrival; "react to each arrival as its own wake-up"), but the workflow bodies layer an all-members placement barrier on top — `create.md` § 1f and `execute.md` § 3f both require every member to show `status: active` with a non-null `pane_id` before the workflow proceeds — and nothing states explicitly that first-task dispatch must not wait for the rest of the team.

Issue #313 ("monitor on appropriate member"): `cafleet monitor <fleet-id>` checks only fleet liveness, multiplexer availability, and the single-instance runtime claim — nothing about who runs it. The monitor member's role file instructs it to launch the loop, but no surface states that it is the *only* legitimate launcher, so a Director (or an operator following ad-hoc advice) can start the loop from the wrong pane.

Finally, monitor wakes fire only on the fleet-level schedule (`wake_interval_seconds`, default 600 s). When an operator sees a stuck fleet in the WebUI, there is no way to trigger a health-check pass now — the only knob is `PATCH /api/monitor`, which changes the cadence but cannot fire an immediate wake, and cannot fire any wake at all when the interval is 0.

Both GitHub issues have empty bodies; the scope above was confirmed through the clarification round with the user.

---

## Specification

### Change 1 — Dispatch-on-ready (documentation/skills only)

No CLI or server code changes. The canonical rule lands in `skills/cafleet/reference/supervision.md` (the single owner); every other surface echoes it in one sentence with a pointer.

**Canonical rule** (wording to be added to supervision.md, in the spawn-and-verify / facilitation area):

> **Dispatch-on-ready.** When a member's ready signal arrives, ACK it and dispatch that member's first task in the same turn, provided the task's inputs exist. First-task dispatch is per-member: never hold a ready member's dispatch waiting for other members' ready signals or placements. A member whose first task genuinely depends on an input that does not yet exist (e.g. a deliverable another member has not produced) legitimately stays idle until that input lands — dispatch whatever is dispatchable, to whoever is ready.

| Surface | Edit |
|---|---|
| `skills/cafleet/reference/supervision.md` | Add the canonical rule; align the end-turn decision table rows ("just spawned", "waiting on multiple members") and facilitation step 3 with it. |
| `skills/cafleet-design-doc/create/create.md` | Reframe § 1f "Verify members are live" as a spawn-health placement audit that does not gate Step 2 or any dispatch: the Director acts on each ready signal as it arrives (the Drafter's first task is embedded in its spawn prompt and needs no separate dispatch). |
| `skills/cafleet-design-doc/execute/execute.md` | Reframe § 3f the same way; Step 4's first dispatch to the Tester is keyed to the Tester's ready signal (its input, the approved design doc, already exists), not to full-team placement. |
| `skills/cafleet-research/report/report.md`, `skills/cafleet-research/presentation/presentation.md` | One-sentence echo of the rule at the spawn blocks (members are spawned on demand; each gets its first task on its own ready). |
| The four workflow Director role files (`cafleet-design-doc/create/roles/director.md`, `cafleet-design-doc/execute/roles/director.md`, `cafleet-research/report/roles/director.md`, `cafleet-research/presentation/roles/director.md`) | One-sentence echo with a pointer to supervision.md. |
| `docs/docs/concepts/monitoring.md` (the page owning the spawn/ready lifecycle) | State dispatch-on-ready in the Lifecycle → Spawn narrative. |

The `monitor live` gate is unchanged: the monitor member is still spawned first, and its `monitor live` signal still gates the first ordinary `member create`. Dispatch-on-ready governs what happens *after* each ordinary member's ready signal.

### Change 2 — Monitor-loop launch ownership (documentation/skills only)

Decision (user-confirmed): **no CLI enforcement**. There is no pane-identity check and no exit-1 path in `cafleet monitor`; the fix is unambiguous instruction. `cafleet monitor scan` also stays unrestricted — it is read-only and useful from any pane for debugging.

**Ownership statement** (affirmative, paired with the prohibition):

> The fleet's monitor member launches `cafleet monitor <fleet-id>` in its own pane as part of its startup sequence, and is the only party that does so. The Director never runs the loop itself — it spawns the monitor member and waits for `monitor live`. Ordinary members never run it.

| Surface | Edit |
|---|---|
| `skills/cafleet/roles/monitor.md` | Add the ownership statement to the Startup sequence (which already instructs the launch). |
| `skills/cafleet/reference/supervision.md` | Director-side governance: the Director never launches the loop; the monitor member owns the launch. |
| `skills/cafleet/SKILL.md` § Team supervision | Align the existing description with the ownership statement (smallest edit). |
| `docs/docs/concepts/monitoring.md` | State the ownership in the lifecycle narrative. |
| `docs/docs/spec/webui-api.md` | Already says "run by the monitor member" — verify, no drift expected. |

### Change 3 — Forced monitor wake (the only code change)

Mechanism (user-confirmed): a durable wake request persisted in `monitor_runtime`, honored by the loop on its next tick (≤ one tick latency, default 5 s). This works regardless of where the server process runs and respects the loop's single-instance ownership model — the server never keystrokes panes for this feature.

#### Data model — migration V7

```sql
-- cafleet/migrations/V7__monitor_wake_request.sql
ALTER TABLE monitor_runtime ADD COLUMN wake_requested_at TEXT;
```

`wake_requested_at` is NULL when no request is pending, else the UTC ISO timestamp of the latest operator request. Repeat requests overwrite the timestamp — they coalesce into a single wake.

#### Broker (`cafleet/src/broker/monitor.rs`)

| Function | Change |
|---|---|
| `request_monitor_wake(conn, fleet_id, when) -> Result<bool, CafleetError>` (new) | `UPDATE monitor_runtime SET wake_requested_at=?1 WHERE fleet_id=?2`; `false` ⇔ no row. Ownership-free, mirroring `set_monitor_wake_interval`. |
| `claim_monitor_runtime` | The reclaim UPDATE resets `wake_requested_at = NULL` — a pending request never survives into a later loop instance. The INSERT branch creates the row with the column NULL by default (no reset needed: it fires only when no row exists). |
| `record_monitor_wake` | Becomes `UPDATE monitor_runtime SET last_wake_at=?1, wake_requested_at=NULL WHERE fleet_id=?2` — the one write that clears the request exactly when a wake actually fired (a scheduled wake also clears any pending request; the wake the operator asked for has happened). |
| `read_monitor_runtime` | Include `wake_requested_at` in the returned payload (the loop reads it per tick). |

`clear_monitor_runtime` is unchanged — the claim-time reset is the single guard against stale requests.

#### Loop semantics (`cafleet/src/monitor/mod.rs` `monitor_tick`)

After the per-tick runtime re-read, `forced = wake_requested_at is non-null`:

1. When `forced`, skip both the `wake_interval == 0` early-return and the `wake_due` gate — an explicit operator action bypasses a disabled or not-yet-due schedule.
2. The pane-resolution skips (no active monitor member, no pane, pane not live) are unchanged and do not consume the request: on a skipped wake the request stays pending and retries next tick, matching the scheduled wake's "stays due" semantics.
3. On a delivered wake, `record_monitor_wake` stamps `last_wake_at` and clears the request in one write. A forced wake therefore resets the schedule baseline.
4. Echo line for a wake fired with a pending request: `{iso} tick -> forced wake monitor {monitor_id} ({N} members)`; the scheduled line is unchanged.

#### HTTP API — `POST /api/monitor/wake`

Registered alongside the existing monitor routes: `.route("/monitor/wake", post(post_monitor_wake))`. No request body; any body is ignored. Shared `X-Fleet-Id` dependency, same order as the other fleet-scoped endpoints.

| Condition | Response |
|---|---|
| Missing/empty `X-Fleet-Id` | 400 `{"detail": "X-Fleet-Id header required"}` |
| Non-integer `X-Fleet-Id` | 400 `{"detail": "X-Fleet-Id must be an integer"}` |
| Unknown fleet | 404 `{"detail": "Fleet not found"}` |
| Monitor loop not running (`monitor_is_live` false: no runtime row, cleared slot, or stale heartbeat) | 404 `{"detail": "monitor is not running for this fleet"}` |
| Success | 200 `{"wake_requested_at": "<UTC ISO>"}` |

Decision: the 404 gate is **liveness**, not row-existence as in `PATCH /api/monitor`. A wake request needs a live consumer — against a dead loop it would silently never fire — whereas the interval is a durable setting. The check-then-write pair is not transactional; the race is benign because `claim_monitor_runtime` clears any request left by a previous instance. If `request_monitor_wake` returns `false` after the liveness gate passed (the row vanished between the check and the write, e.g. a concurrent `fleet delete`), the handler returns the same 404 `{"detail": "monitor is not running for this fleet"}`.

`GET /api/monitor` payload is unchanged — the UI does not need to display a pending request, and the smallest contract edit wins.

#### WebUI (`admin/`)

| File | Change |
|---|---|
| `admin/src/api.ts` | `postMonitorWake(): Promise<{ wake_requested_at: string }>` → `POST /monitor/wake` (fleet header injected by `request`). |
| `admin/src/components/AppHeader.tsx` (`MonitorIndicator` popover) | A "Wake now" button beneath the wake-interval control. Disabled when `!running` or while a request is in flight (spinner, matching the Save button's pattern). On success: show a transient note "Wake requested — fires within one tick", and fire the existing refresh callback so the polled payload picks up the new `last_wake_at` once the wake lands. Errors render in the popover's existing error slot. |

`MonitorRuntime` in `admin/src/types.ts` is unchanged (GET payload unchanged).

#### Tests

| Area | Cases |
|---|---|
| Broker | `request_monitor_wake` returns `true`/`false` on row/no-row; a reclaim resets a pending request, and a fresh claim reads `wake_requested_at = NULL`; `record_monitor_wake` clears the request. |
| Monitor tick | Forced wake fires when `wake_interval_seconds = 0`; forced wake fires when the schedule is not yet due; a skipped wake (dead/missing pane) leaves the request pending; a delivered forced wake clears the request, stamps `last_wake_at`, and emits the `forced wake` echo line. |
| Routes (`cafleet/tests/webui_routes.rs`) | 400 on missing/non-integer header; 404 unknown fleet; 404 for each not-running shape — no runtime row, stale heartbeat, and cleared slot (`pid` NULL after a clean loop exit, the common "monitor stopped" case); 200 stamps `wake_requested_at`. |
| Chain guard | `cafleet/src/db/mod.rs` contiguity test bumped to head `(7, "monitor_wake_request")` with versions 1..7; `cafleet/tests/cli_setup_doctor.rs` head assertions updated (`applied migrations to head (7).`, `schema 7 (head)`, `head_version: 7`). |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first, per the project's documentation-maintenance rule; code only after Steps 1–2.

### Step 1: User-facing documentation (docs/, SPEC.md)

- [ ] `docs/docs/spec/webui-api.md`: add `POST /api/monitor/wake` to the routes table and as a full section (contract table above, verbatim error strings); note the liveness-vs-row-existence 404 distinction next to the PATCH section's shared-dependency note. <!-- completed: -->
- [ ] `docs/docs/spec/data-model.md`: add `wake_requested_at` to the `monitor_runtime` columns with its NULL/coalescing semantics and both clearing writes (a delivered wake via `record_monitor_wake`, and the reclaim reset in `claim_monitor_runtime`). <!-- completed: -->
- [ ] `docs/docs/concepts/monitoring.md`: dispatch-on-ready in the Lifecycle → Spawn narrative; monitor-loop launch ownership; a forced-wake paragraph in the cadence section (bypasses a disabled/not-due schedule, resets the baseline, ≤ one tick latency). <!-- completed: -->
- [ ] `docs/docs/how-to/use-the-webui.md`: document the "Wake now" control alongside the wake-interval control, and correct the page's summary of what the UI can write. <!-- completed: -->
- [ ] `SPEC.md`: `monitor_runtime` DDL with `wake_requested_at`; §6.6 loop semantics (forced-wake gate order, request consumption on a delivered wake, the claim-time reset — a pending request never survives into a later loop instance — and the echo line); §6.8 the new route's full contract. <!-- completed: -->

### Step 2: Skills (changes 1 and 2)

- [ ] `skills/cafleet/reference/supervision.md`: add the canonical dispatch-on-ready rule and align the end-turn decision table + facilitation step; add the Director-side loop-ownership statement. <!-- completed: -->
- [ ] `skills/cafleet-design-doc/create/create.md`: reframe § 1f as a non-gating placement audit; Step 2 proceeds on the Drafter's ready. <!-- completed: -->
- [ ] `skills/cafleet-design-doc/execute/execute.md`: reframe § 3f; key Step 4's first dispatch to the Tester's ready signal. <!-- completed: -->
- [ ] `skills/cafleet-research/report/report.md` and `skills/cafleet-research/presentation/presentation.md`: one-sentence dispatch-on-ready echo at the spawn blocks. <!-- completed: -->
- [ ] The four workflow Director role files: one-sentence echo with a pointer to supervision.md. <!-- completed: -->
- [ ] `skills/cafleet/roles/monitor.md`: ownership statement in Startup; `skills/cafleet/SKILL.md` § Team supervision aligned. <!-- completed: -->

### Step 3: Migration V7

- [ ] Add `cafleet/migrations/V7__monitor_wake_request.sql` (the `ALTER TABLE` above). <!-- completed: -->
- [ ] Update the chain-guard tests: `cafleet/src/db/mod.rs` (head `(7, "monitor_wake_request")`, versions 1..7) and every head assertion in `cafleet/tests/cli_setup_doctor.rs`. <!-- completed: -->

### Step 4: Broker and monitor loop

- [ ] `cafleet/src/broker/monitor.rs`: add `request_monitor_wake`; reset `wake_requested_at` in `claim_monitor_runtime`'s reclaim UPDATE; clear it in `record_monitor_wake`; expose it in `read_monitor_runtime`. <!-- completed: -->
- [ ] `cafleet/src/monitor/mod.rs`: the `forced` gate in `monitor_tick` (bypass interval-0 and `wake_due`; skips leave the request pending) and the `forced wake` echo line. <!-- completed: -->
- [ ] Broker + tick tests per the Tests table. <!-- completed: -->

### Step 5: HTTP route

- [ ] `cafleet/src/webui/mod.rs`: register `/monitor/wake` and implement `post_monitor_wake` (header deps → fleet check → `monitor_is_live` gate → `request_monitor_wake` → 200 payload). <!-- completed: -->
- [ ] Route tests in `cafleet/tests/webui_routes.rs` per the Tests table. <!-- completed: -->

### Step 6: WebUI frontend

- [ ] `admin/src/api.ts`: `postMonitorWake`. <!-- completed: -->
- [ ] `admin/src/components/AppHeader.tsx`: the "Wake now" button with disabled/in-flight/success/error states. <!-- completed: -->

### Step 7: Verification

- [ ] `mise //admin:lint`, `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` all pass. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-16 | Initial draft |
| 2026-08-16 | Reviewer round 1: claim-time reset scoped to the reclaim branch; row-vanished 404 specified; cleared-slot route test added; `wake_requested_at` lifecycle made explicit in the SPEC/data-model tasks |
