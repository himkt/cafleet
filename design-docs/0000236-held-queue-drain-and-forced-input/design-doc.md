# Held runner queue drain and Director forced flush

**Status**: Complete
**Progress**: 43/43 tasks complete
**Last Updated**: 2026-11-20

## Overview

The member runner's deferred-input FIFO can stop reaching its queue turn (GitHub issue #623). When it does, a head whose message the member already consumed stays `ready`, later previews queue behind it as `earlier_input`, and no `input_delivery_stalled` is published. This change moves queue housekeeping and stall detection off the write path, so the queue always reaches its turn or reports why it cannot. It also adds `cafleet member flush`, a Director-only command that rebuilds every queued preview of one member from its stored message and submits them through the outer pane in one forced submission, even while the queue turn is stalled.

## Success Criteria

- [x] Background § Root cause records the gate that held run 34's queue head, established by Step 1's investigation. A committed in-file unit test reproduces that sequence: it fails on the pre-change code and passes after the fix, with the queued receipt settled `not_submitted/message_consumed`.
- [x] An immediate ordinary attempt that ends after its first child byte always records its receipt `uncertain/write_failed` before the runner replies, including when the deadline expires inside `advance` or in the pre-write drain of `complete_pending_input`.
- [x] Every queued receipt whose message is consumed is cancelled as `not_submitted/message_consumed` within one housekeeping interval (1 s). This holds regardless of drain state, in-flight immediate attempts or sweeps, pending storage retries or swallowed attempt errors.
- [x] A queue head that has not started an attempt for 3 minutes publishes exactly one `input_delivery_stalled` carrying a `cause`. Time during which an open interaction blocks the head is excluded, as a deliberate scope decision (§ Stall watchdog).
- [x] A composer that shows only a native suggestion is classified empty for every backend whose native CLI renders suggestions, proven by bundled native fixtures including Claude Code 2.1.322 on Herdr 0.9.5.
- [x] `cafleet member flush MEMBER_ID --from-member-id DIRECTOR_ID` submits ` [cafleet: ignore any text before this marker, if present] ` followed by the queued preview lines in the runner's in-memory FIFO order, then the profile submit key, through the outer pane. It sends no Escape and no clear key. It flushes every queued preview, or the longest FIFO prefix within the plain limit, and reports the rest in `remaining_input_ids`. Each flushed receipt is settled `submitted` with `method=forced`.
- [x] The flush works while the runner's queue turn is stalled. Its runner actions are served on the control-socket path, which runs before and outside `tick_running`, `retry_deferred`, the drain and `start_deferred`. Unit tests show the flush either succeeding or refusing with each § Gate tolerance row's named cause through `flush_received`, and prove the control-socket dispatch path while the queue turn is held.
- [x] The flush refuses, writing zero bytes, on every condition in § Refusals.
- [x] Each flushed preview publishes one `input_delivery_result` with `delivery.method=forced` and its `flush_input_id`.
- [x] `SPEC.md`, `docs/`, `assets/skills/cafleet/reference/cli.md` and the Director and member roles describe the new behavior; `mise //:test`, `mise //:lint` and `mise //docs:build` pass.
- [x] A live re-run of the issue #623 reproduction on Claude Code 2.1.322 / Herdr 0.9.5 shows the queue draining or publishing `input_delivery_stalled`, and `member flush` delivering the queued previews.

---

## Background

### Observed failure (run 34, CAFleet 0.35.4, Claude Code 2.1.322, Herdr 0.9.5)

| Input | Message | Observed state | Meaning |
|---|---|---|---|
| 4804 | 6721 | `in_progress`, `stop_cause=null` until the Director reconciled it `submitted` at 12:06:36 | Immediate preview whose sender reported `operation exceeded its deadline` / `status=uncertain`; the text was submitted |
| 4805 | 6723 | `ready`, `earlier_input`, `updated_at` equal to `created_at` (12:04:08.106) | Queue head; no attempt and no cancellation ever touched the row, although the member consumed 6723 |
| 4836 | 6773 | `ready`, `earlier_input` | Queued behind 4805; the member also consumed 6773 |

The runner main loop stayed alive:

- `member press` keys reached the child through it.
- It served the Director's `input reconcile` exchange. That exchange is a control-socket `ReceiptAction`, handled by `serve_client` → `serve_input` → `submit` → `InputCoordinator::receive` → `receipt_received` with no drain and no queue turn.

The runner writes diagnostics such as `member runtime: deferred input: …` to stderr, which is the member pane, so the TUI overdraws them.

### Defects established by code reading

| ID | Location | Defect |
|---|---|---|
| D1 | `src/runtime/input.rs` `InputCoordinator::advance`, error mapping after its closure | When a written immediate ordinary attempt fails, for example because `effective_deadline` fails at an expired deadline, the `map_err` calls `hold_possible_ordinary_input`, `retire_failed_deferred` (deferred attempts only) and `retain_permission_failure` (returns early for non-permission input). None of them records the receipt `uncertain`, so it stays `in_progress` with `stop_cause=null` (4804). |
| D2 | `src/runtime/service.rs` `complete_pending_input` | A `quiesce` error for the in-flight immediate attempt maps straight to the client reply, skipping coordinator bookkeeping, which produces the same unrecorded `in_progress` receipt. |
| D3 | `src/runtime/input.rs` `start_deferred` | A head blocked by an unresolved `in_progress` receipt that is not the held composer returns at `unresolved_ordinary` with no timer and no event, so it can wait silently forever. |
| D4 | `src/runtime/service.rs` `tick_running`, `retry_deferred` | The queue turn, including its message-consumption check, runs only when all of these gates pass: (a) no immediate client attempt or preview sweep is in flight; (b) `quiesce` returns `Quiescent`; (c) `start_deferred` passes its `input_fenced`, `finalization` and `pending_stamp` early returns. A gate that fails returns silently, and a non-fatal `advance` error is only written to stderr. |
| D5 | `src/runtime/input.rs` receipt `Reconcile` action | The action updates the `operations` cache but never the FIFO, so a queued receipt settled through it stays queued, and a later zero-byte settlement expecting `ready` fails permanently, fencing input. |

### Root cause

**Established:** 4805's `updated_at` equals `created_at` after its message was consumed. That proves the consumption check never ran on any tick after the consumption, so one of the D4 gates held the head.

The evidence already rules out:

- **A pending attempt left in `InputPhase::Blocked`.** `ReceiptAction::Reconcile` requires `self.pending.is_none()`, and the 12:06:36 reconcile of 4804 succeeded, so `pending` was `None` at that moment.
- **`complete_pending_input` spinning forever on one client.** `quiesce_for_input` fails at an expired deadline (`transport.rs` `check_deadline` at entry).

**Hypothesis tested:** gate (b), with `quiesce` returning `Pending` on every 100 ms retry, plausibly left by the keystrokes injected during the 4804 write. Step 1 confirmed gate (b) and its blocker `observation incomplete`, but as a permanent latch rather than repeated `ROUND_LIMIT` spinning.

**Gate finding (Step 1):** gate (b), drain blocker `observation incomplete`, held as a permanent latch rather than a transient re-arm.

- `Transport::observe_drain_child` sets `observation_complete = false` and restores it only on `Ok`. A deadline expiry inside it (the entry `check_deadline`, the `DeadlineObserver` around the observer callback, or the post-feed check) or an observer error returns `Err` and leaves the flag `false`.
- Every later restore copies that `false` value back, and nothing else sets it `true`. Every later `quiesce_for_input` that reaches a quiet round therefore returns `Pending("observation incomplete")` for the Transport's lifetime, so `retry_deferred` never calls `advance` and `start_deferred`'s consumption check never runs.
- The unfed remainder of the interrupted chunk is never fed to the parser, so the emulated screen has dropped bytes.
- The `ObservationController` stays healthy throughout (`degraded=false`, `pending_count=0`, eligibility `ComposerEmpty`), so nothing reports the fence. The latch also blocks every `permission respond` for the rest of the run.
- A scratch model of the run 34 sequence on the pre-change code reproduced it: the deadline-expired immediate write left its receipt `in_progress` with `stop_cause=null` (D2), the queued head stayed `ready` with `updated_at == created_at` after its message was consumed, and every drain blocker was `observation incomplete`. One `advance` with no quiesce gate settled the head `not_submitted/message_consumed`.
- In run 34 the injected keystrokes kept every drain round non-quiet until the client deadline, and with the child echoing, the observer callback dominates each round, so the expiry most plausibly landed inside `observe_drain_child`.
- **CPU observation:** not explained by `ROUND_LIMIT` spinning, because with a quiet terminal the latched drain returns on its first quiet round. One cheap `quiesce` per 100 ms retry is consistent with a modest CPU rise above idle but does not establish the 2x figure, so the CPU note stays consistent but unconfirmed.

