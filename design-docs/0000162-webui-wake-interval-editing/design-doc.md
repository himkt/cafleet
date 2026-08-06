# Restore WebUI editing of the Director wake interval

**Status**: Approved
**Progress**: 20/22 tasks complete
**Last Updated**: 2026-08-07

## Overview

Persist the fleet-level Director wake interval in a new
`monitor_runtime.wake_interval_seconds` column that the monitor loop stamps at
every start and re-reads on every tick, expose it through a new
`PATCH /api/monitor` endpoint, and add an interval editor to the admin WebUI
header. This restores the interval editing the WebUI lost when design 0000158
replaced per-member monitoring with the fleet-level Director wake, refitted to
the current architecture.

## Success Criteria

- [ ] `monitor_runtime` has a nullable `wake_interval_seconds` column; the
      migration chain is contiguous from 1 with head **5** and the chain-guard
      test matches.
- [ ] Every `cafleet monitor` start stamps its startup-resolved interval
      (`--interval` > `CAFLEET_MONITOR_WAKE_INTERVAL` > 600) into the column;
      each tick gates the wake on the column's current value, so an external
      update changes the running loop's cadence within one tick without a
      restart (pinned by a `monitor_tick` test).
- [ ] `PATCH /api/monitor` updates the column under the `X-Fleet-Id` header
      with the exact 200/400/404/422 contract in § *S3*, pinned by
      `webui_routes` tests.
- [ ] `GET /api/monitor` includes `wake_interval_seconds` in both the running
      and not-running payload shapes.
- [ ] The WebUI header monitor indicator opens a popover with the interval
      editor; the editor saves via `PATCH /api/monitor` and is disabled when
      the monitor is not running.
- [ ] `docs/`, `SPEC.md`, and the skill references are drift-free for the new
      surfaces; `mise //cafleet:test`, `mise //cafleet:lint`,
      `mise //cafleet:typecheck`, and `mise //admin:lint` pass.

---

## Background

Design 0000158 removed the per-member monitoring editor (interval input +
enable/disable toggle, backed by `PATCH /api/members/{member_id}/monitor` over
the `monitor_config` table) when per-member monitoring was replaced by the
fleet-level Director wake. Today the wake interval is resolved once at
`cafleet monitor` startup (`--interval N` > `CAFLEET_MONITOR_WAKE_INTERVAL` >
default 600; 0 disables) and held only in the running process — it is threaded
as a parameter from `run_monitor_loop` into every `monitor_tick` call and is
not persisted anywhere. The WebUI's only monitor surface is the read-only
`GET /api/monitor` payload, consumed as a running/stopped dot in the dashboard
header.

---

## Specification

### S1. Persistence and precedence

A new nullable `INTEGER` column `wake_interval_seconds` on the per-fleet
`monitor_runtime` table (migration `V5__monitor_wake_interval.sql`):

```sql
ALTER TABLE monitor_runtime ADD COLUMN wake_interval_seconds INTEGER;
```

The precedence model is **restart-stamps**: the column is a live mirror of the
running loop's interval, not a durable override of the CLI.

| Event | Effect on `wake_interval_seconds` |
|---|---|
| `cafleet monitor` start (claim or reclaim) | Stamped with the startup-resolved value (`--interval` > env > 600). |
| Each tick | Re-read; the read value gates the wake. |
| `PATCH /api/monitor` | Overwritten; the running loop obeys it within one tick (≤ `tick_seconds`). |
| Loop stop (`clear_monitor_runtime`) | Preserved, like `tick_seconds`. |
| `fleet delete` | Row removed inside the delete transaction (existing behavior). |

Consequences: a WebUI edit lasts until the next monitor start, which re-stamps
from the CLI/env resolution; the CLI `--interval` flag, the
`CAFLEET_MONITOR_WAKE_INTERVAL` env var, their resolution order, and the
`0`-disables semantics are unchanged in behavior (their accepted numeric
domain narrows to `0..=i64::MAX` per the § *S2* value domain). `NULL` occurs
only in rows that predate
the migration and have not been re-claimed since; a running loop's row is
always stamped, because the loop itself wrote the value at claim.

### S2. Monitor loop: re-read per tick

`monitor_tick` drops its `wake_interval` parameter and reads the interval from
the runtime row it already fetches; `run_monitor_loop` keeps its
`wake_interval` parameter but uses it only to stamp the claim.

