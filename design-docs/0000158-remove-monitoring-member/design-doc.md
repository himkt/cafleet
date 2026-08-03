# Remove the monitoring member — one fleet-level tick to the Director

**Status**: Approved
**Progress**: 32/60 tasks complete
**Last Updated**: 2026-08-03

## Overview

Replace the dedicated monitoring member and its per-member ping/stall-check
cadence with a single fleet-level tick that wakes the **Director** and asks it
to health-check its own members. Both pane-injected triggers gain a
"resume your work if something was still running" clause so a keystroke that
lands mid-turn cannot strand the recipient.

## Success Criteria

- [ ] `cafleet member create` has no `--role` flag; no member kind marker,
      one-per-fleet guard, or placement-required guard for a monitoring member
      survives anywhere in the codebase, the database, or the docs.
- [ ] `cafleet monitor start` runs in the **Director's** pane and keystrokes the
      wake into that same pane, `Esc`-first, once per `600s` by default.
- [ ] `monitor_config` no longer exists; the migration chain is contiguous from
      1 with head **4** and the chain-guard test matches.
- [ ] Both pane-injected trigger strings carry the resume clause verbatim as
      specified in § *Contract strings*, pinned in `SPEC.md` and in tests.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`,
      and `mise //admin:lint` pass.
- [ ] Outside `design-docs/` and `cafleet/migrations/`, no repository file
      mentions a monitoring member, `--role monitor`, `ready: monitor live`,
      `monitor_config`, or `CAFLEET_MONITOR_STALL_INTERVAL`.

The two carve-outs are permanent, not temporary tolerances. `design-docs/` is
committed in this project (`.claude/rules/git-workflow.md` override), so this
document's own text always matches. The migration chain is immutable history:
`V1__baseline.sql` creates `monitor_config`, `V2__drop_director_monitor_enrollment.sql`
deletes rows from it, `V4` drops it, and `V3__strip_monitoring_member_kind.sql`
carries `monitoring-member` in both its filename and its comment. Migrations
are never rewritten to scrub a term.

---

## Background

GitHub issue #260: *members get stuck by monitoring nudge; all members stalled.*

Today a fleet runs a dedicated monitoring member that owns
`cafleet monitor start` in its own pane. The loop computes a per-member due set
from `monitor_config` (ping interval `720s`) plus a separate stall-check
cadence (`CAFLEET_MONITOR_STALL_INTERVAL`, `240s`) plus an edge-triggered
native `agent_status` transition into `done`, and wakes the watcher. The
watcher then captures each due pane, classifies it against its backend overlay,
confirms quiet across two wakes, and may fire `cafleet member ping`.

That machinery has two failure modes the issue names. The nudge itself is a
keystroke into a member's pane; landing mid-turn it can displace the member's
in-progress work instead of resuming it. And the whole fleet's supervision
depends on one extra agent whose own stalls cascade — when the watcher stops
classifying, nothing recovers anyone.

This design deletes the watcher and the per-member cadence. The Director — the
one member that already owns facilitation and already receives `Esc`-safeguarded
keystrokes on every inline message preview — becomes the sole recipient of a
periodic tick that says "health-check your members, and resume your own work if
something was running".

---

## Specification

### S1. Supervision model, before and after

| Aspect | Today | After |
|---|---|---|
| Loop host | The monitoring member's pane | The Director's pane |
| Who launches | The monitoring member, gated by `ready: monitor live` | The Director, immediately after `cafleet fleet create` |
| Wake recipient | The monitoring member | The Director |
| Wake trigger | Per-member: `interval` / `stall-check` / `status:done`, annotated `unacked` | Unconditional fleet-level interval |
| Per-member schedule | `monitor_config` rows, editable in the WebUI | none |
| Pane capture + classify | The monitoring member, every wake, automatically | The Director, at its own discretion, using its existing pre-ping capture gate |
| Automatic `member ping` | The monitoring member's fixed-ping exception | none — `member ping` stays a Director-only manual primitive |
| Teardown | Delete the monitoring member first (first-out) | The Director stops its background task, then deletes members |

The Director's **existing** pre-ping capture gate is unchanged and remains the
only sanctioned automatic pane-state read. The capture-state taxonomy
(`awaiting_user` / `finished` / `working` / `stall_candidate`) and the
per-backend capture cues in the overlays survive — their consumer changes from
the monitoring member's wake routine to the Director's on-tick health check.

**The gate keeps its established name.** "Pre-ping capture gate" is the
repository's term — a § heading at `skills/cafleet/reference/supervision.md`,
a required term in three `docs_sync.rs` tests, and named in all three backend
overlays. It stays verbatim: `cafleet member ping` survives as a Director
primitive, and the gate already covers `cafleet message send` as well, so
nothing about the name goes stale. This design renames no gate and adds no
`docs_sync` required-term swap for it.

### S2. Cadence

| Knob | Value | Set by |
|---|---|---|
| Director wake interval | `600s` | `CAFLEET_MONITOR_WAKE_INTERVAL` / `monitor start --interval N` |
| Scan tick | `5s` | `monitor start --tick N` |
| Runtime staleness | `max(3 × tick, 15s)` | `MONITOR_STALE_FACTOR` / `MONITOR_STALE_FLOOR_SECONDS` (unchanged) |

