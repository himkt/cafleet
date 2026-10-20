# Restore Forced Delivery and Reset the Database

**Status**: Complete
**Progress**: 26/26 tasks complete
**Last Updated**: 2026-10-20

## Overview

Revert PR #466 (design 0000182, composer-safe notification delivery) in full, restoring held delivery with a bounded hold that forces the delivery into the pane after `CAFLEET_DELIVERY_HOLD_TIMEOUT`. Make every forced preview carry the resume clause, including on an unclassified pane, which today gets the bare payload. Ship the post-revert schema as a fresh database file (`cafleet_v7.db`) built from a single squashed `V1__baseline.sql` instead of a forward migration.

## Success Criteria

- [x] `git diff 7ba5251f^1 HEAD` touches no file that PR #466 changed, apart from the deliberate changes this document specifies (forced-preview payloads, database reset, the related documentation and tests).
- [x] A forced preview into a `working`, `awaiting_user`, or unclassified pane ends with `[cafleet] Resume your work if something was still running.`; a forced preview into an `awaiting_user` pane also carries the dismissed-prompt note.
- [x] A Claude Director pane launched by the user in auto mode with no pane title receives every held preview within `CAFLEET_DELIVERY_HOLD_TIMEOUT` (default 300 s) plus one monitor tick, and receives the monitor's stalled-Director ping. (manual verification skipped by user decision)
- [x] The default database URL is `sqlite:///<HOME>/.local/share/cafleet/cafleet_v7.db`.
- [x] `cafleet/migrations/` contains exactly `V1__baseline.sql`; `cafleet setup` on a fresh path prints `Created <db_file> and applied migrations to head (1).`, and the resulting schema includes `member_placements.forced_at`.
- [x] Every command run against a pre-reset database (ledger version 9 or 10) fails with the existing ahead-of-head errors and leaves the file unchanged.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //admin:test`, and `mise //admin:lint` pass.
- [x] `SPEC.md`, `docs/docs/**`, `skills/cafleet/**`, and `.claude/rules/**` describe forced delivery, the unified forced-preview payload, the single-baseline chain, and `cafleet_v7.db`, with no mention of composer-safe holding or of migrations V2–V10.

---

## Background

PR #466 (merged 2026-10-18, merge commit `7ba5251fb907d0e0010fba28166fb09c34c94829`) replaced a loose cue-based classifier and a timed forced fallback with a strict per-agent layout parser and no fallback. Under it, a pane receives automatic input only after two fresh `Finished` observations; every other state holds indefinitely. It also removed the Escape safeguard from automatic input and dropped `member_placements.forced_at` in `V10__composer_safe_delivery.sql`.

The strict parser fails on the most important pane. `claude_composer` (`cafleet/src/pane_state.rs:135`) recognizes an idle Claude pane only when the last line is the `⏵⏵ don't ask on (shift+tab to cycle)` footer and the composer's top border carries a pane title, which only `claude --name` members have. A Director pane the user launched runs in auto mode (`⏵⏵ auto mode on …`) without a title, so it is always `Unclassified`. On 2026-10-20 (cafleet 0.27.0, herdr), every `ready` from Claude, Codex, OpenCode, and monitor members stayed in the Director inbox, and no preview reached the Director pane. The monitor's stalled-Director ping is gated the same way, so nothing re-engaged the Director.

The intent behind 0000182 was narrower: every forced delivery should tell the recipient that it may have interrupted its work and should resume. Before #466, only the `working` and `awaiting_user` forced payloads said anything; a forced delivery into an unclassified pane carried the bare payload.

Only Renovate dependency bumps and a model-list refresh (`skills/cafleet/reference/coding-agents.md`, `docs/docs/concepts/coding-agents.md`, `.claude/skills/cafleet-model-list-refresh/SKILL.md`) have landed since #466. None touches a file #466 changed, so `git revert -m 1 7ba5251f` applies cleanly.

