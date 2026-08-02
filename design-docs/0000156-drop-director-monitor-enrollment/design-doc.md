# Drop the root Director from the monitor watched set

**Status**: Complete
**Progress**: 37/37 tasks complete
**Last Updated**: 2026-08-02

## Overview

The root Director is enrolled in `monitor_config` at 180 s, so it accumulates
wake reasons, advances `last_ping_at`, and is rendered as a due entry in every
wake payload — yet the monitoring member is forbidden from capturing or acting
on a due director entry. This design deletes the Director's enrollment and the
`DIRECTOR_PING_INTERVAL_SECONDS` constant, leaving exactly one enrollment class
(ordinary pane-bound members), and removes the director-role fields that become
unreachable as a result.

## Success Criteria

- [x] `create_fleet` leaves the root Director unenrolled; `get_monitor_config`
      for a freshly bootstrapped Director returns `None`.
- [x] `DIRECTOR_PING_INTERVAL_SECONDS` does not exist anywhere in the
      repository.
- [x] `V2__drop_director_monitor_enrollment.sql` removes every pre-existing
      Director row from `monitor_config`, so migrated and fresh databases have
      identical watched sets.
- [x] `is_director` is absent from `list_monitor_targets` scan rows and from
      wake-payload due entries; `role` is absent from `monitor_members_payload`
      rows.
- [x] `MEMBER_PING_INTERVAL_SECONDS` remains 720 and
      `CAFLEET_MONITOR_STALL_INTERVAL` remains 240; no new interval knob exists.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:format`
      all pass.
- [x] No repository surface states that the root Director is watched, pinged,
      or checked more often than an ordinary member.

---

## Background

Design 0000151 introduced the direct member nudge; design 0000154 deleted the
stall-episode machine and established that the monitoring member never captures
or acts on a due director entry. Since then the Director's `monitor_config` row
has served no purpose beyond pacing the wake cadence: the loop wakes the
monitoring member every 180 s naming a target it is instructed to ignore. The
row reads as "the monitor still pings the Director" — an attractive nuisance the
role protocol has to explicitly defend against.

The Director's supervision does not depend on this row. The monitoring member
reaches the Director through plain per-event `cafleet message send` calls, and
the Director descriptor that identifies the report recipient rides in the wake
payload independently of the watched set.

---

## Specification

### One enrollment class

Enrollment collapses to a single rule: **a pane-bound ordinary member is
enrolled at `MEMBER_PING_INTERVAL_SECONDS` (720 s) by `register_member`;
nothing else is enrolled.**

| Member class | Enrolled | `monitor` field on `GET /api/members` |
|---|---|---|
| An ordinary member with a placement | yes, at 720 s | A schedule object |
| The fleet's root Director | no | `null` |
| The dedicated monitoring member | no — it is the watcher | `null` |
| A deregistered member | no — the row is deleted on deregistration | `null` |
| A registry row without a placement | no | `null` |

### Cadence consequences

These are accepted outcomes of the change, not problems to mitigate. Nothing is
added to compensate: no monitoring-member self-enrollment, no minimum-wake
floor, no new interval knob.

| Condition | Effective wake cadence |
|---|---|
| Default (`CAFLEET_MONITOR_STALL_INTERVAL=240`), ≥1 ordinary member | 240 s — `min(720 interval, 240 stall-check)`, stall-check-driven |
| `CAFLEET_MONITOR_STALL_INTERVAL=0`, ≥1 ordinary member | 720 s — interval-driven only |
| Fleet holding only the Director and the monitoring member | no wakes at all |

In the third case the loop still claims the runtime slot, heartbeats
`monitor_runtime` every tick, and stays alive; the due set is simply empty, so
`send_wake_trigger` is never called. The monitoring member sits idle after its
`ready: monitor live` handshake until the first ordinary member is spawned.

### Contract-surface changes

| Surface | Change |
|---|---|
| `list_monitor_targets` scan row | `is_director` removed; remaining keys unchanged |
| Wake-payload due entry | `is_director` removed; `member_id`, `name`, `coding_agent`, `wake_reasons` unchanged |
| Rendered wake text | **Byte-identical** — the entry prefix becomes the literal `member`, previously derived from a flag that can now only be false |
| Wake-payload `director` descriptor | **Unchanged** — `{member_id, coding_agent}` identifies the monitoring member's report recipient and was never part of the watched set |
| `monitor_members_payload` row (`GET /api/monitor`) | `role` removed; the Director's row no longer appears at all |
| `GET /api/members` | The Director's `monitor` field is `null` |
| `GET /api/members/{director_id}/monitor` | `404 {"detail":"Member not enrolled"}` |

The admin frontend does not read `role`, so dropping it has no UI impact.

**Explicitly out of scope**: `derive_member_kind`'s own `is_director` parameter
in the member registry, the `kind` column of `member list`, and the `director`
value it prints. Those describe registry identity, not monitor enrollment, and
stay exactly as they are.

### Migration

`cafleet/migrations/V2__drop_director_monitor_enrollment.sql`:

```sql
DELETE FROM monitor_config
WHERE member_id IN (
    SELECT director_member_id FROM fleets WHERE director_member_id IS NOT NULL
);
```

The chain becomes contiguous `1..2` with the single baseline at 1. Soft-deleted
fleets already had their `monitor_config` rows cascaded away by `delete_fleet`,
so the statement is a no-op for them.

### Loud Director-agent resolution

`monitor_tick` currently resolves the Director's `coding_agent` by searching
`targets` first, then falling back to `get_member`, then
`.unwrap_or_default()` — a silent empty string. After this change the targets
branch is dead (the Director is never a target) and the silent fallback violates
`.claude/rules/code-quality.md`. Replace the whole chain with:

```rust
let director_agent = broker::get_member(conn, director_id, fleet_id)?
    .expect("a live fleet's Director is registered")["placement"]["coding_agent"]
    .as_str()
    .expect("the root Director is pane-bound")
    .to_string();