```rust
// broker
pub fn claim_monitor_runtime(conn: &mut Connection, fleet_id: i64, pid: i64,
    tick_seconds: i64, wake_interval: i64, when: &str) -> Result<bool, CafleetError>;
pub fn set_monitor_wake_interval(conn: &mut Connection, fleet_id: i64,
    wake_interval: i64) -> Result<bool, CafleetError>;  // false ⇔ no row

// monitor
pub fn monitor_tick(conn: &mut Connection, mux: &dyn MonitorMux,
    out: &mut dyn Write, fleet_id: i64, pid: i64, now: DateTime<Utc>)
    -> Result<TickResult, CafleetError>;
pub fn run_monitor_loop(conn: &mut Connection, mux: &dyn MonitorMux,
    out: &mut dyn Write, fleet_id: i64, tick_seconds: i64,
    wake_interval: i64) -> Result<(), CafleetError>;    // stamp-only use
```

**Value domain.** The interval is `i64` seconds, non-negative, end to end —
bounded so every persisted value fits SQLite's `INTEGER` column and no
boundary can accept a value whose stamp or update would fail at runtime:

| Boundary | Enforcement |
|---|---|
| `--interval` | `clap::value_parser!(i64).range(0..)`, the style `--tick` already uses; negative or above-`i64::MAX` values fail clap's standard invalid-value error (exit 2). |
| `CAFLEET_MONITOR_WAKE_INTERVAL` | `Settings.monitor_wake_interval` becomes `i64` with an explicit `>= 0` check preserving the existing error string `CAFLEET_MONITOR_WAKE_INTERVAL must be a non-negative integer (got '<raw>')`. |
| `PATCH /api/monitor` | Validated via `as_i64()` plus `>= 0`; a JSON number above `i64::MAX` fails the same 422 `detail` (§ *S3*). |
| `wake_due` | Takes `i64` directly, dropping today's internal `as i64` cast. |

`claim_monitor_runtime` writes `wake_interval_seconds` in both its INSERT and
its reclaim UPDATE, exactly as it writes `tick_seconds`.
`set_monitor_wake_interval` is a single ownership-free
`UPDATE monitor_runtime SET wake_interval_seconds=?1 WHERE fleet_id=?2`
returning `changed == 1`; zero rows changed means the fleet's monitor has
never run (rows are never deleted outside `fleet delete`, so "no row" is
exactly "never run").

The tick order changes only in that the runtime-row read moves ahead of the
interval gate (it must, to learn the interval):

1. Ownership-checked heartbeat → `Stop` on displacement (unchanged).
2. Fleet liveness → `Stop` when deleted (unchanged).
3. Read the runtime row (the heartbeat just matched, so it exists); take
   `wake_interval_seconds` with
   `.expect("the owning loop stamped the interval at claim")` — the owning
   loop wrote it in step 0 of its own lifetime, so `NULL` here is corrupt
   state and fails loudly.
4. Interval `0` → `Continue` without waking (unchanged semantics, now
   evaluated per tick).
5. `wake_due(last_wake_at, started_at, wake_interval, now)` — the pure
   function is unchanged in behavior (the interval parameter becomes `i64`
   per the value domain), including the design-0000161 deferred first wake
   (first wake at `started_at + interval`).
6. Director-pane resolution, the single wake, the `woke`-gated ledger write
   and echo (all unchanged).

Mid-run edit semantics fall out of the re-read plus the unchanged `wake_due`:

| Edit while the loop runs | Observable effect |
|---|---|
| Shrink below the time already elapsed since the baseline | The fleet is due on the next tick (≤ `tick_seconds` later). |
| Shrink or grow, not yet elapsed | The next wake fires at `baseline + new interval`. |
| Set `0` | Wakes stop from the next tick; the loop keeps heartbeating. |
| Raise from `0` | Wakes resume, gated against the existing baseline. |
| Edit before the first wake | The first-wake boundary moves to `started_at + new interval`. |

The startup line `monitor loop started (fleet <fleet_id>, tick <tick>s,
pid <pid>)` is unchanged.

### S3. HTTP API

#### PATCH /api/monitor — Update the wake interval

**Request**: `X-Fleet-Id: <fleet_id>` header; JSON body
`{"wake_interval_seconds": N}`. `N` must be a JSON integer in `0..=i64::MAX`,
validated via `as_i64()` plus `>= 0` — floats, stringified integers,
negatives, and numbers above `i64::MAX` (where `as_i64()` returns nothing)
are rejected, not coerced, mirroring the send endpoint's strictness. `0`
disables the wake; there is no application-level cap below `i64::MAX`.

**Response** (200 OK):

```json
{"wake_interval_seconds": 300}
```

**Errors** (all `{"detail": <string>}`-shaped):