---

## Specification

### Decisions

| Topic | Decision | Rationale |
|---|---|---|
| Revert scope | Wholesale revert of #466: classifier, delivery gate, ping and wake gating, pane-claim receipts, `PaneState::Draft`, sanitized pane-state fixtures, docs, and tests | The user chose a full revert. A bounded hold guarantees delivery to every pane, which matters more than the draft protection #466 added. |
| Escape | Restore the pre-#466 Escape-first transport for every automatic input (preview, exec dispatch, exec resume, ping, wake, `/exit`) | This is pre-#466 behavior. The `awaiting_user` note ("This delivery's Escape dismissed a pending prompt…") is accurate only because Escape precedes the payload. |
| Forced-preview payload | The resume clause on every forced preview; `awaiting_user` adds the dismissed-prompt note after it | The user's intent for 0000182, without losing the `awaiting_user` explanation |
| Hold timeout | `CAFLEET_DELIVERY_HOLD_TIMEOUT`, default `300`; `0` disables forcing | Unchanged from pre-#466 |
| Schema delivery | A new default file `cafleet_v7.db`, the chain squashed to a single `V1__baseline.sql`, and V2–V10 deleted | A fresh file avoids a `V11` that re-adds `forced_at` on top of `V10`, which dropped it. |
| Old databases | No migration path; the existing ahead-of-head refusal handles them | A ledger at 9 or 10 is ahead of head 1. Failing fast is the intended behavior. |
| Duplicate-monitor migration diagnostic | Removed | It guards only the `V8` unique-index migration on a populated database. With a single baseline, every database at head was created with the index, so the check can never fire. |

Issue #459 (a preview typed into an operator's unsent draft, and Escape interrupting a running Director turn) returns with the revert. The bounded hold limits its frequency: at most one forced delivery per pane per timeout window. This is the accepted trade-off.

### Forced-preview payload

The table replaces the pre-#466 payload table in `SPEC.md` §6.5 and `docs/docs/spec/multiplexer-backends.md`:

| Delivery | Payload |
|---|---|
| Ordinary (pane `finished`) | The base payload |
| Forced on `working` or unclassified | The base payload, then `[cafleet] Resume your work if something was still running.` |
| Forced on `awaiting_user` | The base payload, then `[cafleet] Resume your work if something was still running.`, then `[cafleet] This delivery's Escape dismissed a pending prompt in your pane; that rejection did not come from the user. Re-issue the tool call if you still need it.` |

The base payload is unchanged: `[cafleet msg <id> from <sender> <ts>]` followed by the truncated text, with line breaks folded to `⏎`. Each note sits on its own line after the base payload, so a forced preview is three or four lines. These are soft line breaks typed with a single `-l` keystroke and submitted with one `Enter`.

Forced exec dispatch and exec resume keep their pre-#466 payloads. The dispatch line `! cafleet member exec-run <id>` is a shell command and cannot take a trailing clause. The resume prompt already instructs the member to continue. Ping and wake payloads already end with the resume clause.

`delivery.rs` after the revert:

```rust
macro_rules! resume_clause {
    () => {
        "[cafleet] Resume your work if something was still running."
    };
}

const RESUME_CLAUSE: &str = resume_clause!();
const AWAITING_USER_NOTE: &str = concat!(
    resume_clause!(),
    "\n[cafleet] This delivery's Escape dismissed a pending prompt in your pane; \
     that rejection did not come from the user. Re-issue the tool call if you still need it.",
);

/// The lines a forced preview adds for the state the broker observed.
fn forced_note(state: PaneState) -> &'static str {
    match state {
        PaneState::AwaitingUser => AWAITING_USER_NOTE,
        PaneState::Working | PaneState::Unclassified => RESUME_CLAUSE,
        PaneState::Finished => unreachable!("a finished pane receives an ordinary delivery"),
    }
}
```