```

Both invariants are guaranteed: `create_fleet` always inserts the Director's
placement, and `member delete` of the root Director is rejected. A genuine DB
error now propagates via `?` instead of being swallowed by `.ok()`.

### Documentation surfaces

| Surface | Edit |
|---|---|
| `docs/docs/concepts/monitoring.md` | Opening paragraph's watched-set parenthetical; the watched-set table; the "checked far more often" sentence; the "Root Director ping interval" knob row; step 1's due-director-entry sentence; new cadence-consequence prose |
| `SPEC.md` §5 | The `monitor_config` field table's `interval_seconds` row, which names both enrollment values |
| `SPEC.md` §6.2 | Enrollment intervals; `create_fleet`'s ordered steps; `list_monitor_targets` row shape; `monitor_members_payload` row shape |
| `SPEC.md` §6.5 | `send_wake_trigger` due-entry field list and entry rendering |
| `SPEC.md` §6.6 | The re-export list and the "five constants" count; `should_ping`'s row description and its `is_director` sentence |
| `SPEC.md` §8 | The fresh-DB seeding prose naming the Director at 180 s |
| `SPEC.md` §11 | The "Policy tunables (180/720/3/15) have a single home" decision line |
| `skills/cafleet/reference/supervision.md` | The watched-set sentence; the "the Director **and** each freshly-due member" clause; the example wake payload; the Quick Reference "Run work" row |
| `skills/cafleet/roles/monitor.md` | Step 1's due-director-entry sentence; the `<role>` token in the wake-entry description; the example wake payload |

`README.md` is untouched — the change does not move its pitch, install
commands, or docs-site section links.

Two of these surfaces extend the list confirmed during clarification, and both
follow necessarily from the decisions above: `skills/cafleet/roles/monitor.md`
carries the same due-director-entry defense and example payload as
`concepts/monitoring.md`, and the additional `SPEC.md` sections are the
contract text for the `is_director` / `role` fields being removed. Leaving
either would strand a rule against a state that can no longer occur.

Every rewrite states current behavior. No before/after narration, no note that
the Director "used to be" watched.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] `docs/docs/concepts/monitoring.md` — narrow the opening paragraph's
      watched set to "every ordinary member, each on its own interval"; replace
      the Director row in the watched-set table with a `no` / `null` row;
      delete the "checked far more often than an ordinary member" sentence and
      point instead at the single ordinary-member default. <!-- completed: 2026-08-02T21:14 -->
- [x] `docs/docs/concepts/monitoring.md` — drop the "Root Director ping
      interval | `180s`" row from the knob table; add prose beneath it stating
      the 240 s stall-check-driven baseline, the 720 s cadence under
      `CAFLEET_MONITOR_STALL_INTERVAL=0`, and the no-wake Director-plus-monitor
      fleet. <!-- completed: 2026-08-02T21:14 -->
- [x] `docs/docs/concepts/monitoring.md` — in the monitoring-member step 1,
      replace the "A due `director` entry is not captured" sentence with the
      affirmative statement that the Director descriptor identifies the report
      recipient only and the monitoring member takes no Director-directed pane
      action; update the wake-entry rendering from `<role> <id>` to
      `member <id>`. <!-- completed: 2026-08-02T21:14 -->
- [x] `SPEC.md` §5 and §11 — rewrite the `monitor_config` field table's
      `interval_seconds` row to name only the 720 s member enrollment, and
      narrow the §11 policy-tunables decision line to the surviving
      `720/3/15`. <!-- completed: 2026-08-02T21:15 -->
- [x] `SPEC.md` §6.2 — rewrite the enrollment-intervals bullet to the single
      720 s class and drop `DIRECTOR_PING_INTERVAL_SECONDS`; drop "enroll the
      Director at 180s" from `create_fleet`'s ordered steps. <!-- completed: 2026-08-02T21:15 -->
- [x] `SPEC.md` §6.2 — drop `is_director` from the `list_monitor_targets` row
      shape and `role` (with its `"director"`/`"member"` derivation clause) from
      the `monitor_members_payload` row shape. <!-- completed: 2026-08-02T21:15 -->
- [x] `SPEC.md` §6.5 — drop `is_director` from `send_wake_trigger`'s due-entry
      field list and change the entry rendering to
      `member <id> (<name>; coding_agent=<agent>) [<reasons>]`. <!-- completed: 2026-08-02T21:15 -->
- [x] `SPEC.md` §6.6 — remove `DIRECTOR_PING_INTERVAL_SECONDS` from the
      re-export bullet, change "the five constants" to "the four constants",
      drop `is_director` from `should_ping`'s row description, and delete the
      "`is_director` is **not** consulted (retained only for status labeling)"
      sentence. <!-- completed: 2026-08-02T21:15 -->
- [x] `SPEC.md` §8 — rewrite the fresh-DB seeding prose to name only
      "pane-bound members at 720s by `register_member`". <!-- completed: 2026-08-02T21:15 -->
- [x] `skills/cafleet/reference/supervision.md` — narrow the watched-set
      sentence to ordinary members at 720 s; drop "the Director **and**" from
      the monitoring-member paragraph; update the Quick Reference "Run work"
      row. <!-- completed: 2026-08-02T21:16 -->
- [x] `skills/cafleet/roles/monitor.md` — replace step 1's due-director-entry
      sentence with the affirmative report-recipient statement; change the
      wake-entry description from `<role> <id>` to `member <id>`; rewrite the
      example wake payload with member entries only. <!-- completed: 2026-08-02T21:16 -->

### Step 2: Migration

- [x] Add `cafleet/migrations/V2__drop_director_monitor_enrollment.sql` with
      the `DELETE FROM monitor_config` statement from the Specification.
      <!-- completed: 2026-08-02T21:24 -->
- [x] `cafleet/src/db/mod.rs` — rename
      `migration_chain_is_contiguous_from_1_with_exactly_one_baseline_and_head_1`
      to `..._head_2` and change its final assertion to `Some(&2)`; change
      `refinery_ledger_records_the_baseline`'s expected versions to
      `vec![1, 2]`. <!-- completed: 2026-08-02T21:24 -->
- [x] `cafleet/tests/cli_setup_doctor.rs` — update
      `schema_only_setup_migrates_and_reports_the_head` to expect
      `applied migrations to head (2).` and
      `Already at head (2); nothing to do.`. <!-- completed: 2026-08-02T21:24 -->
- [x] `cafleet/src/db/mod.rs` — rename
      `migrate_reaches_head_version_1_and_is_idempotent` to
      `migrate_reaches_head_version_2_and_is_idempotent` and change both
      `migrate_to_head` return assertions to `2`; drop the stale `` (`1` at
      cutover) `` parenthetical from the `head_version()` doc comment rather
      than re-pinning it to 2, so the comment does not need editing on every
      future migration. <!-- completed: 2026-08-02T21:24 -->

### Step 3: Broker enrollment

- [x] `cafleet/src/broker/members.rs` — delete
      `DIRECTOR_PING_INTERVAL_SECONDS`; reword the surviving constant's doc
      comment for a single cadence; change `enroll` to
      `enroll(conn, member_id)` inserting `MEMBER_PING_INTERVAL_SECONDS`
      directly, and update its call site in `register_member`. <!-- completed: 2026-08-02T21:32 -->
- [x] `cafleet/src/broker/fleets.rs` — delete the
      `enroll(&tx, director_id, DIRECTOR_PING_INTERVAL_SECONDS)?` call and the
      now-unused imports; drop the enrollment clause from `create_fleet`'s doc
      comment. <!-- completed: 2026-08-02T21:32 -->

- [x] `cafleet/src/broker/fleets.rs` — replace
      `create_fleet_enrolls_the_director_at_180` with
      `create_fleet_leaves_the_director_unenrolled`, asserting
      `get_monitor_config(&conn, fleet_id, director_id)` returns `None`.
      <!-- completed: 2026-08-02T21:29 -->

### Step 4: Scan-row and WebUI payload fields

- [x] `cafleet/src/broker/monitor.rs` — in `list_monitor_targets`, drop the
      `director_member_id` correlated subquery from the SELECT list, drop the
      `"is_director"` JSON key, and shift the remaining column indices down by
      one. <!-- completed: 2026-08-02T21:37 -->
- [x] `cafleet/src/broker/monitor.rs` — in `monitor_members_payload`, drop the
      same subquery, the `is_director` tuple element, and the `"role"` JSON key;
      shift the remaining indices. <!-- completed: 2026-08-02T21:37 -->
- [x] `cafleet/src/broker/monitor.rs` tests — retarget every test that uses the
      root Director as a stand-in enrolled member to an ordinary registered
      member: `list_monitor_configs_returns_every_enrolled_member_with_bool_enabled`
      (drop the 180 interval assertion), `record_pings_stamps_last_ping_at_and_ignores_an_empty_list`,
      `record_monitor_dispatch_commits_both_cadences_atomically`,
      `reconcile_monitor_lifecycle_clears_stamps_for_listed_fleet_members_only`,
      `update_monitor_config` / `get_monitor_config` tests, and
      `list_monitor_targets_counts_pending_deliveries`. <!-- completed: 2026-08-02T21:33 -->
- [x] `cafleet/src/broker/monitor.rs` tests — rewrite
      `list_monitor_targets_returns_the_watched_set_with_the_scan_row_shape` so
      the full scan-row shape is pinned on an ordinary member (720 s, no
      `is_director` key) and add an assertion that the Director's `member_id`
      is absent from the returned targets. <!-- completed: 2026-08-02T21:33 -->
- [x] `cafleet/src/broker/monitor.rs` tests — rename
      `members_payload_labels_roles_and_truncates_ages` to
      `members_payload_truncates_ages` (the name must not advertise the removed
      labeling) and update it to assert the absence of `role` and the absence
      of the Director's row. <!-- completed: 2026-08-02T21:33 -->

### Step 5: Monitor loop and wake payload

- [x] `cafleet/src/monitor/mod.rs` — remove `DIRECTOR_PING_INTERVAL_SECONDS`
      from the `pub use` re-export and from the expected-public-API doc comment;
      drop `"is_director": target["is_director"]` from the due-entry JSON.
      <!-- completed: 2026-08-02T21:34 -->
- [x] `cafleet/src/monitor/mod.rs` — replace the Director `coding_agent`
      resolution chain with the loud `get_member` form from the Specification.
      <!-- completed: 2026-08-02T21:34 -->
- [x] `cafleet/src/multiplexer/mod.rs` — in the wake-payload builder, delete the
      `role` computation and emit the literal `member` prefix, keeping the
      rendered text byte-identical; drop the `is_director` parameter and JSON
      key from the test-fixture helper. <!-- completed: 2026-08-02T21:34 -->

- [x] `cafleet/src/multiplexer/tmux.rs` and `cafleet/src/multiplexer/herdr.rs`
      — drop the `"is_director": false` keys from the wake-payload test
      fixtures. <!-- completed: 2026-08-02T21:34 -->
- [x] `cafleet/src/monitor/mod.rs` tests — drop the
      `DIRECTOR_PING_INTERVAL_SECONDS == 180` assertion from
      `the_policy_tunables_are_pinned`; drop `"is_director"` from the
      `should_ping` target fixture. <!-- completed: 2026-08-02T21:39 -->
- [x] `cafleet/src/monitor/mod.rs` tests — extend the `monitored_fleet` fixture
      to register a **second** pane-bound ordinary member, so the tests that
      need two due entries still have them once the Director leaves the watched
      set. <!-- completed: 2026-08-02T21:39 -->
- [x] `cafleet/src/monitor/mod.rs` tests — purge `director_id` from the
      `monitor_tick` test module, which breaks in three distinct ways:
      (1) `last_ping` calls on the Director panic on the missing row
      (`due_members_produce_one_wake_and_gated_ledger_writes`,
      `a_failed_wake_records_nothing_and_retries_next_tick`,
      `no_live_watcher_means_no_wake_and_no_ledger_writes`);
      (2) due-set membership assertions on the Director now fail
      (`due_members_produce_one_wake_and_gated_ledger_writes`'s
      `director_entry` and echo line, `dead_panes_are_reconciled_and_never_due`'s
      `due.iter().any(...)`); and (3) `ping_at(&[director_id, …])` calls
      silently match zero rows, so any test relying on them to suppress a due
      row no longer suppresses anything. Retarget each to an ordinary member,
      and additionally register one in `no_live_watcher…` (whose fleet is
      `create_fleet` alone) so its watched set is non-empty and the test stays
      meaningful. Keep every `director` **descriptor** assertion
      (`member_id`, `coding_agent`) intact — that surface is unchanged. Assert
      in at least one tick test that the Director's `member_id` never appears
      in `due`. <!-- completed: 2026-08-02T21:39 -->
- [x] `cafleet/tests/e2e.rs` — register a second ordinary member, retarget the
      `due member 1 (Director)` stdout assertion to it so the `2 members due`
      plural path keeps two end-to-end due entries, and drop member 1 from the
      `last_ping_at` loop — the Director has no `monitor_config` row to read.
      <!-- completed: 2026-08-02T21:44 -->

### Step 6: WebUI expectations

- [x] `cafleet/tests/webui_routes.rs` — in
      `the_roster_wraps_members_and_projects_the_monitor_config`, assert the
      Director's `monitor` is `Value::Null` and move the full projection-shape
      assertion (`{"interval_seconds": 720, "last_ping_at": null, "enabled": true}`)
      onto the ordinary member's row. <!-- completed: 2026-08-02T21:28 -->
- [x] `cafleet/tests/webui_routes.rs` — in
      `member_monitor_get_returns_the_exact_projection_or_404`, add a case
      asserting `GET /api/members/{director_id}/monitor` returns `404` with
      `{"detail":"Member not enrolled"}`. <!-- completed: 2026-08-02T21:28 -->
- [x] `cafleet/tests/webui_routes.rs` — in
      `the_monitor_endpoint_reports_and_masks_the_runtime`, change the expected
      `members` length to 1 and assert the Director's `member_id` is absent from
      the rows. <!-- completed: 2026-08-02T21:28 -->

### Step 7: Verification

- [x] Run `mise //cafleet:format`, `mise //cafleet:lint`, and
      `mise //cafleet:test`; all pass. <!-- completed: 2026-08-02T21:48 -->
- [x] Grep the repository for `DIRECTOR_PING_INTERVAL`, `Root Director ping`,
      `far more often`, `is_director` under the monitor surfaces, and the bare
      literal `180`; confirm no residue outside the registry-identity uses
      named as out of scope. The bare-`180` sweep is the backstop that catches
      prose naming the value without naming the constant. <!-- completed: 2026-08-02T21:48 -->
- [x] `docs/docs/spec/webui-api.md` — in the `GET /api/members` response
      example, set the root Director's `monitor` field to `null`; in the
      `GET /api/members/{member_id}/monitor` response example, change
      `interval_seconds` from `180` to `720`. Surfaced by the bare-`180` sweep;
      the Documentation-surfaces table did not list this page.
      <!-- completed: 2026-08-02T21:48 -->
- [x] `.claude/rules/documentation-tables.md` — the Echo rule's illustrative
      quote is "the Director is checked far more often than an ordinary member",
      which now describes a state the system cannot reach. Replace it with a
      qualitative-magnitude example that is still true, keeping the rule's point
      about magnitude-not-values intact. <!-- completed: 2026-08-02T21:51 -->
