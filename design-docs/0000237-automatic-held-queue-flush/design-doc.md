# Runner automatic flush of a stalled queue and runner diagnostic trace

**Status**: Approved
**Progress**: 0/41 tasks complete
**Last Updated**: 2026-11-21

## Overview

When a queued preview head stalls, the member runner performs the forced flush itself, so the Director-only `cafleet member flush` and its user confirmation are removed. A refused flush reaches the Director as `input_delivery_stalled` and is retried automatically once the blocker is resolved. A bounded runner diagnostic trace in the database, read through `cafleet member trace`, records receipt history, queue and preview-sweep gates, observer state and runner failures, so held-delivery defects like #623 can be diagnosed from the database alone.

## Success Criteria

- [ ] A queue head that has started no attempt for `COMPOSER_BUSY_BOUND` (3 minutes), excluding time an open interaction blocks it, triggers an automatic flush from runner housekeeping with no Director command. When no refusal holds, every selected preview settles `submitted` with method `forced`, and one `input_delivery_result` is published per preview plus one `input_queue_flushed`.
- [ ] The automatic flush writes ` [cafleet: ignore any text before this marker, if present] `, the preview lines in FIFO order, waits 500 ms without blocking the event loop, then writes the profile submit bytes. All bytes go straight into the child PTY under the runner's own pane input lock, with no input-drain (`quiesce`) gate and no Escape or clear key. Checks 5 and 6 are re-run before every write after the first, and a failure writes no further byte.
- [ ] Every § Refusals condition writes zero bytes and publishes `input_delivery_stalled` carrying `flush_refusal`, at most once per head for each refusal kind and refused input. The § Director response table gives every published kind a prescribed Director action. After the Director resolves the blocker, the next housekeeping sweep flushes without another command.
- [ ] A successful flush whose stall cause is `queue_turn_not_reached` publishes `input_queue_flushed` with a `report_runner_defect` action. An uncertain flush publishes `input_queue_flushed` with outcome `uncertain` and a `review_uncertain_flush` action. Every `input_queue_flushed` payload stays under the event payload limit for any queue length.
- [ ] `cafleet member flush`, the `flush` control operation, `src/runtime/flush.rs`, `Multiplexer::send_text`, `flush_argv` and the `forced_flush` stall cause are gone. Outside `design-docs/`, a repository search for `member flush`, `flush_argv`, `forced_flush`, `send_text`, `send-text`, `FlushAction`, `FlushReply` and `FlushWriteOutcome` finds nothing.
- [ ] `member_runtime_trace` records each `member_input_operations` insert and each change of a traced column in the same transaction as the receipt write, for every writer, and never makes a receipt write fail for payload size. It also records queue-head gate changes, automatic-flush steps, preview-sweep decisions, held submissions that create no receipt, observer state changes, runner failures and trace gaps. It holds no literal text, keeps at most 2,048 rows per run, keeps traces only for each member's 8 most recent runs, and bounds every payload to 2 KiB by construction.
- [ ] From the trace alone, a re-run of this session's held reproduction (§ Background) shows, in time order:
  - one `delivery_held` row per held reply, from the control socket and from the sweep, with its reason, `gate_reason`, `diagnostic` and `gap_open`;
  - a `preview_sweep` row on each change of sweep decision or backoff delay;
  - the `observer_state` changes around them.
- [ ] Step 7 runs the § Post-turn suggestion rule recipe within its attempt budget and records the outcome per its outcome table. When a suggestion-styled post-turn rendering is captured, its bundled fixture (with `cell_styles`) classifies as an empty composer under the Claude suggestion rule. When normal-styled text is captured, its fixture classifies non-empty.
- [ ] `cafleet member trace MEMBER_ID [--run-id RUN_ID] [--input-id INPUT_ID] [--kind KIND] [--limit N] [--json]` returns the selected rows oldest first with the run's pruned count. From the trace alone, a re-run of the #623 sequence shows the head's receipt history, the gate that held it and the automatic-flush outcome.
- [ ] `SPEC.md`, `docs/`, `assets/skills/cafleet/reference/cli.md` and `assets/skills/cafleet/roles/director.md` describe the new behavior. `mise //:test`, `mise //:lint` and `mise //docs:build` pass.
- [ ] A live run with the branch binary on a dedicated database (Claude Code on Herdr) shows a stalled queue flushed automatically, `input_queue_flushed` delivered to the Director, and the run's history in `member trace`.

---

## Background