The call site becomes `let note = forced.then(|| forced_note(state));`. Both constants expand the same `resume_clause!` literal, so the two copies cannot drift. `inline_preview_payload` and the `PaneIo` / `Multiplexer` `note: Option<&str>` signatures do not change: an embedded `\n` in the note is a soft break, the same as the existing separator between the base payload and the note.

### Delivery behavior after the revert

The pre-#466 contract in `SPEC.md` §6.5 *Held delivery* and `docs/docs/spec/multiplexer-backends.md#held-delivery` comes back verbatim, except for the payload table above and the gate-table row below:

| Observed state | Before the timeout | At or after the timeout |
|---|---|---|
| `finished` | Ordinary delivery | Ordinary delivery |
| `awaiting_user` | Hold | Forced delivery; a preview adds the resume clause and the dismissed-prompt note |
| `working` | Hold | Forced delivery; a preview adds the resume clause |
| unclassified | Hold | Forced delivery; a preview adds the resume clause |

The hold age is `now − max(item timestamp, finished_at of the pane's latest exec, forced_at)`. A forced delivery stamps `member_placements.forced_at`, which restarts the clock for everything queued behind it. `PaneOutcome::Fired { item, forced }` and the monitor log line `<iso-ts> tick -> forced preview msg <id> member <member_id>` return unchanged.

### Database reset

**Default URL.** `Settings::from_lookup` (`cafleet/src/config.rs:26`) defaults to `sqlite:///{home}/.local/share/cafleet/cafleet_v7.db`. `CAFLEET_DATABASE_URL` still overrides it.

**Single baseline.** `cafleet/migrations/V1__baseline.sql` is rewritten to the post-revert head schema, which is `SPEC.md` §8 at `7ba5251f^1` with V2–V9 folded in. It is DDL only, with no data statements:

```sql
-- members is created first: members.fleet_id forward-references the
-- still-uncreated fleets table, which SQLite tolerates because FK
-- enforcement is per-connection (PRAGMA foreign_keys=ON), not DDL-time.

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL,
    deregistered_at TEXT,
    member_card_json TEXT NOT NULL
);

CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
ON members(fleet_id)
WHERE status = 'active'
  AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT
);

CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    cafleet_version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT NOT NULL,
    mux_pane_id TEXT,
    backend TEXT NOT NULL DEFAULT 'tmux',
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL,
    keystroke_at TEXT,
    forced_at TEXT,
    silence_notice_at TEXT
);

CREATE TABLE monitor_runtime (
    fleet_id INTEGER NOT NULL PRIMARY KEY REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    pid INTEGER,
    started_at TEXT,
    last_tick_at TEXT,
    tick_seconds INTEGER NOT NULL DEFAULT 5,
    last_wake_at TEXT,
    wake_interval_seconds INTEGER,
    wake_requested_at TEXT
);

CREATE TABLE messages (
    message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    owner_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    from_member_id INTEGER NOT NULL,
    to_member_id INTEGER,
    type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    status_state TEXT NOT NULL,
    status_timestamp TEXT NOT NULL,
    origin_message_id INTEGER,
    text TEXT NOT NULL,
    notified_at TEXT
);

CREATE INDEX idx_messages_owner_member_status_ts ON messages (owner_member_id, status_timestamp);
CREATE INDEX idx_messages_from_member_status_ts ON messages (from_member_id, status_timestamp);

CREATE TABLE member_execs (
    exec_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    command TEXT NOT NULL,
    created_at TEXT NOT NULL,
    dispatched_at TEXT,
    started_at TEXT,
    pid INTEGER,
    finished_at TEXT,
    exit_code INTEGER,
    resumed_at TEXT
);
```

The per-member monitor schedule table that V1 created and V4 dropped is not part of the new baseline. The existing head-schema assertions in `cafleet/src/db/mod.rs` (`baseline_creates_exactly_the_head_tables`, the index, default, nullability, FK, CHECK, and `asset_installs` primary-key tests) are the guard: they must pass unchanged against the new baseline, apart from any assertion that names the head version.