**How `600s` compares to today — stated precisely, because the two old knobs
move in opposite directions.** Today's *effective* cadence is the shorter of
the two triggers across the watched set: a fleet with any ordinary member is
woken every **240 s** under the default stall-check interval. Against that,
`600s` is a 2.5× increase, which is what issue #260 asks for. Against the
*per-member ping interval* of **720 s** it is a decrease. Both statements are
true of different knobs; the design deliberately does not claim a blanket
increase. What actually shrinks is the number of keystrokes: today a wake fires
at the watcher every 240 s and can fan out to N member pings; after this change
one keystroke lands at one pane every 600 s regardless of fleet size.

`CAFLEET_MONITOR_WAKE_INTERVAL=0` (or `--interval 0`) disables the wake while
the loop keeps claiming the runtime slot and heartbeating — the same
disable-semantics `CAFLEET_MONITOR_STALL_INTERVAL=0` carried.

The tick fires whenever the interval has elapsed and the Director's pane is
alive, **including when the fleet has no other members**. The Director is
itself a supervision target now: the resume clause is the remedy for a stalled
Director, and a fleet with no members is a transient bootstrap state, not a
steady state (the Director stops the loop at teardown).

### S3. Contract strings

Both strings are contract surfaces: pinned in `SPEC.md`, in
`docs/docs/spec/multiplexer-backends.md`, and in the test suite.

**(a) Member-facing poll trigger** — `Multiplexer::send_poll_trigger`, used by
`cafleet member ping`, typed `Esc`-first then `Enter`:

```
cafleet message poll --fleet-id <fleet-id> --member-id <member-id> — then resume your work if something was still running.
```

**(b) Director-facing wake** — `build_wake_payload`, replacing the
`[monitor] wake: …  Follow your monitor role protocol.` form:

```
[cafleet] tick: fleet <fleet-id> — health-check your <N> members: <entries>. Poll your inbox, ACK, dispatch. Resume your work if something was still running.
```

Grammar, fully specified:

| Element | Rule |
|---|---|
| `<entries>` | `<member-id> (<name>; coding_agent=<agent>; unacked=<pending-count>)`, joined by `, `, ordered by `member_id` ascending |
| `<name>` | passed through `sanitize_wake_field` (unchanged) |
| `<pending-count>` | count of the member's `input_required` unicast deliveries |
| `N == 1` | `health-check your 1 member: …` (singular noun) |
| `N == 0` | the clause becomes `no members to health-check.` — no `<entries>` segment |
| Invalid `coding_agent` | the wake aborts with `member <id> has invalid coding_agent '<agent>'` and no keystroke is sent (unchanged guard, member-side only) |

Worked example, two members:

```
[cafleet] tick: fleet 3 — health-check your 2 members: 4 (drafter; coding_agent=claude; unacked=2), 5 (reviewer; coding_agent=codex; unacked=0). Poll your inbox, ACK, dispatch. Resume your work if something was still running.
```

The Director descriptor is dropped from the payload — the Director is now the
recipient, not a referent.

### S4. Keystroke safety

**`Esc` first.** `send_wake_trigger` gains the leading `Esc` + `0.1s` settle
delay that `send_poll_trigger` and `send_inline_preview` already use, so the
tmux/herdr event sequence becomes `Escape` → sleep `0.1` → `-l <payload>` →
sleep `1.0` → `Enter`. The old no-`Esc` form was safe only because the
watcher's pane was never on a permission prompt; the Director's pane can be.
Every other keystroke into the Director's pane — one per inbound member message
via `send_inline_preview` — is already `Esc`-first, so this makes the wake match
the established norm rather than introducing one.

**Self-keystroke.** The loop runs as a background child of the shell in the
Director's own pane and addresses that pane by multiplexer pane id. A pane is
writable by any process holding its id, so a process writing to the pane it was
launched from is not a special case — it is the same `tmux send-keys -t %N`
path every other trigger uses.

**Operator-typing hazard, documented not guarded.** If the operator is
mid-composition at the Director's pane when a wake lands, the `Esc` clears any
pending prompt box and the payload is appended to whatever text is already in
the composer, then `Enter` submits both together. This is real. It is also
exactly the hazard `send_inline_preview` already carries at that same pane on
every member message, and the project accepts it there; adding a new guard for
the wake alone would be inconsistent without removing the hazard. The
`docs/docs/concepts/monitoring.md` page states it explicitly so operators can
choose `--interval 0` during hands-on sessions.

### S5. Removal surface

Everything in this table is deleted outright — no deprecation shim, no
accepted-but-ignored flag value (`.claude/rules/removal.md`).

