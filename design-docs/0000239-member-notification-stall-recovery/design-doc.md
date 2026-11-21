# Member notification stall recovery

**Status**: Approved
**Progress**: 51/57 tasks complete
**Last Updated**: 2026-11-21

## Overview

This design supersedes `design-docs/0000237-automatic-held-queue-flush` (issue #633). A runtime event that the runner's event writer can never persist is replaced by a durable `runner_degraded` diagnostic, so it can no longer hold every member input as `held / observer_unavailable` indefinitely. `cafleet member show` and `cafleet doctor` now show the observation and writer state, and other changes cover:

- persisted runner diagnostics;
- re-observation of the idle screen when a gap recovers;
- a reconcile action for input held behind an uncertain prompt or exec.

## Success Criteria

- [ ] A transition whose persistence fails with a replaceable permanent cause (§ Writer failure classification) is replaced within the same writer step by a `runner_degraded` event with cause kind `event_rejected` at the same `event_seq`. A successful replacement leaves the writer diagnostic, the gap count and the observation state untouched, so no gap opens, and the writer continues with the next transition. A transient cause (busy, deadline or SQLite I/O) keeps retrying with the 100 ms → 2 s backoff, and the gap stays open while it lasts. A replacement that also fails permanently fences the writer.
- [ ] Every `event_seq` slot a live runner materializes is filled. A stalled-head `input_delivery_stalled` record whose input is no longer `ready` at persistence commits an archived row with `archive_reason` `head_not_ready`. A delivery record that duplicates an already persisted one commits an archived row with `archive_reason` `duplicate_delivery`. Neither creates an inbox message or a Director-visible event, and the next transition persists normally.
- [ ] `writer.rs` has no `WORK_SLICE` and no per-step work slice. Each writer step is bounded by 5 s of real time and by the shutdown deadline on the injected clock, so a virtual-time jump never expires a step. `set_deadline`/`begin_shutdown` interrupts an in-flight step once the shutdown deadline passes.
- [ ] `ObservationController::update()` keeps `self.next_deadline`. After a gap closes, the unchanged idle screen is re-observed without new child output or a resize, and an unchanged permission form opens its interaction once its candidate settle elapses.
- [ ] `cafleet member show MEMBER_ID --json` reports the live probe outcome (`observation_probe`, `observation`, `observation_error`) and the run's latest 5 `diagnostics`. Its text runtime line appends `observation=<state>`, plus ` degraded` when degraded.
- [ ] `cafleet doctor` prints one `member.<id>.runtime` row per active ordinary member with a non-terminal run. The row is `error` when the probe fails or a gap has been open for more than 30 s, and every `error` row carries the recovery hint. An `error` member row makes doctor exit 1.
- [ ] Every runner `member runtime:` diagnostic, the shutdown outcome diagnostics included, and every writer and observation failure is stored in `member_runner_diagnostics`. A repeat of the same source and message increments `count`, and each run keeps at most 256 rows. Up to 32 entries are buffered while storage is unwritable and flushed after recovery, and the entries still buffered when the event writer finishes are written by the shutdown cleanup transaction. `eprintln!` stays as a secondary copy.
- [ ] A queued head or a pending preview held 3 minutes behind an `uncertain` prompt or exec publishes one `input_delivery_stalled` event, which:
  - names the blocker in `blocked_by_input_id`;
  - carries the director action `reconcile_uncertain_input`;
  - is deduplicated per run and blocking input.

  A head that such an input blocks publishes no `review_stalled_delivery`.
- [ ] `design-docs/0000237-automatic-held-queue-flush/` is deleted.
- [ ] `SPEC.md`, `docs/`, `assets/skills/cafleet/reference/cli.md` and `assets/skills/cafleet/roles/director.md` describe the new behavior. `mise //:test`, `mise //:lint` and `mise //docs:build` pass.
- [ ] The § Live verification run with the branch binary (Claude Code on Herdr, dedicated database) shows the expected `member show` and doctor values for its healthy, permanent-rejection and transient-outage phases.

---

## Background

The investigation of #633 (issue comment 7062880993) established the mechanism behind the issue's Pattern 2, where idle members answered `held / observer_unavailable` for hours:

1. The event worker failed to persist one transition and failed again on every retry. `Mailbox::failure` re-sets the writer diagnostic after every failed step, and only a successful persist clears it.
2. While the diagnostic is set, `ObservationController::poll_at` and `observe_screen` keep a `Sink` gap open, and `ordinary_blocker` answers every send `held / observer_unavailable`.
3. The control socket and health probe still answer, and held sends refresh `last_health_at`, so `doctor` and `member show` report the member as healthy.
4. At shutdown the writer cannot drain, so recovery records `runner_lost (monitor_lost)`.

Which event failed is still unknown, because the runner prints writer failures only to the overdrawn member pane.

Code reading of current `main` (c181cbd8) adds these facts:

| Fact | Consequence |
|---|---|
| `persist_transition_in_transaction` requires `event_seq` = latest committed + 1, and the controller fixes each seq at materialization | Dropping event N makes N+1 fail with `event sequence is not the run's next value` forever, so a skip must fill the slot |
| A stalled-head record waits in `ObservationController::delivery_results` until a quiet poll, and persistence then requires `a new stalled head must be ready` | When the head reserves its attempt or settles first, that record becomes a poison event |
| Many persistence lookups fail as `CafleetError::App` (`interaction_id_for_episode`, `get_input`, `get_message`, `historical_ownership`, `RunLock::validate`) | Treating `App` as retryable would recreate the hold, so only busy, deadline and SQLite I/O classes may retry |
| `poll_at` sets `next_deadline = Some(now)` when a gap closes, and the delivery publication in the same poll calls `retain(self.update(..))`, which resets it to `None` | An idle member stays `Hold(Gap)` until new output arrives, and an unchanged permission form never opens its interaction |
| A delivery dedup hit in `persist_transition_in_transaction` returns `Ok` and inserts nothing at the transition's `event_seq` | A repeated stalled record leaves its slot unfilled, and every later persist fails the sequence check, which is a plausible trigger for Pattern 2 |
| `handle_preview` returns `held("earlier_input")` with no stall watch, and `queue_housekeeping` publishes `review_stalled_delivery` for a head that an uncertain input blocks | A preview behind an uncertain prompt/exec waits silently, and a blocked head gets the wrong Director action |

The writer's 100 ms `WORK_SLICE` (`src/runtime/writer.rs:21`) clips every persist to 100 ms, which turns ordinary SQLite contention into an observation gap. **User decision:** that limit is unnecessary and is removed.

Design `0000237-automatic-held-queue-flush` (Approved, unimplemented) fixes neither pattern and targets code `main` has already removed (`member flush`, `held_composer`, preview receipts). **User decision:** this design supersedes it, and its folder is deleted. Only its runner-failure trace idea survives, in reduced form, as § Runner diagnostics.

Out of scope:

- the issue's Pattern 1 receipt form, which cannot recur on `main`;
- identifying the 0.35.4 poison event;
- the hidden Herdr tab hypothesis, which the investigation ruled out;
- 0000237's automatic flush and `member trace`.

---

## Specification

### Writer failure classification

`From<rusqlite::Error>` gains one mapping. The SQLite I/O-class codes `SystemIoFailure` (`IOERR`), `DiskFull` (`FULL`), `CannotOpen` (`CANTOPEN`), `OutOfMemory` (`NOMEM`), `ReadOnly` (`READONLY`) and `FileLockingProtocolFailed` (`PROTOCOL`) map to the new `CafleetError::DatabaseIo(String)`. That variant keeps the `database error: …` message, has exit code 1 and behaves like `App` everywhere else; `input.rs` `storage_failure` is unchanged. `CafleetError::transient_storage(&self) -> bool` returns true exactly for `DatabaseBusy`, `DeadlineExceeded` and `DatabaseIo`, looking through `WithDiagnostics` to its cause. The event worker classifies every failed step by this predicate alone. Every other cause is permanent.

| Cause | Examples | Class | Outcome |
|---|---|---|---|
| `DatabaseBusy` | `SQLITE_BUSY`, `SQLITE_LOCKED` | Transient | Sets the diagnostic, keeps the transition at the front and retries after the 100 ms → 2 s backoff |
| `DeadlineExceeded` | The 5 s step bound (§ Writer step bound) | Transient | Same as `DatabaseBusy` |
| `DatabaseIo` | `IOERR`, `FULL`, `CANTOPEN`, `NOMEM`, `READONLY`, `PROTOCOL` | Transient | Same as `DatabaseBusy` |
| Transition content | `InvalidStoredValue` from `validate_transition`, cause/evidence/delivery/stalled validation, the 16 KiB payload bound | Permanent | Replaced by its rejection event |
| Missing referenced row | `App` from `interaction_id_for_episode` (`Query returned no rows`), `get_input` (`Input N not found`), `get_message` | Permanent | Replaced by its rejection event |
| Constraint or trigger | `SQLITE_CONSTRAINT*` (including `RAISE(ABORT)` triggers and refused commits) | Permanent | Replaced by its rejection event |
| Sequence slot | `event sequence is not the run's next value` (an unfilled slot below), `conflicting original transition` (the slot holds a different transition) | Permanent | The replacement uses the same `event_seq` and fails the same check, so the writer fences |
| Lost live ownership | `RunLock::validate` `stale runtime owner` and its runtime-path checks; `validate_owner` `stale ownership token`; `terminal runs accept only committed retries or owned recovery`; `live notification requires active runner ownership` after the member, Director or fleet becomes inactive; `require_live_fleet`; `historical_ownership`'s missing member | Permanent | The replacement runs the same check and fails, so the writer fences |
| Corrupt file | `DatabaseCorrupt`, `NotADatabase` | Permanent | Replacement fails as well, so the writer fences |

A `RunLock::validate` failure is therefore permanent. Because `persist_front` validates the lock before `persist_transition`, a run that lost ownership reaches the fence after its first replacement attempt. A sequence-slot failure means an earlier slot was left unfilled. No live path does that: every outcome of `persist_transition_in_transaction` either fails or commits a row at `transition.event_seq` (§ Stalled head no longer ready, § Duplicate delivery records).

**Replacement within the step.** `persist_front` handles a permanent failure inside the same worker step, without calling `Mailbox::failure`:

1. It replaces the queue front with the rejection event (§ Rejection event) and records the rejection under source `event_writer` (§ Runner diagnostics).
2. It persists the replacement at once, in its own step bound (§ Writer step bound).
3. On success it pops the front and calls `Mailbox::success`. The rejection itself never sets the writer diagnostic or increments `gaps`, so it opens no gap.
4. On a transient failure the replacement stays at the front, and the step fails with that cause, which takes the ordinary transient path (diagnostic, backoff, retry of the replacement).
5. On a permanent failure the writer fences.

**Controller hand-off.** `RetainedTransition` gains `reason: RetainReason`, which `ObservationController::route` matches instead of inspecting `cause`:

| `RetainReason` | `try_submit` refusal | `Mailbox::failure` | `route` outcome |
|---|---|---|---|
| `EvidenceInvalid` | `storage::validate_evidence` fails | Not called | Materializes the rejection event for the same `event_seq` and submits it in place of the original, with no `delivery_pressure` call |
| `ChannelFull` | The bounded channel is full or disconnected | Called | Current retained-transition behavior: keeps it and opens a `Sink` gap |
| `Stopped` | `event writer is stopped` | Called | Same as `ChannelFull` |
| `SequenceRegressed` | `accepted event sequence regressed` | Called | Same as `ChannelFull` |
| `Fenced` | `event writer is fenced` | Not called | Same as `ChannelFull` |

`Fenced` skips `Mailbox::failure` because the fence already set the diagnostic and opened the gap. The fence diagnostic `event writer fenced: <cause>` therefore stays in `writer_diagnostic`, and refusals after the fence add no `event_writer` diagnostic row.

A refused replacement is retained under its own reason like any other transition. `run_evidence` always produces valid evidence, so the replacement is never refused as `EvidenceInvalid`; `route` asserts this.

**Fence.** When a rejection event itself fails permanently, the writer:

- discards its queue, counting the discarded transitions as unsaved;
- keeps the permanent diagnostic `event writer fenced: <cause>`;
- records it under source `event_writer` (§ Runner diagnostics);
- accepts no further transitions (`try_submit` returns `event writer is fenced`).

The `Sink` gap stays open, so input stays held, and `member show` and `doctor` report the member as `error` after 30 s. A fenced run recovers only through `member delete` and a replacement member.

### Rejection event

The replacement is a run-level `runner_degraded` transition:

| Field | Value |
|---|---|
| `event_seq` | The rejected transition's `event_seq` |
| `episode_seq`, `classification` | `null` |
| `observed_at` | Wall-clock time of the rejection |
| `evidence` | `storage::run_evidence(run, "runner-lifecycle", screen_seq)`, where `screen_seq` is the rejected evidence's `screen_seq` (writer side, validated by `try_submit`) or the controller's current `screen_seq` (controller side) |
| `cause` | `{"kind":"event_rejected","rejected_kind":KIND,"rejected_observed_at":TIMESTAMP,"error":MESSAGE}` |
| `director_action` | `null`, like every `runner_degraded` |

`MESSAGE` is the cause's message after § Diagnostic text sanitizing. `RuntimeCauseKind` gains `EventRejected => "event_rejected"`, compatible only with `RuntimeEventKind::RunnerDegraded`. Its allowed cause keys are exactly `kind,rejected_kind,rejected_observed_at,error`:

- `rejected_kind` is a valid `RuntimeEventKind`;
- `rejected_observed_at` is RFC 3339;
- `error` satisfies the sanitizing rule.

A rejected `interaction_opened` creates no interaction. A later `interaction_closed` for that episode then fails `interaction_id_for_episode` and is rejected too, with its own `event_rejected`. The Director learns of the lost interaction from `rejected_kind` and handles the visible request on the native surface (§ Skills and documentation).

### Stalled head no longer ready

The stalled-head ready check moves from an error to an archival outcome. `DeliveryFields` gains `archive: Option<ArchiveReason>`. With `published=false`, `stalled_delivery(conn, run, record, published)` sets `archive: Some(HeadNotReady)` when the named head is a queued ordinary input of the run (`deferred_at` set) whose state is not `ready`, in place of the `a new stalled head must be ready` error. `persist_transition_in_transaction` then commits the event as a silent archived row:

- `notification_state='archived'`, `message_id` NULL and `archive_reason='head_not_ready'`;
- no inbox message;
- the normal payload built from the transition's own record, with its `event_seq` filling the slot, so `original()` reproduces the transition and an idempotent retry compares equal.

Every other stalled-head violation, such as another run's input or no deferred metadata, stays a permanent rejection. This covers every path that moves a head out of `ready`, before or after hand-off, with no change to `InputCoordinator`. Deduplication still finds the archived row by run and `input_id`, and the head never returns to `ready`.

### Duplicate delivery records

The delivery dedup branch of `persist_transition_in_transaction`, shared by `input_delivery_result` and `input_delivery_stalled`, currently returns `Ok((existing_event, false))` and inserts nothing at `transition.event_seq`. It now fills the slot. On a dedup hit (a row of the same kind already names the same waiting input, preview or blocker, § Stall behind an uncertain input), it builds the transition's delivery fields exactly as for a first record and commits a silent archived row with `archive_reason='duplicate_delivery'`, through the same mechanism as `head_not_ready`. It returns the new row as inserted. `duplicate_delivery` takes precedence over `head_not_ready`, and both apply only when `historical_ownership` supplies no archive reason. A delivery-field failure on a duplicate is a permanent rejection like any other, which also fills the slot (§ Writer failure classification).

These live paths can materialize a stalled record that duplicates a persisted one:

| Path | Duplicate key |
|---|---|
| `watch_head` clears `stalled` while an interaction blocks the head, and the interaction's closure restarts the watch, which can stall again | The head's `input_id` |
| `watch_preview` restarts the same way after an interaction | The preview's `message_id` |
| A blocker settles and the head's ordinary watch starts afresh (§ Stall behind an uncertain input) | The head's `input_id` |
| `blocked_wait` clears when no waiter remains and restarts for the same blocker when a new waiter appears | `blocked_by_input_id` |

`publish_delivery_result` compares `kind`, `input_id`, `message_id` and `blocked_by_input_id`, so two blocked records that name different blockers are both queued. It still drops a repeat while the first is queued; after hand-off the persistence rule above handles the repeat.

`archive_reason`'s CHECK and `ArchiveReason` gain `head_not_ready` and `duplicate_delivery`, the archive reasons a live runner writes.

### Writer step bound

`WORK_SLICE`, `Worker::work_slice`, `Worker::work_deadline`, `Mailbox::work_deadline` and `WriterHooks::work_slice` are deleted. Each deadline is measured on the clock that owns it: the step bound on real time, as the database budget already is, and the shutdown deadline on the worker's injected clock.

- **Step bound.** Each worker step (receive, then persist with `RunLock::validate` and `owned_transaction`) carries a `StepBound`. Its `until` is `Instant::now() + WRITER_STEP_BOUND`, where `WRITER_STEP_BOUND` is 5 s, equal to `db::connect`'s `busy_timeout`. It also holds the mailbox's `SharedDeadline` and the worker's clock. `StepBound::check()` fails `DeadlineExceeded` when `Instant::now() >= until`, or when the shared shutdown deadline is set and `clock.now()` has reached it. `receive()` calls `check()` where it calls `check_deadline` today. The persist runs in `with_shared_database_deadline(conn, until, shared, operation)`, and `owned_transaction`'s `Deadline::inherited_or_new()` inherits that scope, so one step never exceeds it. An expired step fails `DeadlineExceeded`, a transient cause. A replacement (§ Writer failure classification) gets a fresh `StepBound`.
- **Interruption.** `db.rs` gains `SharedDeadline`, an `Arc<Mutex<Option<Instant>>>`, and `with_shared_database_deadline(conn, until, shared, operation)`. Its scope frame stores the shared cell. `database_cancelled` and `remaining_database_wait` compare `Instant::now()` with `min(frame deadline, current shared value)`, so lowering the shared value takes effect mid-statement: within 1,000 VM steps through the progress handler, or one 10 ms busy-handler slice. The `Mailbox` holds its shutdown deadline in that `SharedDeadline`, and `set_deadline`/`begin_shutdown` lower it. The worker loop's own shutdown check keeps comparing `self.clock.now()` with that value.

In production the worker's clock is `SystemClock`, whose instants are real, so both parts apply to real time. Under a `YieldingClock` over a `TestClock`, a virtual-time jump moves only the injected clock: it can reach the shutdown deadline but never the step bound, so `skip_idle_turns` and `Step::Run` cannot expire a step. The shutdown instant then lies `VIRTUAL_LEAD` in the real future, so it never interrupts a database statement. Tests that assert interruption or a held lock therefore use the real-clock `start` helper.

`EventWriter::finish(until)` waits for the worker thread until it finishes or the worker's clock reaches `until + WRITER_JOIN_GRACE` (50 ms), and then joins it unconditionally. Because an in-flight step aborts at `until`, the join returns, and no transition committed after `finish` returns is counted unsaved. The 100 ms → 2 s retry backoff is unchanged.

`WriterHooks::before(boundary, bound: &StepBound)` receives the step's bound in place of an `Instant`. Every test hook pause ends when `bound.check()` fails, so a pause ends at the step bound or at the lowered shutdown deadline and never blocks the join:

- `Hooks::before` (`writer.rs`) waits on its condition variable in slices of at most 10 ms and returns `DeadlineExceeded` once `bound.check()` fails.
- The `ServiceFixture` `observation_services::Hooks` `block-persist` loop (`service.rs`) checks `bound.check()` on each 2 ms iteration in place of `Instant::now() >= deadline`.

The long test slices are deleted with no replacement, because a virtual-time jump cannot reach a real-time step bound and 5 s of real time is never exhausted by scheduler delay:

- `ScenarioWriterHooks::work_slice` (`service.rs`);
- `Hooks::on_virtual_time` and the `Hooks.work_slice` field (`writer.rs`), whose callers use `Hooks::default()`;
- `InputFixtureWriterHooks::work_slice` (`input.rs`);
- `synthetic_with_unhurried_writer`, its `writer_work_slice_ms` config key and the `observation_services::Hooks.work_slice` field (`service.rs`), whose callers use `synthetic`.

### Deadline fix

The fix is the uncommitted diff found during the investigation, restated here in full because its worktree is ephemeral:

- **Code:** `ObservationController::update()` (`src/runtime/observation.rs`) builds `EpisodeUpdate { transitions, next_deadline: self.next_deadline, ..EpisodeUpdate::new(self.visibility, self.eligibility) }`.
- **New test:** the table-driven `controller_gap_recovery_reobserves_the_unchanged_screen_after_publishing_a_delivery`, with cases `no queued delivery` and `delivery queued during the gap`. Each case:
  1. sets the run's coverage to `unknown` and uses the composer catalog `composer_catalog(None, None)`;
  2. starts a writer whose `Persist` boundary is paused, so the `coverage_unknown` submit sets a writer diagnostic;
  3. feeds the idle composer bytes `\x1b[0m\x1b[2J\x1b[23;1HSynthetic state\r\nState end` and polls at 100 ms, asserting eligibility `Hold(Gap)`;
  4. in the second case, inserts a preview message and calls `publish_delivery_result(preview_delivery_result_cause(message_id, Profile, "partial", ShortWrite))`;
  5. releases the writer, waits until its diagnostic clears and nothing is pending, then polls at 200, 300 and 400 ms with no new output;
  6. asserts eligibility `ComposerEmpty`, naming the case in the message.
- **Pinned scenario:** in `src/runtime/service.rs`, the scenario named `uncertain preview` in `an_uncertain_preview_publishes_one_receipt_free_result_and_is_typed_again_once_verified_empty` expects `events: &["input_delivery_result", "interaction_opened"]`. The form drawn by `FORM_MID_WRITE` stays unchanged on screen, and after the candidate settle the kept deadline now opens its interaction.

### Observation health

`ObservationHealth` (the health control reply's `observation`) gains two fields:

| Field | Runner value | Decode rule |
|---|---|---|
| `writer_diagnostic` | The writer mailbox diagnostic after § Diagnostic text sanitizing, or null | Non-null only when `degraded`; non-empty, at most 512 bytes, no control characters |
| `gap_open_since` | `PendingGap.started_at` while a gap is pending, or null | Non-null only with eligibility `Hold(Gap)` and `degraded`; RFC 3339 |

`HealthSnapshot::decode` enforces both rules alongside its existing unknown-field, enum and size checks. Without control characters, JSON escaping expands only `"` and `\`, so `writer_diagnostic` encodes in at most 1,024 bytes. `ObservationHealth::validate`'s encoded limit therefore rises from 1,024 to 2,048 bytes. `diagnostic` keeps its `ObservationFailure` code (`sink_unavailable` while the writer is degraded).

A gap that `gap()` records after `terminal_fenced` (child exit) has no `PendingGap`, so it reports `Hold(Gap)`, `degraded` and a null `gap_open_since`. Doctor rates that state `warn`, because the run is exiting.

### Diagnostic text sanitizing

Every diagnostic text that this design stores or transmits goes through one function, `diagnostic_text(message) -> String`. That covers `event_rejected.error`, `writer_diagnostic` and `member_runner_diagnostics.message`. The function:

1. replaces each control character (U+0000–U+001F, U+007F) with a space;
2. trims the result;
3. truncates it to at most 512 bytes at a UTF-8 boundary.

Every call site passes a message that starts with a fixed non-empty label, so the result is non-empty, and the function asserts this. Messages are error texts (field names, constraint and SQLite messages) and carry no literal input, message body or pane text.

### `member show`

After its registry read, `member show` probes the runner with `control::probe_health` (1 s deadline) when the run's status is `running` or `degraded`. It appends four keys to the `runtime` object, after `child_start_identity`. Only member show gains them: member create, `member list` and `event show` keep their current runtime object and `Runtime:` line.

| Key | Value |
|---|---|
| `observation_probe` | `skipped` (status not probed), `answered` or `failed` |
| `observation` | An object member show builds from the reply's `ObservationHealth`, with every field of it in the order `visibility,eligibility,degraded,diagnostic,writer_diagnostic,pending_count,gap_count,gap_open_since`. It is null when skipped, failed, or answered by a runner whose child has not started (no `observation` in the reply) |
| `observation_error` | The probe failure's message when `failed`, otherwise null |
| `diagnostics` | The run's latest 5 `member_runner_diagnostics` rows, newest `last_at` then `diagnostic_id` first, each ordered `source,message,first_at,last_at,count` |

`visibility` is the reply's string (`visible`, `obscured`, `not_visible` or `unknown`). The other fields copy the reply's values, except `eligibility`: on the wire it is the tagged object `{"state":"hold","reason":"gap"}`, and member show replaces it with its string rendering, `composer_empty` or `hold:<reason>`, where `<reason>` is a snake-case `GateReason`: `candidate`, `interaction`, `unknown`, `observer_unavailable` or `gap`. The same string appears in the text line and in doctor rows. The text runtime line appends ` observation=<state>`, followed by ` degraded` when degraded:

| Probe outcome | `<state>` |
|---|---|
| `answered` with an observation | The eligibility rendering |
| `answered` without one | `pending` |
| `failed` | `unavailable` |
| `skipped` | `-` |

Example: `Runtime: run=37 mode=auto status=running coverage=tested observation=hold:gap degraded`.

### `doctor` member rows

When the `database` row is `ok`, doctor adds one `member.<member_id>.runtime` row right after it for each active ordinary member whose run status is not terminal, in member-ID order. The probes run concurrently, each with a 1 s deadline, and the batch overlaps the other checks the way the Hermes batch does. The first matching row applies:

| Situation | Status | Detail |
|---|---|---|
| Run `starting`, `prepared` or `recovering` | `info` | `run=<run_id> status=<status>: not probed` |
| Probe failed | `error` | `run=<run_id> health probe failed: <cause>; <hint>` |
| Probe answered without an observation | `info` | `run=<run_id> observation=pending` |
| `gap_open_since` more than 30 s before doctor's wall clock | `error` | `run=<run_id> observation=<eligibility> diagnostic=<code> gap_open_since=<ts>; <hint>` |
| `degraded` otherwise (a younger gap, no pending gap, or a terminal-fenced gap) | `warn` | `run=<run_id> observation=<eligibility> diagnostic=<code>` |
| Otherwise | `ok` | `run=<run_id> observation=<eligibility>` |

`<hint>` is exactly `inspect with 'cafleet member show <member_id> --json'; if it does not recover, run 'cafleet member delete <member_id>' and create a replacement member`. Each quoted command gets ` --database-url URL` appended for a non-default database. The 30 s bound is the constant `DOCTOR_GAP_ERROR_AGE`. An `error` member row follows doctor's existing exit rule, so doctor exits 1 with `Error: doctor found errors` when any member row is `error`. SPEC and `docs/` both state this user-visible change.

### Runner diagnostics

The table goes into `assets/migrations/V1__baseline.sql`, before the ledger table `refinery_schema_history`, which stays the last statement:

```sql
CREATE TABLE member_runner_diagnostics (
    diagnostic_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    source TEXT NOT NULL CHECK(source IN ('event_writer','observation','deferred_input','queue_housekeeping','preview_sweep','input_result','permission_answer','shutdown')),
    message TEXT NOT NULL CHECK(length(trim(message)) > 0 AND length(CAST(message AS BLOB)) <= 512),
    first_at TEXT NOT NULL,
    last_at TEXT NOT NULL,
    count INTEGER NOT NULL CHECK(count > 0),
    UNIQUE(run_id, source, message)
);
CREATE INDEX idx_member_runner_diagnostics_recent ON member_runner_diagnostics(run_id, last_at);
```

`member_runtime_events.archive_reason` CHECK gains `'head_not_ready'` and `'duplicate_delivery'`. The ledger checksum is the SHA-256 of the embedded file, so it changes with the file. The schema-shape guard (`db::reference_schema` / `schema_matches`) derives the required shape from the baseline and needs no change.

| Source | Sites |
|---|---|
| `event_writer` | A new writer diagnostic (a `Mailbox::failure` whose message differs from the current one), a rejection, a fence, a buffer overflow |
| `observation` | A gap opening, as `observation gap opened: <ObservationFailure code>` |
| `deferred_input` | `service.rs` `member runtime: deferred input: …` |
| `queue_housekeeping` | `service.rs` `member runtime: queue housekeeping: …` |
| `preview_sweep` | Both `service.rs` `member runtime: preview sweep: …` sites |
| `input_result` | `input.rs` `input … result is pending storage`, and both `result is pending recovery` sites |
| `permission_answer` | `input.rs` `permission answer for interaction … is uncertain` |
| `shutdown` | Each `service.rs` `member runtime: {diagnostic}` line of the shutdown outcome |

**Buffer.** `RunnerDiagnostics` is a shared `Arc<Mutex<…>>` holding at most 32 entries keyed by source and sanitized message. Each entry carries its count and its first and last times. Recording also prints the existing `eprintln!` line. An overflow drops the oldest entry and adds its count to `dropped`.

**Flush.** The event worker flushes on every loop iteration where the buffer is non-empty or `dropped > 0` and `now >= flush_next`, before that iteration's step. The idle loop counts too, so an eventless member flushes after recovery. Each flush is one ownership-validated immediate transaction in its own cancellable scope, bounded by `DIAGNOSTIC_FLUSH_BOUND` (500 ms of real time, like the step bound) and by the shutdown deadline. `flush_next` is on the worker's injected clock. A flush writes the buffer with the shared write sequence below.

**Write sequence.** One function, `storage::write_runner_diagnostics(tx, run_id, entries, dropped)`, used by both the flush and the shutdown transaction, runs in this order:

1. one upsert per entry: `INSERT … ON CONFLICT(run_id,source,message) DO UPDATE SET last_at=excluded.last_at, count=count+excluded.count`;
2. one `event_writer` row `diagnostic buffer overflow: N entries dropped` when `dropped` > 0, upserted the same way;
3. deletion of the run's rows beyond the newest 256 by `last_at`, then `diagnostic_id`.

| Flush outcome | Effect |
|---|---|
| Success | Clears the flushed entries and `dropped`, and sets `flush_next = now` |
| Transient (`transient_storage`) | Keeps the buffer and sets `flush_next = now + 1 s` |
| Permanent | Drops the flushed entries, adds their count to `dropped` and sets `flush_next = now + 1 s` |

A flush never sets the writer diagnostic, so diagnostics cannot open a gap. A cause that keeps failing permanently, such as lost ownership, therefore costs one failed transaction per second, not one per loop iteration.

**Shutdown.** After the event writer has finished, `service.rs` drains the whole `RunnerDiagnostics` buffer, `dropped` included, and writes it with the write sequence in the same transaction as the `cleanup_state`/`cleanup_diagnostics_json` UPDATE. Its `entries` are the drained entries followed by one `shutdown` entry per outcome diagnostic, so the last `event_writer` failure, a fence recorded after the last flush and everything buffered while storage was unwritable are persisted, and the sequence's single prune runs last. When that transaction fails, the printed lines are the only copy, and the existing `cleanup evidence pending` diagnostic reports it. Recovery and reconciler owners write no diagnostics.

### Stall behind an uncertain input

`storage::first_uncertain_ordinary_input(conn, run_id) -> Result<Option<i64>>` returns the lowest `input_id` among the run's `prompt`/`exec` operations in state `uncertain`. `InputCoordinator` keeps `blocked_wait: Option<BlockedWait { blocker_input_id, since, stalled }>`, maintained by `watch_blocked(conn, observation)`. `queue_housekeeping` calls `watch_blocked` first on every pass, before its `input_fenced || queue.is_empty()` early return, and returns at once when `input_fenced`.

`watch_blocked` reads in its own deferred read transaction under the `DEFERRED_FINALIZATION` budget, separate from the queue read that follows. That read returns the blocker and whether a waiter exists. A waiter exists when the queue is non-empty, or when `broker::oldest_pending_preview(tx, member_id)` returns a message: the same selection the sweep uses, independent of the sweep's throttling and backoff. A storage failure skips the pass and leaves `blocked_wait` unchanged. The pass outcome follows from the read:

| Blocker | Waiter | Effect |
|---|---|---|
| None | — | Clears `blocked_wait` |
| Some(B) | None | Clears `blocked_wait`, because the waiting input was delivered, consumed or settled elsewhere |
| Some(B) | Some | Keeps the wait for B, or starts it at `now` when it is absent or names another blocker |

A kept wait that has lasted `stall_bound` (3 minutes) and is not `stalled` publishes the record below once and sets `stalled`. While `blocked_wait` names a blocker, `watch_head` clears `head_since` and `stalled`, as it does for an interaction, so the head publishes no `review_stalled_delivery`. When the blocker settles, the head's ordinary watch starts afresh. `handle_preview` is unchanged.

**Record and payload.** Every internal stalled record has exactly 5 keys, `kind,input_id,message_id,blocked_by_input_id,waiting_since`, with exactly one of the three IDs a positive integer and the others null:

```json
{"kind": "input_delivery_stalled", "input_id": null, "message_id": null,
 "blocked_by_input_id": 412, "waiting_since": "2026-11-21T12:04:08Z"}
```

| Function (`src/broker/runtime/events.rs`) | Change |
|---|---|
| `stalled_record` (`input.rs`) | Emits `blocked_by_input_id: null` for head and preview waits; a new `blocked_record(blocker, since, now, clock)` emits the blocked form |
| `validate_stalled_record` | Requires the 5 keys and exactly one named ID |
| `original` | Rebuilds the 5-key record from the stored payload, so idempotent retries compare equal |
| `stalled_delivery` | For a blocked record, requires `blocked_by_input_id` to name a `prompt`/`exec` input of the run, with no state requirement, so a reconcile racing the publication cannot reject it. Its action is `reconcile_uncertain_input`; `published=true` re-validation on `get_event` applies the same rule |
| Dedup in `persist_transition_in_transaction` | Head query `…AND json_extract(payload_json,'$.input_id')=?3 AND json_type(payload_json,'$.blocked_by_input_id')='null'`; preview query adds the same `blocked_by_input_id` null condition; a third query matches `json_extract(payload_json,'$.blocked_by_input_id')=?3`. A hit commits a `duplicate_delivery` row (§ Duplicate delivery records) |
| `payload` | Stalled delivery appends `input_id,message_id,blocked_by_input_id,waiting_since` |
| `publish_delivery_result` (`observation.rs`) | Compares `blocked_by_input_id` as well (§ Duplicate delivery records) |

The blocked `director_action` is `{"kind":"reconcile_uncertain_input","instruction":INSTRUCTION,"inspect_argv":["cafleet","input","show","BLOCKER_ID","--json"]}`, with `--database-url URL` appended for a non-default database. INSTRUCTION is exactly:

```text
Inspect the uncertain input and the member's pane, then settle the input with input reconcile from that evidence. Queued input and message previews resume once it settles.
```

### Skills and documentation

| Target | Change |
|---|---|
| `SPEC.md` §6.3 `member show`, Runtime presentation | Live probe and the four member-show keys and text suffix. Replace "These are registry reads and make no live-health claim" with: the registry fields make no live-health claim, and member show's probe keys do |
| `SPEC.md` §6.3 `cafleet doctor` Check rows | `member.<id>.runtime` row, its situation table and the exit-1 consequence |
| `SPEC.md` §6.9 Persistence, notification and read interfaces | `event_rejected` cause; `head_not_ready` and `duplicate_delivery` live archival, so every materialized slot is filled; the stalled key order |
| `SPEC.md` §6.9 Input delivery stalled event | Blocked case, 5-key record, `blocked_by_input_id`, `reconcile_uncertain_input` |
| `SPEC.md` §6.9 Storage pressure and durability boundary | First paragraph: replace "each clipped to the same absolute database/work/shutdown budget" with the 5 s real-time step bound and the interrupting shutdown deadline. Second paragraph: delete only the clause "changes the runner to `degraded`", keep the input hold and gap counter, and rewrite the rest for classification, the in-step rejection that opens no gap, the fence and runner diagnostics |
| `SPEC.md` §6.9 Queue housekeeping, Runner pending-preview sweep | Blocked wait and head-watch suppression |
| `SPEC.md` §8 | Table list row, exact DDL and the two `archive_reason` values |
| `docs/docs/concepts/members.md` | Doctor paragraph (member rows, exit 1 on an `error` row). `member show` paragraph: replace "reads recorded runtime state without a socket probe" with the probe and diagnostics. Stalled-event paragraph: the blocked case |
| `assets/skills/cafleet/reference/cli.md` | Runtime presentation (member show keys), Runtime events (`event_rejected`, blocked stalled event), doctor row |
| `assets/skills/cafleet/roles/director.md` § Reconcile generated input | Steps for `reconcile_uncertain_input`. On `runner_degraded` with `event_rejected`: run `member show --json`, and for `rejected_kind` `interaction_opened` capture the pane and handle the visible request on the native surface |
| `design-docs/0000237-automatic-held-queue-flush/` | Deleted |

`README.md` and `.claude/rules/` are unaffected.

### Testing

Every test is an in-file unit or scenario test in the file that owns the behavior. Failure injection follows the class under test:

| Class | Injection |
|---|---|
| Transient | A writer hook returning `CafleetError::DatabaseBusy`, or a second connection holding `BEGIN IMMEDIATE` on a writer started with the real-clock `start` helper |
| Permanent with a successful replacement | `FaultPlan::Abort` restricted to the rejected kind, for example `on: "BEFORE INSERT ON member_runtime_events WHEN NEW.kind='coverage_unknown'"` |
| Fence | An unrestricted `FaultPlan::Abort` on `member_runtime_events` |

| File | New tests |
|---|---|
| `src/error.rs` | Table-driven `transient_storage` over each variant, `WithDiagnostics` and each `DatabaseIo` code mapping |
| `src/db.rs` | `with_shared_database_deadline` aborts a long statement and a busy wait after the shared deadline is lowered mid-scope; the existing baseline/shape tests pass with the new table |
| `src/runtime/writer.rs` | Replacement and continuation, asserting `diagnostic` stays `None` and `gap_count` stays 0; transient retention; a replacement that fails transiently stays at the front and persists after the backoff; fence; a sequence-slot failure fences; a virtual-clock jump of 10 s while the worker idles leaves `diagnostic` `None`; a paused `Hooks` boundary returns at the lowered shutdown deadline so `finish` joins; a persist that waits beyond 100 ms under a held lock without failing and `finish` joining an in-flight step at the shutdown deadline, both on the real-clock `start` helper; flush success, transient retention, permanent drop with `flush_next` one second later, overflow, dedup and prune |
| `src/runtime/observation.rs` | The adopted deadline test; controller-side evidence rejection submits the replacement with no gap; `publish_delivery_result` queues two blocked records naming different blockers; health field values and the terminal-fenced case |
| `src/runtime/control.rs` | `HealthSnapshot::decode` accepts and refuses `writer_diagnostic` and `gap_open_since` per § Observation health |
| `src/broker/runtime/events.rs` | `event_rejected` cause validation; `head_not_ready` archival; for each of the three dedup queries, a duplicate commits a `duplicate_delivery` row at its `event_seq`, its idempotent retry compares equal, and the next transition persists; blocked record validation and `original` round trip; all table-driven |
| `src/runtime/input.rs` | Blocked wait for a queued head and for a pending preview under `TestClock`, clearing without a waiter, restarting for the same blocker when a new waiter appears, and `review_stalled_delivery` suppression |
| `src/runtime/service.rs` | A scenario with a kind-restricted `Abort`, yielding `event_rejected` with no gap opened and the held preview delivered; a scenario where an interaction restarts a head's stall watch, the head stalls again, the second record commits as `duplicate_delivery` and later events still persist; shutdown diagnostics rows, including buffered entries and the overflow row |
| `src/cli/member.rs`, `src/cli/setup.rs` | `member show` keys and text for each probe outcome; doctor rows for each situation and the exit-1 rule |

These existing tests change:

| Test | Change |
|---|---|
| `writer.rs` `writer_worker_database_failure_retains_the_same_transition_for_retry` | `Hooks::fail_once` now returns `CafleetError::DatabaseBusy` in place of `error("fixture writer boundary failure")`, so the transition stays retained |
| `writer.rs` `writer_retry_exhaustion_reports_a_degraded_status_and_keeps_the_transition` | Its unrestricted `Abort` now fences, so it becomes the fence case: queue discarded, one unsaved, `event writer fenced` diagnostic |
| `writer.rs` tests built on `Hooks::on_virtual_time` | Use `Hooks::default()`; a paused boundary waits for its release within the step bound |
| `service.rs` `an_event_storage_fault_holds_input_until_the_writer_persists_again` | Replaces `Step::Arm(EVENT_FAULT)` / `Step::Clear(EVENT_FAULT)` with a new `Step::FailPersist(bool)`. The step sets an `AtomicBool` on `ScenarioWriterHooks`, which the scenario runner keeps as an `Arc`, and `before(Persist, …)` returns `CafleetError::DatabaseBusy` while it is set. `EVENT_FAULT`, used by no other test, is deleted |
| `service.rs` `fail-persist` / `block-persist` boundary test | Unchanged intent; the `ServiceFixture` `fail-persist` switch now returns `CafleetError::DatabaseBusy`, so it is transient |
| `observation.rs` `service_shutdown_uses_the_active_controller_to_close_before_truthful_exit` | Uses `synthetic` |
| Every other test whose writer failure injection now classifies as permanent | Re-inject with `DatabaseBusy`, so the test keeps its intent |

`recovery.rs` event faults exercise recovery persistence, not the event worker, and are unaffected.

### Live verification

The live run uses the branch binary, Claude Code on Herdr, and a dedicated database `D` (every command gets `--database-url sqlite:///D`), with one member `M`:

| Phase | Procedure | Expected |
|---|---|---|
| Healthy | Create `M` and let it go idle | `member show M --json`: `observation_probe` `answered`, `observation.eligibility` `composer_empty`, `degraded` false, `diagnostics` empty. Doctor: `ok member.M.runtime: run=R observation=composer_empty` |
| Permanent | 1. Ask `M` (`member prompt`) to ask one AskUserQuestion question, and wait for the form. 2. `sqlite3 D "CREATE TRIGGER live_reject BEFORE INSERT ON member_runtime_events WHEN NEW.kind='input_delivery_result' BEGIN SELECT RAISE(ABORT,'live injected rejection'); END;"`. 3. Send a second `member prompt M`; it queues with `defer_reason` `interaction_open`. 4. Answer the form on the native surface; the queued head delivers and its deferred result is rejected. 5. Drop the trigger | `interaction_opened` and `interaction_closed` arrive normally. The Director inbox receives `runner_degraded` with `cause.kind` `event_rejected` and `rejected_kind` `input_delivery_result`, and no `storage_pressure`/`storage_recovered` pair. `member show` reports `degraded` false at every check, and `diagnostics` holds an `event_writer` row whose message names the rejection. Doctor: `ok` |
| Transient | 1. Ask `M` (`member prompt`) to sleep 20 s and then ask one AskUserQuestion question, and wait until `member capture M` shows the sleep running. 2. Start `BEGIN IMMEDIATE;` in an interactive `sqlite3 D` session. 3. Wait for the form to appear on `M`'s Herdr pane. 4. Keep the hold for 120 s from step 2, then `ROLLBACK;` | About 5 s after the form appears, `member show` reports `observation.eligibility` `hold:gap`, `degraded` true, `writer_diagnostic` naming the busy or deadline cause, and `gap_open_since` set. Until 30 s after `gap_open_since`, doctor reports `warn member.M.runtime: …`; after that, `error member.M.runtime: … gap_open_since=…; inspect with …` and exit 1. After `ROLLBACK`: `interaction_opened` arrives, the row returns to `ok`, and `diagnostics` holds the `event_writer` and `observation` rows |

Every expectation on `observation.eligibility` compares the string rendering from § `member show`.

The Permanent phase queues its second prompt behind an open interaction, because a busy native turn with an empty composer does not defer a prompt, and a prompt delivered immediately publishes no `input_delivery_result`. The trigger matches only `input_delivery_result`, so the interaction events persist normally.

`BEGIN IMMEDIATE` blocks writers only, so `member show` and doctor stay readable during the hold. The trigger leaves the database compatible, because §8 accepts unrelated auxiliary triggers while the required definitions stay intact. Every guarded command keeps working with it present.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `SPEC.md` §6.3 `member show`, Runtime presentation and `cafleet doctor` Check rows per § `member show` and § `doctor` member rows, replacing the no-live-health sentence <!-- completed: 2026-11-21T14:01 -->
- [x] Update `SPEC.md` §6.9 Persistence, Input delivery stalled event, Queue housekeeping and Runner pending-preview sweep per § Rejection event, § Stalled head no longer ready, § Duplicate delivery records and § Stall behind an uncertain input <!-- completed: 2026-11-21T14:01 -->
- [x] Rewrite `SPEC.md` §6.9 Storage pressure and durability boundary per § Skills and documentation (step bound, classification, fence, diagnostics; delete only the `degraded` clause) <!-- completed: 2026-11-21T14:01 -->
- [x] Update `SPEC.md` §8 with the table list row, the `member_runner_diagnostics` DDL before the ledger table and the `head_not_ready` and `duplicate_delivery` archive reasons <!-- completed: 2026-11-21T14:01 -->
- [x] Update `docs/docs/concepts/members.md` doctor, `member show` and stalled-event paragraphs <!-- completed: 2026-11-21T14:01 -->
- [x] Update `assets/skills/cafleet/reference/cli.md` Runtime presentation, Runtime events and doctor facts <!-- completed: 2026-11-21T14:01 -->
- [x] Update `assets/skills/cafleet/roles/director.md` § Reconcile generated input with the `reconcile_uncertain_input` and `event_rejected` procedures <!-- completed: 2026-11-21T14:01 -->
- [x] Delete `design-docs/0000237-automatic-held-queue-flush/` <!-- completed: 2026-11-21T14:01 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-21T14:01 -->

### Step 2: Deadline fix

- [x] Apply the `update()` change and add the controller test per § Deadline fix <!-- completed: 2026-11-21T14:06 -->
- [x] Change the `uncertain preview` scenario's expected events <!-- completed: 2026-11-21T14:06 -->

### Step 3: Writer step bound

- [x] Add `SharedDeadline` and `with_shared_database_deadline` to `src/db.rs`, with `database_cancelled` and `remaining_database_wait` reading the shared value, plus their test <!-- completed: 2026-11-21T14:20 -->
- [x] Delete `WORK_SLICE`, `Worker::work_slice`, `Worker::work_deadline`, `Mailbox::work_deadline` and `WriterHooks::work_slice`, and move the mailbox deadline into a `SharedDeadline` <!-- completed: 2026-11-21T14:20 -->
- [x] Add `StepBound` (real-time `until`, shared shutdown deadline on the worker's clock, `check()`), run each receive and persist under it with `with_shared_database_deadline`, and make `finish` wait until `until + WRITER_JOIN_GRACE` before an unconditional join <!-- completed: 2026-11-21T14:20 -->
- [x] Change `WriterHooks::before` to take `&StepBound`, and end the `writer.rs` `Hooks` pause and the `service.rs` `block-persist` loop when `bound.check()` fails <!-- completed: 2026-11-21T14:20 -->
- [x] Remove `Hooks::on_virtual_time` and the `Hooks.work_slice` field in `writer.rs` and `InputFixtureWriterHooks::work_slice` in `input.rs`, pointing callers at `Hooks::default()` <!-- completed: 2026-11-21T14:20 -->
- [x] In `service.rs`, remove `synthetic_with_unhurried_writer`, `writer_work_slice_ms`, the `Hooks.work_slice` field and impl, the config parse and `ScenarioWriterHooks::work_slice`, and point callers at `synthetic` <!-- completed: 2026-11-21T14:20 -->
- [x] Add the virtual-jump, paused-hook join, held-lock persist and shutdown-interrupt tests, the last two on the real-clock `start` helper <!-- completed: 2026-11-21T14:20 -->

### Step 4: Failure classification and rejection

- [x] Add `CafleetError::DatabaseIo`, its code mapping and `transient_storage`, with tests <!-- completed: 2026-11-21T14:35 -->
- [x] Add `diagnostic_text` with its tests <!-- completed: 2026-11-21T14:35 -->
- [x] Add `RuntimeCauseKind::EventRejected` and its cause validation in `src/broker/runtime/events.rs`, with tests <!-- completed: 2026-11-21T14:35 -->
- [x] Classify failures in the worker, and replace a permanently failing front transition with its rejection event within the same step, without `Mailbox::failure` <!-- completed: 2026-11-21T14:35 -->
- [x] Fence the writer when the replacement fails permanently <!-- completed: 2026-11-21T14:35 -->
- [x] Add `RetainReason` to `RetainedTransition`, skip `Mailbox::failure` for `EvidenceInvalid`, and replace an `EvidenceInvalid` refusal in `ObservationController::route` with no `delivery_pressure` call <!-- completed: 2026-11-21T14:35 -->
- [x] Add the replacement, transient-replacement, fence and sequence-slot writer tests, plus the controller rejection test <!-- completed: 2026-11-21T14:35 -->
- [x] Add `Step::FailPersist(bool)` and the `ScenarioWriterHooks` flag, move `an_event_storage_fault_holds_input_until_the_writer_persists_again` onto it and delete `EVENT_FAULT` <!-- completed: 2026-11-21T14:35 -->
- [x] Update the existing tests listed in § Testing <!-- completed: 2026-11-21T14:35 -->

### Step 5: Silent archival of stale and duplicate delivery records

- [x] Add `head_not_ready` and `duplicate_delivery` to the `archive_reason` CHECK in `assets/migrations/V1__baseline.sql` and to `ArchiveReason` <!-- completed: 2026-11-21T14:41 -->
- [x] Add `DeliveryFields.archive`, and set it to `Some(HeadNotReady)` in `stalled_delivery` with `published=false` per § Stalled head no longer ready <!-- completed: 2026-11-21T14:41 -->
- [x] Make `persist_transition_in_transaction` commit a delivery whose `archive` is set as a silent archived row at `transition.event_seq` (`notification_state='archived'`, NULL `message_id`, no inbox message) <!-- completed: 2026-11-21T14:41 -->
- [x] Make the delivery dedup branch build the delivery fields and commit a `duplicate_delivery` archived row returned as inserted, with `duplicate_delivery` taking precedence over `head_not_ready` and both applying only when `historical_ownership` supplies no archive reason, per § Duplicate delivery records <!-- completed: 2026-11-21T14:41 -->
- [x] Add the `events.rs` tests for `head_not_ready` archival and, for the head and preview dedup queries, a duplicate committing a `duplicate_delivery` row at its `event_seq`, its idempotent retry comparing equal and the next transition persisting <!-- completed: 2026-11-21T14:41 -->
- [x] Add the `service.rs` scenario where an interaction restarts a head's stall watch, the head stalls again, the second record commits as `duplicate_delivery` and later events still persist <!-- completed: 2026-11-21T14:56 -->

### Step 6: Runner diagnostics

- [x] Add `member_runner_diagnostics` and its index to `assets/migrations/V1__baseline.sql` before `refinery_schema_history` <!-- completed: 2026-11-21T14:56 -->
- [x] Add the storage upsert/prune and latest-5 read functions under `src/broker/runtime/` <!-- completed: 2026-11-21T14:56 -->
- [x] Add the shared `RunnerDiagnostics` buffer with 32-entry overflow accounting <!-- completed: 2026-11-21T14:56 -->
- [x] Record every source site in § Runner diagnostics through it, keeping the `eprintln!` copy <!-- completed: 2026-11-21T14:56 -->
- [x] Flush from the event worker per § Runner diagnostics, including its bound, `flush_next` and outcome rules <!-- completed: 2026-11-21T14:56 -->
- [x] Write the `shutdown` rows in the cleanup UPDATE transaction <!-- completed: 2026-11-21T14:56 -->
- [x] Add the buffer, flush, dedup, prune and shutdown tests <!-- completed: 2026-11-21T14:56 -->

### Step 7: Observation health and surfaces

- [x] Add `writer_diagnostic` and `gap_open_since` to `ObservationHealth`, apply the decode rules and raise the encoded limit to 2,048 bytes, with tests <!-- completed: 2026-11-21T20:50 -->
- [x] Add the member-show probe, its four `runtime` keys and the text suffix <!-- completed: 2026-11-21T20:50 -->
- [x] Add the doctor `member.<id>.runtime` rows with concurrent 1 s probes and `DOCTOR_GAP_ERROR_AGE` <!-- completed: 2026-11-21T20:50 -->
- [x] Add the `member show` and doctor tests for each situation <!-- completed: 2026-11-21T20:50 -->

### Step 8: Stall behind an uncertain input

- [x] Add `storage::first_uncertain_ordinary_input` <!-- completed: 2026-11-21T20:57 -->
- [x] Add `BlockedWait` and `watch_blocked`, called first in `queue_housekeeping` <!-- completed: 2026-11-21T20:57 -->
- [x] Suppress `watch_head` while a blocker exists <!-- completed: 2026-11-21T20:57 -->
- [x] Move every stalled record to the 5-key form and add `blocked_record` <!-- completed: 2026-11-21T20:57 -->
- [x] Update `validate_stalled_record`, `original`, `stalled_delivery`, the dedup queries and `payload` per § Stall behind an uncertain input, routing a blocked dedup hit through the Step 5 `duplicate_delivery` archival <!-- completed: 2026-11-21T20:57 -->
- [x] Make `publish_delivery_result` compare `blocked_by_input_id` per § Duplicate delivery records <!-- completed: 2026-11-21T20:57 -->
- [x] Add the blocked-wait and suppression tests, the blocked record validation and `original` round-trip tests, the blocked dedup query's `duplicate_delivery` test and the `publish_delivery_result` test for two blocked records naming different blockers <!-- completed: 2026-11-21T20:57 -->

### Step 9: Validation

- [ ] Run `mise //:test` <!-- completed: -->
- [ ] Run `mise //:lint` <!-- completed: -->
- [ ] Run `mise //docs:build` <!-- completed: -->
- [ ] Run the § Live verification phases and record each observed value <!-- completed: -->
- [ ] Search outside `design-docs/` for `WORK_SLICE`, `work_slice`, `unhurried_writer`, `writer_work_slice_ms` and `automatic-held-queue-flush`, and confirm nothing is found <!-- completed: -->
- [ ] Recheck every § Skills and documentation target against the shipped behavior <!-- completed: -->