**Pre-reset databases.** When `CAFLEET_DATABASE_URL` names a pre-reset file, its ledger records version 9 or 10, which is above the new head 1. The existing refusals apply unchanged:

| Command | Outcome |
|---|---|
| `cafleet setup` | `DB schema is at version <M> which is unknown to this version of cafleet. Refusing to downgrade automatically.`; exit 1 |
| Every schema-guarded command | `database schema <M> is newer than this cafleet (head 1); upgrade cafleet`; exit 1 |
| `cafleet doctor` | Reports the database section as ahead of head and exits non-zero |

`docs/docs/concepts/storage.md` tells an operator to unset `CAFLEET_DATABASE_URL` or point it at a new file. The old `cafleet_v6.db` remains on disk untouched. cafleet never deletes or reads it.

**Behind-head guard.** `SchemaState::Behind` and its `outdated … run 'cafleet setup'` error stay: they are the general mechanism for the next `V2`. No database can be behind head 1, so the tests that hand-write a ledger at version 5 (`seed_pre_v6_database`) are retargeted. The helper becomes `seed_pre_reset_database`, which writes the ledger at versions 1–10 to model the real pre-reset file. The tests built on it assert the ahead-of-head outcome.

**Removed with the chain:**

- `cafleet/migrations/V2__…` through `V9__…` (V10 goes with the revert).
- `duplicate_monitor_diagnostic` and both call sites in `cafleet/src/cli/setup.rs`, plus its `seed_version` tests.
- The `old_schema` helper and the old-schema tests in `cafleet/tests/monitor_uniqueness.rs` (`unrelated_ddl_failure_preserves_original_cause_and_pending_schema`, `setup_sorts_duplicate_diagnostics_preserves_data_and_still_installs_assets`). Head-schema index tests stay.
- The documentation of the `active monitor duplicates prevent migration: …` error, the duplicate-monitor recovery procedure (`SPEC.md` §8, `docs/docs/concepts/storage.md#duplicate-monitor-recovery`, `docs/docs/spec/cli-options.md` setup outcome and error-table rows), and every per-version chain description (V2–V9) in `SPEC.md` §8 and `docs/`.

The `V6` asset-row-reset exception in `SPEC.md` §8 goes too. A fresh baseline has no rows to preserve.

### Documentation updates

Restoring the pre-#466 documentation comes from the revert. On top of it:

| Target | Change |
|---|---|
| `SPEC.md` §6.5 *Held delivery* | Gate-table unclassified row and the payload table, per § Forced-preview payload |
| `SPEC.md` §8 | Head is `V1`; describe the single baseline (the DDL above); drop the chain list, the V6 exception, step (6) of the `setup` driver, and the duplicate-monitor recovery; the `Upgraded from <M> to <N>.` sentence names no concrete head beyond `1` |
| `SPEC.md` §6.1 / §6.2 / config table | Default database path `cafleet_v7.db` |
| `SPEC.md` §9 | *Delivery step* bullet: "the forced payload per state (the resume clause on every forced preview)" |
| `docs/docs/spec/multiplexer-backends.md` | Gate table and payload table, as for `SPEC.md` §6.5 |
| `docs/docs/concepts/monitoring.md` | The forced-delivery paragraph says every forced preview tells the recipient to resume, and an `awaiting_user` one also names the dismissed prompt |
| `docs/docs/concepts/storage.md` | Default path `cafleet_v7.db`; the schema is a single embedded baseline; a pre-reset database is refused (point `CAFLEET_DATABASE_URL` elsewhere); remove the duplicate-monitor section and its table row |
| `docs/docs/spec/cli-options.md` | Default database path; remove the duplicate-monitor setup outcome and error rows |
| `skills/cafleet/SKILL.md` § Send | Forced-preview sentence: "is forced into the pane with a `[cafleet]` line telling the recipient to resume its work, plus a note naming a dismissed prompt when one was open" |
| `skills/cafleet/reference/supervision.md` § Broker-held delivery | The "A hold is bounded" bullet says every forced delivery carries the resume clause |
| `README.md` | Run `/update-readme` and apply it only if the pitch or install surface drifted (no change expected) |