The fix in § Runner queue housekeeping takes the consumption check, the stall watchdog and the flush off every D4 gate, so it holds whichever gate fired. § Runner: interrupted drain feed repairs the latch itself.

---

## Specification

### Runner: immediate attempt failure bookkeeping (D1, D2)

- **Rule:** every path that ends an immediate ordinary (`preview`, `prompt`, `exec`) attempt after its first child byte records the receipt `uncertain` with `stop_cause=write_failed` before the runner replies to the client. `write_failed` already covers "PTY/checkpoint error, unrecorded possible I/O". A zero-byte attempt returns to `ready` exactly as today.
- **`advance`:** its error mapping gains an ordinary-attempt branch beside `retain_permission_failure`. When the pending composer attempt is immediate and has written bytes, the branch records `record_input_uncertain(…, StopCause::WriteFailed)`.
- **Expired deadline:** when the caller's deadline has passed, the bookkeeping transaction uses `Instant::now() + DEFERRED_FINALIZATION` as its storage deadline, matching `stop_composer_stages`.
- **`complete_pending_input`:** a `quiesce` failure goes through the same coordinator bookkeeping, `InputCoordinator::pending_attempt_failed(&mut self, conn: &mut Connection, observation: &ObservationController, failure: InputFailure) -> InputFailure`, before `finish_pending_attempt`; it records a written immediate attempt `uncertain/write_failed` and returns the client failure.
- **Held composer:** set exactly as today. The receipt now carries `uncertain/write_failed`, which satisfies `has_stalled_blocker_provenance`.

### Runner: interrupted drain feed (gate (b) repair)

**User decision:** repair the `observation incomplete` latch by retaining the unfed remainder (§ Root cause), so no child byte is dropped.

- **Retention:** when a deadline expiry or an observer error interrupts `observe_drain_child` (or the ordinary-progress feed in `tick`) partway through a child chunk, the transport retains the suffix the parser has not consumed as a pending observation remainder. The prefix already fed stays fed, and no byte is replayed.
- **Resumption:** the next drain round or ordinary tick feeds the retained remainder before any new child read, under that call's own deadline and the same child byte quota as deferred child bytes. Display ordering is unchanged, because the display queue already holds the complete chunk.
- **Restoration:** `observation_complete` becomes `true` again once the remainder has been fed completely with a successful observer callback. Until then, drains return `Pending("observation incomplete")` without proof, exactly as today.
- **Parser failure:** a terminal parser failure keeps today's lifetime fence, because no remainder boundary exists to resume from.
- **Effect:** the permanent latch is gone, so a later quiet drain returns `Quiescent`, `advance` runs again and the queue turn resumes; `permission respond` is likewise unblocked.

### Runner: queue housekeeping (D3, D4, D5)

A new coordinator operation `queue_housekeeping` runs from `tick_running` every `QUEUE_HOUSEKEEPING_INTERVAL` (1 s) while the FIFO is non-empty or a flush slot is outstanding, and input is not fenced, so a flush's expiry applies even after the flush emptied the FIFO (Director decision). It runs before the pending-client branch and independently of `retry_deferred`, the drain and the `finalization` / `pending_stamp` retries. It writes no PTY byte and takes no pane lock.

The 1 s cadence bounds consumption-detection latency to about one second while adding one read transaction per second per queued member. The write path keeps its own pre-write recheck, so housekeeping latency never causes a consumed preview to be typed.

Each sweep:

1. Opens **one read transaction**. It loads every queued entry's receipt (`state`, `delivery_method`) and its message status, plus the head's `deferred_target` inputs (run identity, member activity, placement). It also loads the lowest-ID other ordinary `in_progress` receipt, used by the watchdog.
2. Acts per entry, in FIFO order:

   | Durable receipt state | Message state | Action |
   |---|---|---|
   | `ready` | `input_required` | Keep the entry. |
   | `ready` | any other status | Pop the entry and settle it `not_submitted/message_consumed` in its own owned write transaction (zero bytes; no result published, as today). |
   | any other state | any | Fail with `queued input INPUT_ID changed to STATE outside the runner queue`. |

   The failure fences input as a permanent invariant violation: after this change every receipt write for a queued entry is made by the runner itself (§ Receipt ownership).
3. Applies the head's target result as today (`target_inactive`, `target_changed` → pop and settle).
4. Runs the stall watchdog (§ Stall watchdog).

A transient storage failure (`DatabaseBusy`, `DeadlineExceeded`) on the read leaves every entry unchanged and records the head obstacle `storage_unavailable`. A sweep first retries a pending `finalization`; a cancellation that storage prevents becomes that `finalization`, and the sweep settles nothing further until it commits.

Related changes:

- `start_deferred`, `attempt_deferred` and `advance` keep their recheck just before the first byte. The three duplicated consumption checks become one shared function that housekeeping also calls.
- `retry_deferred` records a non-fatal `advance` error as the head obstacle `attempt_failed`, with the message bounded to 256 bytes, so the watchdog reports it. The stderr line stays.
- `retry_deferred` also records the drain result. On `InputDrain::Pending` it stores `transport.drain_blocker()` as the coordinator's `last_drain_blocker`; `Quiescent` clears it.
- **D5:** the receipt `Reconcile` action pops a queued FIFO entry it settles, in the same call, and publishes nothing. It accepts a queued `ready` prompt or exec with disposition `not_submitted` and settles it zero-byte; this is the cancellation path the flush's prompt/exec refusal names.

### Runner: stall watchdog

Each `QueuedInput` carries `head_since: Option<Instant>`, set when the entry becomes the head.

**Deliberate scope decision (Director-confirmed):** the watchdog excludes time an open interaction blocks the head. `interaction_opened` already notifies the Director and carries its own action. An interaction blocking the head therefore resets `head_since` to `None` and clears the existing per-head `stalled` flag; when it closes, `head_since` restarts.

Housekeeping publishes `input_delivery_stalled` once per head when both hold:

- `now - head_since >= COMPOSER_BUSY_BOUND` (3 minutes);
- no attempt has started for that head.

The event keeps its ordered fields and appends `cause` after `delivery`:

```json
{"input_id": 4805, "message_id": 6723, "blocking_input_id": 4804,
 "waiting_since": "2026-11-19T12:04:08Z",
 "delivery": {"status": "queued", "method": null, "reason": "earlier_input"},
 "cause": {"kind": "unresolved_input", "detail": null}}
```

Housekeeping derives the cause at publication time. It checks the rows below in order and selects the first that holds. Rows marked *live* read current runner state or the sweep's storage read; rows marked *recorded* read the obstacle the latest queue turn recorded on the head.

| `cause.kind` | Source | Holds when | `blocking_input_id` | `detail` |
|---|---|---|---|---|
| `forced_flush` | live | A forced flush reservation is outstanding (§ Forced flush in the runner) | The flush receipt | `null` |
| `immediate_attempt` | live | `pending_input_client` or `pending_sweep` is set | Its receipt, when reserved, else `null` | `null` |
| `pending_finalization` | live | `finalization` or `pending_stamp` is set | That receipt | `null` |
| `held_composer` | live | `held_composer` is set | The held blocker | `null` |
| `unresolved_input` | live | Another ordinary receipt of the run is `in_progress` | That receipt (lowest ID) | `null` |
| `drain_pending` | live | `last_drain_blocker` is set | `null` | The drain blocker string, e.g. `terminal reply pending` |
| `pane_input_busy` | recorded | Obstacle `pane_input_busy`, e.g. a hung `member press` holding the lock | `null` | `null` |
| `delivery_budget_exhausted` | recorded | Obstacle `delivery_budget_exhausted` | `null` | `null` |
| `storage_unavailable` | recorded | Obstacle `storage_unavailable` | `null` | `null` |
| `attempt_failed` | recorded | Obstacle `attempt_failed` | `null` | The bounded error message |
| `composer_busy` | recorded | The last queue turn returned `DeliveryChoice::Waiting` | `null` | `null` |
| `queue_turn_not_reached` | — | No row above holds | `null` | `null` |

`queue_turn_not_reached` means no listed gate explains the stall. It is a runner defect to report, and its INSTRUCTION says so.

**Persistence:** `stalled_delivery` validation in `src/broker/runtime/events.rs` changes per `cause.kind`:

| `cause.kind` | `blocking_input_id` rule |
|---|---|
| `held_composer` | Non-null, differs from the head, and satisfies `has_stalled_blocker_provenance` (today's rule) |
| `forced_flush`, `pending_finalization`, `unresolved_input` | Non-null; a receipt of the same run, differing from the head |
| `immediate_attempt` | Null, or a receipt of the same run differing from the head |
| Every other kind | Null |

The outbox `cause_json` reconstruction carries the `cause` object together with `input_id`, `blocking_input_id` and `waiting_since`. An unknown `cause.kind` fails validation.

`director_action`:

| Cause | Action |
|---|---|
| `held_composer` | Today's `review_stalled_input` action, unchanged |
| Every other cause | `{"kind":"review_stalled_queue","instruction":INSTRUCTION,"inspect_argv":["cafleet","input","show","INPUT_ID","--json"],"flush_argv":["cafleet","member","flush","MEMBER_ID","--from-member-id","DIRECTOR_ID"]}` |

Each argv array gets `--database-url URL` appended for a non-default database.

The `review_stalled_queue` INSTRUCTION is exactly: `Capture the member's pane, inspect the waiting input and confirm with member show that no interaction is open. Resolve the named cause when you can: reconcile an in-progress or uncertain blocker after pane evidence. When the queue stays stalled, ask the user to confirm a forced flush, then run flush_argv. A queue_turn_not_reached cause is a runner defect to report.`

### Profiles: suggestion-only composer

- A native suggestion is text the native CLI draws in the composer that is not part of the input buffer. A composer showing only a suggestion is empty for every purpose: deferred delivery, held-composer clearing and flush draft extraction.
- `composer_empty` rules gain an optional `suggestion` object: `{"style": STYLE}`. STYLE is either `{"faint": true}` or `{"foreground": COLOUR}`, with `COLOUR` in the `cell_backgrounds` colour form (indexed or rgb).
  - The prompt marker of a `suggestion` rule is the match of the region's `start_pattern` on the region's first row (Director decision).
  - A rule with `suggestion` matches when its region matches and every cell from the column right after that match up to the row's last non-blank cell carries STYLE.
  - The cursor must sit on that row at the column right after the match. This is a new `CursorRule::AfterPrompt` (`"cursor": "after_prompt"`).
- Step 6 researches which backends render inline suggestions on their supported native versions. Claude Code renders them even though members launch with `--prompt-suggestions false`, as the issue observed. Each backend that renders them gains a fixture-backed rule; one that never does is recorded as such in `SPEC.md` §6.7.
- The Claude Code 2.1.322 / Herdr 0.9.5 suggestion rendering is captured as a native fixture in `assets/profiles/native/claude.json` and registered in `assets/profiles/bundled.json`.
- **Fixture cell styles:** a native screen object gains optional `cell_styles` beside `cell_backgrounds`, so fixtures reproduce the styles a `suggestion` rule reads.
  - Each run is `{"row":N,"start_col":C,"end_col":E,"faint":BOOL,"foreground":COLOUR_OR_NULL}`, one per maximal run of cells in an unmasked row sharing one non-default (`faint`, `foreground`) pair, with `end_col` exclusive. `COLOUR` uses the `cell_backgrounds` colour form.
  - An absent key means every cell has default style, so existing fixtures and their digests are unchanged.
  - The capture-to-fixture conversion derives the runs from the capture grid's per-cell faint and foreground attributes. Fixture replay applies each run's style before writing its row text.
  - The build guard requires each run inside the geometry and outside masked rows, `start_col` below `end_col`, no overlap with another run, and at least one non-default attribute with a valid colour.

### Profiles: composer draft and mode-switch prefixes

Each backend profile gains a `composer_draft` section. Only the runner's forced-flush reservation evaluates it, against the runner's emulated screen, which supplies screen identity, cursor and cell styles.

```json
"composer_draft": {
  "region": {"screen": "either", "start_pattern": "^❯\\u{a0}", "end_pattern": "^─+ *$",
             "max_rows": 48, "cursor": "inside"},
  "draft_rows": {"first": "start", "last": "before_end"},
  "prompt_pattern": "^❯\\u{a0}",
  "mode_switch_prefixes": ["!", "/"],
  "fixture_ids": ["claude-2.1.322-draft-herdr-094-1", "claude-2.1.322-suggestion-herdr-094-1"]
}
```

| Field | Type | Meaning |
|---|---|---|
| `region` | The `composer_empty` `Region` schema | Locates the composer exactly as the backend's `composer_empty` rule does. It must match only the normal-mode composer. |
| `draft_rows` | Object: `first` is `start` or `after_start`; `last` is `end` or `before_end` | Which rows of the matched region hold the draft, as inclusive bounds relative to the region's start and end rows |
| `prompt_pattern` | regex or `null` | Prompt marker stripped from the first draft row; `null` for a composer without one (pi's editor row between `─` rules) |
| `mode_switch_prefixes` | Nonempty array of 1–4-scalar strings | Draft prefixes that switch the native input mode, each verified against the native CLI |
| `fixture_ids` | Nonempty array | Native fixtures proving extraction, including a suggestion-only fixture where the backend renders suggestions and a mode-switched fixture where the mode changes the marker |

- **Region rows:** the region matcher yields the inclusive row span `[S, E]` from the start row `S` to the end row `E`. Each start row's region ends at the first end-pattern row that leaves the draft rows inside it, and the live region is the single one that satisfies the cursor rule and holds no other start-pattern row (Step 6 § Composer-draft regions).
- **Draft rows:** `draft_rows` selects `[S, E]` for `start`/`end`, `[S+1, E]` for `after_start`, and `[S, E-1]` for `before_end`; the two are combined, so pi's framed composer uses `after_start`/`before_end` = `[S+1, E-1]`.

  | Backend | Region | `draft_rows` |
  |---|---|---|
  | claude | Prompt row and continuation rows through the `─` rule below the composer (`max_rows: 48`) | `start` / `before_end` |
  | codex | Prompt row and continuation rows through the first blank row below them (`max_rows: 48`) | `start` / `before_end` |
  | opencode | Prompt rows through the normal-mode footer row; a wrapped draft is unreadable | `start` / `before_end` |
  | hermes | Prompt row through the `─` rule below it (`max_rows: 4`) | `start` / `before_end` |
  | pi | `─` rule, editor rows, `─` rule | `after_start` / `before_end` |

  Every region uses `cursor: inside`. An empty selected span is an empty draft, not an unreadable one.
- **Extraction:** the `prompt_pattern` match is removed from the first draft row. From every row, cells in the profile's suggestion style are removed and trailing blanks trimmed. The rows are joined with `\n`, and leading and trailing blank rows are dropped.
- **Multi-row fixtures:** every backend whose region can span several rows gets a multi-row draft fixture, including a hermes draft that wraps onto a continuation row.
- **Unreadable:** no matching region means the draft is unreadable. That includes a composer whose mode changed the prompt marker, such as a bash-mode marker; such a composer is refused.
- **Mode check:** only the first scalar of the first draft row is compared, because a native mode switch is decided by the buffer's first character. An empty draft never matches.
- **Candidates:** each is kept only when verified against the profile's native version, and the set is extended by any further prefix the research finds.

| Backend | Verified prefixes |
|---|---|
| claude | `!` (bash mode), `/` (slash command) |
| codex | `!` (shell), `/` (slash command) |
| opencode | `!` (shell), `/` (slash command) |
| hermes | `!` (shell), `/` (slash command) |
| pi | `!` (shell), `/` (slash command) |

- **Interface (Director decision):** `enum ComposerDraft { Readable(String), Unreadable }`, `ProfileCatalog::composer_draft(&self, backend: CodingAgent, native_version: &str, screen: &ScreenSnapshot) -> ComposerDraft` and `ProfileCatalog::mode_switch_prefix(&self, backend: CodingAgent, native_version: &str, draft: &str) -> Option<&str>`. The second returns the declared prefix matching the draft's first scalar, or `None` for an empty draft. Step 8's `Reserve` calls both.
- **Expected extraction (Director decision):** each catalog fixture entry listed in a `composer_draft.fixture_ids` records its expected result as `"composer_draft": {"text": "…"}` or `"composer_draft": "unreadable"`.

The catalog validator rejects a profile that:

- lacks `composer_draft`, or its `draft_rows`;
- has an empty `mode_switch_prefixes`, or a prefix outside 1–4 scalars;
- cites a fixture without an expected `composer_draft` result, or one whose extraction or unreadable classification does not reproduce it.

**Ownership (Director decision):** the Programmer implements the production catalog and detector code, the profile JSON, the native fixtures and `SPEC.md`. The Tester implements every `#[cfg(test)]` `test_support` piece: `cell_styles` replay, the `verify_native_screen` build guard, capture conversion and the synthetic test definitions, which also gain `composer_draft`.

### Forced flush: design choice

Two designs meet the user's hard requirement that the flush works while the queue turn is stalled:

- a CLI-side capture with direct CLI receipt writes;
- runner control-socket actions served outside the deferred-queue path, with only the pane write done by the CLI.

This doc uses the second, for three reasons:

1. **Requirement met.** The control-socket path serves every frame in `RunnerLoop::serve` → `serve_client` → `serve_input` → `submit` → `InputCoordinator::receive` before `tick_running` runs. None of `tick_running`, `complete_pending_input`, `retry_deferred`, `quiesce` or `start_deferred` is on it. Run 34 proved this path live: it served `input reconcile` while the queue turn never ran. The new `Flush` operation is dispatched in `serve_input` beside `PermissionInspect`, and unlike `inspect_within_deadline` it calls no `quiesce`, so gate (b) cannot hold it. Gates (a) and (c) live in `tick_running` and `start_deferred` and are never reached.
2. **Receipt ownership stays single-writer.** Every `member_input_operations` write remains an ownership-checked `owned_transaction` by the run owner, and the runner updates its FIFO and `operations` cache in the same call. The CLI never writes a receipt.
3. **The screen model is the real one.** The runner evaluates interactions and the draft on its emulated screen with cursor, screen identity and styles. No input path captures the pane, so `SPEC.md` §6.5 ("No input path captures the pane or consults its state.") and §9 stay true.

### Forced flush in the runner

New control operation `InputControlOperation::Flush(FlushAction)`, served by `InputCoordinator::flush_received(&mut self, conn, action, observation, transport: &Transport, deadline) -> Result<FlushReply, InputFailure>` and answered as `InputControlReply::Flush(FlushReply)` (Director decision, adopting the Tester's skeleton):

```rust
enum FlushAction {
    Reserve { director_member_id: i64, expires_at_mono_ns: u64 },
    Settle { flush_input_id: i64, outcome: FlushWriteOutcome },
}
enum FlushWriteOutcome { Acknowledged, NotStarted, Uncertain }
enum FlushReply {
    Refused { cause: String },
    Empty,
    Reserved { flush_input_id: i64, literal: String, submit_key: String, settle_ms: u64,
               flushed_input_ids: Vec<i64>, message_ids: Vec<i64>, remaining_input_ids: Vec<i64> },
    Settled { flush_input_id: i64 },
    NotOutstanding { flush_input_id: i64, state: InputState, disposition: Option<InputDisposition> },
}
const FLUSH_LITERAL_LIMIT: usize = 65_536;
struct PendingFlush { flush_input_id: i64, entries: Vec<QueuedInput>, expires_at: Instant }
```

The coordinator keeps `flush: Option<PendingFlush>`, and `queue_housekeeping` applies its expiry.

#### `Reserve`

`Reserve` evaluates, in order, and replies `FlushReply::Refused { cause }` on the first failing check (zero bytes, no receipt):

1. Input is not fenced, manual accounting has not failed, and no flush reservation is outstanding.
2. `self.pending` is `None`: no runner attempt holds a written or in-flight composer write.
3. No ordinary receipt of the run is `in_progress`, `accepted` or `uncertain`. The one exception is the `accepted` preview held as `pending_stamp`, which is excluded: its bytes are complete and only its notification stamp awaits storage.
4. No queued `ready` prompt or exec in the FIFO.
5. Observation permits a forced write (§ Gate tolerance): no current interaction or candidate (visibility `Visible` or `Obscured`, or eligibility `Hold(Interaction)` or `Hold(Candidate)`), visibility not `Unknown`, no degraded observation, both terminal ends open. `Visibility` describes the interaction, so an idle `NotVisible` screen proceeds (Director decision).
6. The draft is readable (`composer_draft` on the current snapshot), and its first row does not start with a mode-switch prefix.
7. The profile has a prompt composer plan for the recorded native version.
8. The FIFO holds at least one preview whose message is `input_required`. Consumed ones are left for housekeeping. With none, reply `FlushReply::Empty`.

#### Gate tolerance

The flush is meant for a queue turn held by a gate that does not make a forced write unsafe. The table states, for each D4 gate and each drain blocker, whether `Reserve` proceeds and which `ObservationStatus` / `ObservationHealth` / transport field it reads. `src/runtime/input.rs` covers every row through `flush_received`, and `src/runtime/service.rs` proves the dispatch path with four deterministic cases.

| Gate | Field read by `Reserve` | Reserve | Reason |
|---|---|---|---|
| (a) immediate client attempt or preview sweep in flight | `self.pending` | Refuses: `a runner input attempt is in flight; retry` | The attempt may be writing; it ends at its own deadline, so the gate is transient |
| (b) `terminal input interrupted` | — (per-read `EINTR`) | Proceeds | Transient read interruption; the screen model is current |
| (b) `terminal reply pending` | — (reply prefix, expires after `REPLY_PREFIX_BOUND`) | Proceeds | The pending prefix resolves within 100 ms, and the flush bytes are human input either way |
| (b) `terminal output still arriving` | — | Proceeds | Child output is fed into the screen model as it arrives |
| (b) `observation incomplete` | `observation_complete` | Proceeds (user decision, last-resort override) | The screen may be stale while the flag is unset, and the user accepts that risk; the open-interaction, mode-prefix and unreadable-draft refusals are still evaluated on the current snapshot |
| (b) `observation transitions pending` | `ObservationHealth.pending_count` | Proceeds | Event persistence lags; screen classification does not |
| (b) `observation evidence pending` | `ObservationHealth.pending_evidence` | Proceeds | Evidence persistence lags; screen classification does not |
| (b) `observation degraded: CODE` | `ObservationHealth.degraded`, `ObservationStatus.diagnostic` | Refuses: `the member's screen observation is unavailable: observation degraded: CODE` | Interactions and the draft cannot be classified reliably |
| (b) `outer terminal closed` | transport `outer_eof` | Refuses: `the member's screen observation is unavailable: outer terminal closed` | The outer-pane write would reach no runner input |
| (b) `native output closed` | transport `child_eof` | Refuses: `the member's screen observation is unavailable: native output closed` | The child has exited |
| — interaction or candidate on screen | `ObservationStatus.visibility` = `Visible` or `Obscured`, `ObservationStatus.eligibility` = `Hold(Interaction)` or `Hold(Candidate)`, or a recorded `current_interaction_id` | Refuses: `an interaction is open (CLASSIFICATION)` | User request: refuse an open interaction |
| — visibility unknown | `ObservationStatus.visibility` = `Unknown` | Refuses: `the member's screen observation is unavailable: VISIBILITY` | The draft and interactions cannot be read; an idle `NotVisible` screen proceeds |
| (c) `input_fenced` | `input_fenced` | Refuses: `the member's input is fenced` | The runner is stopping or failed permanently |
| (c) `finalization` retry | `finalization` | Proceeds | A zero-byte settlement of an entry already out of the FIFO |
| (c) `pending_stamp` retry | `pending_stamp` | Proceeds (check 3 exception) | A submitted preview awaiting its stamp |
| D3 unresolved `in_progress` blocker | Storage | Refuses: `input INPUT_IDS is unresolved; …` | The blocker may still be writing; the Director reconciles it first |
| Held composer | `held_composer` | Proceeds | The flush appends after whatever the composer holds |

It then assembles the literal (§ Submission literal, § Plain limit). In one owned `IMMEDIATE` transaction it:

- inserts the flush receipt `in_progress` (§ Data model);
- moves each selected preview `ready → in_progress` with `delivery_method=forced` and `flush_input_id` set;
- revalidates every selected message as `input_required`.

The selected entries move from `queue` into a new coordinator slot `flush: Option<PendingFlush { flush_input_id, entries, expires_at }>`, preserving their order. While the slot is set, `start_deferred` returns without starting an attempt, and the preview sweep treats the run as owning preview delivery. The reply is `FlushReply::Reserved { flush_input_id, literal, submit_key, settle_ms, flushed_input_ids, message_ids, remaining_input_ids }`.

- `submit_key` is the plan's `submit_hex` as a `PressKeys` name; `0d` → `Enter`.
- `settle_ms` is fixed at `STAGE_SETTLE` (500 ms), the literal-to-submit interval every runner submission plan uses. No plan field declares a settle value. The measured post-submit windows in some fixtures, such as Codex `post-1900ms`, are verification windows after the submit key, and the flush performs no post-submit verification.

#### `Settle`

`Settle` settles the slot in one owned transaction, publishes the events and clears the slot:

| Outcome | Flush receipt | Each flushed preview | Events |
|---|---|---|---|
| `Acknowledged` | `settled/submitted`, `evidence_ref="director=DIRECTOR_ID; multiplexer acknowledged forced flush"` | `settled/submitted` with the notification stamp `settle_preview` applies, `evidence_ref="forced flush input FLUSH_INPUT_ID"` | One `input_delivery_result` per preview, stage `submitted` |
| `NotStarted` | `settled/not_submitted`, `evidence_ref="director=DIRECTOR_ID; no multiplexer process started"` | Back to `ready`, `delivery_method` and `flush_input_id` cleared; entries return to the FIFO front in their original order | None |
| `Uncertain` | `uncertain`, `stop_cause=write_failed` | `uncertain`, `stop_cause=write_failed` | One `input_delivery_result` per preview, stage `unknown`, `cause={"kind":"write_failed"}` |

**Expiry:** when `expires_at` passes with no `Settle`, housekeeping applies `Uncertain`. The CLI may have died after a partial write, and `DeferTermination` does not cover SIGKILL. Recovery for an uncertain flush is `cafleet input reconcile FLUSH_INPUT_ID`.

**Late or mismatched `Settle`:** a `Settle` whose `flush_input_id` is not the outstanding slot writes nothing. The runner replies `FlushReply::NotOutstanding { flush_input_id, state, disposition }`, reading the receipt's durable state; an unknown ID or one belonging to another run fails as a receipt-identity error. The CLI then exits 1:

| Durable flush receipt | CLI error (exit 1) |
|---|---|
| `uncertain` (expiry applied first) | `Error: queued input flush to member ID is uncertain: the flush reservation expired before settlement (input_id=FLUSH_INPUT_ID). Inspect the pane, then reconcile input FLUSH_INPUT_ID.` |
| `settled` | `Error: queued input flush to member ID was already settled DISPOSITION before the flush reported its write outcome (input_id=FLUSH_INPUT_ID).` |

**Reconcile during a reservation:** the receipt `Reconcile` action refuses a flush receipt whose slot is still outstanding with `flush FLUSH_INPUT_ID is still outstanding`, so a late `Settle` can only meet a receipt that expiry or a later reconcile settled.

**Run end:** `fence()` settles an outstanding slot as `Uncertain` with `stop_cause=runner_stopped`. Recovery under reconciler ownership leaves `in_progress` forced rows and the flush receipt `uncertain/runner_lost` with the same cascade.

**Watchdog:** entries in the flush slot are not in the FIFO and get no watchdog. The queue head behind an outstanding slot reports `forced_flush`.

### CLI: `cafleet member flush`

```
cafleet member flush MEMBER_ID --from-member-id DIRECTOR_ID [--json]
```

`member flush` joins the Director-only command guard, and its implementation lives in `src/runtime/flush.rs`, mirroring `press.rs`: `FlushOutcome` (with `to_json` and `text`), a `FlushChannel` trait whose `exchange` carries the control frames, `flush_member_with(conn, settings, member, deadline, mux, channel, sleep)` and the production `flush_member`, whose channel is `crate::runtime::flush::ControlFlushChannel` implementing `FlushChannel` over the run's control socket. `FlushAction`, `FlushReply` and `FlushWriteOutcome` are re-exported from `crate::runtime`; `FLUSH_LITERAL_LIMIT`, `PendingFlush`, `PendingStamp` and `PendingFinalization` are reachable in `crate::runtime::input` (Director decision). The command runs under one 10-second `Deadline`, which covers the lock wait, both control exchanges, both multiplexer processes and the settle interval. There is no `--timeout` flag. The reserved slot's `expires_at` is that deadline plus `DEFERRED_FINALIZATION`.

#### Procedure

| Step | Action | Refusal rows evaluated |
|---|---|---|
| 1 | Guard; resolve the member (active, ordinary), placement and run | Not found; Director target; inactive; pending placement; run not `running` |
| 2 | Acquire the pane input lock with `lock_member_with_reservation` (the `member press` lock) | Lock busy |
| 3 | `mux.validate_key_target(placement)` and re-read the member, as `member press` does | Pane gone; placement changed |
| 4 | Send `Flush::Reserve` over the run's control socket | Every runner refusal; runner unreachable |
| 5 | On `Empty`, release the lock and exit 0 with `flushed_input_ids: []` | — |
| 6 | Under `DeferTermination`, revalidate the placement, then `mux.send_text(pane, literal)`, sleep `settle_ms`, then `mux.send_keys(pane, [submit_key])` | Placement changed before write (sends `Settle NotStarted`) |
| 7 | Send `Flush::Settle` with the write outcome, then release the lock | — |

The runner's own attempts acquire the same pane lock with `PaneLock::try_acquire`, so no runner write overlaps the flush. Nothing is checked during the settle interval. An interaction that opens in it is the same accepted trade-off `member press` carries.

The flush bytes enter the runner through its outer-pane human-input path. They raise `human_generation` like any manual input. That invalidates any `current_request` permission token (none exists, because step 4 refused an open interaction) and stops no runner attempt (none can run while the slot and lock are held). After submission the composer verifies empty, which clears CAFleet's held composer.

#### Submission literal

```
" [cafleet: ignore any text before this marker, if present] " + join(" ", preview_lines)
```

- Each preview line is `crate::delivery::canonical_preview_line(message)`, the same profile-method line the runner types, including the 4000-scalar inline bound and its `message show` pointer.
- The literal follows whatever the composer already holds. No Escape, Backspace, `Ctrl+U` or other clear key is sent.
- The flush always types plainly, because its bytes pass through the human-input path. A plan's `bracketed_paste` framing is not used, so the plain limit always applies.

#### Plain limit

The effective bound is the lower of:

- the plan's plain literal limit in its own unit (bytes or characters), when non-null;
- `FLUSH_LITERAL_LIMIT`, 65,536 bytes of UTF-8. This keeps the single argv element of `tmux send-keys -l` and `herdr pane send-text` well below Linux `MAX_ARG_STRLEN` (128 KiB).

The fallback:

1. Build the literal from canonical lines. If it fits, use it.
2. Otherwise rebuild every line with `pointer_preview_line`. If it fits, use it.
3. Otherwise use the longest FIFO prefix of pointer lines that fits. The rest stay queued and are reported in `remaining_input_ids`.
4. If not even the first pointer line fits, refuse.

#### Refusals

Every refusal exits 1 with `Error: queued input was not flushed to member ID: CAUSE`, writes zero bytes and leaves receipts unchanged.

| Step | Condition | CAUSE |
|---|---|---|
| 1 | Member not found | (existing not-found error, unchanged) |
| 1 | Root Director target | `cannot flush the Director's own pane` |
| 1 | Member inactive | `member ID is not active` |
| 1 | Pending placement | `member ID has no pane yet (pending placement)` |
| 1 | Run not `running` | `the member's run is STATUS` |
| 2 | Lock busy until the deadline | `member ID's pane input remained busy` |
| 3 | Target check fails | `press target context does not match the recorded placement` |
| 3, 6 | Placement changed | `member ID placement changed before flush` |
| 4 | Runner control socket unreachable | `the member's runner is unreachable: CAUSE` |
| 4 | Ordinary input fenced or manual accounting failed | `the member's input is fenced` |
| 4 | Earlier flush outstanding or runner attempt in flight | `a runner input attempt is in flight; retry` |
| 4 | Unresolved receipt | `input INPUT_IDS is unresolved; reconcile it with cafleet input reconcile before flushing` |
| 4 | Queued prompt or exec | `queued KIND input INPUT_ID cannot be rebuilt from its receipt; settle it with 'cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition not-submitted --evidence-ref REFERENCE', then resend it` |
| 4 | Interaction or candidate shown | `an interaction is open (CLASSIFICATION)` |
| 4 | Observation not visible or degraded | `the member's screen observation is unavailable: REASON` |
| 4 | Draft unreadable | `the composer draft cannot be read from the screen` |
| 4 | Draft starts with a mode prefix | `the composer draft starts with mode prefix 'PREFIX'` |
| 4 | No composer plan | `the BACKEND profile has no submission plan for native version VERSION` |
| 4 | First pointer line above the bound | `the first queued preview is N UNIT as a pointer line, above the flush limit of MAX UNIT` |
| 4 | A selected receipt changed during reservation | `queued input changed while the flush was prepared; retry` |

Receipt refusals list every matching input ID, ascending and comma-separated. With several queued prompt/exec inputs, the error names the lowest one and lists the rest after `; also queued: IDS`.

Write failures, after `Settle`:

| Boundary | Error (exit 1) |
|---|---|
| Before any process started | `Error: queued input was not flushed to member ID: CAUSE` |
| After a process started, or an unreadable acknowledgment | `Error: queued input flush to member ID is uncertain: CAUSE (input_id=FLUSH_INPUT_ID). Inspect the pane, then reconcile input FLUSH_INPUT_ID.` |

#### Output

JSON fields, in order: `member_id,pane_id,flush_input_id,flushed_input_ids,message_ids,remaining_input_ids,delivery`. `delivery` is `{"status":"submitted","method":"forced","reason":null}`. Both `flush_input_id` and `delivery` are `null` when nothing was queued.

Text output is one of:

- `Flushed N queued previews to member ID (input FLUSH_INPUT_ID): messages M1,M2`, followed by `Still queued: IDS` when `remaining_input_ids` is nonempty;
- `No queued previews for member ID` when nothing was queued.

### Multiplexer: `send_text`

`Multiplexer::send_text(pane_id, text, deadline)` writes `text` literally:

- tmux: `send-keys -t PANE -l TEXT`;
- herdr: `herdr pane send-text PANE TEXT`, with the subcommand and argument form verified against Herdr 0.9.5 help in Step 7.

Acknowledgment and error mapping match `send_keys`: an empty acknowledgment succeeds, and a process that starts and fails is `after_write`.

### Data model

`assets/migrations/V1__baseline.sql`, `member_input_operations`, edited in place:

| Change | New definition |
|---|---|
| `kind` CHECK | `kind IN ('preview','prompt','exec','permission_response','flush')` |
| `delivery_method` CHECK | `delivery_method IS NULL OR delivery_method IN ('profile','fallback','forced')` |
| New column | `flush_input_id INTEGER REFERENCES member_input_operations(input_id) ON DELETE RESTRICT` |
| New CHECK | `flush_input_id IS NULL OR (kind = 'preview' AND delivery_method = 'forced')` |
| New CHECK | `kind != 'flush' OR (message_id IS NULL AND delivery_method = 'forced' AND deferred_at IS NULL)` |
| New CHECK | `delivery_method IS NOT 'forced' OR kind IN ('preview','flush')` |
| `stop_cause` CHECK | Adds `runner_lost` for the recovery cascade of an outstanding flush |
| New index | `CREATE INDEX member_input_operations_flush ON member_input_operations(flush_input_id) WHERE flush_input_id IS NOT NULL` |

- **Flush receipt identity:** `operation_key` is `flush:<run_id>:<ordinal>`, where `ordinal` is one more than the run's existing flush receipts. `payload_digest` is the SHA-256 of the literal followed by the submit key's bytes. The literal is never stored.
- **FIFO barriers:** `flush` receipts are excluded. `unresolved_ordinary` and preview latest-attempt selection already filter `kind IN ('preview','prompt','exec')`. `has_unresolved_ordinary_attempt` (`src/broker/runtime/inputs.rs`) changes from `kind<>'permission_response'` to `kind IN ('preview','prompt','exec')`. A flush still blocks through its `in_progress` preview rows while it writes.
- **Capacity:** `require_capacity` keeps counting `flush` receipts through the kind-agnostic `UNRESOLVED_INPUT` fragment, deliberately, because an unresolved flush is outstanding run input. Step 7 audits every other kind-agnostic or `kind<>'permission_response'` query and records each decision in `SPEC.md` §6.9.
- **`unresolved_input_ids`** includes `flush` receipts in `in_progress` or `uncertain`.

### Receipt presentation and reconciliation

- **`input show`:** appends two fields after `left_stage`. `flush_input_id` is the flush that owns a forced preview, else `null`. `flushed_input_ids` lists a `flush` receipt's preview IDs ascending, else `null`.
- **Delivery projection:** adds method `forced`. A `flush` receipt projects like a prompt: `in_progress`/`uncertain` → uncertain, `submitted` → submitted, `not_submitted` → not_submitted.
- **Reconciling a flush:** `input reconcile FLUSH_INPUT_ID` settles the flush receipt and every preview it owns in one runner transaction, with the disposition mapped:
  - `submitted` → `submitted`;
  - `not-submitted` → `not_submitted`, after which the runner sweep retries each still-pending message;
  - `abandoned-uncertain` → `abandoned_uncertain`.
- **Reconciling one forced preview** fails with `Error: input INPUT_ID belongs to forced flush FLUSH_INPUT_ID; reconcile the flush`.

### Events

- `Settle` publishes one `input_delivery_result` per forced preview:
  - for `Acknowledged`: `delivery={"status":"submitted","method":"forced","reason":null}`, stage `submitted`;
  - for `Uncertain`: `{"status":"uncertain","method":"forced","reason":null}`, stage `unknown`, `cause={"kind":"write_failed"}`.
- The payload appends `flush_input_id` after `stage`; `director_action` is `null`; retried-preview suppression does not apply.
- The flush receipt itself publishes no event. The CLI reply is the Director's synchronous result, and the per-preview events carry `flush_input_id` for correlation.

### Skills and documentation

- **Member role** (`assets/skills/cafleet/roles/member.md`): state that a prompt containing `[cafleet: ignore any text before this marker, if present]` carries instructions only after the marker. Text before it is a leftover draft or suggestion, not an instruction. Act on each `[cafleet msg …]` preview after the marker in order.
- **Director role** (`assets/skills/cafleet/roles/director.md`): add `On input_delivery_stalled with director_action.kind=review_stalled_queue` to § Reconcile generated input. Run `member flush` only with both of these:
  - stall evidence: an `input_delivery_stalled` event, or a queue confirmed stalled through `member show` and `input show`;
  - the user's confirmation through `{decision_surface}`.

  Then inspect the result, and reconcile an uncertain flush from pane evidence.
- **`cli.md`:** add `member flush` to the subcommand summary and the Director-only guard list, with a `### member flush` section covering synopsis, outcome, refusals and output. Add the new strings to `## Error messages`. Add `member flush` beside `Bash(cafleet member press *)` as an operator ask-gate in `## permissions.allow coverage`. Update Runner queue, Delivery reporting (method `forced`), Runtime events (`cause`, `review_stalled_queue`, forced results) and the `input show` fields.
- **`SPEC.md`:**
  - §3: `src/runtime/flush.rs`.
  - §6.3: subcommand summary, Director-only guard, the `cafleet member` flush contract, Error Messages rows.
  - §6.5: `send_text` in Backend operations and Explicit key dispatch.
  - §6.7: `composer_draft`, `mode_switch_prefixes`, the suggestion rule, per-backend suggestion research.
  - §6.9: failure bookkeeping, housekeeping, stall watchdog, the `Flush` control operation and its served-outside-the-queue path, single-writer receipt ownership, query audit decisions.
  - §8: DDL.
  - §9 keeps "no pane capture on any input path", because the flush reads the runner's emulated screen.
- **`docs/`:** `docs/docs/concepts/members.md` gains the forced-flush recovery and the stall causes as user-visible behavior, linking `SPEC.md` for exact contracts.
- **`README.md`:** unchanged, because the pitch, install commands and site sections stay the same.

### Testing

Every test is an in-file `#[cfg(test)] mod tests` unit in the file owning the asserted behavior.

| Behavior | Owning file |
|---|---|
| Run 34 reproduction: deadline-expired immediate write, queued head, reconciled blocker, consumed message, the head held on the Step 1 gate → head cancelled `message_consumed`, driven on an in-process `Host` through the production entry points in `tick_running` order | `src/runtime/input.rs` |
| Written immediate attempt failing in `advance` or in `complete_pending_input`'s drain records `uncertain/write_failed` | `src/runtime/input.rs` |
| A deadline or observer error mid-feed retains the unfed remainder, the next drain or tick feeds it, `observation_complete` is restored and no byte is dropped; a parser failure keeps the fence | `src/runtime/transport.rs` |
| Housekeeping cancels a consumed head and a consumed non-head entry while (a) an immediate attempt is in flight, (b) the drain stays `Pending`, (c) a `finalization` retry is outstanding | `src/runtime/service.rs` |
| A non-`ready` queued receipt fails housekeeping loudly | `src/runtime/input.rs` |
| `Reconcile` pops a settled queued entry; `ready` prompt/exec cancellation | `src/runtime/input.rs` |
| Watchdog: one event per head for every cause row, interaction reset, live versus recorded derivation, `queue_turn_not_reached`, action shapes | `src/runtime/input.rs` |
| `stalled_delivery` validation per `cause.kind`; outbox carries `cause` | `src/broker/runtime/events.rs` |
| Suggestion-only composer empty; typed draft non-empty | `src/coding_agent/detector.rs` |
| `composer_draft` validation, single-row and framed extraction, mode-switched marker unreadable, prefix match; `cell_styles` build-guard validation and replay | `src/coding_agent/catalog.rs` |
| `send_text` argv and acknowledgment mapping | `src/multiplexer/tmux.rs`, `src/multiplexer/herdr.rs` |
| `Flush::Reserve` refusals in order, selection in in-memory FIFO order including a head requeued by `push_front` with a higher receipt ID, plain-limit fallbacks and `FLUSH_LITERAL_LIMIT`, `Settle` per outcome with events, expiry → uncertain, late or mismatched `Settle` → `NotOutstanding` with no write, `Reconcile` refused while a slot is outstanding, `fence()` cascade | `src/runtime/input.rs` |
| `Flush` served on the control-socket dispatch path while the queue turn is held, with four deterministic cases (Director decision) | `src/runtime/service.rs` |
| Every § Gate tolerance row through `flush_received`, asserting proceed or the row's refusal cause | `src/runtime/input.rs` |
| CLI exit and error for each `NotOutstanding` state | `src/runtime/flush.rs` |
| CLI procedure: step order, refusal mapping, settle sleep, placement revalidation, write-failure errors, output shapes, with a fake multiplexer and runner | `src/runtime/flush.rs` |
| Flush reconcile cascade and single-preview refusal | `src/runtime/input.rs` |
| Schema CHECKs; `has_unresolved_ordinary_attempt` ignores `flush` | `src/broker/runtime/inputs.rs` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Investigate the root cause

This step commits no code. A scratch test may be used locally and discarded; the committed reproduction comes in Step 3.

- [x] Model the run 34 sequence against the pre-change runner, and determine which D4 gate holds the head. Confirm or rule out the drain-`Pending` hypothesis, including the CPU observation <!-- completed: 2026-11-19T13:41 -->
- [x] If the modeled sequence does not hold the head, inspect run 34 read-only (`cafleet input show`, `cafleet member show 41 --json`, `cafleet member capture 41`) and model the drain blocker or storage retry its evidence implies, until a sequence holds the head; not needed: the modeled sequence held the head <!-- completed: 2026-11-19T13:42 -->
- [x] Record the gate in Background § Root cause with a Changelog row, and adjust § Specification wherever the finding changes it <!-- completed: 2026-11-19T13:41 -->

### Step 2: Documentation

- [x] Update `SPEC.md` §3, §6.3, §6.5, §6.7, §6.9 and §8 per § Skills and documentation <!-- completed: 2026-11-19T13:41 -->
- [x] Update `docs/docs/concepts/members.md` and any other `docs/` page that describes stalls or queued delivery <!-- completed: 2026-11-19T13:41 -->
- [x] Update `assets/skills/cafleet/reference/cli.md` (summary, guard list, `member flush`, Error messages, `permissions.allow` coverage, Runner queue, Delivery reporting, Runtime events, `input show`) <!-- completed: 2026-11-19T13:41 -->
- [x] Update `assets/skills/cafleet/roles/member.md` with the marker rule <!-- completed: 2026-11-19T13:41 -->
- [x] Update `assets/skills/cafleet/roles/director.md` with the `review_stalled_queue` procedure and the forced-flush preconditions <!-- completed: 2026-11-19T13:41 -->

### Step 3: Reproduction and failure bookkeeping

- [x] Commit the run 34 reproduction test in `src/runtime/input.rs` (in-process `Host`, production entry points in `tick_running` order) on the Step 1 gate, asserting the post-fix outcome; confirm it fails on the pre-change code <!-- completed: 2026-11-19T14:06 -->
- [x] Add the ordinary-attempt `uncertain/write_failed` branch to `InputCoordinator::advance`'s error mapping, with the fallback bookkeeping deadline <!-- completed: 2026-11-19T13:55 -->
- [x] Route `complete_pending_input` `quiesce` failures through the coordinator bookkeeping <!-- completed: 2026-11-19T13:55 -->
- [x] Add the D1/D2 unit tests in `src/runtime/input.rs` <!-- completed: 2026-11-19T14:06 -->

### Step 4: Queue housekeeping

- [x] Extract the shared consumption check used by housekeeping, `start_deferred`, `attempt_deferred` and `advance` <!-- completed: 2026-11-19T14:04 -->
- [x] Add `queue_housekeeping` (one read transaction per sweep, 1 s cadence) and call it from `tick_running` before the pending-client branch <!-- completed: 2026-11-19T14:04 -->
- [x] Record `attempt_failed` and `last_drain_blocker` in `retry_deferred` <!-- completed: 2026-11-19T14:04 -->
- [x] Make the receipt `Reconcile` action pop settled queued entries and accept `ready` prompt/exec cancellation <!-- completed: 2026-11-19T14:04 -->
- [x] Repair the `observation incomplete` latch per § Runner: interrupted drain feed so a healthy queue turn resumes on it <!-- completed: 2026-11-19T13:55 -->
- [x] Add the housekeeping and reconcile tests; confirm the reproduction test passes <!-- completed: 2026-11-19T14:06 -->

### Step 5: Stall watchdog

- [x] Add `head_since`, obstacle tracking and the interaction reset to `QueuedInput` <!-- completed: 2026-11-19T14:22 -->
- [x] Derive and publish `input_delivery_stalled` with `cause` from housekeeping, replacing the held-composer-only publication <!-- completed: 2026-11-19T14:22 -->
- [x] Change `stalled_delivery` validation per `cause.kind` and carry `cause` through the outbox `cause_json` in `src/broker/runtime/events.rs` <!-- completed: 2026-11-19T14:22 -->
- [x] Add the `review_stalled_queue` action and INSTRUCTION <!-- completed: 2026-11-19T14:22 -->
- [x] Add the watchdog and event validation tests <!-- completed: 2026-11-19T23:40 -->

### Step 6: Profiles

- [x] Research inline-suggestion rendering on each backend's supported native version, and record the result in `SPEC.md` §6.7 <!-- completed: 2026-11-19T23:40 -->
- [x] Add `CursorRule::AfterPrompt` and the `suggestion` style field to `composer_empty` rules in the catalog and detector, and the optional fixture `cell_styles` runs to capture conversion, replay and the build guard <!-- completed: 2026-11-19T23:40 -->
- [x] Verify each candidate mode-switch prefix and each mode's marker behavior against each backend's native CLI <!-- completed: 2026-11-19T23:40 -->
- [x] Capture the suggestion, draft and mode-switched fixtures for every backend, including Claude Code 2.1.322 / Herdr 0.9.5; register them in `assets/profiles/bundled.json` and add `composer_draft` to every profile <!-- completed: 2026-11-19T23:40 -->
- [x] Add catalog validation and detector/extraction tests <!-- completed: 2026-11-19T23:40 -->

**Strict idle rules (user decision):** the claude, codex and opencode profiles split at the installed native versions: new profiles from claude 2.1.322, codex 0.167.0 and opencode 2.2.7 carry strict idle rules (a blank remainder after the prompt marker; the suggestion rule carries the style check). Every fixture the build guard requires for each new interval, including detection, annotation and permission-answer evidence, is re-captured with `cell_styles` through the authorized research setup, and the Claude 2.1.322 queued-message hint style is verified during re-capture. The older claude, codex and opencode profiles, and the hermes and pi profiles (Hermes cannot complete a turn here), keep their current idle rules; `SPEC.md` §6.7 records that the cursor-at-marker draft limit still applies to them. The Tester re-bases the affected tests on the new boundaries and fixture IDs. The codex profile splits at 0.167.0 (user decision, after Codex 0.167.0 was reinstalled): the new profile's evidence is captured on 0.167.0, and the older profile `[0.163.0, 0.167.0)`, whose composer evidence was all on 0.167.0 fixtures, gets its composer, detection and answer evidence re-captured on the installed 0.166.0. Every research native session is stopped before keys are sent to its pane. Capture results: every carried detection rule with an answer plan matched its re-captured form on the new version (claude 2.1.322 ten forms plus the auto-default nudge and the env-setup wizard; codex 0.166.0 and 0.167.0 approval form; opencode 2.2.7 at 120, 57, 55 and 36 columns, including the reject-reason continuations). Not re-verified on the new versions, and carried unchanged: the claude `claude-question-pending-notice` detection rule and the claude and codex auto-review refusal annotation rules (this host's auto reviewer approved the `.invalid` setup command that drew a refusal on the earlier versions), so the annotation capabilities cite the auto-mode indicator screens. Forms were captured under private Claude configurations and with `--config approvals_reviewer=user` for codex, because the shared settings suppress the forms.

**Profile-split research access (user and Director decisions):** Hermes stays unchanged, because its continuation fixtures need a completed turn. For the Claude dialog states the user authorizes one private `CLAUDE_CONFIG_DIR` under `/tmp` holding a copy of `~/.claude/.credentials.json` (mode 0600) and the seeded global state; the shared `~/.claude.json` is never written for these dialogs, and the whole private directory, credential included, is deleted as soon as the Claude captures end. The Director authorizes branch-binary members on a private database under `/tmp` (never the default database) in the research tmux server and the `cafleet-programmer-research` herdr session, for the composer evidence. Permission and dialog forms are captured with the approved `script(1)` method, one model turn per form.

**Native research access (user decision):** Step 6 research uses the shared HOME configuration with minimal writes (trusting only a `/tmp` research folder) and copies no credentials. Model turns are authorized on each backend's cheapest model with one-word prompts and a few turns per backend. The Programmer runs the native capture driver with a private case, and herdr runs in a separate `cafleet-programmer-research` session; fleet 2's herdr session and panes are never touched.

**Step 6 research results and decisions:** Research notes for this step stay under `/tmp/cafleet-programmer-research/`.

| Backend (version) | Empty-composer hint | Verified mode prefixes |
|---|---|---|
| claude (2.1.322; tmux and Herdr 0.9.5) | Startup hint `Try "…"`, faint, cursor right after `❯\u{a0}`; no post-turn suggestion in 4 turns, with and without `--prompt-suggestions false` | `!` (consumed; marker becomes `!`), `/`; `#` is plain text and is dropped |
| codex (0.167.0) | `Ask Codex to do anything`, faint, at startup and after every turn | `!` (marker becomes a red `!`), `/` |
| opencode (2.2.7) | `Ask anything, / for commands, @ for context…`, foreground rgb(128,128,128), at startup only | `!` (same `┃` glyph; footer becomes `Shell · esc normal`), `/` |
| hermes (2026.9.24) | Rotating placeholder, italic with indexed foreground 136, at startup and after the failed turn | `!` (added; hint `! shell`), `/` |
| pi (1.5.2) | None | `!` (border recolours only), `/` |

- (Director) The Claude Code 2.1.322 / Herdr 0.9.5 startup-hint screen is the Claude suggestion-only fixture, and `SPEC.md` §6.7 records that the post-turn suggestion was not reproduced in 4 runs.
- (Director) STYLE takes the full colour form: `{"foreground": COLOUR}` with `COLOUR` in the `cell_backgrounds` form (indexed or rgb), beside `{"faint": true}`.
- (User) OpenCode shell mode is fixed in this change. The OpenCode `composer_draft` region spans the composer row through the footer row and ends on the normal-mode footer, with `draft_rows` `start`/`before_end`. The `composer_empty` rule gets the same footer guard, so a shell-mode composer never counts as empty. Fixtures cover both.
- (User) Hermes stays at what is researchable: the placeholder that returns after the failed turn is captured, and post-turn rendering is recorded as not researched because the configured provider has no token.

**Fixture capture method (Director decision):** the native capture driver runs only its permission-probe case registry, so the Step 6 composer fixtures are captured outside it. Each native CLI runs under `script -q -f -O RAW -c CMD` at the pane geometry, in the research tmux server or the `cafleet-programmer-research` herdr session, and is driven with send-keys. The byte offset of each target state is recorded, and that stream prefix is converted by a scratch converter under `/tmp` built on the runner's parser (`vt100` 0.18.1). The converter emits the screen object with `cell_backgrounds` and `cell_styles` and computes the registry digest in the build guard's canonical form. It stays out of the repository; the build guard and the Tester's replay tests validate every fixture.

**Composer-draft regions (Director decisions):** each `composer_draft` region uses `cursor: inside`. Claude reads its prompt row and continuation rows through the `─` rule below the composer, and codex its prompt row and continuation rows through the first blank row below them, both `start`/`before_end` with `max_rows` 48, so a leftover preview that wraps onto several rows reads as Readable; the mode check uses the first row. Each region ends at the first end-pattern row that leaves its draft rows inside it, so a history row's region stops before the composer; this keeps a shell-mode codex composer, whose `!` marker opens no region, Unreadable instead of reading a history row's region. Hermes reads `^❯ ` through the `─` rule below it (`start`/`before_end`, `max_rows` 4), because a hermes continuation row does not repeat `❯`; pi reads between its `─` rules. The live region is the single matching region with no other start-pattern row inside it, which drops a region opening on a history row such as codex `› …` or hermes `❯ hi`. A live region whose row above also matches the start pattern continues a composer that repeats its marker on every row and is Unreadable: OpenCode draws `┃ ` at the start of every composer row, so a wrapped OpenCode draft is Unreadable and the flush refuses it with zero bytes, matching the rule to refuse an unreadable draft. Wrapped-draft fixtures, including one with a leftover `[cafleet msg …]` line, record each result.

**SC11 one-row refusal (investigation):** not reproduced. On Claude Code 2.1.322 / Herdr 0.9.5, Ctrl+U deletes one visual row at a time, and the resulting one-row draft is a clean single composer row with the cursor on it. Herdr 0.9.5 gives a pane one more column while its child is on the alternate screen than on the primary screen (a shell reports 93 columns where Claude draws 94; in one zoomed pane OpenCode draws 91 and Codex 92), so a member pane's width changes whenever its child switches screens. Claude answers each resize with a full repaint (`ESC[2J`), and replaying captured streams through the runner's parser with the resize at the measured offset stays consistent with herdr's own view. The cause of the live refusal stays unconfirmed without the live runner's emulated screen.

**Native fixture compaction (Director decision):** the first Step 6 conversion split a run at every wide character, because the parser gives a wide character's continuation cell default attributes, and it kept `cell_styles` runs on trailing blanks; the five fixture files grew from 2.6 MB on `main` to 12.8 MB. Every run now spans continuation cells with their leading cell, and no `cell_styles` run lies wholly after its row's last non-blank cell (runs 47,787 → 3,782), and the two uncited captures `claude-2.1.322-env-not-now-post-stage-tmux-120-1` and `claude-2.1.322-env-dont-show-post-stage-tmux-120-1` are dropped. The files total 3.8 MB in the existing pretty-JSON convention. `SPEC.md` §6.9 states both run rules.

### Step 7: Schema, receipts and multiplexer

- [x] Edit `assets/migrations/V1__baseline.sql` per § Data model and update the setup/guard required column shapes <!-- completed: 2026-11-19T15:00 -->
- [x] Extend `InputKind`, `DeliveryMethod`, `StopCause` and the receipt record with `flush`, `forced`, `runner_lost` and `flush_input_id`; update `input show` and the delivery projection <!-- completed: 2026-11-19T15:00 -->
- [x] Change `has_unresolved_ordinary_attempt` to `kind IN ('preview','prompt','exec')`, audit the other kind-agnostic queries, and record each decision in `SPEC.md` §6.9 <!-- completed: 2026-11-19T15:00 -->
- [x] Implement the flush reconcile cascade and the single forced-preview refusal <!-- completed: 2026-11-19T15:12 -->
- [x] Verify the herdr `pane send-text` form against Herdr 0.9.5 help; add `Multiplexer::send_text` for tmux and herdr <!-- completed: 2026-11-19T14:56 -->
- [x] Add the schema, reconcile and `send_text` tests <!-- completed: 2026-11-19T23:40 -->

### Step 8: Forced flush

- [x] Add `InputControlOperation::Flush`, its codec, and dispatch in `serve_input` with no `quiesce` <!-- completed: 2026-11-19T15:12 -->
- [x] Implement `Reserve`, the `flush` slot, `Settle`, expiry, and the `fence()` and recovery cascades in `InputCoordinator` <!-- completed: 2026-11-19T15:12 -->
- [x] Add the `Flush` subcommand to `src/cli/member.rs` and the Director-only guard list <!-- completed: 2026-11-19T15:12 -->
- [x] Implement the CLI procedure, output and error strings in `src/runtime/flush.rs` <!-- completed: 2026-11-19T15:12 -->
- [x] Add the runner flush tests, the served-while-stalled tests and the CLI tests <!-- completed: 2026-11-19T23:40 -->

### Step 9: Validation

- [x] Run `mise //:test` and `mise //:lint`. Environment limitation: under host load (load average 184 on 64 cores during Steps 3–4) the default 32 test threads time out in unrelated timing-bound tests, so run with `RUST_TEST_THREADS=8` and record the thread count used <!-- completed: 2026-11-20T01:56 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-20T01:56 -->
- [x] Re-run the issue #623 reproduction live with the branch binary on a dedicated database (no reinstall; the active fleet keeps the installed 0.35.4 runtime) <!-- completed: 2026-11-20T01:56 -->
- [x] Mark the design doc complete <!-- completed: 2026-11-20T02:20 -->

**Step 9 results:** `mise //:test` with `RUST_TEST_THREADS=8` passed 2066 tests with 0 failures and 11 ignored; `mise //:lint` passed; `mise //docs:build` built 8 pages with no dead links, run in a `/tmp` copy of the documentation package with a `/tmp` pnpm store. The live re-run used the branch binary on `/tmp/cafleet-live/live.db` with Claude Code 2.1.322 on Herdr 0.9.5 (Verifier record `.verifier/live-sc11.md`): queued previews drained, `input_delivery_stalled` published once with cause `held_composer`, and `member flush` delivered the queued previews, including after a draft wrapped over 6 rows and a one-row leftover preview line.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-19 | Initial draft |
| 2026-11-19 | Review round 1: flush reservation and settlement moved to runner control actions served outside the queue path; FIFO order by `input_id`; one-read housekeeping at 1 s; live cause derivation and event validation; region-based `composer_draft`; settle interval, plain framing and absolute literal bound; Step 1 reframed as investigation |
| 2026-11-19 | Review round 2: flush order defined as the runner's in-memory FIFO order (replacing `input_id` ascending); explicit `draft_rows`; § Gate tolerance per D4 gate and drain blocker; `settle_ms` fixed at `STAGE_SETTLE`; late `Settle` handling |
| 2026-11-19 | Step 1 finding: gate (b) `observation incomplete` latch in `Transport::observe_drain_child` recorded in § Root cause; user decisions: § Runner: interrupted drain feed retains and later feeds the unfed remainder, and the § Gate tolerance `observation incomplete` row proceeds as a user-accepted last-resort override |
| 2026-11-19 | Director decisions: optional fixture `cell_styles` runs so suggestion fixtures reproduce cell style; a terminal parser failure keeps the lifetime observation fence |
| 2026-11-19 | Step 6 research results; Director and user decisions: Claude startup-hint suggestion fixture, full-colour suggestion STYLE, OpenCode normal-mode footer guard for `composer_draft` and `composer_empty`, Hermes post-turn rendering not researched |