Design doc 0000236 (issue #623, PR #627) added queue housekeeping, the stall watchdog with its `cause`, suggestion-only composer detection and the Director-only `cafleet member flush`. The flush rebuilds the queued previews and types them through the outer pane behind the ignore marker, after a user confirmation through the Director.

**User decision:** users never draft in member panes. A member composer can therefore hold only leftovers (partial earlier writes, left-in-composer previews) and native suggestions or hints, and the member-role marker rule already neutralizes all of them. The confirmation step protects nothing, so the runner flushes on its own at the stall trigger.

The #623 investigation also showed what the database cannot answer today:

| Gap | Today |
|---|---|
| Receipt history | `member_input_operations` keeps only each receipt's latest state; `updated_at` overwrites it |
| Which gate held the head | Not recorded: input fenced, input-drain result, finalization or stamp retry, `InputPhase::Blocked`, flush refusal |
| Runner failures | Written with `eprintln!` to the member pane, where the TUI overdraws them; never persisted |

`member_runtime_events` persists published events only, so it cannot fill these gaps.

### Observed held reproduction (this session)

Installed cafleet 0.35.4, Claude Code on Herdr:

1. After about 2.5 hours idle, two sends at 02:51:36 and 02:51:39 UTC to members 24 and 25 each returned `Delivery status=held reason=observer_unavailable`. Neither created an input receipt, and `unresolved_input_ids` was empty.
2. `cafleet doctor` reported both runtimes healthy: owner lock held, health probe answered.
3. Neither message was delivered for over 2 minutes, although the preview-sweep backoff caps at 30 s.
4. The Director recovered both members with `member press` typing `poll inbox` and Enter.

It reproduced again at 03:01:42 UTC, when a send to member 24 was again held with `observer_unavailable`. A capture at that time showed member 24's idle composer containing `poll inbox`, the last prompt the Director had typed. The Reviewer's composer had earlier shown `ready (doc)` the same way.

At 03:09:46 UTC, message 1606 to the Reviewer was held with `observer_unavailable`, while a capture showed the Reviewer's composer empty, with no visible suggestion. This is a counterexample to a suggestion-only cause.

**Candidate causes.**

| Candidate | Status | Basis |
|---|---|---|
| An observation gap after an idle turn | Leading | The receipt-less `held` reply with reason `observer_unavailable` comes only from `ordinary_blocker`, and only for eligibility `Hold(ObserverUnavailable)` or `Hold(Gap)` or an observer diagnostic. Observation gaps open on sequence, sink and detector failures |
| A Claude Code 2.1.322 post-turn native suggestion the bundled rule does not match | Weakened | The 03:09:46 hold occurred with an empty composer. An unmatched composer classifies `Hold(Unknown)`, which holds delivery but does not produce the `observer_unavailable` reason |

The suggestion candidate stays open as a possible contributor. 0000236 recorded that the post-turn suggestion was not reproduced in 4 runs, so the bundled Claude suggestion rule (`claude-suggestion-composer`) is proven only by the startup-hint fixture `claude-2.1.322-suggestion-herdr-094-1`. The `poll inbox` and `ready (doc)` composer texts are the only evidence for it.

No receipt exists, so the queue watchdog and the automatic flush never see these messages. Nothing in the database records why the runner answered `held`, whether the preview sweep ran, what backoff it applied, or which observer state held it. This design therefore makes two changes:

- **Trace rows:** the trace records preview-sweep decisions, held submissions that create no receipt, and observer state changes (§ Runner diagnostic trace). Each `delivery_held` row carries `gate_reason`, `diagnostic` and `gap_open`, which tells an observation gap from an unmatched composer.
- **Post-turn suggestion capture:** a capture attempt runs, and its fixture and rule change are conditional on what it shows (§ Post-turn suggestion rule).

Fixing the gap cause the trace confirms is outside this design.

---

## Specification

### Automatic flush trigger

The stall watchdog (`InputCoordinator::watch_head`, run by `queue_housekeeping` every `QUEUE_HOUSEKEEPING_INTERVAL`, 1 s) keeps its trigger:

- `now - head_since >= COMPOSER_BUSY_BOUND` (3 minutes);
- no attempt has started for the head;
- an interaction that blocks the head resets `head_since` and clears `due`, exactly as today.

When the trigger holds, the head becomes **flush-due**. On that sweep and on every later sweep while it stays flush-due, housekeeping evaluates § Refusals and either reserves the flush (§ Write) or records the refusal (§ Events). The per-head `stalled: bool` is replaced by:

```rust
struct HeadFlushState {
    due: bool,
    published: BTreeSet<(FlushRefusalKind, Option<i64>)>,
    traced_refusal: Option<(FlushRefusalKind, Option<i64>)>,
}
```

`published` and `traced_refusal` live as long as the head and survive an interaction reset. Each pair is a refusal kind and its `flush_refusal.input_id`.

While a flush slot is set (`self.flush.is_some()`), housekeeping runs neither the watchdog nor § Refusals for any head, so the remainder head behind a slot held by a failing settlement never becomes flush-due. The `head_gate` row `flush_writing` records that state.

The stall cause (§ Stall causes) is derived when the head becomes flush-due and at each publication. Every flush, refused or reserved, carries the cause that held the head.

`queue_housekeeping` takes `(&mut self, conn, observation: &ObservationController, transport: &Transport, now)`. Checks 6–8 read the transport's terminal-end state and snapshot.

### Refusals

`reserve_automatic_flush` evaluates these checks in order. The first failing check writes zero bytes and creates no receipt.

| # | Condition | `flush_refusal.kind` | `flush_refusal.input_id` | `flush_refusal.message` |
|---|---|---|---|---|
| 1 | Manual input accounting failed | `input_fenced` | `null` | `the member's input is fenced` |
| 2 | A runner attempt holds a written or in-flight composer write (`self.pending`) | `attempt_in_flight` | `null` | `a runner input attempt is in flight` |
| 3 | An ordinary receipt of the run is `in_progress`, `accepted` or `uncertain`, except the `pending_stamp` preview and the `finalization` input | `unresolved_input` | The lowest listed input | `input INPUT_IDS is unresolved; reconcile it with cafleet input reconcile` |
| 4 | The FIFO holds a queued `ready` prompt or exec | `queued_explicit_input` | The lowest queued input | `queued KIND input INPUT_ID cannot be rebuilt from its receipt; settle it with 'cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition not-submitted --evidence-ref REFERENCE', then resend it` |
| 5 | An interaction or candidate is shown (visibility `Visible`/`Obscured`, or eligibility `Hold(Interaction)`/`Hold(Candidate)`) | `interaction_open` | `null` | `an interaction is open (CLASSIFICATION)` |
| 6 | Visibility `Unknown`, degraded observation, or either terminal end closed | `observation_unavailable` | `null` | `the member's screen observation is unavailable: REASON` |
| 7 | `composer_draft` is `Unreadable` on the current snapshot | `draft_unreadable` | `null` | `the composer draft cannot be read from the screen` |
| 8 | The draft's first scalar is a mode-switch prefix | `mode_prefix` | `null` | `the composer draft starts with mode prefix 'PREFIX'` |
| 9 | No prompt composer plan for the recorded native version | `no_submission_plan` | `null` | `the BACKEND profile has no submission plan for native version VERSION` |
| 10 | Not even the first pointer line fits the plain limit (§ Literal) | `plain_limit` | The head | `the first queued preview is N UNIT as a pointer line, above the flush limit of MAX UNIT` |
| 11 | `PaneLock::try_acquire` finds the pane input lock held | `pane_input_busy` | `null` | `the member's pane input is busy` |
| 12 | A selected receipt changed inside the reservation transaction | `queue_changed` | `null` | `queued input changed while the flush was prepared` |

- Fenced input (`input_fenced` set by shutdown or native exit) never reaches the checks, because housekeeping returns before the watchdog. Check 1 covers manual accounting failure, which leaves housekeeping running.
- With no live preview left (every queued message consumed), nothing is reserved and no refusal is recorded; housekeeping cancels the consumed entries.
- Receipt refusals list matching input IDs ascending and comma-separated. With several queued prompt/exec inputs, the message names the lowest and appends `; also queued: IDS`.
- `DIRECTOR_ID` is the run's `director_member_id`. Each `message` is at most 512 bytes.
- **Storage failure:** a `DatabaseBusy` or `DeadlineExceeded` from the check-3 read, the live-message read or the reservation transaction ends the attempt with zero bytes and no receipt. The head keeps `due`, records the head obstacle `storage_unavailable`, and is re-evaluated on the next sweep. No event is published. The `runner_failure` row with site `auto_flush_reservation` carries the message; like every runner row, it is dropped when storage stays down (§ Runner-written rows).
- § Gate tolerance in 0000236 stays in force for checks 5 and 6:
  - Drain blockers `terminal input interrupted`, `terminal reply pending`, `terminal output still arriving`, `observation incomplete`, `observation transitions pending` and `observation evidence pending` proceed.
  - `observation degraded: CODE`, `outer terminal closed` and `native output closed` refuse.
  - The `observation incomplete` override stays the user-accepted last resort.
  - A held composer, a pending finalization and a pending stamp retry proceed; check 3 exempts the last two by input ID, as `reserve_flush` does today.

### Literal

The literal is unchanged from 0000236. It is ` [cafleet: ignore any text before this marker, if present] ` followed by the live previews' `canonical_preview_line`s, joined by one space, in the runner's in-memory FIFO order. Selection takes at most `FLUSH_PREVIEW_LIMIT` (128) previews from the front of the FIFO. The effective byte bound is the lower of the plan's plain literal limit, in its own unit, and `FLUSH_LITERAL_LIMIT` (65,536 bytes). The fallback order is: canonical lines, then pointer lines, then the longest fitting FIFO prefix of pointer lines, then refusal (check 10).

Previews outside the selected prefix stay queued in order, and the first of them becomes the new head with a fresh `head_since`. Normal delivery therefore gets a full watchdog interval before another automatic flush (user decision).

### Write

**User decision:** the runner writes straight into the child PTY, under its own pane input lock, with no `quiesce` gate.

#### Reservation

After the checks pass, one owned `IMMEDIATE` transaction:

- inserts the flush receipt `in_progress`, with `operation_key` `flush:<run_id>:<ordinal>` and `payload_digest` the SHA-256 of the literal followed by the submit bytes;
- moves each selected preview `ready → in_progress` with `delivery_method=forced` and `flush_input_id` set;
- revalidates every selected message as `input_required`.

The selected entries move from the FIFO into the flush slot in order:

```rust
pub(crate) struct PendingFlush {
    pub(crate) flush_input_id: i64,
    pub(crate) entries: Vec<QueuedInput>,
    head_input_id: i64,
    cause: StallCause,
    waiting_since: chrono::DateTime<chrono::Utc>,
    pane: PaneLock,
    bytes: FlushBytes,
    deadline: Instant,
}
struct FlushBytes {
    literal: Vec<u8>,
    submit: Vec<u8>,
    phase: FlushPhase,
}
enum FlushPhase {
    Literal { written: usize },
    Settle { until: Instant },
    Submit { written: usize },
}
const FLUSH_WRITE_BUDGET: Duration = Duration::from_secs(10);
const FLUSH_PREVIEW_LIMIT: usize = 128;
```

`deadline` is the reservation time plus `FLUSH_WRITE_BUDGET`. While the slot is set, `start_deferred` starts no attempt, the preview sweep treats the run as owning preview delivery, and the receipt `Reconcile` action refuses the flush receipt with `flush FLUSH_INPUT_ID is still outstanding` (unchanged).

#### Driving the write

`InputCoordinator::drive_flush(&mut self, conn, transport: &mut Transport, pty: &mut impl PtyIo, observation, now)` runs from `tick_running` on every tick, right after `housekeep_queue` and before the pending-client branch. Like housekeeping, it is independent of `retry_deferred`, the drain and the finalization and stamp retries.

Every call that is about to write a byte first re-runs checks 5 and 6 on the current observation and transport: before the first literal chunk, before each later literal chunk, and before the submit bytes. Each call advances at most one phase:

| Phase | Action |
|---|---|
| `Literal` | Re-run checks 5 and 6, then write the next chunk with `Transport::write_forced`; on completion enter `Settle { until: now + STAGE_SETTLE }` (500 ms) |
| `Settle` | Wait until `until`, then enter `Submit` |
| `Submit` | Re-run checks 5 and 6, then write the remaining submit bytes with `Transport::write_forced`; on completion settle `Acknowledged` |

A flush reserved on one tick can wait for its first byte on a later one, and a partial `try_write` spreads the literal across ticks, so the screen is checked again before every write.

`Transport::write_forced(&mut self, pty: &mut impl PtyIo, bytes: &[u8]) -> Result<usize>`:

- returns `Ok(0)` and writes nothing while the human forwarding queue (`self.input`) is non-empty or a terminal-reply prefix is pending (`self.replies.pending()`), so forced bytes never split or overtake human bytes or a terminal reply;
- otherwise calls `pty.try_write(bytes)` once, mapping `WouldBlock`/`Interrupted` to `Ok(0)` and any other error to `Err`;
- invalidates the retained input-drain proof (`invalidate_input_drain`), so no programmatic attempt can reuse a proof taken before the forced bytes;
- leaves `human_generation` and `pointer_generation` unchanged, because the bytes are runner-authored.

#### Settlement

"Bytes written" counts the bytes `write_forced` reported for this flush, across all calls.

| Outcome | When | Flush receipt | Each flushed preview | Events |
|---|---|---|---|---|
| `Acknowledged` | Every literal and submit byte written | `settled/submitted`, `evidence_ref="runner forced flush after stall of input HEAD_INPUT_ID"` | `settled/submitted` with its notification stamp, `evidence_ref="forced flush input FLUSH_INPUT_ID"` | One `input_delivery_result` per preview (stage `submitted`), then `input_queue_flushed` outcome `submitted` |
| `NotStarted` | Zero bytes written when the deadline passes, when checks 5/6 fail, or when `write_forced` returns `Err` | `settled/not_submitted`, `evidence_ref="runner forced flush wrote no byte"` | Back to `ready`, `delivery_method` and `flush_input_id` cleared, returned to the FIFO front in order with the head still flush-due | None; the next sweep re-evaluates § Refusals |
| `Uncertain(write_failed)` | At least one byte written, then `write_forced` returns `Err` or the deadline passes | `uncertain`, `stop_cause=write_failed` | `uncertain`, `stop_cause=write_failed` | One `input_delivery_result` per preview (stage `unknown`), then `input_queue_flushed` outcome `uncertain` |
| `Uncertain(interaction_open)` | At least one byte written, then check 5 fails before a later write | `uncertain`, `stop_cause=interaction_open` | `uncertain`, `stop_cause=interaction_open` | As above |
| `Uncertain(observer_unavailable)` | At least one byte written, then check 6 fails before a later write | `uncertain`, `stop_cause=observer_unavailable` | `uncertain`, `stop_cause=observer_unavailable` | As above |

An `Uncertain` settlement writes no further byte, so a dialog that opens mid-literal receives none of the remaining literal or submit bytes.

The pane lock is released when the slot clears. If storage prevents a settlement transaction, the slot is kept (as `apply_flush_settlement` does today) and the next tick retries it with a `DEFERRED_FINALIZATION` storage deadline. No further byte is written in the meantime, and § Automatic flush trigger keeps the watchdog off while the slot is set. Orderly shutdown settles an outstanding slot `uncertain/runner_stopped`. Recovery under reconciler ownership records `uncertain/runner_lost` with the same cascade. Both are unchanged.

After submission, a fresh verified empty composer clears a held composer, as today. The flush performs no post-submit verification.

### Stall causes

The `forced_flush` row is removed: a flush slot now lives only for the runner's own write, and the watchdog never runs while a slot is set (§ Automatic flush trigger). Every other row of the 0000236 cause table, and its live-then-recorded order, is unchanged. `stalled_delivery` validation drops `forced_flush` from the blocker-required kinds, and an event carrying it fails validation as an unknown kind.

### Events

#### `input_delivery_stalled`

The event is published only for a refused automatic flush. Its ordered fields gain `flush_refusal` after `cause`:

```json
{"input_id": 4805, "message_id": 6723, "blocking_input_id": 4804,
 "waiting_since": "2026-11-19T12:04:08Z",
 "delivery": {"status": "queued", "method": null, "reason": "earlier_input"},
 "cause": {"kind": "unresolved_input", "detail": null},
 "flush_refusal": {"kind": "unresolved_input", "input_id": 4804,
                   "message": "input 4804 is unresolved; reconcile it with cafleet input reconcile"}}
```

- **Publication and deduplication:** one rule governs both. A head publishes at most one event per `(flush_refusal.kind, flush_refusal.input_id)` pair for its lifetime. A refusal whose pair is already in `published` is not republished, including after an interaction reset or a kind sequence such as A→B→A, because the Director already holds that event. A refusal naming a new blocking input (for example a second unresolved input after the first is reconciled) is a new pair and is published. Persistence deduplicates on `(run_id, kind, input_id, flush_refusal.kind, flush_refusal.input_id)`, replacing `(run_id, kind, input_id)`, so recovery replay stays idempotent. `queue_changed` is a runner-internal race and is retried silently, never published.
- **Outbox record:** the stalled `cause_json` record has exactly six keys: `kind` (`input_delivery_stalled`), `input_id`, `blocking_input_id`, `waiting_since`, `cause` and `flush_refusal`. `validate_stalled_record` requires the six keys, and `original()` rebuilds `flush_refusal` from the payload beside `cause`.
- **Validation:** `flush_refusal` is an object with exactly `kind`, `input_id` and `message`. `kind` is in the § Refusals vocabulary except `queue_changed`. `input_id` is a run receipt for the kinds whose § Refusals row names one, and `null` otherwise. `message` is non-blank and at most 512 bytes.
- **`held_composer`:** keeps its `review_stalled_input` action and INSTRUCTION unchanged. Its blocker is `uncertain` or settled `left_in_composer`; an `uncertain` blocker refuses the flush as `unresolved_input`.
- **Every other cause:** `{"kind":"review_stalled_queue","instruction":INSTRUCTION,"inspect_argv":["cafleet","input","show","INPUT_ID","--json"]}`, with `--database-url URL` appended for a non-default database. INSTRUCTION is exactly:

```text
Capture the member's pane, inspect the waiting input and confirm with member show that no interaction is open. Then act on flush_refusal.kind: reconcile an unresolved input from pane evidence; cancel a queued prompt or exec with input reconcile --disposition not-submitted, then resend it; clear an unreadable or mode-prefixed composer draft with member press; answer an open interaction; report input_fenced, no_submission_plan, plain_limit or a lasting observation_unavailable to the user with member trace output; for attempt_in_flight and pane_input_busy take no action. The runner retries the flush automatically. A queue_turn_not_reached cause is a runner defect to report.
```

#### Director response

`director.md` and `cli.md` carry this table. The runner retries every refusal on each housekeeping sweep.

| `flush_refusal.kind` | Clears when | Director action |
|---|---|---|
| `input_fenced` | Never within the run | Capture the pane and report to the user with `member trace` output; replacing the member is the user's decision |
| `attempt_in_flight` | The runner's attempt settles | None |
| `unresolved_input` | The Director reconciles the named inputs | Capture the pane, then `input reconcile` each named input from that evidence |
| `queued_explicit_input` | The Director cancels the queued input | `input reconcile INPUT_ID --disposition not-submitted`, then resend the prompt or exec |
| `interaction_open` | The interaction closes | Handle the interaction through its own `interaction_opened` event |
| `observation_unavailable` | The observer recovers | Capture the pane; when the refusal persists past one watchdog interval, report to the user with `member trace` output |
| `draft_unreadable`, `mode_prefix` | The composer is cleared | Capture the pane, then clear the composer with `member press` (the profile's clear keys) |
| `no_submission_plan`, `plain_limit` | Never within the run | Report a profile defect to the user with the event and `member trace` output |
| `pane_input_busy` | The overlapping `member press`, `prompt` or `exec` finishes | None |

#### `input_queue_flushed` (new kind)

Published after an `Acknowledged` or `Uncertain` settlement, after its per-preview `input_delivery_result` events. A `NotStarted` settlement publishes nothing.

| Field | Value |
|---|---|
| `kind` | `input_queue_flushed` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_lifecycle` |
| `decision_outcome` | `"unknown"` |
| `evidence` | Ordered run evidence with null screen-derived fields |
| `input_id` | The stalled head, which is `flushed_input_ids[0]` |
| `flush_input_id` | The flush receipt |
| `flushed_input_ids` | The flushed previews in FIFO order, at most `FLUSH_PREVIEW_LIMIT` |
| `message_ids` | Their messages, in the same order |
| `remaining_count` | Live previews left queued behind the selected prefix |
| `next_input_id` | The new head after the flush, or `null` when `remaining_count` is 0 |
| `waiting_since` | UTC RFC 3339 `head_since` of the stalled head |
| `cause` | The stall cause object `{"kind":KIND,"detail":DETAIL}` |
| `outcome` | `submitted` or `uncertain` |
| `director_action` | Below |

`remaining_count` and `next_input_id` replace a full remainder list, because the FIFO behind the prefix is unbounded. The two ID lists hold at most 128 entries each, about 5 KiB at the 19-digit maximum, so the payload stays well under `PAYLOAD_LIMIT` (16 KiB).

| Outcome and cause | `director_action` |
|---|---|
| `uncertain`, any cause | `{"kind":"review_uncertain_flush","instruction":INSTRUCTION_U,"inspect_argv":["cafleet","input","show","FLUSH_INPUT_ID","--json"]}` |
| `submitted`, cause `queue_turn_not_reached` | `{"kind":"report_runner_defect","instruction":INSTRUCTION_D}` |
| `submitted`, any other cause | `null` |

INSTRUCTION_U is exactly `Capture the member's pane and inspect the flush input. Reconcile it from that evidence with input reconcile; the reconciliation settles every preview it owns.`

INSTRUCTION_D is exactly `The runner flushed a queue that no known gate held (cause queue_turn_not_reached). Record this event and report it as a runner defect for correction.`

**Outbox record.** The runner hands the writer a `cause_json` record with exactly these keys:

- `kind` (`input_queue_flushed`);
- `input_id`, `flush_input_id`, `flushed_input_ids`, `message_ids`;
- `remaining_count`, `next_input_id`;
- `waiting_since`, `cause` and `outcome`.

`payload()` copies them in field order and appends `director_action`. Persistence is wired in `src/broker/runtime/events.rs`:

- `validate_transition` gains an `InputQueueFlushed` branch, `validate_flushed_record`, beside the result and stalled branches, so the record never reaches `validate_cause`.
- `original()` rebuilds the same record from the stored payload, so `get_event` re-derives an identical payload.

Persistence validates the following, deduplicates on `(run_id, kind, flush_input_id)` and appends `--database-url URL` to `inspect_argv` for a non-default database:

- `flush_input_id` is a `flush` receipt of the run;
- `flushed_input_ids` equals that receipt's previews, and `message_ids` their messages;
- `input_id` is the first flushed preview;
- `remaining_count` is a non-negative integer, and `next_input_id` is non-null exactly when it is positive;
- `outcome` matches the receipt (`settled/submitted` ↔ `submitted`, `uncertain` ↔ `uncertain`);
- `cause` passes the stalled-cause validation.

`member_runtime_events.kind`, `RuntimeEventKind` and the cli.md kinds list gain `input_queue_flushed`. The per-preview `input_delivery_result` contract, including its `flush_input_id`, is unchanged.

### Removal

Per the Removal rule, these go in the same change, with every mention:

| Item | Location |
|---|---|
| `member flush` subcommand, its dispatch, guard entry and `FLUSH_DEADLINE` | `src/cli/member.rs`, `src/cli/mod.rs` |
| The CLI flush procedure, `FlushOutcome`, `FlushChannel`, `ControlFlushChannel` and their tests | `src/runtime/flush.rs` (file deleted), `src/runtime/mod.rs` |
| `InputControlOperation::Flush`, `InputControlReply::Flush`, `FlushAction`, `FlushReply`, `FlushWriteOutcome`, their wire codec and tests | `src/runtime/control.rs`, `src/runtime/mod.rs` re-exports |
| `serve_input`'s Flush dispatch and `RunnerLoop::flush` | `src/runtime/service.rs` |
| `flush_received`, `reserve_flush`, `settle_flush`, `submit_key_name`, `PendingFlush.expires_at` and `director_member_id`, and the housekeeping reservation-expiry branch | `src/runtime/input.rs` (replaced by § Write) |
| `Multiplexer::send_text` and its tmux and herdr implementations and tests | `src/multiplexer/mod.rs`, `tmux.rs`, `herdr.rs` |
| `member_flush_argv`, `flush_argv` and the `forced_flush` cause | `src/broker/runtime/events.rs` |
| `director={ID}; …` flush evidence references | `src/broker/runtime/inputs.rs` `settle_flush` (now runner-authored, § Settlement) |
| The pane-input row's "the Director's forced flush" | `SPEC.md` §1 |
| CLI contract, Director-only guard entry, Error Messages rows | `SPEC.md` §6.3 |
| The `send_text` backend operation, the `member flush` pane-lock rows, § Termination during pane input (the `member flush` span row and the SIGKILL row), the § Explicit key dispatch send-text paragraph, the § Push notifications forced-flush trade-off paragraph | `SPEC.md` §6.5 |
| The Claude permission profile `ask` entry, the § Composer drafts `§6.9 Forced flush` pointer | `SPEC.md` §6.7 |
| § Forced flush with its reservation and expiry contract, the delivery-result paragraph's "the `member flush` reply is the Director's synchronous result", the preview-sweep precondition's flush wording | `SPEC.md` §6.9 |
| The "forced flush served while the queue turn is held" test clause | `SPEC.md` §9 |
| `member flush` summary row, guard list, `permissions.allow` entry, `### member flush`, Error messages rows, Runner queue pointer, the `forced` delivery-method description ("submitted by `member flush`"), the `write_failed` and `runner_stopped` stop-cause rows' "expired forced flush" | `assets/skills/cafleet/reference/cli.md` |
| `member flush` command row, Director-owned list and guard list, the `review_stalled_queue` steps 4–6 and the closing flush paragraph | `assets/skills/cafleet/roles/director.md` |
| The forced-flush procedure and `forced_flush` row | `docs/docs/concepts/members.md` |
| `Bash(cafleet member flush *)` | `docs/docs/quickstart.md`, `docs/docs/concepts/coding-agents.md` |

The `flush` receipt kind, `forced` delivery method, `flush_input_id`, the flush reconcile cascade, the single-forced-preview reconcile refusal, `runner_lost` and the `member.md` marker rule stay.

### Runner diagnostic trace

#### Data model

`assets/migrations/V1__baseline.sql` gains, after `member_permission_response_claims`:

```sql
CREATE INDEX member_runs_member ON member_runs(member_id, run_id);

CREATE TABLE member_runtime_trace (
    trace_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    seq INTEGER NOT NULL CHECK(seq > 0),
    observed_at TEXT NOT NULL CHECK(length(trim(observed_at)) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('receipt_transition','head_gate','auto_flush',
        'preview_sweep','delivery_held','observer_state','runner_failure','trace_gap')),
    input_id INTEGER CHECK(input_id IS NULL OR input_id > 0),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'
        AND length(CAST(payload_json AS BLOB)) <= 2048),
    UNIQUE(run_id, seq)
);
CREATE INDEX member_runtime_trace_input ON member_runtime_trace(input_id, seq)
    WHERE input_id IS NOT NULL;

CREATE TRIGGER member_runtime_trace_cap AFTER INSERT ON member_runtime_trace
BEGIN
    DELETE FROM member_runtime_trace WHERE run_id = NEW.run_id AND seq <= NEW.seq - 2048;
END;

CREATE TRIGGER member_runtime_trace_run_retention AFTER INSERT ON member_runs
BEGIN
    DELETE FROM member_runtime_trace WHERE run_id IN (
        SELECT run_id FROM member_runs WHERE member_id = NEW.member_id
        ORDER BY run_id DESC LIMIT -1 OFFSET 8);
END;

CREATE TRIGGER member_input_operations_trace_insert AFTER INSERT ON member_input_operations
WHEN NEW.run_id IN (SELECT r.run_id FROM member_runs r
    WHERE r.member_id = (SELECT member_id FROM member_runs WHERE run_id = NEW.run_id)
    ORDER BY r.run_id DESC LIMIT 8)
BEGIN
    INSERT INTO member_runtime_trace(run_id, seq, observed_at, kind, input_id, payload_json)
    VALUES (NEW.run_id,
        COALESCE((SELECT MAX(seq) FROM member_runtime_trace WHERE run_id = NEW.run_id), 0) + 1,
        NEW.updated_at, 'receipt_transition', NEW.input_id,
        json_object('input_kind', NEW.kind, 'message_id', NEW.message_id,
            'from_state', NULL, 'state', NEW.state, 'disposition', NEW.disposition,
            'stop_cause', NEW.stop_cause, 'left_stage', NEW.left_stage,
            'delivery_method', NEW.delivery_method, 'delivery_reason', NEW.delivery_reason,
            'defer_reason', NEW.defer_reason, 'flush_input_id', NEW.flush_input_id));
END;

CREATE TRIGGER member_input_operations_trace_update AFTER UPDATE ON member_input_operations
WHEN (OLD.state IS NOT NEW.state OR OLD.disposition IS NOT NEW.disposition
    OR OLD.stop_cause IS NOT NEW.stop_cause OR OLD.left_stage IS NOT NEW.left_stage
    OR OLD.delivery_method IS NOT NEW.delivery_method
    OR OLD.delivery_reason IS NOT NEW.delivery_reason
    OR OLD.deferred_at IS NOT NEW.deferred_at OR OLD.flush_input_id IS NOT NEW.flush_input_id)
    AND NEW.run_id IN (SELECT r.run_id FROM member_runs r
        WHERE r.member_id = (SELECT member_id FROM member_runs WHERE run_id = NEW.run_id)
        ORDER BY r.run_id DESC LIMIT 8)
BEGIN
    INSERT INTO member_runtime_trace(run_id, seq, observed_at, kind, input_id, payload_json)
    VALUES (NEW.run_id,
        COALESCE((SELECT MAX(seq) FROM member_runtime_trace WHERE run_id = NEW.run_id), 0) + 1,
        NEW.updated_at, 'receipt_transition', NEW.input_id,
        json_object('input_kind', NEW.kind, 'message_id', NEW.message_id,
            'from_state', OLD.state, 'state', NEW.state, 'disposition', NEW.disposition,
            'stop_cause', NEW.stop_cause, 'left_stage', NEW.left_stage,
            'delivery_method', NEW.delivery_method, 'delivery_reason', NEW.delivery_reason,
            'defer_reason', NEW.defer_reason, 'flush_input_id', NEW.flush_input_id));
END;
```

- **Seq:** `seq` is a contiguous per-run ordinal, `MAX(seq)+1` read through the `UNIQUE(run_id, seq)` index inside the inserting transaction. SQLite serializes writers, so two inserts never read the same maximum.
- **Retained runs:** a run is retained while it is among its member's 8 highest `run_id`s. Every trace insert, from a trigger or from the runner, is made only for a retained run. A run outside the window therefore never gains a row after retention deleted its rows.
- **Bounds:**
  - The cap trigger keeps the newest 2,048 rows of the run, and the retention trigger keeps traces only for retained runs. Both prune in the inserting transaction.
  - The worst case is 4 MiB per run and 32 MiB per member; typical rows are about 200 bytes.
  - Runs, receipts and events themselves remain unpruned. They are retained for the database file's lifetime, as today.
- **Payload size:** every payload is built from fixed fields: integers, booleans, stored-enum values, RFC 3339 timestamps, SHA-256 digests and messages truncated to 512 bytes. No payload carries an unbounded list or a free-form receipt column; `operation_key` is excluded because the column is unbounded. The 2 KiB CHECK therefore cannot fail on a receipt write. A receipt row is well under 1 KiB at maximum integer and enum widths.
- **Receipts are not referenced:** `input_id` is a plain integer with no foreign key, so the trace never blocks a receipt deletion. A trace row may outlive a receipt that tests delete, and the existing receipt-deletion tests stay unchanged.
- **Atomicity:** a receipt transition commits or rolls back with the receipt write. A failing trace insert fails that receipt transaction exactly like any other storage failure on it. All runner writers, CLI reconciliation, shutdown and recovery are covered without per-site code.
- **Privacy:** no literal or screen-derived text. Receipt rows copy receipt enums and IDs only. Runner rows carry IDs, enums, digests, observation diagnostic codes and bounded `CafleetError` messages.
- **Guard:** setup and the command guard require the table, both indexes and the four triggers, by name and SQL, as part of the required shapes of the running binary's baseline.

#### Runner-written rows

`storage::append_trace(conn: &mut Connection, owner: &RunOwner, kind: TraceKind, input_id: Option<i64>, payload: &Value, deadline: Instant) -> Result<()>` runs one owned `IMMEDIATE` transaction:

1. It validates that the run's `owner_epoch` still matches `owner`; a mismatch fails as a stale owner.
2. For a run outside the retained window, it inserts nothing and returns `Ok`.
3. Otherwise it inserts the row with `seq` computed as above.

Its deadline is `TRACE_WRITE_BUDGET` (50 ms) from now. Every payload is built from the fixed fields in the table below; `append_trace` asserts the 2 KiB bound before the insert as a programming-error check.

**Writers.** Each thread that holds a runner `Connection` owns one `TraceWriter { owner: RunOwner, dropped: u64 }`:

| Writer | Thread and connection | Rows |
|---|---|---|
| `runner` | The runner loop's connection, held by `InputCoordinator` and reached from `RunnerLoop` through it | `head_gate`, `auto_flush`, `preview_sweep`, `delivery_held`, `observer_state`, `runner_failure` except the two sites below |
| `event_writer` | The `EventWriter` thread's own connection | `runner_failure` site `effective_policy_dropped` |
| Cleanup | The final cleanup transaction that persists `cleanup_state` | `runner_failure` site `runner_diagnostic`, inserted in that transaction after its `lock.validate`, so they commit or roll back with the cleanup write |

**Failure policy (diagnostic sink).** A `TraceWriter` trace write that fails is dropped. The writer then:

- keeps the stderr line it already writes;
- increments its own `dropped` counter;
- never fences input, delays an attempt or retries the row.

That writer's next successful write first appends `trace_gap` `{"writer":WRITER,"dropped":N}` and resets its counter. A rolled-back cleanup transaction loses its `runner_diagnostic` rows, and their stderr lines remain.

| `kind` | Written when | `input_id` | Payload (ordered) |
|---|---|---|---|
| `head_gate` | At each housekeeping sweep, only when `(head_input_id, gate, detail, blocking_input_id)` differs from the last `head_gate` row the runner wrote | The head, or null for `empty` | `head_input_id, gate, detail, blocking_input_id, head_since` |
| `auto_flush` | Flush-due; each refusal whose `(kind, input_id)` differs from `traced_refusal`; reservation; each phase completion; settlement | The stalled head | `step, flush_input_id, refusal_kind, refusal_input_id, flushed_count, first_flushed_input_id, last_flushed_input_id, remaining_count, written_bytes, outcome, stop_cause, literal_digest, cause_kind` (nulls where a step has no value) |
| `preview_sweep` | At each sweep decision, only when `(decision, message_id, input_id, held_reason)` differs from the last `preview_sweep` row, or when the backoff delay changes | The swept receipt, or null | `decision, message_id, input_id, attempt, held_reason, backoff_ms, eligibility, gate_reason` |
| `delivery_held` | Each submission the runner answers `held` without creating a receipt, from the control socket or the sweep | null | `source, message_id, input_kind, held_reason, eligibility, gate_reason, visibility, diagnostic` |
| `observer_state` | When `(eligibility, gate_reason, visibility, diagnostic, gap_open, retained_transition)` changes, at most once per `OBSERVER_TRACE_INTERVAL` (1 s) | null | `eligibility, gate_reason, visibility, diagnostic, gap_open, retained_transition, gap_count, changes` |
| `runner_failure` | At each runner diagnostic site in the table below | The input the site names, else null | `site, message` (`message` is the `CafleetError` message, truncated to 512 bytes at a scalar boundary) |
| `trace_gap` | Before a writer's first successful row after dropped rows | null | `writer, dropped` |

Every row kind has a fixed field set with no list, so each payload is provably under 1 KiB.

**Head gate placement.** `queue_housekeeping` computes and traces the gate before its early returns:

- With input fenced, it writes gate `input_fenced` for the FIFO front (null when the FIFO is empty) and then returns.
- With an empty FIFO and no slot, it writes gate `empty` and returns.

The change rule writes each of these rows once per state.

`gate` takes the first value that holds, in this order:

1. `input_fenced`;
2. `empty`;
3. `flush_writing` (a flush slot is set; `detail` is the phase name);
4. `immediate_attempt`;
5. `deferred_attempt` (`detail` is the `InputPhase` name: `await_reservation`, `await_before_write`, `armed`, `blocked`, `closing`, `verifying`);
6. `pending_finalization`;
7. `pending_stamp`;
8. `held_composer`;
9. `unresolved_input`;
10. `drain_pending` (`detail` is the blocker);
11. `interaction`;
12. each recorded head obstacle (`pane_input_busy`, `delivery_budget_exhausted`, `storage_unavailable`, `attempt_failed` with its detail, `composer_busy`);
13. `waiting` (no gate holds; the queue turn should reach the head).

`step` is one of `due`, `refused`, `reserved`, `literal_written`, `settle_elapsed`, `submit_written` and `settled`.

**Preview sweep decisions.** `sweep_previews` and `finish_sweep` record `decision` as one of:

| `decision` | When |
|---|---|
| `owned_by_queue` | The coordinator owns preview delivery, so the sweep is skipped |
| `backoff_wait` | The run-wide backoff has not expired |
| `no_candidate` | No unnotified pending preview, or the latest attempt waits as deferred |
| `settled_history` | An accepted or submitted earlier attempt is settled without typing |
| `submitted`, `held`, `queued`, `rejected`, `uncertain`, `left_in_composer` | The sweep attempt's `SubmissionStatus` |
| `failed` | The sweep read or attempt failed; the `runner_failure` row carries the message |

`backoff_ms` is the run-wide backoff delay after the decision, and `eligibility` and `gate_reason` are the observer's input eligibility at the sweep. The change rule keeps the one-second sweep cadence from filling the cap: a pass with an unchanged decision and backoff writes nothing.

**Observer state.** `RunnerLoop` reads `ObservationController::status()` on every tick. A changed tuple is written when the last `observer_state` row is at least `OBSERVER_TRACE_INTERVAL` old. Otherwise the change is held, and the row written once the interval elapses carries the latest tuple and `changes`, the number of tuple changes it summarizes. `diagnostic` is the observer's `ObservationFailure` code. `gap_open` is true while an observation gap interval is pending. The throttle bounds a flapping observer to one row per second.

`runner_failure` sites:

| `site` | Source today |
|---|---|
| `deferred_input` | `service.rs` `retry_deferred` |
| `queue_housekeeping` | `service.rs` `housekeep_queue` |
| `auto_flush_reservation` | `reserve_automatic_flush` storage failure (new) |
| `preview_sweep` | `service.rs` `sweep_previews` and `finish_sweep` |
| `preview_sweep_settlement` | `service.rs` `finish_sweep` settlement failure |
| `result_pending_storage` | `input.rs` zero-byte finalization deferred by storage |
| `stamp_pending_storage` | `input.rs` notification stamp deferred by storage |
| `result_pending_recovery` | `input.rs` deferred result left to recovery |
| `effective_policy_dropped` | `writer.rs` `policy_dropped` |
| `runner_diagnostic` | `service.rs` run-outcome diagnostics, written in the final cleanup transaction |

Each site keeps its stderr line. The trace row carries the same message.

#### CLI: `cafleet member trace`

```
cafleet member trace MEMBER_ID [--run-id RUN_ID] [--input-id INPUT_ID] [--kind KIND] [--limit N] [--json]
```

It is read-only and has no Director-only guard (like `input show`), and it needs no live runner. One deferred read transaction serves the whole command.

| Rule | Behavior |
|---|---|
| Member | Existing not-found error |
| Run | `--run-id` must belong to the member: `Error: run RUN_ID does not belong to member MEMBER_ID`. Omitted, it selects the member's highest `run_id`; a member with no run fails with `Error: member MEMBER_ID has no runtime run` |
| `--input-id` | Must belong to the selected run: `Error: input INPUT_ID does not belong to run RUN_ID`. Selects rows whose `input_id` is that input |
| `--kind` | One of the `member_runtime_trace.kind` values; help lists them from the stored-enum registry |
| `--limit` | 1–2,048, default 200; returns the newest N matching rows |
| Order | Oldest first by `seq` |

JSON: ordered `{member_id, run_id, retained, pruned_count, rows}`.

- `retained` is `false` when the run is outside the member's 8 most recent runs. Its rows are then `[]`, because no trace insert targets a non-retained run.
- `pruned_count` is computed over the run's retained rows regardless of `--input-id`, `--kind` and `--limit`: `MIN(seq) - 1`, or `0` when the run has no row. For a non-retained run it is `null`.
- Each row is ordered `{trace_id, seq, observed_at, kind, input_id, payload}`.

Text: `Trace: member MEMBER_ID run RUN_ID rows N pruned P`, followed by one line per row: `SEQ OBSERVED_AT KIND input=INPUT_ID PAYLOAD_JSON`, with `-` for a null input. A run outside retention prints `Trace: member MEMBER_ID run RUN_ID not retained`.

**Why a new subcommand:** the trace is member- and run-scoped like `member show`, and `member show` must stay a compact status read. `doctor --verbose` is a setup and health surface with no run or input selector. `--input-id` answers the #623 question ("what happened to input 4805") in one command, and `--kind delivery_held` with `--kind preview_sweep` answers the § Background reproduction.

### Post-turn suggestion rule

This step tests the weakened suggestion candidate (§ Background). Its fixture and rule change are conditional on what the capture shows.

**Recipe.** It follows the observed sequence, not the single completed turn that 0000236 failed to reproduce in 4 runs:

1. Launch Claude Code 2.1.322 in a Herdr 0.9.5 pane through the authorized research setup from 0000236. Use a private Claude configuration with the member launch flags, including `--prompt-suggestions false`.
2. Type the prompt into the pane as key input through the outer pane, as `member press` does: the text `poll inbox`, then Enter. The turn is a short text-only reply with no tool call.
3. After the turn completes, leave the pane idle with no input for 15 minutes.
4. Capture the screen with `cell_styles` right after the turn and at 5, 10 and 15 minutes of idle.
5. Stop the research native session before any further key is sent to its pane.

**Attempt budget.** Three attempts, each a fresh research session. An attempt reproduces when any capture shows non-blank text after the composer prompt marker. The step stops at the first reproducing attempt.

**Classification check.** For the reproducing capture, read the `cell_styles` of every cell from the column after the prompt marker to the row's last non-blank cell:

- **Suggestion-styled:** every cell carries the profile's suggestion STYLE (faint, or the suggestion foreground colour).
- **Normal-styled:** any cell carries the ordinary input style. The text is a retained draft. It is real composer input and stays non-empty; the suggestion rule must not match it.

**Outcomes.**

| Capture result | Fixture | Rule | `SPEC.md` §6.7 Claude research row |
|---|---|---|---|
| Suggestion-styled text the bundled rule does not match | `claude-2.1.322-post-turn-suggestion-herdr-095-1` registered | `claude-suggestion-composer` extended, or a second suggestion rule added, so the fixture classifies `ComposerEmpty` with `MatchEvidence::Complete` and `composer_draft` yields an empty draft; the startup-hint fixture keeps matching | Records the post-turn rendering and its fixture |
| Suggestion-styled text the bundled rule already matches | Registered as evidence | Unchanged | Records the rendering; the Changelog records the candidate as ruled out |
| Normal-styled text | `claude-2.1.322-post-turn-draft-herdr-095-1` registered | Unchanged; the fixture classifies non-empty and `composer_draft` returns the text | Records that the text is a retained draft; the Changelog records it for the Director as a separate input defect |
| Not reproduced in three attempts | None | Unchanged | Replaces "no post-turn suggestion in 4 turns" with the combined count, including these attempts and the recipe; the Changelog records the failed attempts |

Registered fixtures go in `assets/profiles/native/claude.json` and `assets/profiles/bundled.json`.

### Skills and documentation

- **`SPEC.md`:**
  - §1: the pane-input row drops "the Director's forced flush" and names the runner's automatic flush.
  - §3: drop `src/runtime/flush.rs`; add the trace module.
  - §6.3: remove `member flush` (summary row, output shape, Director-only guard list, `##### member flush`, Error Messages rows) and add `member trace` (summary, output shape, contract, error rows).
  - §6.5: remove `send_text` (Backend operations, the send-text row, the § Explicit key dispatch send-text paragraph), the `member flush` pane-lock rows, the § Termination during pane input `member flush` span and SIGKILL rows, and the § Push notifications forced-flush trade-off paragraph; add the automatic flush's lock hold and its termination row.
  - §6.7: remove the `member flush` `ask` entry from the Claude permission profile; § Composer drafts points at §6.9 Automatic flush, and the suggestion text names the automatic flush; the Claude suggestion research row is updated per the § Post-turn suggestion rule outcome.
  - §6.9:
    - Replace § Forced flush with § Automatic flush, carrying § Refusals, § Write, § Settlement and the § Director response table.
    - Update § Queue housekeeping and the stalled event (`flush_refusal`, the publication and deduplication rule, the six-key record, INSTRUCTION, no `forced_flush`).
    - Update § Delivery reporting: the delivery-result paragraph names `input_queue_flushed` instead of the `member flush` reply.
    - Update the preview-sweep precondition and § Stop cause wording.
    - Add § Input queue flushed event and § Runtime trace (writers, failure policy, row kinds, retention).
  - §8: the DDL above and the `input_queue_flushed` event kind.
  - §9: replace the "forced flush served while the queue turn is held" clause with the automatic flush running while the queue turn is held; keep "no pane capture on any input path".
- **`docs/docs/concepts/members.md`:** § Stalled queues and forced flush becomes "Stalled queues and automatic flush": the runner flushes on its own, refusals reach the Director, retries are automatic, and `member trace` gives the history. It links `SPEC.md` for exact contracts.
- **`docs/docs/concepts/storage.md`:** one sentence that the runtime trace is bounded per run and per member, growing to at most a few tens of MiB per member, and is read with `member trace`.
- **`docs/docs/quickstart.md`, `docs/docs/concepts/coding-agents.md`:** drop the `member flush` `ask` entry.
- **`assets/skills/cafleet/reference/cli.md`:**
  - Removal: the § Removal rows.
  - Runtime events: `flush_refusal`, the new `review_stalled_queue` INSTRUCTION, the § Director response table, `input_queue_flushed` with its actions, and the kinds list.
  - Delivery methods and stop causes: `forced` is "submitted by the runner's automatic flush"; the `write_failed` and `runner_stopped` rows name an interrupted automatic flush; `observer_unavailable` covers an observer lost mid-flush.
  - Runner queue: the automatic flush in one paragraph.
  - New `### member trace` with synopsis, outputs, row kinds and errors, its summary row and its Error messages rows.
- **`assets/skills/cafleet/roles/director.md`:**
  - § Reconcile generated input `review_stalled_queue`: verify, capture, inspect, act on `flush_refusal.kind` per the § Director response table, then let the runner retry.
  - New procedures for `review_uncertain_flush` (capture, then reconcile `FLUSH_INPUT_ID`) and `report_runner_defect` (record and route).
  - `member trace` joins the investigation commands, including for a send reported `held` that is not delivered.
  - `member flush` leaves the command table and the Director-owned and guard lists.
- **`assets/skills/cafleet/roles/member.md`:** unchanged; its marker rule already covers runner-authored flushes.
- **`README.md`:** unchanged.

### Testing

Every test is an in-file `#[cfg(test)] mod tests` unit in the file owning the asserted behavior.

| Behavior | Owning file |
|---|---|
| Flush-due trigger at 3 minutes, interaction reset, no Director command; the selected previews settle `submitted/forced` | `src/runtime/input.rs` |
| Each § Refusals row writes zero bytes and creates no receipt, in order; ID lists and `; also queued`; a pending finalization and a pending stamp do not refuse | `src/runtime/input.rs` |
| A storage failure in the check-3 read, the live-message read or the reservation writes no byte and no receipt, records `storage_unavailable`, publishes nothing, and the next sweep flushes | `src/runtime/input.rs` |
| Retry on the next sweep after the blocker is resolved (an uncertain blocker reconciled, then flushed) | `src/runtime/input.rs` |
| Publication once per `(kind, input_id)` pair per head: A→B→A publishes two events, an interaction reset publishes nothing new, a new unresolved input publishes; `queue_changed` silent | `src/runtime/input.rs` |
| While a slot is held by a failing settlement past 3 minutes, the remainder head never becomes flush-due and nothing is published | `src/runtime/input.rs` |
| Plain-limit prefix and the 128-preview cap: the remainder becomes the head with a fresh `head_since` | `src/runtime/input.rs` |
| `drive_flush` phases across ticks: partial writes, 500 ms settle without blocking, submit bytes; checks 5/6 before every write; an interaction after the first chunk settles `uncertain/interaction_open` and an observer loss `uncertain/observer_unavailable` with no further byte; deadline and `Err` before the first byte settle `NotStarted`, after it `uncertain/write_failed` | `src/runtime/input.rs` |
| `write_forced` waits on queued human bytes and a pending reply prefix, invalidates the drain proof and leaves the generations unchanged | `src/runtime/transport.rs` |
| `drive_flush` runs on the tick path while the drain stays `Pending` and an immediate attempt is in flight | `src/runtime/service.rs` |
| A busy pane lock refuses `pane_input_busy`; `member press` cannot interleave while the slot holds the lock | `src/runtime/input.rs` |
| Settlement storage failure keeps the slot and writes no byte until retried; shutdown and recovery cascades | `src/runtime/input.rs` |
| `input_delivery_stalled` `flush_refusal` validation, the six-key record, `original()` round trip and pair deduplication; `forced_flush` rejected; INSTRUCTION text; no `flush_argv` | `src/broker/runtime/events.rs` |
| `input_queue_flushed` record validation, `original()` round trip through `get_event`, deduplication, the three action rows and a 128-preview payload under `PAYLOAD_LIMIT` | `src/broker/runtime/events.rs` |
| Trace triggers: insert and each traced-column update write one row in the receipt's transaction; untraced updates write none; a rolled-back receipt write leaves no row; a non-retained run gets no row; a receipt with the longest `operation_key` still writes | `src/broker/runtime/trace.rs` |
| Cap keeps the newest 2,048 rows with contiguous `seq`; retention keeps 8 runs per member; 2 KiB payload CHECK; `append_trace` refuses a stale owner and skips a non-retained run | `src/broker/runtime/trace.rs` |
| Each `TraceWriter` drops on storage failure, counts its own drops, then writes its own `trace_gap` first | `src/broker/runtime/trace.rs` |
| `head_gate` written only on change, every gate value including `input_fenced` and `empty` before the early returns; `auto_flush` steps | `src/runtime/input.rs` |
| `preview_sweep` decisions on change and on backoff change; `delivery_held` for a control-socket and a sweep held reply with no receipt; `observer_state` throttle with `changes`; the runner-thread `runner_failure` sites | `src/runtime/service.rs` |
| `runner_diagnostic` rows commit with the cleanup transaction and roll back with it | `src/runtime/service.rs` |
| `effective_policy_dropped` row from the writer thread's own `TraceWriter` | `src/runtime/writer.rs` |
| `member trace` selection, filters, limits, ordering, `retained`, filter-independent `pruned_count`, text and JSON, each error | `src/cli/member.rs` |
| Schema guard requires the trace table, both indexes and the triggers | `src/db.rs` |
| Per the § Post-turn suggestion rule outcome: a suggestion-styled post-turn fixture classifies `ComposerEmpty` with complete evidence and an empty `composer_draft`, or a normal-styled one classifies non-empty and returns its draft; the startup-hint fixture still classifies empty | `src/coding_agent/detector.rs` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [ ] Update `SPEC.md` §1, §3, §6.3, §6.5, §6.7 (including the Claude post-turn suggestion research row), §6.9, §8 and §9 per § Skills and documentation <!-- completed: -->
- [ ] Update `docs/docs/concepts/members.md`, `docs/docs/concepts/storage.md`, `docs/docs/quickstart.md` and `docs/docs/concepts/coding-agents.md` <!-- completed: -->
- [ ] Update `assets/skills/cafleet/reference/cli.md` (removals, Runtime events, Director response table, delivery methods and stop causes, Runner queue, `### member trace`, summary and Error messages) <!-- completed: -->
- [ ] Update `assets/skills/cafleet/roles/director.md` (`review_stalled_queue` with the Director response table, `review_uncertain_flush`, `report_runner_defect`, `member trace`, `member flush` removal) <!-- completed: -->

### Step 2: Remove the Director flush

- [ ] Remove the `member flush` subcommand, dispatch, guard entry and `FLUSH_DEADLINE` from `src/cli/member.rs` and `src/cli/mod.rs` <!-- completed: -->
- [ ] Delete `src/runtime/flush.rs` and its `src/runtime/mod.rs` module and re-exports <!-- completed: -->
- [ ] Remove the `Flush` control operation, reply and codec from `src/runtime/control.rs`, and the dispatch from `src/runtime/service.rs` <!-- completed: -->
- [ ] Remove `Multiplexer::send_text` and its tmux and herdr implementations and tests <!-- completed: -->
- [ ] Remove `member_flush_argv`, `flush_argv` and the `forced_flush` cause from `src/broker/runtime/events.rs` and the watchdog <!-- completed: -->

### Step 3: Automatic flush in the runner

- [ ] Replace the per-head `stalled` flag with `HeadFlushState`, make the watchdog trigger mark the head flush-due, skip it while a slot is set, and pass `&Transport` to `queue_housekeeping` <!-- completed: -->
- [ ] Implement `reserve_automatic_flush` with § Refusals in order, the 128-preview cap, the reservation transaction and the storage-failure outcome <!-- completed: -->
- [ ] Add `Transport::write_forced` <!-- completed: -->
- [ ] Implement `PendingFlush` phases and `drive_flush` with checks 5/6 before every write, called from `tick_running` after `housekeep_queue` <!-- completed: -->
- [ ] Implement the five settlements with runner-authored evidence references in `src/broker/runtime/inputs.rs` and `src/runtime/input.rs` <!-- completed: -->
- [ ] Add the trigger, refusal, storage-failure, retry, slot, prefix, phase, lock and settlement tests <!-- completed: -->
- [ ] Add the `write_forced` tests and the tick-path test while the drain is `Pending` <!-- completed: -->

### Step 4: Events

- [ ] Add `flush_refusal` to `input_delivery_stalled`: payload, six-key record validation, `original()` reconstruction, pair deduplication and pair publication <!-- completed: -->
- [ ] Replace the `review_stalled_queue` action and INSTRUCTION <!-- completed: -->
- [ ] Add the `input_queue_flushed` kind to `V1__baseline.sql` and `RuntimeEventKind`, with its outbox record, `validate_flushed_record` branch, `original()` reconstruction, payload construction, deduplication and the three actions <!-- completed: -->
- [ ] Add the event tests <!-- completed: -->

### Step 5: Trace storage

- [ ] Add the `member_runs_member` index and the `member_runtime_trace` table, index and triggers to `V1__baseline.sql`, and the required shapes to the setup and command guard <!-- completed: -->
- [ ] Add `src/broker/runtime/trace.rs` with `TraceKind`, `TraceWriter`, `append_trace` (owner check, retained-run skip) and the read query <!-- completed: -->
- [ ] Add the trigger, retention, cap, payload-bound, writer and guard tests <!-- completed: -->

### Step 6: Runner trace writes

- [ ] Give the runner loop and the `EventWriter` thread their own `TraceWriter`, and write `runner_diagnostic` rows in the final cleanup transaction <!-- completed: -->
- [ ] Write `head_gate` rows on change from housekeeping, before its early returns, with the § Runner-written rows gate order <!-- completed: -->
- [ ] Write `auto_flush` rows at each step <!-- completed: -->
- [ ] Write `preview_sweep` rows from `sweep_previews` and `finish_sweep`, and `delivery_held` rows for every held reply with no receipt <!-- completed: -->
- [ ] Write throttled `observer_state` rows from the runner loop <!-- completed: -->
- [ ] Write `runner_failure` rows at every listed site, keeping each stderr line <!-- completed: -->
- [ ] Add the runner trace tests <!-- completed: -->

### Step 7: Post-turn suggestion rule

- [ ] Run the § Post-turn suggestion rule recipe within its three-attempt budget, classify any reproducing capture by `cell_styles`, and record the outcome row in this doc <!-- completed: -->
- [ ] Apply the outcome row: register the fixture and extend the rule and its `fixture_ids`, register a draft fixture, or change nothing; update the `SPEC.md` §6.7 Claude research row accordingly <!-- completed: -->
- [ ] Add the detector tests the outcome requires (the post-turn fixture empty or non-empty, and the startup-hint fixture still empty) <!-- completed: -->

### Step 8: `cafleet member trace`

- [ ] Add the `Trace` subcommand to `src/cli/member.rs` with `--kind` choices from the stored enum <!-- completed: -->
- [ ] Implement selection, filters, ordering, `retained`, `pruned_count`, text and JSON output and the error strings <!-- completed: -->
- [ ] Add the CLI tests <!-- completed: -->

### Step 9: Validation

- [ ] Run `mise //:test` and `mise //:lint`, recording the test thread count used <!-- completed: -->
- [ ] Run `mise //docs:build` <!-- completed: -->
- [ ] Search the repository outside `design-docs/` for `member flush`, `flush_argv`, `forced_flush`, `send_text`, `send-text`, `FlushAction`, `FlushReply` and `FlushWriteOutcome`, and confirm nothing remains <!-- completed: -->
- [ ] Re-run a stalled-queue reproduction live with the branch binary on a dedicated database (Claude Code on Herdr; no reinstall), and confirm the automatic flush, `input_queue_flushed` and the `member trace` history, including `preview_sweep`, `delivery_held` and `observer_state` rows <!-- completed: -->
- [ ] Record the results and mark the design doc complete <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-21 | Initial draft |
| 2026-11-21 | Review round 1: Overview trimmed; check 3 exempts finalization; reservation storage-failure outcome; checks 5/6 before every write; `observer_unavailable` and zero-byte `Err` settlements; watchdog off while a slot is set; per-pair publication and deduplication; `flush_refusal.input_id`; Director response table; `input_queue_flushed` outbox record and bounded payload (`remaining_count`, `next_input_id`, 128-preview cap); trace `input_id` without FK, `operation_key` dropped, retained-run gating, fixed-field payloads, per-thread `TraceWriter`, head-gate placement; filter-independent `pruned_count`; added removal surfaces; held reproduction with `preview_sweep`, `delivery_held` and `observer_state` rows |
| 2026-11-21 | Review round 1 addendum: second held reproduction (03:01:42 UTC) and the post-turn suggestion leading hypothesis in Background; § Post-turn suggestion rule with fixture capture, rule extension, tests and Step 7 |
| 2026-11-21 | Review round 2: 03:09:46 UTC counterexample; observation gap after an idle turn is the leading candidate and the suggestion candidate is weakened; § Post-turn suggestion rule gains the observed-sequence recipe, a three-attempt budget, the `cell_styles` classification check and a conditional outcome table; held-reproduction criterion restated as `delivery_held` per held reply plus `preview_sweep` on change |
| 2026-11-21 | User approved; status Approved |
