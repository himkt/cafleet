# Deferred First Director Wake

**Status**: Approved
**Progress**: 5/12 tasks complete
**Last Updated**: 2026-08-06

## Overview

Defer the monitor's first fleet-level Director wake: when `monitor_runtime.last_wake_at` is `NULL`, the due-check falls back to `monitor_runtime.started_at` as the baseline, so the first wake fires at `started_at + wake_interval` instead of on the first tick. Today that first-tick wake lands right after fleet creation, while the Director is still spawning members, and is pure noise.

## Success Criteria

- [ ] A freshly claimed monitor (`last_wake_at` `NULL`) fires its first wake only once `wake_interval` has elapsed since `started_at`, not on the first tick.
- [ ] `last_wake_at` remains the `woke`-gated delivery ledger: it is never written at claim time and only records delivered wakes (its `GET /api/monitor` exposure is unchanged).
- [ ] An unparsable `last_wake_at` remains immediately due; a `NULL` `last_wake_at` paired with a `NULL` or unparsable `started_at` is also immediately due (corrupt state, no panic).
- [ ] A present `last_wake_at` always wins as the baseline — the cadence-survives-restart behavior is unchanged.
- [ ] SPEC.md §6.6 (and the §10 monitor test-plan line), the docs monitoring concepts page, and the colocated tests all reflect the new contract; `mise //cafleet:test` and `mise //cafleet:lint` pass.

---

## Background

`wake_due` (`cafleet/src/monitor/mod.rs`) treats a `NULL` or unparsable `last_wake_at` as immediately due. Since `claim_monitor_runtime` (`cafleet/src/broker/monitor.rs`) inserts a fresh slot with `last_wake_at` `NULL`, the first wake fires on the first tick after `cafleet monitor` starts — immediately after `cafleet fleet create`, before any member exists. The claim already stamps `started_at`, giving the due-check a natural baseline for the not-yet-woken state without any schema change.

---

## Specification

### `wake_due` — new signature and policy

The pure check gains a `started_at` parameter; the whole baseline policy lives inside the function (no call-site branching):

```rust
pub fn wake_due(
    last_wake_at: Option<&str>,
    started_at: Option<&str>,
    wake_interval: u64,
    now: DateTime<Utc>,
) -> bool;
```

