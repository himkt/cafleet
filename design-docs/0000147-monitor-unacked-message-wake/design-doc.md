# Monitor Unacked-Message Wake Reason

**Status**: Approved
**Progress**: 14/14 tasks complete
**Last Updated**: 2026-07-24

## Overview

Add a fourth monitor wake reason, `unacked`: a watched member whose oldest un-acked delivery (`status_state = 'input_required'`) has waited at least one full member interval is flagged due, waking the monitoring member so the Director can re-trigger the missed inbox poll with `cafleet member ping`. The broker-side message status becomes a supervision clue, closing the gap where a member misses the broker's auto-fired inline-preview keystroke and the delivery sits un-acked indefinitely (GitHub issue #219).

## Success Criteria

- [x] A watched member (root Director included) with an `input_required` non-`broadcast_summary` delivery older than its own `monitor_config.interval_seconds` is woken with wake reason `unacked`, re-flagged every `interval_seconds` while the delivery stays un-acked, and stops being flagged once every such delivery is acked.
- [x] An unacked-only wake never advances `last_ping_at` (`record_pings` keeps excluding it), and `should_ping` remains interval-only.
- [x] The wake-nudge payload (byte-identical on tmux and herdr) instructs the watcher to report an `unacked`-tagged member unless its pane classifies `awaiting_user` or `unknown` — including `working` panes — and its closing re-engagement sentence lists that unacked report alongside stalled / finished.
- [x] `cafleet monitor status` (text + `--json`) and WebUI `GET /api/monitor` expose `oldest_pending_ts` / `oldest_pending_age_seconds` per watched member, alongside `pending_count`.
- [x] `skills/cafleet/reference/supervision.md` documents `cafleet member ping` as the canonical, capture-gated Director response to an unacked report.
- [x] No new setting, no new `CAFLEET_*` env var, no schema change, no Alembic migration.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

The broker delivers a message by inserting an `input_required` row and best-effort keystroking an inline preview into the recipient's pane. When that keystroke is missed (pane mid-turn, renderer hiccup) the delivery stays `input_required` until a manual `cafleet message poll` or a Director `cafleet member ping` — but nothing currently *notices* the miss. The monitor loop already computes `pending_count` per watched member in `list_monitor_targets` yet deliberately ignores it: `should_ping` is interval-only by design, and no wake reason carries pending-message evidence. The message row itself holds the needed signal — `status_timestamp` is set at send time and only changes on ack, so a stale `input_required` row's age is exactly how long the recipient has failed to consume it.

---

## Specification

### Wake reason and threshold

The wake-reason set becomes `{"interval", "status:done", "stall-check", "unacked"}`.

| Property | Decision |
|---|---|
| Reason token | `unacked` (the `status:` prefix stays reserved for native agent status) |
| Staleness threshold | The member's own `monitor_config.interval_seconds` — 180 s for the root Director, 720 s for ordinary members by default, per-member tunable via `cafleet monitor config --member-id <m> --interval <s>` |
| Re-fire period | Also `interval_seconds`: the member is re-flagged every interval while a stale un-acked delivery remains |
| Coverage | All watched members uniformly, root Director included |
| Configuration | None added — no new setting, no new `CAFLEET_*` env var |
| Disable | No dedicated switch. `cafleet monitor config --member-id <m> --disable` (`enabled = 0`) silences the member from **all** triggers; `CAFLEET_MONITOR_STALL_INTERVAL=0` disables only stall-check and does **not** affect `unacked` |

Reusing `interval_seconds` couples staleness tolerance to check cadence by design: tightening a member's ping interval also tightens how long its deliveries may sit un-acked before the monitor reacts, and there is no way to tune one without the other. A delivery younger than one interval is never flagged — the normal deliver-then-ack cycle produces no wakes.

### Broker: `list_monitor_targets` row

Each scan row gains one field:

| Field | Type | Definition |
|---|---|---|
| `oldest_pending_ts` | `str \| None` | `MIN(status_timestamp)` over messages with `owner_member_id = member_id`, `status_state = 'input_required'`, `type != 'broadcast_summary'` (the `NOT_BROADCAST_SUMMARY` predicate); `None` when the member has no pending delivery |

Implemented as a second correlated scalar subquery beside `pending_sq` in `cafleet/src/cafleet/broker/monitor.py`, over the same predicates. The existing index `idx_messages_owner_member_status_ts` on `messages(owner_member_id, status_timestamp)` covers it. No schema change and no Alembic migration.

`should_ping` is unchanged: it consults neither `pending_count` nor `oldest_pending_ts`.

### Loop: `_flag_unacked_due` and bookkeeping

`cafleet/src/cafleet/monitor/loop.py` gains a third process-local map and a third flag helper, mirroring the stall-check pattern:

- **`_last_unacked_wake_at: dict[int, datetime]`** — last successful unacked wake per member. Cleared per run in `run_monitor_loop` alongside `_last_member_status` and `_last_stall_check_at`; never persisted.
- **`_flag_unacked_due(targets, due, now)`** — called from `monitor_tick` after `_flag_stall_check_due` and before `_flag_native_status_due`, so a multi-reason member's `wake_reasons` order is `interval`, `stall-check`, `unacked` (the native trigger only appends fresh targets). For each target, all of:
  1. `enabled` is true, `pane_id` is set, and `pane_alive` is true (skip otherwise, mirroring the other triggers);
  2. `oldest_pending_ts` is not `None` and `(now − oldest_pending_ts) ≥ interval_seconds` (the member's own interval, read from the same row);
  3. the re-fire gate passes: the member's `_last_unacked_wake_at` entry is absent, or `(now − entry) ≥ interval_seconds`;

  → union `unacked` onto the member's existing `wake_reasons`, or append the target with `["unacked"]`.
- **Commit gating in `monitor_tick`** — on `woke == True`, commit `_last_unacked_wake_at[id] = now` for every due member whose reasons include `unacked`, alongside the existing stall-check commit; a failed keystroke commits nothing, so the member re-flags next tick. The `ping_ids` filter is untouched — only `interval` / `status:done` reach `record_pings`, so an unacked-only wake never advances `last_ping_at` and the cadences stay independent.

The re-fire map is keyed by member, not by message. Consequence: after an unacked wake, a *new* stale episode beginning less than one interval later has its first wake delayed until the gate reopens — bounded by one `interval_seconds`, and accepted for the simplicity of one map entry per member.

### Wake-nudge payload

The due-list rendering (`<role> <id> (<name>) [<reasons>]`) needs no change — it joins `wake_reasons` generically. The single-line instruction changes in two places, byte-identical in `multiplexer/tmux.py` and `multiplexer/herdr.py`: it gains one sentence inserted after the stall-check sentence, and its closing re-engagement sentence is revised to list the unacked report (the "when an unacked-tagged member is reportable per its rule above" clause):

```
[monitor] wake: {N} {noun} due — {due_list}. Capture each named pane read-only, with the Director pane ({director_member_id}) always inspected. From capture content only, classify each pane in this precedence order: awaiting_user, unknown, finished, stalled, working. For a member tagged stall-check, compare its capture against your previous stall-check capture of that pane, then keep the new capture as that pane's baseline; with no previous stall-check capture, classify unknown. For a member tagged unacked, its oldest un-acked delivery has waited at least one full interval: report it to the Director unless its pane classifies awaiting_user or unknown — including working panes. Never re-engage a pane classified awaiting_user: when the Director is awaiting_user, send nothing this wake, whatever the other panes show. Otherwise re-engage the Director via cafleet message send when a due member is stalled or finished, when an unacked-tagged member is reportable per its rule above, or the Director is finished with un-acked work.
```

### Watcher routine

The monitoring member's routine keeps the five-state classification and its actions; the unacked rule is an additional reason-scoped report rule layered on top, like the stall-baseline rule is for `stall-check`:

- For a member tagged `unacked`, report it to the Director whenever its pane classifies `finished`, `stalled`, or `working` — the point is a missed poll trigger, so a busy pane is still reported and the Director decides.
- `awaiting_user` suppresses the report (the member is waiting on the user, not on a nudge), and `unknown` suppresses it too: an unreadable capture cannot rule out a pending prompt, so the existing fail-safe applies.
- The global suppression is unchanged: when the Director's own pane is `awaiting_user`, the watcher sends nothing this wake; the suppressed report re-surfaces on the re-fire cadence.

### Director response

`skills/cafleet/reference/supervision.md` documents `cafleet member ping` as the canonical response to an unacked report — it re-injects the missed `cafleet message poll` and is pre-approved in `permissions.allow`. The existing capture gate is unchanged and applies: the Director takes its own fresh `cafleet member capture` of the target and fires the ping only on `finished` or `stalled`; on `working` or `awaiting_user` it defers the round, relying on the re-fire cadence to resurface the report.

### `monitor status` and WebUI exposure

Each members row of the `{runtime, members}` payload gains two fields, mirroring the `last_ping_at` / `last_ping_age_seconds` pair:

| Field | Type | Definition |
|---|---|---|
| `oldest_pending_ts` | `str \| None` | Passed through from the scan row |
| `oldest_pending_age_seconds` | `int \| None` | `int((now − oldest_pending_ts).total_seconds())` computed with the payload's single `now`; `None` when no pending delivery |

The members-row assembly moves into a shared broker builder so the CLI and API payloads cannot drift: a new `monitor_members_payload(fleet_id, now) -> list[dict]` in `cafleet/src/cafleet/broker/monitor.py`, beside `monitor_runtime_payload`, builds each row (`member_id`, `name`, `role`, `interval_seconds`, `last_ping_at`, `last_ping_age_seconds`, `enabled`, `pending_count`, `oldest_pending_ts`, `oldest_pending_age_seconds`) from `list_monitor_targets`. `cli/monitor.py` replaces its inline loop with a call to this builder, and `webui/api.py` calls the same builder; each call site passes the one `now` it also passes to `monitor_runtime_payload`.

The text table appends an `unacked` column after `pending`, rendered with the existing `_format_ping_age` helper (`<age>s ago` / `-`); `pending` gains `:<7` padding now that it is no longer last:

```
  member_id  name         role      interval  last_ping  enabled  pending  unacked
  ---------  -----------  --------  --------  ---------  -------  -------  -------
  12         alice        member    720s      63s ago    yes      2        811s ago
```

WebUI `GET /api/monitor` gains an additive top-level `"members"` key carrying the shared builder's rows (including `pending_count` and the two new fields), computed with one `now` shared with the runtime dict. The existing flat runtime keys are unchanged, so the SPA's sole consumer (the `running` indicator in `Dashboard.tsx`) needs no change; exposure is API-level only, with no new SPA rendering.

### Invariants preserved

- `should_ping` stays interval-only; the monitoring member stays the unenrolled watcher; the loop's only keystroke stays the wake nudge into the watcher's own pane.
- All per-tick bookkeeping commits (`record_pings`, `_last_stall_check_at`, `_last_member_status` pending reads, `_last_unacked_wake_at`) stay gated on `woke == True`.
- The stdout heartbeat line format is unchanged — `unacked` simply appears in the joined `[<reasons>]` suffix.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/concepts/monitoring.md`: add `unacked` to the wake-reason list and the watched-set section, add the watcher's unacked report rule, and add a cadence-table row stating the threshold/re-fire period is the member's own `interval_seconds` (no knob) <!-- completed: 2026-07-24T11:45 -->
- [x] Update `SPEC.md` §6.2: `list_monitor_targets` row shape gains `oldest_pending_ts`, and add a `monitor_members_payload(fleet_id, now)` bullet with its exact row fields (per § *`monitor status` and WebUI exposure*); §6.6: extend `monitor_tick` step 5 with the unacked trigger bullet, add an *Unacked-delivery due trigger* subsection (due condition, re-fire gate, commit gating, `record_pings` exclusion), and extend the `run_monitor_loop` per-run clear list <!-- completed: 2026-07-24T11:45 -->
- [x] Update `SPEC.md` §6.5 (wake payload contract) with the exact revised payload text including the revised closing re-engagement sentence; §6.3 `monitor` group for the `monitor status` field additions; §6.4 *Exact text layouts* for the new `unacked` column and `pending` padding; §6.8 for the additive `members` key on `GET /api/monitor` <!-- completed: 2026-07-24T11:45 -->
- [x] Update `skills/cafleet/roles/monitor.md`: reason list in the on-wake step, the unacked report rule, and the example wake nudge <!-- completed: 2026-07-24T11:45 -->
- [x] Update `skills/cafleet/reference/supervision.md`: `cafleet member ping` as the canonical, capture-gated response to an unacked report <!-- completed: 2026-07-24T11:45 -->

### Step 2: Broker

- [x] Add the `oldest_pending_ts` correlated `MIN(status_timestamp)` subquery to `list_monitor_targets` in `cafleet/src/cafleet/broker/monitor.py` and update its docstring <!-- completed: 2026-07-24T11:49 -->
- [x] Broker tests: `oldest_pending_ts` is `None` with no pending rows, picks the minimum `status_timestamp`, excludes acked (`completed`) and `broadcast_summary` rows <!-- completed: 2026-07-24T11:49 -->

### Step 3: Monitor loop

- [x] Add `_last_unacked_wake_at`, `_flag_unacked_due`, the `monitor_tick` call ordering (after stall-check, before native status) and woke-gated commit, and the `run_monitor_loop` per-run clear in `cafleet/src/cafleet/monitor/loop.py` <!-- completed: 2026-07-24T11:55 -->
- [x] Loop tests: fresh-due flagging at exactly one interval, not-yet-stale skip, re-fire gate (absent entry due; entry younger than interval skipped), disabled/dead-pane skip, reason union with an interval-due member, commit gated on `woke` (true commits, false re-flags), unacked-only member excluded from `record_pings`, per-run map clear <!-- completed: 2026-07-24T11:55 -->

### Step 4: Wake-nudge payload

- [x] Insert the unacked sentence into the payload and revise its closing re-engagement sentence in `multiplexer/tmux.py` and `multiplexer/herdr.py` (byte-identical) <!-- completed: 2026-07-24T11:57 -->
- [x] Update the payload tests pinning the exact text and tmux/herdr byte-equality <!-- completed: 2026-07-24T11:57 -->

### Step 5: `monitor status` and WebUI

- [x] Add the shared `monitor_members_payload(fleet_id, now)` builder in `broker/monitor.py` (rows including `oldest_pending_ts` / `oldest_pending_age_seconds`), switch `cli/monitor.py` to it, add the `unacked` column (+ `pending` padding) in `output/formatters.py`, and add the additive `members` key in `webui/api.py`'s `GET /api/monitor` via the same builder <!-- completed: 2026-07-24T12:36 -->
- [x] Tests: `monitor status --json` payload fields, exact text-table layout with and without a pending age, `GET /api/monitor` response carries `members` with unchanged runtime keys <!-- completed: 2026-07-24T12:36 -->

### Step 6: Verification

- [x] Full pass: `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` <!-- completed: 2026-07-24T12:36 -->