| Symbol / surface | Home |
|---|---|
| `--role` flag (both values) | `cafleet/src/cli/member.rs` |
| `MONITORING_MEMBER_KIND`, `active_monitoring_member_id`, the one-per-fleet guard, the placement-required guard | `cafleet/src/broker/members.rs` |
| `kind` parameter of `register_member` and `member_card` | `cafleet/src/broker/members.rs` |
| `enroll`, `MEMBER_PING_INTERVAL_SECONDS` | `cafleet/src/broker/members.rs` |
| `find_monitoring_member`, `get_monitor_config`, `list_monitor_configs`, `update_monitor_config`, `record_pings`, `record_monitor_dispatch`, `reconcile_monitor_lifecycle`, `list_monitor_targets`, `monitor_members_payload` | `cafleet/src/broker/monitor.rs` |
| `should_ping`, `stall_check_due`, `unacked_overdue`, `MonitorTickState`, the `agent_status` scan and the `done`-transition machinery | `cafleet/src/monitor/mod.rs` |
| `MonitorMux::agent_status` | `cafleet/src/monitor/mod.rs` (the `Multiplexer::agent_status` trait method **stays**; herdr keeps implementing it) |
| `monitor_stall_interval`, `CAFLEET_MONITOR_STALL_INTERVAL` | `cafleet/src/config.rs` |
| The `monitor start` no-monitoring-member warning | `cafleet/src/cli/monitor.rs` |
| `DELETE FROM monitor_config …` in `delete_fleet` | `cafleet/src/broker/fleets.rs` |
| `GET /api/members/{id}/monitor`, `PATCH /api/members/{id}/monitor`, `monitor_projection`, the per-member `monitor` key on the roster payload | `cafleet/src/webui/mod.rs` |
| `MonitorConfig`, `"monitor"` from the `kind` union, `monitor: MonitorConfig \| null` | `admin/src/types.ts` |
| `skills/cafleet/roles/monitor.md` | deleted entirely |
| `{monitor_model}` placeholder | all four `skills/cafleet/reference/coding-agent/*.md`, the documented-defaults table in `skills/cafleet/SKILL.md`, `OVERLAY_PLACEHOLDERS` in `docs_sync.rs` |

`derive_member_kind` narrows to a two-value collapse over the SQL-supplied
`is_director` flag alone, returning `"director"` or `"member"`; the card-JSON
argument goes away.

`monitor capture` and `member ping` **stay** as Director primitives.
`cafleet monitor` keeps its name across the CLI group, the `monitor` module,
`monitor_runtime`, and `docs/docs/concepts/monitoring.md` — issue #260 names
`cafleet monitor start` directly.

### S6. Schema — two migrations, head 4

Two single-purpose files keep the data change independently reviewable from the
schema change. Both are data-preserving; neither drops or recreates a populated
parent table.

**`cafleet/migrations/V3__strip_monitoring_member_kind.sql`**

```sql
-- The monitoring-member kind marker no longer has a reader; strip it so no
-- inert marker survives in existing member cards. `cafleet.kind` was the sole
-- key under `$.cafleet`, so the whole object goes.

UPDATE members
SET member_card_json = json_remove(member_card_json, '$.cafleet')
WHERE json_extract(member_card_json, '$.cafleet.kind') = 'monitoring-member';
```

**`cafleet/migrations/V4__fleet_level_wake_schedule.sql`**

```sql
-- Replace the per-member monitor schedule with the fleet-level wake cadence:
-- the tick is unconditional and periodic, so the only durable schedule state
-- is one timestamp per fleet.

DROP TABLE monitor_config;

ALTER TABLE monitor_runtime ADD COLUMN last_wake_at TEXT;
```

Head schema for `monitor_runtime`:

```sql
CREATE TABLE monitor_runtime (
    fleet_id INTEGER NOT NULL PRIMARY KEY REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    pid INTEGER,
    started_at TEXT,
    last_tick_at TEXT,
    tick_seconds INTEGER NOT NULL DEFAULT 5,
    last_wake_at TEXT
);
```

`last_wake_at` is **durable across loop restarts** and is preserved by
`clear_monitor_runtime` (which already preserves `tick_seconds` and nulls only
the process fields). An immediate restart therefore honors the remaining wake
cadence rather than firing instantly — the property `last_stall_check_at`
carried for the old stall-check.

### S7. The tick

`MonitorMux` narrows to two methods:

```rust
pub trait MonitorMux {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[Value],
    ) -> Result<bool, MultiplexerError>;
}
```

```rust
pub fn monitor_tick(
    conn: &mut Connection,
    mux: &dyn MonitorMux,
    out: &mut dyn Write,
    fleet_id: i64,
    pid: i64,
    wake_interval: u64,
    now: DateTime<Utc>,
) -> Result<TickResult, CafleetError>;
```

Steps, in order:

1. `heartbeat_monitor_runtime` — a non-owner's tick matches zero rows → `Stop`.
   (Unchanged.)
2. `get_fleet` — missing or soft-deleted → `Stop`. (Unchanged.)
3. `wake_interval == 0` → `Continue`. Heartbeat only.
4. `wake_due(last_wake_at, wake_interval, now)` — a `NULL` or unparsable
   `last_wake_at` is immediately due; otherwise
   `(now - last_wake_at).num_seconds() >= wake_interval`. Not due → `Continue`.
5. Resolve the Director: `fleet.director_member_id` → its placement's
   `mux_pane_id`. A Director with no pane, or a pane absent from
   `mux.list_pane_ids()`, → `Continue` with **no** wake and **no** stamp.
6. Build the roster: every `status='active'` member of the fleet **except** the
   Director, joined to `member_placements` for `coding_agent`, with
   `pending_count` from the existing unicast `input_required` subquery, ordered
   by `member_id`. This is the new `broker::list_fleet_wake_targets`, replacing
   `list_monitor_targets`.
7. `mux.send_wake_trigger(director_pane, fleet_id, &roster)`.
8. On success only (`woke == true`): `broker::record_monitor_wake(conn,
   fleet_id, &iso)` writes `last_wake_at`, and one echo line goes to stdout:

   ```
   <iso> tick -> wake director <director-member-id> (<N> members)
   ```

   A failed wake commits nothing and retries on the next tick — the
   `woke`-gated write discipline is preserved unchanged.