| Condition | Status | `detail` |
|---|---|---|
| Missing or empty `X-Fleet-Id` | 400 | `X-Fleet-Id header required` |
| Non-integer `X-Fleet-Id` | 400 | `X-Fleet-Id must be an integer` |
| Unparsable JSON body | 422 | `invalid JSON body: <parse error>` |
| `wake_interval_seconds` missing, or not an integer in `0..=i64::MAX` | 422 | `wake_interval_seconds must be a non-negative integer` |
| Unknown fleet | 404 | `Fleet not found` |
| No `monitor_runtime` row for the fleet | 404 | `monitor has never run for this fleet` |

Resolution order (the table's row order): header errors, then body
validation, then the fleet check, then the row update — matching
`POST /api/messages/send`, whose body parse likewise precedes the fleet
check, so an unknown fleet plus an invalid body yields 422 on both
endpoints.

#### GET /api/monitor — payload addition

The runtime payload gains a `wake_interval_seconds` key, placed immediately
after `tick_seconds` in the pinned JSON key order, in both shapes:

| Shape | Value |
|---|---|
| Running | The column value (a stamped row is non-null). |
| Not running — stale or cleared row | **Preserved** from the row, like `tick_seconds`; `null` when the row predates the migration and was never re-stamped. |
| Not running — no row has ever existed | `null`, like `tick_seconds`. |

### S4. WebUI

The header monitor indicator (the running/stopped dot in `AppHeader`) becomes
the trigger for a popover carrying the interval editor.

| Element | Behavior |
|---|---|
| Trigger | The existing indicator dot, now a button; the tooltip text stays, except the stopped-state tooltip's launch command reads `cafleet monitor <fleet-id>` (the prior text cited a CLI form that no longer exists). |
| Interval input | Numeric input in seconds (integer ≥ 0), seeded from `wake_interval_seconds` in the polled `GET /api/monitor` payload. |
| Zero hint | Inline hint at value `0`: the Director wake is disabled while the loop keeps running. |
| Save | Calls `PATCH /api/monitor`, surfaces a `detail` error inline on failure, and triggers the dashboard's existing refresh on success. |
| Not-running state | The editor is disabled with a hint that the interval is re-stamped from the CLI/env at each monitor start, so there is nothing durable to edit. |

There is no enable/disable toggle — `0` in the numeric input is the disable
affordance. Frontend changes: `MonitorRuntime` gains
`wake_interval_seconds: number | null`, `api.ts` gains a `patchMonitor`
helper, and `Dashboard` passes the monitor payload plus its refresh trigger
into `AppHeader`. Client-side validation disables Save for anything but a
non-negative integer.

### S5. Out of scope

- Editing the scan tick (`tick_seconds`) from the WebUI.
- Starting or stopping the monitor from the WebUI.
- Any return of per-member schedules.
- Changes to the `--interval`/`CAFLEET_MONITOR_WAKE_INTERVAL` resolution
  order, the 600 default, the `0`-disables semantics, or the startup-line
  format (the accepted numeric domain does narrow to `0..=i64::MAX` per the
  § *S2* value domain).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Update `docs/docs/concepts/monitoring.md` § cadence: the interval is
      stamped per fleet at each monitor start, re-read per tick, and editable
      from the admin WebUI (effective within one tick; re-stamped at the next
      start). <!-- completed: 2026-08-07T05:55 -->
- [x] Update `docs/docs/spec/webui-api.md`: add `PATCH /api/monitor` to the
      route table and as a section with the § *S3* contract verbatim; add
      `wake_interval_seconds` to the `GET /api/monitor` example and the
      not-running field table. <!-- completed: 2026-08-07T05:55 -->
- [x] Update `docs/docs/spec/data-model.md` § `monitor_runtime`: the
      `wake_interval_seconds` column and its stamp/re-read/preserve
      lifecycle. <!-- completed: 2026-08-07T05:55 -->
- [x] Update `docs/docs/spec/cli-options.md` (monitor loop form): the resolved
      interval is stamped into `monitor_runtime` at each start.
      <!-- completed: 2026-08-07T05:55 -->
- [x] Update `SPEC.md`: §6.2 (`claim_monitor_runtime` /
      `set_monitor_wake_interval` / payload keys and the schema DDL), §6.3
      (`--interval` stamping and the `i64` domain), §6.6 (per-tick re-read
      order, the dropped `monitor_tick` parameter, and `wake_due`'s `i64`
      interval), §6.8 (`PATCH /api/monitor`, the GET payload addition, and
      the route-catalog title bump "The 7 routes" → 8), §7.1
      (`monitor_wake_interval` domain), and the head version in the migration
      inventory. <!-- completed: 2026-08-07T05:55 -->
- [x] Verify `skills/cafleet/reference/cli.md` and
      `reference/supervision.md` stay accurate (their `--interval`/env/0
      statements are unchanged by this design); edit only if a statement
      drifts. <!-- completed: 2026-08-07T05:55 -->

### Step 2: Migration

- [x] Add `cafleet/migrations/V5__monitor_wake_interval.sql` with the § *S1*
      DDL. <!-- completed: 2026-08-07T06:02 -->
- [x] Bump every head-4 assertion in `cafleet/src/db/mod.rs` to head 5 —
      `migrate_reaches_head_version_4_and_is_idempotent` (renamed),
      `refinery_ledger_records_the_baseline` (`vec![1, 2, 3, 4]` → `..5`),
      and `migration_chain_is_contiguous_from_1_with_exactly_one_baseline_and_head_4`
      (renamed) — and the `applied migrations to head (4).` assertion in
      `cafleet/tests/cli_setup_doctor.rs`. <!-- completed: 2026-08-07T06:02 -->

### Step 3: Broker layer

- [x] Extend `claim_monitor_runtime` with the `wake_interval` parameter,
      stamped in both the INSERT and the reclaim UPDATE.
      <!-- completed: 2026-08-07T06:11 -->
- [x] Carry `wake_interval_seconds` through `RuntimeRow`,
      `read_monitor_runtime`, and `monitor_runtime_payload` (both shapes, key
      after `tick_seconds`); keep `clear_monitor_runtime` preserving it.
      <!-- completed: 2026-08-07T06:11 -->
- [x] Add `set_monitor_wake_interval` returning `changed == 1`.
      <!-- completed: 2026-08-07T06:11 -->
- [x] Colocated broker tests: stamp on claim and reclaim, preservation across
      clear, the payload key in running/stale/no-row shapes, and the
      `set_monitor_wake_interval` true/false split.
      <!-- completed: 2026-08-07T06:11 -->

### Step 4: Monitor loop

- [x] Rework `monitor_tick` to the § *S2* order (drop the parameter, read the
      row before the interval gate, `expect` the stamped value); update
      `run_monitor_loop` to pass the startup interval only to the claim, and
      `wake_due` to take `i64`. <!-- completed: 2026-08-07T06:19 -->
- [x] Apply the § *S2* value domain at the CLI/config boundaries: `--interval`
      switches to `clap::value_parser!(i64).range(0..)` in
      `cafleet/src/cli/monitor.rs`, and `Settings.monitor_wake_interval`
      becomes `i64` with an explicit non-negative check preserving the
      existing error string; update the config tests.
      <!-- completed: 2026-08-07T06:19 -->
- [x] Monitor tests: a mid-run column update changes the cadence on the next
      tick (shrink-to-due, set-0-disables, raise-re-enables), and the
      first-wake boundary follows an edit made before the first wake.
      <!-- completed: 2026-08-07T06:19 -->

### Step 5: HTTP endpoint

- [x] Add the `PATCH /api/monitor` handler in `cafleet/src/webui/mod.rs` with
      the § *S3* resolution order, validation, and error strings; register the
      route and correct the module doc's route count to 8 (it reads "the 9
      `/api` routes" today while 7 are registered).
      <!-- completed: 2026-08-07T06:24 -->
- [x] `webui_routes` tests: 200 round-trip (PATCH then GET reflects the
      value), both 400s, fleet 404, both 422s (including
      float/string/negative/above-`i64::MAX` rejection and the
      422-before-fleet-404 ordering), and the no-row 404.
      <!-- completed: 2026-08-07T06:24 -->

### Step 6: Admin WebUI

- [x] Extend `MonitorRuntime` in `admin/src/types.ts` and add `patchMonitor`
      to `admin/src/api.ts`. <!-- completed: 2026-08-07T06:29 -->
- [x] Turn the `AppHeader` monitor indicator into the popover editor per
      § *S4* (input, zero hint, save with inline error, disabled not-running
      state). <!-- completed: 2026-08-07T06:29 -->
- [x] Wire `Dashboard` to pass the monitor payload and refresh trigger into
      `AppHeader`. <!-- completed: 2026-08-07T06:29 -->

### Step 7: Verification

- [ ] `mise //cafleet:format`, `mise //cafleet:lint`,
      `mise //cafleet:typecheck`, `mise //admin:lint`. <!-- completed: -->
- [ ] `mise //cafleet:test` — full suite green. <!-- completed: -->