Every page touched by #466 is restored by the revert, including `skills/cafleet/roles/director.md`, `skills/cafleet/roles/monitor.md`, `.claude/rules/bash-tool.md`, `docs/docs/how-to/use-the-webui.md`, `docs/docs/spec/data-model.md`, and `docs/docs/spec/webui-api.md`. After the revert, check the `docs_sync.rs` assertions against the edited pages.

### Tests

| Test | Change |
|---|---|
| `cafleet/tests/delivery.rs` (restored by the revert) | The forced-on-unclassified case expects the resume-clause line; the forced-on-`awaiting_user` case expects the resume clause followed by the dismissed-prompt note |
| `cafleet/src/delivery.rs` unit tests, if any | Cover `forced_note` for the three forcible states |
| `cafleet/src/multiplexer/tmux.rs` / `herdr.rs` | One case types a two-line note (`resume\nnote`) after the base payload in a single `-l` keystroke |
| `cafleet/src/config.rs` (`config.rs:108`) | Default URL ends in `cafleet_v7.db` |
| `cafleet/tests/common/mod.rs`, `cli_global.rs`, `cli_setup_doctor.rs` | Expect `cafleet_v7.db`; `seed_pre_v6_database` becomes `seed_pre_reset_database` (ledger 1–10); tests that used the behind-head seed assert the ahead-of-head errors and report shapes (`schema_version` = `10`) |
| `cafleet/src/db/mod.rs` | `migrate_reaches_head_version_1_and_is_idempotent`; the chain guard becomes `migration_chain_is_contiguous_from_1_with_exactly_one_baseline_and_head_1` and expects head `V1__baseline` |
| `cafleet/tests/composer_safe_migration.rs` | Deleted by the revert |
| `cafleet/src/cli/setup.rs`, `cafleet/tests/monitor_uniqueness.rs` | Remove the old-version and duplicate-diagnostic tests listed under § Database reset |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Revert PR #466

- [x] Create a feature branch from `main` and run `git revert -m 1 7ba5251fb907d0e0010fba28166fb09c34c94829`, committing it as `revert: restore forced delivery by reverting #466` <!-- completed: 2026-10-20T10:45 -->
- [x] Run `mise //cafleet:test` and `mise //cafleet:lint` on the revert alone (the database is still at head 9) to confirm the pre-#466 baseline is green before further edits <!-- completed: 2026-10-20T10:45 -->

### Step 2: Documentation — forced-preview payload

- [x] Update `docs/docs/spec/multiplexer-backends.md` gate table and payload table per § Forced-preview payload <!-- completed: 2026-10-20T10:45 -->
- [x] Update `docs/docs/concepts/monitoring.md` forced-delivery paragraph <!-- completed: 2026-10-20T10:45 -->
- [x] Update `SPEC.md` §6.5 gate table, payload table, and §9 *Delivery step* bullet <!-- completed: 2026-10-20T10:45 -->
- [x] Update `skills/cafleet/SKILL.md` § Send and `skills/cafleet/reference/supervision.md` § Broker-held delivery <!-- completed: 2026-10-20T10:45 -->

### Step 3: Documentation — database reset