Errors and edge cases:

| Condition | Behavior |
|---|---|
| Director pane missing from `list_pane_ids` | `Continue`; no wake, no stamp, no echo |
| Fleet has no non-Director members | wake fires with the `N == 0` payload form |
| A member has a placement but no pane yet (`mux_pane_id IS NULL`) | included in the roster — the Director should know it is pending |
| `send_wake_trigger` returns `false` (backend missing, keystroke failed) | `Continue`; nothing recorded; retried next tick |
| A roster member's `coding_agent` is unregistered | `send_wake_trigger` returns `Err`; the tick surfaces it as `CafleetError::App` and the loop exits — unchanged from today's abort semantics |

`run_monitor_loop` drops its `MonitorTickState` and takes `wake_interval` in
place of `monitor_stall_interval`. The single-instance claim, the SIGTERM/SIGINT
stop flag, `interruptible_sleep`, and the ownership-checked exit clear are
unchanged.

### S8. CLI and configuration

| Surface | Change |
|---|---|
| `cafleet monitor start --interval N` | new; `u64`, range `0..`; when omitted, falls back to `settings.monitor_wake_interval` |
| `cafleet monitor start --tick N` | unchanged (`i64`, range `1..`, default `5`) |
| `cafleet monitor start` warning | deleted |
| `cafleet monitor capture` | unchanged |
| `cafleet member create --role` | deleted |
| `cafleet member ping` | unchanged surface; the injected payload gains the resume clause (§ S3a) |
| `CAFLEET_MONITOR_WAKE_INTERVAL` | new; `u64`, default `600`; `0` disables the wake. A non-integer value fails loudly, as `CAFLEET_MONITOR_STALL_INTERVAL` did |
| `CAFLEET_MONITOR_STALL_INTERVAL` | deleted |

`Settings.monitor_stall_interval` → `Settings.monitor_wake_interval`.

### S9. HTTP API and admin WebUI

| Endpoint | Change |
|---|---|
| `GET /api/monitor` | kept. The `runtime` object gains `last_wake_at` and `last_wake_age_seconds`, both masked to `null` when the slot is not running (matching `last_tick_at`). The `members` array is **re-sourced** from `members` + the pending-delivery subqueries |
| `GET /api/members/{id}/monitor` | deleted (404 by absence of the route) |
| `PATCH /api/members/{id}/monitor` | deleted |
| `GET /api/members` roster | the per-member `monitor` key is removed; `kind` narrows to `"director" \| "member"` |

New `members` element of `GET /api/monitor`, in key order:

```json
{
  "member_id": 4,
  "name": "drafter",
  "pending_count": 2,
  "oldest_pending_ts": "2026-08-03T09:00:00.000000+00:00",
  "oldest_pending_age_seconds": 120
}
```

`interval_seconds`, `enabled`, `last_ping_at`, and `last_ping_age_seconds` are
gone from that element.

Admin (`admin/src/`): drop `MonitorConfig` and the `monitor` field from
`types.ts`, narrow the `kind` union, drop the two member-monitor calls from
`api.ts`, and remove the per-member schedule editor from `MemberDetail.tsx`.
`AppHeader.tsx`, `Dashboard.tsx`, and `Sidebar.tsx` keep their runtime/roster
views against the reshaped payloads; the `monitor` kind badge is removed from
the roster rendering.

### S10. Lifecycle — launch, teardown, recovery

**Launch.** Immediately after `cafleet fleet create` and **before** the first
`cafleet member create`, the Director launches

```bash
cafleet monitor start --fleet-id <fleet-id>
```