| `last_wake_at` | `started_at` | Outcome |
|---|---|---|
| present, parsable | (ignored) | due iff `now − last_wake_at` ≥ `wake_interval` (whole seconds, unchanged comparison) |
| present, unparsable | (ignored) | immediately due (corrupt state, unchanged) |
| `NULL` | present, parsable | due iff `now − started_at` ≥ `wake_interval` |
| `NULL` | `NULL` or unparsable | immediately due (corrupt state — in a running loop the claim has just stamped `started_at`, so this cannot happen on the healthy path; degrading to today's behavior beats a panic in the heartbeat loop) |

Precedence rationale: a present `last_wake_at` always wins, even when it is older than a fresh post-reclaim `started_at` — that preserves the existing "an immediate restart honors the remaining wake cadence rather than firing instantly" behavior for fleets that have already received a wake.

### `monitor_tick` — call-site change

Step 4 (compute due-ness) reads both stamps from the runtime row it already fetched and passes them through:

```rust
if !wake_due(
    runtime["last_wake_at"].as_str(),
    runtime["started_at"].as_str(),
    wake_interval,
    now,
) {
    return Ok(TickResult::Continue);
}
```

No other tick step changes. The `woke`-gated `record_monitor_wake` + echo ordering invariant is untouched; skipped wakes (dead Director pane, failed keystroke) still stamp nothing, so an unstamped-but-due fleet stays due on the next tick.

### Accepted trade-off: reclaim resets the first-wake timer

`claim_monitor_runtime` re-stamps `started_at` on every claim and reclaim while preserving `last_wake_at` (both unchanged). Consequently a fleet that has **never** received its first wake and whose monitor crashes/restarts waits a fresh full `wake_interval` from the new `started_at`. This is accepted: the first-wake state is exactly the state in which a deferred wake is wanted, and the alternative (persisting a first-deadline stamp) would require a schema change. The trade-off is documented on the docs monitoring concepts page and in SPEC §6.6.

### Broker layer — no change

`claim_monitor_runtime`, `record_monitor_wake`, `clear_monitor_runtime`, and the `GET /api/monitor` payloads are untouched. `last_wake_at` stays never-pre-stamped, preserved across reclaim and clear; `started_at` stays stamped at claim and nulled at clear. No schema change, no CLI flag change, no API payload change.

### Contract-surface edits

| Surface | Edit |
|---|---|
| SPEC.md §6.6 *Public surface* (`wake_due`) | New four-parameter signature; state the full baseline policy table above: present `last_wake_at` wins (unparsable → immediately due); `NULL` `last_wake_at` → baseline `started_at`; `NULL`/unparsable `started_at` with `NULL` `last_wake_at` → immediately due. |
| SPEC.md §6.6 `monitor_tick` step 4 | "Read the runtime row's `last_wake_at` **and `started_at`** and call `wake_due(last_wake_at, started_at, wake_interval_seconds, now)`." |
| SPEC.md §6.6 `run_monitor_loop` step 2 (reclaim note) | Extend the existing "a reclaim leaves `last_wake_at` untouched" sentence: a reclaim re-stamps `started_at`, so a fleet that never received its first wake waits a fresh full interval from the restart. |
| SPEC.md §10 test-plan *Monitor* line | The `wake_due` clause enumerates only states the pure function sees — interval gating / `last_wake_at` precedence / `started_at` baseline / corrupt stamps; the zero-interval state moves to the `monitor_tick` clause (the interval-0 gate precedes the due-check, so `wake_due` never sees interval 0). |
| `docs/docs/concepts/monitoring.md` § Cadence and tick precision | State the first-wake baseline: the first wake fires once the interval has elapsed since the monitor started, so a freshly created fleet gets its Director's spawning window undisturbed. Extend the restart-durability sentence with the trade-off: a fleet that has already been woken keeps its remaining cadence across restarts; a fleet that has never been woken restarts its first-wake timer. |

The doc row for `last_wake_at` in SPEC §6.2 and `docs/docs/spec/data-model.md` ("timestamp of the last successfully delivered Director wake") stays accurate as-is; no other page pins the first-tick behavior.

### Test changes (colocated, `cafleet/src/monitor/mod.rs`)

`wake_due_tests`:

| Test | Change |
|---|---|
| `a_missing_or_unparsable_stamp_is_immediately_due` | Replace: an unparsable `last_wake_at` stays immediately due regardless of `started_at`; `NULL` `last_wake_at` with `NULL` or unparsable `started_at` is immediately due. |
| new: `a_null_last_wake_defers_to_the_started_at_baseline` | `NULL` `last_wake_at` + parsable `started_at`: not due at `started_at + interval − 1`, due at `started_at + interval`. |
| new: `a_present_last_wake_wins_over_a_fresher_started_at` | `last_wake_at` older than `started_at` (post-reclaim shape): due-ness follows `last_wake_at`. |
| `the_interval_gates_a_stamped_fleet` | Pass a `started_at` argument; assertions unchanged. |

`monitor_tick_tests` — every test that relied on "`NULL` stamp → due at claim time" now advances `now` past the interval (the fleets are claimed at `base_now()`, so ticking at `base_now() + 600s` makes them due):

| Test | Change |
|---|---|
| `a_due_tick_wakes_the_director_and_stamps_last_wake_at` | Tick at `now + 600s`; echo/stamp assertions follow the new tick time. |
| `the_wake_interval_gates_the_next_wake` | New sequence pinning the deferral: tick at claim time → **no** wake; at `+599s` → not due; at `+600s` → first wake; at `+1199s` → not due; at `+1200s` → second wake. |
| `a_dead_director_pane_skips_the_wake_without_stamping` | Tick at `now + 600s` so the fleet is due; skip/no-stamp assertions unchanged. |
| `a_failed_wake_commits_nothing_and_retries_next_tick` | First (failed) wake at `now + 600s`, retry at `now + 605s`; assertions otherwise unchanged. |
| `a_fleet_with_no_members_still_wakes_the_director` | Tick at `now + 600s`; assertions follow. |
| `a_zero_interval_heartbeats_without_waking` | Unchanged (the interval gate precedes the due-check). |

`cafleet/src/broker/monitor.rs` tests (`last_wake_at_survives_a_reclaim`, `claim_inserts_a_fresh_slot_and_refuses_a_live_one`, `clear_is_ownership_checked_and_preserves_tick_seconds_and_last_wake_at`) already pin the unchanged broker semantics and need no edits.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (docs first)

- [x] Update `docs/docs/concepts/monitoring.md`: first-wake baseline in § Cadence and tick precision, and the restart-durability sentence extended with the never-woken-fleet trade-off <!-- completed: 2026-08-06T12:20 -->

### Step 2: SPEC.md

- [x] Update §6.6 *Public surface*: the four-parameter `wake_due` signature and the full baseline policy <!-- completed: 2026-08-06T12:24 -->
- [x] Update §6.6 `monitor_tick` step 4 to read and pass both stamps <!-- completed: 2026-08-06T12:24 -->
- [x] Extend §6.6 `run_monitor_loop` step 2 with the reclaim first-wake trade-off <!-- completed: 2026-08-06T12:24 -->
- [x] Reword the §10 test-plan *Monitor* line: the `wake_due` clause carries interval gating / `last_wake_at` precedence / `started_at` baseline / corrupt stamps; zero-interval moves to the `monitor_tick` clause <!-- completed: 2026-08-06T12:24 -->

### Step 3: Code

- [ ] Extend `wake_due` in `cafleet/src/monitor/mod.rs` with the `started_at` parameter and the baseline policy, restating the new policy in both the module-header API sketch and `wake_due`'s own `///` contract comment <!-- completed: -->
- [ ] Update the `monitor_tick` call site to pass `runtime["started_at"].as_str()` <!-- completed: -->

### Step 4: Tests and verification

- [ ] Rework `wake_due_tests`: corrupt-state test, `started_at`-baseline test, `last_wake_at`-precedence test, extended-signature gating test <!-- completed: -->
- [ ] Rework the five `monitor_tick_tests` that assumed first-tick due-ness; pin the deferred first wake in `the_wake_interval_gates_the_next_wake` <!-- completed: -->
- [ ] Confirm the broker-layer tests still pass unchanged <!-- completed: -->
- [ ] `mise //cafleet:test` passes <!-- completed: -->
- [ ] `mise //cafleet:lint` passes <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-06 | Initial draft |
| 2026-08-06 | Reviewer fixes (§10 test-plan attribution, `///` contract-comment task); approved |