- [x] Update `docs/docs/concepts/storage.md`: `cafleet_v7.db`, single baseline, pre-reset refusal guidance; remove the duplicate-monitor section and row <!-- completed: 2026-10-20T10:45 -->
- [x] Update `docs/docs/spec/cli-options.md`: default path; remove the duplicate-monitor setup outcome and error rows <!-- completed: 2026-10-20T10:45 -->
- [x] Rewrite `SPEC.md` §8 to the single-baseline schema and setup driver; update the default path in §6.1 / §6.2 and the configuration table <!-- completed: 2026-10-20T10:45 -->
- [x] Run `/update-readme` to confirm `README.md` and `SPEC.md` agree with `docs/`; grep `docs/`, `SPEC.md`, `skills/`, `.claude/rules/` for `cafleet_v6`, `V9`, `V10`, `duplicates prevent migration`, `composer-safe`, and `Draft` leftovers <!-- completed: 2026-10-20T10:45 -->

### Step 4: Forced-preview payload code

- [x] Write the failing tests: forced-on-unclassified and forced-on-`awaiting_user` payload expectations in `cafleet/tests/delivery.rs`; the two-line note keystroke in the tmux and herdr transport tests <!-- completed: 2026-10-20T10:45 -->
- [x] Implement `forced_note` and the shared resume-clause constants in `cafleet/src/delivery.rs` per § Forced-preview payload <!-- completed: 2026-10-20T10:45 -->
- [x] Run `mise //cafleet:test` <!-- completed: 2026-10-20T10:45 -->

### Step 5: Database reset code

- [x] Change the default database URL in `cafleet/src/config.rs` and its test to `cafleet_v7.db`; update `cafleet/tests/common/mod.rs`, `cli_global.rs`, and `cli_setup_doctor.rs` expectations <!-- completed: 2026-10-20T10:45 -->
- [x] Rewrite `cafleet/migrations/V1__baseline.sql` to the DDL in § Database reset and delete `V2__…` through `V9__…` <!-- completed: 2026-10-20T10:45 -->
- [x] Update `cafleet/src/db/mod.rs`: head-version test to 1, chain guard to a single baseline; confirm the head-schema tests pass unchanged <!-- completed: 2026-10-20T10:45 -->
- [x] Remove `duplicate_monitor_diagnostic`, its call sites, and its tests from `cafleet/src/cli/setup.rs` <!-- completed: 2026-10-20T10:45 -->
- [x] Remove the old-schema helper and tests from `cafleet/tests/monitor_uniqueness.rs` <!-- completed: 2026-10-20T10:45 -->
- [x] Replace `seed_pre_v6_database` with `seed_pre_reset_database` (ledger 1–10) and retarget its callers to the ahead-of-head outcomes <!-- completed: 2026-10-20T10:45 -->
- [x] Add a CLI test: `setup` against a pre-reset seed exits 1 with the ahead-of-head refusal and leaves the ledger unchanged <!-- completed: 2026-10-20T10:45 -->
- [x] Run `mise //cafleet:test` <!-- completed: 2026-10-20T10:45 -->

### Step 6: Verification

- [x] Run `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:test`, `mise //admin:lint`, and `mise //admin:test` <!-- completed: 2026-10-20T10:45 -->
- [x] Run `git diff 7ba5251f^1 HEAD --stat` and confirm that every difference from pre-#466 is a change this document specifies <!-- completed: 2026-10-20T10:45 -->
- [x] Run `mise //cafleet:install`, then `cafleet setup` (creates `~/.local/share/cafleet/cafleet_v7.db`) and `cafleet doctor` (skipped by user decision) <!-- completed: 2026-10-20T11:00 -->
- [x] Manual check on herdr: from a user-launched auto-mode Claude Director pane with no title, create a fleet with a monitor; confirm the monitor's `ready` reaches the Director pane, ordinarily or as a forced preview ending in the resume clause, within the hold timeout (skipped by user decision) <!-- completed: 2026-10-20T11:00 -->
- [x] Commit the design doc alongside the implementation (`docs: add design doc 0000183-restore-forced-delivery-and-db-reset`) <!-- completed: 2026-10-20T10:45 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-20 | Initial draft |