as a background task in its own pane — each skill page names the launch
primitive by its overlay placeholder, as it already does elsewhere — and
confirms the startup line
`monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in the task
output before spawning any member. That confirmation replaces the
`ready: monitor live` handshake as the gate on the first `member create`.

**Teardown**, in order:

1. Stop the background task (the backend's stop primitive, resolved from the
   overlay). The loop's SIGTERM handler runs its
   ownership-checked `clear_monitor_runtime`, nulling `pid` / `started_at` /
   `last_tick_at` and preserving `tick_seconds` and `last_wake_at`.
2. `cafleet member delete` each member (each kills the pane immediately).
3. `cafleet member list` to verify only the root Director's row remains.
4. `cafleet fleet delete --fleet-id <fleet-id>`.
5. `cafleet fleet list` to confirm.

**The `ON DELETE RESTRICT` question, answered.**
`monitor_runtime.fleet_id REFERENCES fleets ON DELETE RESTRICT` never blocks
teardown: `delete_fleet` **soft**-deletes the fleet row and explicitly runs
`DELETE FROM monitor_runtime WHERE fleet_id=?1` inside the same transaction, so
there is no dependent row and no hard fleet delete for the constraint to
restrict. The constraint is unchanged by this design.

**Recovery when the Director's pane dies without stopping the loop.** The loop
process dies with the pane and its `finally`-equivalent never runs, so a stale
`monitor_runtime` row survives with a non-null `pid`. That row reads as **dead**
on both liveness axes — the heartbeat goes stale after `max(3 × tick, 15s)`,
and the signal-0 probe reports no-such-process — so `claim_monitor_runtime`
reclaims it and a fresh `cafleet monitor start` succeeds. `cafleet fleet delete`
removes the row unconditionally. No manual cleanup step exists or is needed.

### S11. Test-contract changes

| Test surface | Change |
|---|---|
| `cafleet/src/db/mod.rs` | `APP_TABLES` 7 → 6; head `2` → `4` in `migrate_reaches_head_version_*` and the chain guard (rename both); `refinery_ledger_records_the_baseline` expects `[1, 2, 3, 4]`; drop the `monitor_config` rows from `autoincrement_*`, `ddl_defaults_*`, and `foreign_key_on_delete_rules_*`; add a `monitor_runtime.last_wake_at` nullability assertion; update the `has_check_constraint` comment that cites `last_stall_check_at` |
| `cafleet/src/monitor/mod.rs` colocated tests | rewrite: delete `should_ping_tests` and every reason-specific tick test; add wake-due gating, the durable `last_wake_at` stamp, the `woke`-gated write, the dead-Director-pane skip, the `interval 0` disable, and the `N == 0` payload |
| `cafleet/src/broker/monitor.rs` colocated tests | delete every `monitor_config` test; add `record_monitor_wake` and `list_fleet_wake_targets`; keep the runtime claim / heartbeat / clear / liveness suite and extend `clear` to assert `last_wake_at` survives |
| `cafleet/src/broker/members.rs` colocated tests | delete the one-per-fleet, placement-required, and `derive_member_kind == "monitor"` cases; assert `register_member` writes no `monitor_config` row (by the table's absence) |
| `cafleet/src/broker/fleets.rs` colocated tests | drop the `monitor_config` count assertion from `delete_fleet_soft_deletes_and_cascades`; delete `create_fleet_leaves_the_director_unenrolled` outright — its whole body is a `get_monitor_config` call, so it cannot survive that function's removal |
| `cafleet/src/broker/messaging.rs` colocated tests | the broadcast test registers a monitoring member via `register_member(…, Some("monitoring-member"))` and asserts the recipient set; drop the `kind` argument and reword the assertion to an ordinary pane-bound member — the intent (a non-Director member with a pane receives a broadcast) is unchanged |
| `cafleet/src/broker/test_support.rs` | the module doc block is the project's broker API catalogue (every source file points at it). Delete the nine functions § S5 removes from its `// monitor` section and add `record_monitor_wake` / `list_fleet_wake_targets` |
| `cafleet/src/output/formatters.rs` colocated tests | the `member list` roster test pins `monitor` as a rendered `kind` value in both its fixture and its expected output row; that fixture becomes unconstructible under the two-value union — rewrite it to `director` / `member` |
| `cafleet/src/multiplexer/{mod,tmux,herdr}.rs` colocated tests | `build_wake_payload` tests rewritten to the § S3b grammar including the singular and `N == 0` forms; `send_wake_trigger` asserts the `Esc`-first event sequence; `send_poll_trigger` asserts the § S3a string |
| `cafleet/src/config.rs` colocated tests | `monitor_wake_interval` default `600`, env override, `0` valid, non-integer fails loudly |
| `cafleet/tests/e2e.rs` | drop `--role monitor` from the spawn; delete `monitor_start_without_a_watcher_warns_and_still_runs` outright; assert the new echo line and the `[cafleet] tick:` payload |
| `cafleet/tests/cli_member.rs` | add only a regression guard that `--role` is no longer a recognized option (clap's own `unexpected argument` error). The file carries no `--role` spawn today — its `monitor capture` fixtures never passed one, and the sole test-side `--role monitor` lives in `e2e.rs` (row above) |
| `cafleet/tests/webui_routes.rs` | delete `member_monitor_get_*` and `patch_monitor_*`; reshape `the_roster_wraps_members_and_projects_the_monitor_config` to the two-value `kind` union with no `monitor` key; extend `the_monitor_endpoint_reports_and_masks_the_runtime` for `last_wake_at` and the re-sourced `members` array |
| `cafleet/tests/docs_sync.rs` | see below |

**Where the § S3a string is actually pinned.** Only the multiplexer colocated
tests assert it in full. `cli_member.rs`'s two `member ping` tests match the
keystroke with a substring `contains` on
`cafleet message poll --fleet-id N --member-id M`, so the appended resume clause
leaves them passing unchanged — they need no edit, and the implementer should
not "fix" them.

`docs_sync.rs` specifically:

- `REMOVED_VOCABULARY` gains `monitoring member`, `--role monitor`,
  `ready: monitor live`, `monitor_config`,
  `CAFLEET_MONITOR_STALL_INTERVAL`, `stall-check` (array length grows 10 → 16).
- `OVERLAY_PLACEHOLDERS` drops `monitor_model` (10 → 9); the
  `every_backend_overlay_defines_the_full_placeholder_vocabulary` and
  `every_brace_token_in_skills_belongs_to_the_known_vocabulary` tests follow.
- `the_monitor_role_is_the_sole_normative_protocol_carrier` is deleted (its
  target file is deleted).
- `data_model_defines_the_trimmed_monitor_config` is replaced by a
  `monitor_runtime`-only assertion (`monitor_runtime`, `last_wake_at`,
  `tick_seconds`).
- `multiplexer_backends_pins_the_pure_trigger_payload` swaps `[monitor] wake:`
  and `Follow your monitor role protocol` for `[cafleet] tick:` and
  `Resume your work if something was still running`.
- `spec_defines_the_ping_skip_and_monitor_group_contract`,
  `cli_options_defines_the_ping_skip_and_moved_capture`,
  `the_supervision_contract_covers_quiet_members_and_plain_messages`,
  `the_director_and_member_roles_keep_the_ping_protocol`,
  `the_cafleet_skill_and_bash_rule_document_the_fixed_ping_exception`, and
  `monitoring_concept_covers_the_judgment_protocol_and_pure_trigger_wake` get
  their required-term lists rewritten to the new protocol.
- `every_backend_overlay_defines_the_capture_cues` changes on two axes: its
  `_template.md` required-term list drops the literal `monitoring member`, and
  its per-overlay `assert_absent` starts biting on that same term once
  `REMOVED_VOCABULARY` gains it — which is exactly what the Step 2 overlay
  retarget is for, and is the test that proves the retarget landed. Its
  per-overlay `pre-ping capture gate` required term is **unchanged** (§ S1).
- `fixed_ping_surfaces_carry_no_nudge_vocabulary` drops
  `skills/cafleet/roles/monitor.md` from its path list.
- The module-header doc comment (the `//!` block) still narrates "the monitoring
  member's two-wake in-context judgment"; it is rewritten to the Director-tick
  contract. Leaving it would park removal residue at the top of the very file
  that polices residue.
- `ROLE_FILE_WITHOUT_REQUIRED_READING` is unaffected (it targets
  `skills/cafleet-research/report/roles/web-researcher.md`).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
>
> Documentation precedes code (`.claude/rules/documentation-maintenance.md`).
> Steps 1–4 land the full documentation surface; only then does Step 5 begin.

### Step 1: Concepts and user-facing docs

- [x] Rewrite `docs/docs/concepts/monitoring.md` end to end: the Director-hosted loop, the single wake interval, the § S3b payload, the `Esc`-first keystroke and the documented operator-typing hazard, the launch/teardown/recovery lifecycle. Delete § *The monitoring member* and the per-member cadence table <!-- completed: 2026-08-03T11:52 -->
- [x] Update `docs/docs/concepts/overview.md` on four surfaces: the Core terms row for `monitor` (it defines the loop as waking the monitoring member), the CLI-groups table's `monitor` row (also correct its stale `start`, `status`, `config` subcommand list to `start`, `capture`), the `member list` kinds sentence (must follow the two-value kind union), and § Monitoring. Keep concept-level naming per `.claude/rules/user-facing-docs.md` <!-- completed: 2026-08-03T11:56 -->
- [x] Update `docs/docs/concepts/model-selection.md`: drop the `Monitor` row from the policy-exception table and reword "The monitor and the reviewer are policy exceptions on **every** team spawn" to reviewer-only. The Step 9 sweep cannot catch this page — it carries no literal `monitoring member` or `monitor_model` — so this task is the only thing standing between the change and a stale model policy <!-- completed: 2026-08-03T11:57 -->
- [x] Update `docs/docs/spec/cli-options.md`: `monitor start --interval`, the deleted warning row, the deleted `--role` row, the new `CAFLEET_MONITOR_WAKE_INTERVAL` entry, the `member ping` payload <!-- completed: 2026-08-03T12:03 -->
- [x] Update `docs/docs/spec/data-model.md`: delete the `monitor_config` table, add `monitor_runtime.last_wake_at` <!-- completed: 2026-08-03T12:05 -->
- [x] Update `docs/docs/spec/webui-api.md`: delete both `/api/members/{id}/monitor` sections, reshape `GET /api/monitor` <!-- completed: 2026-08-03T12:10 -->
- [x] Update `docs/docs/spec/multiplexer-backends.md`: the new wake payload, the `Esc`-first sequence, the poll-trigger string <!-- completed: 2026-08-03T12:14 -->
- [x] Update `docs/docs/how-to/mixed-backend-team.md` — drop the monitoring-member spawn from the walkthrough <!-- completed: 2026-08-03T12:16 -->
- [x] Run the `/update-readme` skill to sync `README.md` and `SPEC.md` <!-- completed: 2026-08-03T12:33 -->
- [x] Hand-verify `SPEC.md` against §§ S2–S10 at these headings: §5.4 *Member kind discriminator*; §6.2 *Broker* (its monitor schedule/runtime subsections); §6.3 *CLI* (its `monitor` group subsection); §6.6 *Monitor heartbeat loop*; §8 *Database schema*; §10 *CLI command checklist* <!-- completed: 2026-08-03T12:48 -->

### Step 2: The `cafleet` skill

- [x] Delete `skills/cafleet/roles/monitor.md` <!-- completed: 2026-08-03T12:49 -->
- [x] Rewrite `skills/cafleet/reference/supervision.md`: § Monitor Lifecycle → Director-hosted loop; § Spawn Protocol loses the first-in gate and gains the startup-line confirmation; § Stall Response becomes the Director's own on-tick health check; § Cleanup Protocol takes the § S10 order <!-- completed: 2026-08-03T12:24 -->
- [x] Update `skills/cafleet/SKILL.md`: § Team supervision rewritten; § *Fixed monitoring-member ping exception* deleted; `{monitor_model}` row removed from the documented-defaults table; the `roles/monitor.md` pointer removed <!-- completed: 2026-08-03T12:27 -->
- [x] Update `skills/cafleet/roles/director.md` — Required-reading and the launch step <!-- completed: 2026-08-03T12:30 -->
- [x] Update `skills/cafleet/roles/member.md` — delete the monitoring-member exception paragraph <!-- completed: 2026-08-03T12:30 -->
- [x] Update `skills/cafleet/reference/director.md`: delete the `--role` row from the `member create` flag table; keep § *Member Ping* with the new payload <!-- completed: 2026-08-03T12:34 -->
- [x] Update `skills/cafleet/reference/recovery.md` § Shutdown Protocol to the § S10 order and the stale-row recovery note <!-- completed: 2026-08-03T12:36 -->
- [x] Update `skills/cafleet/reference/cli.md`: `monitor start --interval`, the Director as launcher <!-- completed: 2026-08-03T12:38 -->
- [x] Update `skills/cafleet/reference/model-list.md`: § *Monitor and reviewer defaults* → § *Reviewer defaults* <!-- completed: 2026-08-03T12:40 -->
- [x] Update all four `skills/cafleet/reference/coding-agent/*.md` on four surfaces each: drop the `{monitor_model}` row; retarget the *Pane-state capture cues* Note row (it cites the deleted `roles/monitor.md` § On each wake) to the Director's on-tick health check; retarget the *Pane-state capture cues* intro sentence and the `awaiting_user` / `stall_candidate` cue prose, all of which name the monitoring member as the classifier; and **replace § *Worked resolution*** — its entire content today is the canonical `cafleet member create --role monitor …` spawn command. Its replacement is that backend's fully resolved Director-side background launch of `cafleet monitor start --fleet-id <fleet-id>`, which keeps the section's purpose (one fully materialized command per backend) while the command it demonstrates changes <!-- completed: 2026-08-03T12:46 -->

### Step 3: The workflow skills

- [x] Update `skills/cafleet-design-doc/`: `SKILL.md`, `create/create.md`, `create/roles/director.md`, `execute/execute.md`, `execute/roles/director.md`, `interview/interview.md` — replace the monitor-member spawn + `ready: monitor live` gate with the Director's `monitor start` launch; update each teardown block <!-- completed: 2026-08-03T12:58 -->
- [x] Update `skills/cafleet-research/`: `SKILL.md`, `report/report.md`, `report/roles/director.md`, `presentation/presentation.md`, `presentation/roles/director.md` — same substitution <!-- completed: 2026-08-03T13:05 -->
- [x] Update `.claude/skills/clean-docs/**` — the `monitor` member row in its team table and every spawn/teardown mention <!-- completed: 2026-08-03T12:43 -->
- [x] Update `.claude/skills/skill-author/SKILL.md` — §§ 2.3, 2.5, the worked `summarize-pr` example, and both troubleshooting entries <!-- completed: 2026-08-03T12:43 -->
- [x] Update `.claude/skills/cafleet-model-list-refresh/SKILL.md` — drop the monitor-model refresh obligation <!-- completed: 2026-08-03T12:43 -->

### Step 4: Project rules

- [x] Update `.claude/rules/bash-tool.md`: delete the monitoring-member fixed-ping exception; keep the Director-side `member ping` / `member prompt --shell` protocol; update the quoted injection description ("injects `Esc` + `cafleet message poll --fleet-id <s> --member-id <m>` + Enter through `send_poll_trigger`") to the § S3a payload <!-- completed: 2026-08-03T12:53 -->
- [x] Sweep every other page that quotes the injected poll line verbatim and bring each to § S3a — `rg -n "message poll --fleet-id <" docs/ skills/ .claude/ SPEC.md` finds them <!-- completed: 2026-08-03T12:49 -->
- [x] Update `.claude/rules/coding-agent-overlay.md`: drop `{monitor_model}` from the model-policy paragraph and the overlay token list <!-- completed: 2026-08-03T12:53 -->

### Step 5: Migrations

- [x] Add `cafleet/migrations/V3__strip_monitoring_member_kind.sql` (§ S6) <!-- completed: 2026-08-03T13:00 -->
- [x] Add `cafleet/migrations/V4__fleet_level_wake_schedule.sql` (§ S6) <!-- completed: 2026-08-03T13:00 -->
- [x] Update the chain guard in `cafleet/src/db/mod.rs`: `APP_TABLES`, head `4`, the refinery ledger vector, and the per-table DDL assertions (§ S11) <!-- completed: 2026-08-03T13:00 -->
- [x] Apply and verify with `cafleet setup --skip claude --skip codex --skip opencode` <!-- completed: 2026-08-03T13:00 -->

### Step 6: Rust — broker and member registry

COMMENT(director): approved — Steps 6–8 are one compile unit. Order: (1) the Tester lands the Step 7 rewritten colocated suites (monitor/mod.rs, multiplexer/{mod,tmux,herdr}.rs, config.rs, output/formatters.rs per § S11) in one test commit; (2) the Programmer then implements Steps 6+7+8 together and verifies with the full suite; (3) per-step checkbox updates still happen per file as tasks complete. Rationale: whole-crate compilation makes intermediate green states impossible; test-first is preserved by landing all affected suites before implementation.

- [ ] `cafleet/src/broker/members.rs`: delete `MONITORING_MEMBER_KIND`, `MEMBER_PING_INTERVAL_SECONDS`, `enroll`, `active_monitoring_member_id`, both monitoring-member guards, and the `kind` parameter of `register_member` / `member_card`; narrow `derive_member_kind` to `is_director` <!-- completed: -->
- [ ] `cafleet/src/broker/monitor.rs`: delete every `monitor_config` function and `find_monitoring_member`; add `record_monitor_wake` and `list_fleet_wake_targets`; extend `runtime_row` / `monitor_runtime_payload` for `last_wake_at` <!-- completed: -->
- [ ] `cafleet/src/broker/fleets.rs`: drop the `DELETE FROM monitor_config` statement from `delete_fleet` <!-- completed: -->
- [ ] `cafleet/src/broker/messaging.rs`: drop the `kind` argument from the broadcast test's `register_member` call and reword its recipient-set assertion to an ordinary pane-bound member (§ S11) <!-- completed: -->
- [ ] `cafleet/src/broker/test_support.rs`: rewrite the `// monitor` section of the module doc catalogue — delete the nine functions § S5 removes, add `record_monitor_wake` and `list_fleet_wake_targets` <!-- completed: -->
- [ ] Update the colocated tests in `members.rs`, `monitor.rs`, and `fleets.rs` (§ S11) <!-- completed: -->
- [ ] Bring each touched `cafleet/src/broker/*.rs` module `//!` header in line with the surviving surface (`monitor.rs`'s reads "Monitor schedule + runtime DB layer" and no longer describes a schedule) <!-- completed: -->


### Step 7: Rust — the tick, the multiplexer, the CLI, and config

- [ ] `cafleet/src/multiplexer/mod.rs`: rewrite `build_wake_payload` to the § S3b grammar (drop the `director` argument, take `fleet_id`); update the `send_wake_trigger` trait signature; update `send_poll_trigger`'s payload to § S3a <!-- completed: -->
- [ ] `cafleet/src/multiplexer/tmux.rs` and `herdr.rs`: `send_wake_trigger` becomes `Esc`-first; both `send_poll_trigger` implementations carry the new string <!-- completed: -->
- [ ] `cafleet/src/monitor/mod.rs`: narrow `MonitorMux`; delete `should_ping`, `stall_check_due`, `unacked_overdue`, `MonitorTickState`, and the `agent_status` scan; add `wake_due`; rewrite `monitor_tick` and `run_monitor_loop` to § S7 <!-- completed: -->
- [ ] `cafleet/src/config.rs`: `monitor_stall_interval` → `monitor_wake_interval`, env `CAFLEET_MONITOR_WAKE_INTERVAL`, default `600` <!-- completed: -->
- [ ] `cafleet/src/cli/monitor.rs`: add `--interval`, delete the warning, thread the setting through <!-- completed: -->
- [ ] `cafleet/src/cli/member.rs`: delete the `--role` flag and its `kind` derivation <!-- completed: -->
- [ ] `cafleet/src/output/formatters.rs`: rewrite the `member list` roster test fixture and its expected output row, both of which pin `monitor` as a rendered `kind` value (§ S11) <!-- completed: -->
- [ ] Update the colocated tests in all seven files (§ S11) <!-- completed: -->

### Step 8: HTTP API and admin WebUI

- [ ] `cafleet/src/webui/mod.rs`: delete both `/api/members/{id}/monitor` routes and `monitor_projection`; drop the per-member `monitor` key from the roster; reshape `GET /api/monitor`'s `members` array and add the `last_wake_at` runtime keys <!-- completed: -->
- [ ] `admin/src/types.ts`: delete `MonitorConfig`, narrow `kind`, drop the `monitor` field <!-- completed: -->
- [ ] `admin/src/api.ts`: delete the two member-monitor calls <!-- completed: -->
- [ ] `admin/src/components/MemberDetail.tsx`: remove the per-member schedule editor <!-- completed: -->
- [ ] `admin/src/components/{AppHeader,Dashboard,Sidebar}.tsx`: adapt to the reshaped payloads; remove the `monitor` kind badge <!-- completed: -->

### Step 9: Integration tests and verification

- [ ] `cafleet/tests/e2e.rs` per § S11 <!-- completed: -->
- [ ] `cafleet/tests/cli_member.rs` per § S11 <!-- completed: -->
- [ ] `cafleet/tests/webui_routes.rs` per § S11 <!-- completed: -->
- [ ] `cafleet/tests/docs_sync.rs` per § S11 <!-- completed: -->
- [ ] `rg -n "monitoring member|--role monitor|ready: monitor live|monitor_config|CAFLEET_MONITOR_STALL_INTERVAL|monitor_model" -g '!design-docs/**' -g '!cafleet/migrations/**'` returns no hit. Both exclusions are permanent, for the reasons given under Success Criteria — they are not a licence to leave residue anywhere else <!-- completed: -->
- [ ] `mise //admin:lint` and `mise //admin:build` pass <!-- completed: -->
- [ ] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` pass <!-- completed: -->
- [ ] `mise //cafleet:install`, then a manual smoke run: `cafleet fleet create`, `cafleet monitor start --interval 60` in the background, confirm one `Esc`-first wake lands in the Director's pane with the § S3b payload <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-03 | Initial draft |
| 2026-08-03 | Review round 1: scoped the removal criterion to exclude `design-docs/` and `cafleet/migrations/`; adopted the repository's established "pre-ping capture gate" term; added `broker/messaging.rs`, `broker/test_support.rs`, `output/formatters.rs`, and `docs/docs/concepts/model-selection.md` to the change surface |
| 2026-08-03 | Approved by the user |
