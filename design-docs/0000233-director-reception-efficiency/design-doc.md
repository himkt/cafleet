# Director Reception Efficiency

**Status**: Complete
**Progress**: 38/38 tasks complete
**Last Updated**: 2026-11-17

## Overview

`message poll` becomes the only consuming endpoint: it completes every message it returns. Because of that, a listener notice re-lists an ID only through the 300 s ceiling re-notice, which then fires only for genuinely unpolled IDs, and `message ack` and `message wait` are deleted. The member's own runner takes over ordinary-member preview retries, and the Director wake runs no command for an already-consumed notice and supervises only overdue assignments.

## Success Criteria

- [x] `cafleet message poll MEMBER_ID [--json]` completes every message it returns in one write transaction, so two concurrent polls never return the same message, and its records show `status_state` `completed`. <!-- completed: 2026-11-17T12:59 -->
- [x] `cafleet message ack` and `cafleet message wait` are absent: clap rejects both as unrecognized subcommands, and the repository outside `design-docs/` mentions neither. <!-- completed: 2026-11-17T12:59 -->
- [x] Any consuming poll releases the listener's hold. Within one listener process, a message ID appears in two notices only through a ceiling re-notice. That re-notice fires only while held IDs remain unpolled and fresh IDs exist, at least 300 s after `held_at`, as one bulk `pending` record naming every unpolled ID. The listener emits at most 1 notice, plus one per poll that consumed held IDs, plus one per ceiling firing. <!-- completed: 2026-11-17T12:59 -->
- [x] A Director notice whose every ID the Director already consumed ends the turn with zero commands. <!-- completed: 2026-11-17T12:59 -->
- [x] Director wake runs `member list` / `member capture` only for an assignment past its recorded expected reply point. <!-- completed: 2026-11-17T12:59 -->
- [x] An ordinary member's unnotified pending preview is retried by that member's runner, with no send, wait or Director command. Delivery is strictly oldest-first, and the sweep submits through the runner's control-socket input path. A clientless `pending_sweep` slot tracks its in-flight attempt, so it never overlaps another immediate attempt. <!-- completed: 2026-11-17T12:59 -->
- [x] A preview receipt settled `not_submitted` with a retried-preview reason (`pane_input_busy`, `storage_unavailable`, `observer_unavailable`, `delivery_budget_exhausted`, `runner_stopped`, `runner_lost` or `message_consumed`) publishes no `input_delivery_result`, whatever its origin. This holds both live and through `unpublished_delivery_results` on shutdown and recovery, and `input show` still reports the receipt. <!-- completed: 2026-11-17T12:59 -->
- [x] Every backend's Director receiver (Claude Monitor, Codex `--codex-thread` forwarder, and the opencode, hermes and pi extensions) delivers the unchanged ID-only notices, and its instructions follow the new wake procedure. <!-- completed: 2026-11-17T12:59 -->
- [x] The subscribe and stream probes are restated in consuming-poll terms, with no ACK or `wait`. The subscribe probe's hold-ceiling step uses an unpolled held notice. Every backend's `listen` eligibility row records a live re-run of the revised subscribe probe before release. <!-- completed: 2026-11-17T12:59 -->
- [x] Before release, a live check shows that `input_delivery_stalled` recovery through `member press` works. On a verified-composer run whose composer holds retained CAFleet text, the event reaches the Director after 3 minutes with the `review_stalled_input` instruction. The Director's `press_argv` (`member press ... Enter`) submits the retained literal, and the held composer clears. The waiting head and the later queued inputs then deliver in order. The record names the backend, native version and date. <!-- completed: 2026-11-17T12:59 -->
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass. <!-- completed: 2026-11-17T13:45 -->

---

## Background

On 2026-11-17 a Codex Director was running `cafleet message listen 1 --codex-thread` on cafleet 0.35.2. During its long turns, `[cafleet listen record] pending` notices piled up in `codex queue`. After each turn they drained as separate turns, and each one cost about 20 s: a `message poll` that returned `[]`, plus a `member list`. For example, message 6645 was created at 07:55, ACKed at 08:16 and notified again at 08:36–37. Message 6836 was ACKed at 08:24 and notified at 08:37.

There are two causes:

1. A queued notice is a snapshot, and it goes stale before Codex delivers it.
2. The 300 s `HOLD_CEILING` re-notice (SPEC.md § Notice hold; `NoticeHold` in `src/cli/message.rs`) uses ACK as a proxy for "the receiver read the notice". It therefore fires during busy turns, producing growing superset notices that re-list IDs the Director already handled.

The 20 s per notice comes from Director wake steps 3–4 in `assets/skills/cafleet/roles/director.md`, which run a poll and `member list` on every wake.

---

## Specification

### 1. Consumption model

| Command | New contract |
|---|---|
| `message poll MEMBER_ID [--json]` | Returns and completes every `input_required` message addressed to `MEMBER_ID` (§2) |
| `message show MESSAGE_ID [--json]` | Unchanged: read-only, returns one record in any state; the recovery read for a consumed body |
| `message listen MEMBER_ID …` | Unchanged read-only, ID-only observer; any consuming poll releases its hold, so the ceiling re-notice fires only for unpolled IDs (§3) |
| `message send` | Unchanged except the not-notified diagnostic's retry clause (§8) |
| `message ack` | Removed |
| `message wait` | Removed (§5) |

Message state stays `input_required` → `completed`, and no column is added. `status_timestamp` becomes the consumption time. The roster field `last_ack` in `member list` is renamed `last_consumed` (same value: the latest `status_timestamp` of the member's completed incoming messages). This is a hard rename with no alias.

**Message ID order is commit order.** `messages.message_id` is `AUTOINCREMENT`, allocated inside the inserting write transaction, and SQLite serializes writers. A larger ID therefore always commits after a smaller one. SPEC §6.2 Messaging states this as a contract, because the Director's consumed watermark (§6) depends on it.

### 2. `message poll`

`cafleet message poll MEMBER_ID [--json]`:

1. Open one `BEGIN IMMEDIATE` transaction under the ordinary database timeout.
2. Validate an active member. An unknown or inactive member returns `Member ID not found`, exits 1 and consumes nothing.
3. Select every `to_member_id=MEMBER_ID AND status_state='input_required'` row, ordered by `status_timestamp DESC, message_id DESC`. This order is evaluated before the update, so it matches today's poll order.
4. Update exactly the selected IDs to `status_state='completed'` with one `status_timestamp` (now), and re-read them in the step-3 order.
5. Commit, then print. JSON is the array of post-consumption records. Text mode keeps its 200-codepoint body truncation, and `message show ID --json` recovers a full body.

An empty inbox prints `[]` (text: today's empty output), commits a no-op and exits 0. A failure before commit rolls back and consumes nothing. A stdout failure after commit leaves the messages consumed; this is the durability case in §10. `ack_message`, its `Cannot ACK message in state <state>` error and the `MessageCommand::Ack` variant are deleted.

A consumed message leaves `PENDING_PREVIEW` (`status_state='input_required' AND notified_at IS NULL`). A runner whose FIFO holds that message's zero-byte preview cancels it on the next queue turn as `not_submitted/message_consumed` (existing §6.9 rule, reworded from "ACK" to "consumption").

### 3. Notice hold and the ceiling re-notice

The listener's emission state is the hold. The *held set* `H` is the IDs named by the last delivered notice that are still pending, and `held_at` is the monotonic instant that notice was delivered. `H` is never empty while the hold exists, and the listener starts with no hold. `HOLD_CEILING` stays at 300 s. On every successful snapshot with ordered pending set `P`:

1. **Reconcile**: `H ← H ∩ P`. Consumed IDs leave, and an empty `H` ends the hold and its `held_at`.
2. **Fresh IDs**: `F ← P \ H`.
3. **Decide**:

| Condition | Action |
|---|---|
| `P` empty | Emit nothing |
| No hold and `P` non-empty | Emit `pending(P)` |
| A hold, `F` non-empty and at least `HOLD_CEILING` since `held_at` | Emit `pending(P)`, the ceiling re-notice: one bulk record naming every unpolled ID |
| Otherwise | Emit nothing |

4. **Commit**: when the output reports the notice delivered, `H ← P` and `held_at` ← now. A refused notice leaves the hold as reconcile left it.

The decision table and `NoticeHold`'s logic are unchanged. What changes is the meaning of "still pending": an ID now leaves `P` only through a consuming poll, not through an ACK.

**Re-listing happens only for unpolled IDs.** `poll` consumes the subject's whole pending set atomically, so a snapshot sees either all held IDs pending or none of them. Any poll by the receiver therefore releases the hold. The next ordinary notice names only IDs committed after that poll, and the listener keeps no per-ID history. An ID appears in two notices of one listener process only through a ceiling re-notice. That re-notice fires only when no poll has run for 300 s since the held notice was delivered, so every ID it names, held or fresh, is genuinely unpolled. The only other re-listing is a new listener's startup replay of still-unpolled IDs.

**Why the ceiling stays.** The ceiling is the liveness backstop for a delivered notice that no turn ever acted on: a Codex queued submission dropped by an interrupt, a Monitor event suppressed by its rate limit, or a Director that deferred the poll behind a pending user question. Without it, every later message would stay held, and an idle Director would never be woken again. In 0.35.2 the ceiling fired during busy turns because ACK, not reading, released the hold. A mid-turn consuming poll now releases the hold immediately, so a re-notice reaches the Director only in the rescue case. Its IDs are above the consumed watermark (§6), and the one poll it prompts is the intended recovery.

**Notice bound.** Each ordinary notice after the first requires a release, and each release requires a poll. Each ceiling firing emits at most one notice and restarts the ceiling. The number of notices is therefore at most 1, plus the number of polls that consumed held IDs, plus the number of ceiling firings (at most one per 300 s, and only while held IDs stay unpolled with fresh IDs present). A turn with k mid-turn polls can queue at most k + 1 notices behind it, and each of them drains as a zero-command turn under §6. A burst of any size adds no notice beyond the outstanding one. The Codex 100-submission cap is reached only with more than 100 consuming polls or ceiling firings queued behind one Director turn.

| Situation | Behavior |
|---|---|
| A delivered notice whose IDs are never polled, and a new message arrives | One ceiling re-notice naming the held and fresh IDs, at least 300 s after `held_at`; the Director's watermark check sends it to one poll |
| A delivered notice whose IDs are never polled, and nothing new arrives | No re-notice; the ceiling fires only when fresh IDs exist |
| The receiver polls before any notice names an ID | That ID is never notified |
| A message arrives while a notice is held | Not notified until the receiver polls or the ceiling fires; any `poll` returns it immediately |
| Mid-turn poll, then the queued notice is delivered | The queued notice names only consumed IDs; the Director ends that turn with no commands (§6) |
| Listener restart | `ready`, then one notice naming every pending (unpolled) ID |

Codex forwarding keeps its refused-notice rebuild unchanged: a rebuilt notice whose pending set became empty, or whose re-notice no longer has fresh IDs, is abandoned. Listener memory holds the current snapshot, one record and the held IDs.

### 4. Ordinary-member preview retry owner

Removing `message wait` removes its rotation, the only external retry of a held or failed preview. Today `message send`'s synchronous drain is the first attempt, and a preview the runner accepted into its FIFO already retries itself. The gap is a message that never reached the FIFO: a busy pane lock through the send budget, a runner/transport/storage failure, `delivery_budget_exhausted`, or a pending placement at send time.

| Candidate owner | Needs a caller command | Fit | Decision |
|---|---|---|---|
| The recipient's own runner (sweep) | No | It already owns the FIFO, the 100 ms timer, the pane lock protocol, a database connection and the run identity. Its scope is its own member, so it needs no cross-member cursor. | **Chosen** |
| The Director's `message listen` | No | Contradicts the read-only observer contract, and it exists only while a Director listener runs | Rejected |
| Director `message poll` | Yes (the Director's poll) | Adds pane I/O and up to 5 s to every consuming poll, and does nothing while the Director is idle | Rejected |
| The next `message send` only | Yes | The preview waits indefinitely if nobody sends | Rejected |

**Runner pending-preview sweep** (new SPEC §6.9 subsection beside Runner-owned deferred delivery):

- **Cadence.** From the runner event loop, at most once per `PREVIEW_SWEEP_INTERVAL` = 1 s, and not before the run-wide backoff (below) expires. It runs only when all of these hold:
  - the run is live and the native child is running;
  - no immediate attempt is in flight. `pending_input_client` is empty, meaning no control-socket input request (a `message send` preview, `member prompt` or `member exec`) has been accepted for immediate submission and is still awaiting its reply. `pending_sweep` (below) is also empty, so two sweep attempts never overlap and a sweep attempt never overlaps a client-owned one;
  - the runner FIFO holds no preview request. A FIFO preview already retries itself, and the sweep never duplicates it.
- **Selection: strict oldest-first.** One short read snapshot confirms three things: the run's member is an active ordinary member, its recorded run is this run, and it has a concrete placement. The snapshot then takes the oldest `to_member_id=<member> AND PENDING_PREVIEW` row by `(created_at, message_id)`. Stamped self-sends are excluded by `notified_at`. The sweep skips a message only for an existing per-message hold under the latest-attempt table: an in-progress, uncertain or abandoned receipt on a verified-composer run moves on to the next message. Every other hold, and every per-message hold on a run without composer verification, ends the sweep for this tick. This matches the send drain.
- **Attempt decision.** Apply the existing latest-attempt table for `preview:<run_id>:<message_id>:<attempt>`:
  - No receipt starts attempt 1.
  - A not-submitted settlement starts N+1.
  - Accepted/submitted history settles and stamps without typing.
- **Submission: the control-socket path.** The sweep acquires no lock beforehand. It hands the canonical preview to the same runner-internal entry point that a control-socket preview request reaches (`InputCoordinator::handle_received` in `src/runtime/input.rs`), as an `InputControlRequest` built with the run's own control identity. That entry point reserves the `preview:` receipt, applies §6.9 Delivery method and interaction priority, and either submits immediately or queues with zero bytes. For the actual stages, the runner acquires the pane input lock nonblockingly from its event loop (§6.5 Pane input lock), so a merely queued sweep request holds no lock. A send's synchronous drain and the sweep serialize through that lock and the runner's under-lock revalidation of message status, notification stamp and receipt.

- **Clientless tracking: the `pending_sweep` slot.** A sweep request has no control-socket client. The service therefore never records it in `pending_input_client`; `src/runtime/service.rs` gains a distinct `pending_sweep: Option<PendingSweep { deadline }>` slot.
  - When `handle_received` returns `None` for a sweep request (an immediate attempt still awaiting its result), the service records `pending_sweep` with the attempt deadline.
  - The completion path (`complete_pending_input`'s quiesce/advance loop, extended to either slot) drives that attempt to its reply, then settles it without replying to any client, feeding the reply's delivery status and reason to the backoff.
  - A reply returned at once (queued or settled) is handled the same way, and no slot is recorded.
  - The existing "accepted pending input already has a client" invariant becomes "an accepted immediate attempt requires both slots empty".
  - A control-socket request that arrives while `pending_sweep` is set is handled exactly as one arriving while a client-owned attempt is in flight.
  - Orderly shutdown settles a pending sweep attempt like a client-owned one, with no reply.
- **Settlement owner.** There is no client to settle a sweep attempt, so the runner settles it. An accepted attempt's settlement transaction stamps `notified_at` and marks the receipt settled/submitted. This is the same transaction the runner already commits when a queued preview drains (§6.9 Runner-owned deferred delivery), not the sender-side settlement of §6.5 Push notifications step 7.
- **Run-wide backoff.** Every transient cause is pane- or run-wide: a sweep attempt that settles `not_submitted` with `pane_input_busy`, `storage_unavailable`, `observer_unavailable` or `delivery_budget_exhausted`. Such an attempt publishes no `input_delivery_result` (the retried-preview set below) and sets a memory-only run-wide backoff: 1 s, doubling, capped at 30 s. An accepted sweep attempt, an `interaction_closed`, or a candidate clearing resets the backoff to zero. The message stays at the head, and strict oldest-first order is preserved.
- **Accepted receipt growth.** Each backed-off retry settles one more durable `not_submitted` receipt, attempt N+1, for the head message only. The backoff caps this at 1 receipt per 30 s once the cap is reached, about 125 receipts in the first hour of a continuous obstacle. The design accepts that growth: receipts are metadata-only rows, each cafleet version starts a fresh database, and a persistent pane-wide obstacle also blocks every other writer, so it surfaces through the Director's overdue check (§6).
- **Lifecycle.** Orderly shutdown and runner loss stop the sweep. A recovered or new run sweeps under its own run key. A member with no live runner keeps its messages pending, and the Director's overdue check (§6) surfaces it.

**Retried preview outcomes publish no event, from every origin.** A preview receipt that settles `not_submitted` leaves its message pending, so either the sweep retries it or the member has already consumed it. An `input_delivery_result` for it would only cost the Director a wake and a poll. The suppressed set is derived from durable receipt fields alone (kind plus delivery status and reason), so it does not matter whether a send, a re-delivery successor or the sweep originated the preview:

| Receipt | Delivery | Reason | `input_delivery_result` |
|---|---|---|---|
| `kind=preview` | `not_submitted` | `pane_input_busy`, `storage_unavailable`, `observer_unavailable`, `delivery_budget_exhausted` | None; the sweep retries the still-pending message |
| `kind=preview` | `not_submitted` | `runner_stopped`, `runner_lost` | None; the next run's sweep retries the still-pending message |
| `kind=preview` | `not_submitted` | `message_consumed` | None; the member's poll already consumed the message |
| `kind=preview` | `not_submitted` | `target_inactive`, `target_changed` | Published, as today |
| Any kind | `left_in_composer`, `uncertain`, submitted after deferral | — | Published, as today |
| `prompt`, `exec` | `not_submitted` (any reason) | — | Published, as today |

One predicate in `src/broker/runtime/inputs.rs`, `is_retried_preview_result(kind, delivery)`, defines the set. Both live publication in the runner and `unpublished_delivery_results` use it, so orderly shutdown and recovery republish none of these receipts. `input show` still reports every receipt. `runner_stopped` and `runner_lost` join the set for the same reason as the transient causes: their message stays unnotified, and the next run's sweep delivers it under its own run key.

`message send` keeps its synchronous drain of the recipient's pending messages within its five-second budget, because that drain is the fast first attempt.

### 5. Removal of `message wait`

Delete the subcommand, `delivery::wait`, `ReceiveIo`, `BrokerReceiver`, `WaitOutcome`, the rotation cursor and the tests of each, along with every mention outside `design-docs/` (Removal rule). Manual Director reception is poll-only: when a receiver is unavailable, the Director runs `message poll <director-id> --json` at its wake points. Every "finite `wait`" phrase in SPEC §6.7, the Director role, coding-agents.md and the docs becomes "manual `poll`". The `permissions.allow` coverage sentence keeps `listen` only.

### 6. Director wake

Director wake (`assets/skills/cafleet/roles/director.md`) becomes:

1. Retain one owned receiver for the fleet's lifetime, as today.
2. Consume complete `ready`, `pending` and `closed` records as the receiver delivers them, as today.
3. **Consumed watermark.** In host-owned state kept across compaction, record W, the highest message ID any of your polls returned.
   - **Already-consumed notice.** On a `pending` notice whose every `message_ids` entry is ≤ W, end the turn with no commands. These IDs were consumed by an earlier poll, because poll consumes the whole inbox and IDs follow commit order (§1).
   - **New-ID notice.** Otherwise, and also when W is unknown or a record is unparseable or truncated (for example a Claude Monitor line truncated at 500 characters), run `cafleet message poll <director-id> --json` once.
4. **Record immediately.** Before running any other command after a poll, record every returned message's ID, sender, required action and artifact pointers in host-owned state, and raise W. Then dispatch work whose inputs are ready. Mid-turn polls stay, including the poll before reporting completion.
5. **Supervise only overdue work.** At dispatch, record each assignment's expected reply point, a per-assignment estimate chosen from the task's size. On a wake, run `member list` / `member capture` and check artifacts only for assignments past that point. An overdue check starts with one `poll --json`, because a lost closed notice or a dead runner surfaces there. Spawn, pane discovery and fleet-deletion failures keep their own `member list` uses.
6. Keep user decisions pending, as today.

The Director's "close a member request with a reasoned refusal followed by ACK", "Receive each member's ready and ACK it" and runtime-event "ACK consumption" steps become "record it".

**Stalled preview head (`input_delivery_stalled`).** No command replaces the ACK of a waiting preview, and the Director never polls another member's inbox. Director recovery (`director.md` § Reconcile generated input) becomes:

1. Verify the event and record its waiting head and blocking input IDs.
2. Capture the pane and inspect the blocking input with its `inspect_argv`.
3. When the capture confirms that the whole CAFleet literal remains and `member show` reports no current interaction, submit it with `member press ... Enter` (the supplied `press_argv` when present). When the capture shows that other keys are needed, send those keys with `member press` under host authorization. Otherwise ask the user to clear or submit the composer on the member's native surface.
4. Reconcile an uncertain blocker only after that evidence.

A verified empty composer resumes the FIFO, and the waiting head delivers on the next queue turn. If the member's poll already consumed the message, the runner instead cancels the head as `message_consumed`. A composer rule that never verifies empty after clearing is a profile bug: record it and route it for correction of the bundled profile. Today's step 4 ("ACK the waiting message …") is deleted.

Director Sender follow-up is removed: the runner sweep retries a persisted-but-undelivered send, so the extra message only duplicates it. The member role's matching "send exactly one follow-up …" rule is removed with it.

### 7. Member reception

Members poll after ready, on every wake (a preview arriving is a wake) and before completion reports. Poll consumes, so members record actionable content before other work and never ACK. A preview line carries the body inline (≤ 4000 scalars) or a `message show` pointer, but the member still runs `poll --json` on that wake: that poll consumes the message and any others in one command. Duplicate preview lines are deduplicated by message ID. The pi member gate's allowlist, the opencode `cafleet` and `cafleet-ask` preset permission maps and their bodies list `send`, `poll` and `show`.

### 8. Contract text changes

| Surface | Current | New |
|---|---|---|
| Not-notified diagnostic (`delivery::notification_error`, SPEC §6.3 Delivery outcome, cli.md) | `…: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.` | `…: CAUSE; member MEMBER_ID's runner retries it.` |
| Storage-failure diagnostic scope | "or the selected wait-rotation entry" | Removed; names the current candidate |
| `input show` latest-attempt row "Not submitted" | Next send/wait starts N+1 | The runner sweep or the next send starts N+1 |
| Error table `message ack` / `message show` rows | Unknown ID; `Cannot ACK message in state <state>` | `message show` unknown ID only |
| `review_stalled_input` instruction (`src/broker/runtime/events.rs`, SPEC §6.9 Input delivery stalled event INSTRUCTION, cli.md Runtime events) | "… Reconcile an uncertain blocker after that evidence. ACK the waiting preview when its composer stays unverified after clearing or its message no longer needs delivery." | "Capture the member's pane and inspect the blocking input. When its whole literal remains and member show reports no current interaction, submit it with press_argv; when the capture shows other keys are needed, send them with member press; otherwise ask the user to clear or submit the composer on the member's native surface. Reconcile an uncertain blocker after that evidence. A composer that never verifies empty after clearing is a profile defect to report for correction." |
| Roster field | `last_ack` | `last_consumed` |
| `input_delivery_result` publication (SPEC §6.9 Runner-owned deferred delivery outcome table and Persistence, notification and read interfaces; cli.md Runner queue and Runtime events) | Published for every deferred settlement, including transient `not_submitted`, `runner_stopped`, `runner_lost` and `message_consumed` previews | Preview receipts in the §4 retried-preview set publish nothing, whatever their origin, live or by the unpublished-result scan; `target_inactive`, `target_changed`, prompt/exec results, `left_in_composer`, `uncertain` and deferred submissions publish as today; `input show` reports every receipt |
| Runner queue cancellation wording | "check the head preview's ACK"; "ACK cancels its zero-byte receipt" | "check whether the head preview's message is consumed"; "consumption cancels its zero-byte receipt" |
| Extension `cafleet_listen` tool descriptions (opencode, hermes, pi) | "… naming unACKed message IDs; run 'cafleet message poll <director_id> --json' and ACK each consumed message." | "… naming unpolled message IDs; when an ID is above your consumed watermark, run 'cafleet message poll <director_id> --json', which consumes what it returns." |

### 9. Backend receivers

Every receiver keeps its transport and its ID-only notice. Only the instructions change.

| Backend | Receiver | Change |
|---|---|---|
| `claude` | One Monitor running `cafleet message listen <id> --timeout 1770` | Recipe step uses §6 steps 3–4; the truncation note keeps "a truncated line prompts one poll" |
| `codex` | `--codex-thread` forwarder via `codex queue` | Recipe step 3 uses §6; queue-cap note restated as "more than 100 consuming polls or ceiling firings queued behind one turn"; step 5's "per-wake poll" becomes the overdue check's poll |
| `opencode` | `cafleet` plugin `cafleet_listen` (`session.synthetic`) | Tool description (§8) and recipe |
| `hermes` | `cafleet` plugin `cafleet_listen` (gateway injection) | Tool description (§8) and recipe |
| `pi` | CAFleet pi extension `cafleet_listen` (`followUp`) | Tool description (§8), recipe, member-gate allowlist (§7) |

### 10. Durability trade-off of poll-time consumption

A poll removes messages from the pending set before the Director acts. If a Director polls and then loses its session or context before recording (§6 step 4), or if stdout fails after commit, those messages are consumed but unhandled, and no listener or poll lists them again. The design accepts this risk, which no new command mitigates:

- **Prevention:** record immediately after every poll, before any other command (§6 step 4). This narrows the window to the poll output itself.
- **Recovery:** a resumed Director treats every in-flight assignment as overdue. It captures member panes and artifacts; a member's send output shows its `[MESSAGE_ID | from:…]` record. It recovers each body with `message show MESSAGE_ID --json`, and otherwise asks the member through the broker to restate its status.

The same applies to members: a member that polls and then crashes loses the assignment until the Director's overdue check re-sends it.

### 11. Listen probes and eligibility evidence

SPEC §6.7 Listen requirement changes as follows.

**Probe table.** The Subscribe probe's pass condition is "each probe ID is first observed in a notice; the Director then consumes it through the poll that notice prompts". Its invalidation rule is "a `poll` or `show` that first reveals a probe ID without a preceding notice turn". In the Stream probe row, "ACKed while the same listener process still runs" becomes "consumed by one notice-prompted poll while the same listener process still runs". Any `wait` mention is removed.

**Subscribe-probe instruction:**

> Consume broker messages only through your receiver. On each `[cafleet listen record]` turn, end the turn with no commands when every named ID is at or below your consumed watermark; otherwise run `cafleet message poll <director-id> --json` once and raise the watermark. Run no other `poll` or `show` unless a step below directs it. When I say `hold`, end only the next notice turn without polling, then apply the rule above to every later notice turn, including the ceiling re-notice.

| Step | Procedure | Pass condition |
|---|---|---|
| Idle | Send one message to the idle Director | A notice naming it starts a turn without input; the poll it prompts returns it |
| Busy | During a foreground command of at least 60 s, send two messages | The command is not interrupted; a notice arrives no later than the end of that turn, and its poll returns both IDs |
| Burst | During a foreground command of at least 60 s and under 300 s, send 150 messages back to back | At most two notice turns arrive for the burst, and every ID is consumed through a notice-prompted poll. No ID appears in two notices. On Codex the forwarder stays alive and no `codex queue` refusal occurs. |
| Consumed notice | Direct one turn: a 30 s foreground command while m1 is sent, one poll, a second 30 s command while m2 is sent, one poll, then end the turn | Every notice turn delivered after that turn names only consumed IDs and runs no command; no ID appears in two notices |
| Hold ceiling | Say `hold`; send `hold-probe 1` to the idle Director and record t1, the time its notice turn starts (the Director ends it without polling). After that turn ends and 60 s have passed since t1, send `hold-probe 2`. | No notice arrives before t1 + 285 s. One notice naming both IDs arrives between t1 + 285 s and t1 + 315 s; its IDs are above the watermark, so the Director polls once and the poll returns both |
| Owner exit | End the Director session | No `cafleet message listen` survives (`ps`) |

The t1 tolerance paragraph stays: the listener sets `held_at` when delivery succeeds, which precedes t1 by the receiver latency, and it checks the ceiling once per one-second idle interval, so the ±15 s tolerance around t1 + 300 s covers both.

In the stream-probe prompt, two phrases change:

- "ACK each message ID it names with `cafleet message ack <message-id>`" becomes "run `cafleet message poll <director-id> --json` once".
- "run no `cafleet message poll`, `wait` or `show`" becomes "run no other `poll` or `show`".

Elsewhere in §6.9:

- The Native acceptance latency sentence becomes "while a notice is held, the committed ID reaches receiver output in the next notice, after the receiver's poll releases the hold or the ceiling fires".
- The `Held composer` acceptance case's "ACK cancels the next queue turn" becomes "consumption cancels it on the next queue turn".

**Eligibility evidence: live re-check before release (user decision).** Before release, the revised subscribe probe runs live on the installed native version of every backend with a Director `listen` row: claude, codex, opencode, hermes and pi. Each SPEC §6.7 Role eligibility `listen` row and each coding-agents.md Basis cell is then rewritten with the revised step list (idle wake, busy, burst, consumed notice, the 300 s hold ceiling, owner exit), the native version and the new check date. The subscribe transports are unchanged. The re-check is still required because the recorded evidence describes ACK-released holds, and the Consumed-notice step directly verifies the queued-notice behavior observed on Codex.

### 12. Documentation surfaces

Per `.claude/rules/documentation-maintenance.md`, update docs first, in order: SPEC.md, `docs/`, README.md, `assets/skills/`, then project rules and project skills. Every file outside `design-docs/` that matches the Step 1 inventory pattern changes. That pattern covers the removed commands, their code symbols and formatters, ACK as a consumption step, `last_ack`, the `finite receive` term, bare `` `wait` `` and wait rotation. Every SPEC.md section that carries a changed fact:

| SPEC section | Change |
|---|---|
| §1 Overview & goals | Drop "finite receive" from the reproduced semantics |
| §2 Architecture stance | Replace "finite receive owns bounded waits and notification retries" with the runner sweep (pointer to §6.9), and "consumed deliveries are explicitly ACKed" with poll-time consumption |
| §6.1 Persistence & Schema | "send, ACK and notification stamp" becomes "send, poll consumption and notification stamp" |
| §6.2 Members | Roster activity and `idle` derive from `last_consumed` |
| §6.2 Messaging | Message-ID commit-order contract (§1); the "already-ACKed rows fail" validation clause is removed with ACK |
| §6.3 | Subcommand summary, Output shapes `message ack` row, `message poll` (§2), removal of the `message ack` and `message wait` sections, roster field, `permissions.allow` coverage, `message send` retry sentence, Delivery outcome diagnostics and its wait-rotation row, `message listen` hold/bound/situations, Codex refused-notice clause, error table |
| §6.4 Persisted shape / Rendered shape | Consumption by poll; drop the wait result and "explicit ACK consumption" |
| §6.5 Push notifications, Held delivery, Accepted trade-offs | Retry owner is the runner sweep (pointer to §6.9); "until its recipient ACKs" becomes "until its recipient polls"; poll/ACK trade-off bullets become poll |
| §6.5 Pane input lock | Writer table row `message send` / `message wait` delivery becomes `message send` delivery |
| §6.5 Termination during pane input | Span row and SIGKILL row drop `message wait`; the SIGKILL recovery becomes "the recipient's runner sweep retries it" |
| §6.6 Receiving and pane input | Replace the `message wait` receive-task paragraph: ordinary-member preview retry belongs to the runner sweep (§6.9), and Director reception is the listener plus consuming poll (§6.3) |
| §6.7 Role requirements, Listen requirement, Role eligibility | Manual-reception wording becomes `poll` only; probe table and probe texts (§11); evidence rows |
| §6.7 Per-backend `build_spawn_argv`, opencode preset, pi member gate, Director extensions | The claude `--allowedTools` coverage sentence ("including finite `message wait` and `message listen`") names `message listen` only; the preset shell ruleset, the gate allowlist and the extension tool descriptions list `send`, `poll` and `show` with poll-time consumption |
| §6.9 Input delivery stalled event, §6.5 Accepted trade-offs | INSTRUCTION text per §8; the held-composer trade-off recovery becomes press, user clearing or uncertain reconciliation, and a composer that never verifies empty is a profile defect |
| §6.9 Durable input receipts and settlement, Delivery reporting | "Every sender, wait rotation and runner decision" becomes "Every sender, sweep and runner decision"; the latest-attempt rows and settlement sentences that say "verify ACK" or "ACK settlement" say "verify consumption" |
| §6.9 Runner-owned deferred delivery, Persistence, notification and read interfaces | Consumption-based cancellation, the retried-preview set and its shared predicate (§4), the new Runner pending-preview sweep subsection with its `pending_sweep` slot (§4) |
| §6.9 Director consumption and continuation, Native acceptance and coverage | Consumption through listener-prompted `poll --json`; the Consumption stage's evidence is the consuming poll plus the Director-owned record; held messages retried by the sweep; acceptance cases and latency sentence (§11) |
| §8 Database schema | The `messages` row describes `completed` as set by the consuming poll |
| §9 Testing strategy | Replace "point-to-point/ACK/pending", "finite receive deadlines, round-robin retries and diagnostic clearing" and "ACK races" with consuming-poll exactness, the runner sweep and concurrent-poll races |

### 13. Testing

All tests are in-file `#[cfg(test)] mod tests` units (`.claude/rules/code-quality.md` § Test placement).

| File | Behavior asserted |
|---|---|
| `src/broker/messaging.rs` | Poll returns the newest-first pending set as `completed` records and consumes it; a second poll returns `[]`; two concurrent polls on separate connections return disjoint sets whose union is the pending set; an unknown member consumes nothing; IDs from interleaved inserts increase in commit order |
| `src/cli/message.rs` | Listener over a broker driven by consuming polls: a poll releases the hold, and the next notice names only post-poll IDs; over a scripted sequence of mid-turn polls no ID appears in two notices, and notices ≤ 1 + consuming polls; a held notice left unpolled with a fresh arrival re-notices once at `HOLD_CEILING` naming held and fresh IDs, and never before it; an unpolled hold with no fresh ID never re-notices; a poll before the ceiling cancels the re-notice; `ack` and `wait` fail clap parsing as unrecognized subcommands |
| `src/runtime/service.rs` | Sweep submits the oldest unnotified pending preview with no caller command; it does not run while the FIFO holds a preview or either `pending_input_client` or `pending_sweep` is set; an immediate sweep attempt that returns `None` from `handle_received` is recorded in `pending_sweep`, never in `pending_input_client`, is completed by the completion path with no client reply, and blocks a second sweep attempt until it settles; a control-socket request arriving meanwhile is handled as behind an in-flight attempt; it holds no pane lock before submission, and with the lock held by another process the request queues with zero bytes and its immediate stage acquires the lock in the runner once it is free; an accepted sweep attempt's receipt is reserved by the runner and its settlement stamps `notified_at`; strict oldest-first: a transient `not_submitted` on message A sets the run-wide backoff (1 s doubling to 30 s, reset on acceptance) and the next attempt is A again, never a newer B; per-message holds skip only on a verified-composer run; a consumed message is never selected |
| `src/runtime/input.rs` | Live publication skips the retried-preview set: a queued preview whose message is consumed by poll settles `not_submitted/message_consumed`, and a transient deadline settles `not_submitted/pane_input_busy`. Neither publishes an `input_delivery_result`, whether the head is send-originated, a re-delivery successor or a sweep head; `target_inactive` still publishes |
| `src/runtime/input.rs` (stalled-head recovery) | With the held composer set by a `left_in_composer` blocker and a waiting preview head plus a later queued input, an Enter press that makes the composer verify empty clears the held composer. The FIFO then resumes and delivers the waiting head first and the later input after it |
| `src/broker/runtime/inputs.rs` | `is_retried_preview_result` matches exactly the §4 set; `unpublished_delivery_results` omits preview receipts in that set (each transient reason, `runner_stopped`, `runner_lost`, `message_consumed`), and still returns `target_inactive` previews, prompt/exec `not_submitted` and `left_in_composer` receipts |
| `src/delivery.rs` | Not-notified diagnostic text names the runner retry |
| `src/broker/members.rs` | Roster `last_consumed` equals the latest consumption timestamp and feeds `idle` |
| `src/broker/runtime/events.rs` | `review_stalled_input` instruction text equals the §8 text: press or user clearing, uncertain-blocker reconciliation and the profile-defect report, with no ACK or poll step |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Inventory

- [x] Run the inventory pattern `git grep -nE "message ack|message wait|\bACK|unACKed|ack_message|MessageCommand::(Ack|Wait)|format_message_(wait|acknowledged)|finite receive|finite .?wait|\`wait\`|rotation|last_ack" -- . ":!design-docs"` and record each hit's owning step; record a hit that names no removed surface (for example the Hermes plugin's session-id rotation) as kept <!-- completed: 2026-11-17T09:32 -->

### Step 2: SPEC.md

- [x] §1, §2, §6.1 and §6.2 Members/Messaging per the §12 table, including the message-ID commit-order contract <!-- completed: 2026-11-17T09:33 -->
- [x] §6.3: subcommand summary, Output shapes, `message poll` (§2), removal of the `message ack` and `message wait` sections, roster `last_consumed`, `permissions.allow` coverage, `message send` retry sentence, Delivery outcome diagnostics, error table <!-- completed: 2026-11-17T09:33 -->
- [x] §6.3 `message listen`: Notice hold, Notice bound and situation table per §3; Codex forwarding refused-notice clause <!-- completed: 2026-11-17T09:33 -->
- [x] §6.4 Persisted shape and Rendered shape; §6.5 Push notifications, Held delivery, Accepted trade-offs, Pane input lock writer table and Termination during pane input span/SIGKILL rows <!-- completed: 2026-11-17T09:36 -->
- [x] §6.6 Receiving and pane input: replace the `message wait` receive-task paragraph with the runner sweep and listener-plus-poll pointers <!-- completed: 2026-11-17T09:36 -->
- [x] §6.7: Role requirements manual-reception wording, probe table, subscribe/stream probe texts and steps per §11, the claude `--allowedTools` coverage sentence, opencode preset, pi member gate and Director extensions rows <!-- completed: 2026-11-17T09:36 -->
- [x] §6.9: Runner pending-preview sweep subsection (§4), Runner-owned deferred delivery cancellation and `message_consumed` publication, Durable input receipts and Delivery reporting wording, Input delivery stalled event INSTRUCTION, Director consumption and continuation, acceptance cases and latency sentence <!-- completed: 2026-11-17T09:38 -->
- [x] §8 `messages` row and §9 Testing strategy per the §12 table <!-- completed: 2026-11-17T09:38 -->

### Step 3: User docs and README

- [x] `docs/docs/concepts/overview.md`: delete the `finite receive` Core terms row, and restate the `listener` row as notifying unpolled message IDs, with `message poll` consuming them <!-- completed: 2026-11-17T09:40 -->
- [x] Rewrite in the same edit every page that uses the removed term or ACK consumption: `docs/docs/concepts/members.md` ("a later send or finite receive retries it" becomes the runner retry; "A preview whose message is ACKed is cancelled" becomes "…is consumed…"), `docs/docs/mixed-backend-team.md` (manual reception becomes `message poll` only; "Poll and ACK your assignment" becomes "Poll your assignment"), `docs/docs/concepts/coding-agents.md`, and `docs/docs/contributing.md` (subscribe and stream probe prompts, steps and classification per §11) <!-- completed: 2026-11-17T09:40 -->
- [x] Check README.md against `docs/docs/index.md` via `/update-readme`; edit only if drifted <!-- completed: 2026-11-17T09:40 -->

### Step 4: Skills, presets and rules

- [x] `assets/skills/cafleet/SKILL.md`: rename "Poll and ACK" to "Poll" and restate consumption; Send section retry clause <!-- completed: 2026-11-17T09:41 -->
- [x] `assets/skills/cafleet/reference/cli.md`: mirror every Step 2 §6.3/§6.5/§6.9 fact it carries <!-- completed: 2026-11-17T09:43 -->
- [x] `assets/skills/cafleet/roles/director.md`: Director wake per §6, receiver recovery manual reads, runtime-event steps, stalled-head recovery per §6 (step 4 deleted), Sender follow-up removal, Authority/Spawn Protocol ACK wording <!-- completed: 2026-11-17T09:44 -->
- [x] `assets/skills/cafleet/roles/member.md`: §7 reception, follow-up rule removal <!-- completed: 2026-11-17T09:44 -->
- [x] `assets/skills/cafleet/reference/coding-agents.md`: every backend's notes, recipes and Role eligibility Basis per §9 and §11 <!-- completed: 2026-11-17T09:44 -->
- [x] `assets/skills/cafleet-design-doc/` files from the inventory (coordination payload exemptions, roles, workflows): replace ACK steps <!-- completed: 2026-11-17T09:44 -->
- [x] `assets/presets/opencode/cafleet.md` and `cafleet-ask.md`: permission maps and bodies per §7 <!-- completed: 2026-11-17T09:44 -->
- [x] `.claude/skills/` and `.claude/rules/` hits from the inventory <!-- completed: 2026-11-17T09:45 -->

### Step 5: Broker consumption

- [x] Rewrite `poll_messages` in `src/broker/messaging.rs` as the §2 consuming transaction and add its tests <!-- completed: 2026-11-17T09:56 -->
- [x] Delete `ack_message`, `MessageCommand::Ack`, its `src/cli/mod.rs` dispatch, `format_message_acknowledged` in `src/output/text.rs`, its JSON output and their tests <!-- completed: 2026-11-17T09:56 -->
- [x] Migrate every test that calls `ack_message` to simulate consumption (in `src/broker/members.rs`, `src/broker/messaging.rs`, `src/broker/runtime/events.rs`, `src/broker/runtime/inputs.rs`, `src/cli/message.rs`, `src/delivery.rs` and `src/runtime/input.rs`) to the consuming `poll_messages`. A test that needs another message still pending inserts it after the poll, because production cannot consume a single message out of a pending set <!-- completed: 2026-11-17T10:16 -->
- [x] Rename roster `last_ack` to `last_consumed` in `src/broker/members.rs`, `src/broker/mod.rs`, `src/output/json.rs`, `src/output/text.rs` and tests <!-- completed: 2026-11-17T09:56 -->
- [x] Add the ID commit-order test in `src/broker/messaging.rs` <!-- completed: 2026-11-17T09:56 -->

### Step 6: Listener hold

- [x] Keep `NoticeHold` and `HOLD_CEILING` in `src/cli/message.rs` as §3 specifies, and restate the `NoticeHold` doc comment in consumption terms ("still pending" means "not yet polled") <!-- completed: 2026-11-17T09:56 -->
- [x] Rewrite the ACK-driven hold tests (for example `acking_every_held_id_releases_the_hold_and_the_next_notice_names_the_full_pending_set`) to drive consumption through `poll_messages`, and add the §13 listener tests <!-- completed: 2026-11-17T09:56 -->

### Step 7: Remove `message wait` and add the runner sweep

- [x] Delete `MessageCommand::Wait`, its payload and `format_message_wait` (`src/cli/message.rs`, `src/output/text.rs`), `delivery::wait`, `ReceiveIo`, `BrokerReceiver`, `WaitOutcome`, rotation selection, and their tests, including the `wait` parse tests in `src/cli/mod.rs` <!-- completed: 2026-11-17T10:16 -->
- [x] Implement the §4 sweep in the runner event loop (`src/runtime/service.rs`): `PREVIEW_SWEEP_INTERVAL`, the cadence conditions, strict oldest-first selection, the latest-attempt decision, submission through `InputCoordinator::handle_received` with no lock held beforehand, the clientless `pending_sweep` slot completed without a reply, runner-owned settlement, and the memory-only run-wide backoff <!-- completed: 2026-11-17T10:16 -->
- [x] Add `is_retried_preview_result` to `src/broker/runtime/inputs.rs` and apply it in `unpublished_delivery_results`. In `src/runtime/input.rs`, apply it to live `input_delivery_result` publication, and restate ACK-based cancellation as consumption <!-- completed: 2026-11-17T10:16 -->

- [x] Update `delivery::notification_error` text and remove the wait-rotation storage scope <!-- completed: 2026-11-17T10:16 -->
- [x] Add the §13 sweep and diagnostic tests <!-- completed: 2026-11-17T10:16 -->

### Step 8: Runtime instructions and extensions

- [x] Replace the `review_stalled_input` instruction in `src/broker/runtime/events.rs` with the §8 text and update its test <!-- completed: 2026-11-17T10:16 -->
- [x] Update the `cafleet_listen` descriptions in `assets/extensions/opencode/cafleet.ts`, `assets/extensions/hermes/cafleet/__init__.py` and `assets/extensions/pi/cafleet.ts`, and the pi member-gate allowlist and refusal text <!-- completed: 2026-11-17T10:16 -->

### Step 9: Verification

- [x] Run `mise //:test`, `mise //:lint` and `mise //docs:build`; inspect the rendered HTML of any changed piped table cell <!-- completed: 2026-11-17T13:45 -->
- [x] Rerun the Step 1 inventory pattern and confirm that every remaining hit is one Step 1 recorded as kept <!-- completed: 2026-11-17T12:59 -->
- [x] Before release, run the live subscribe probe (§11) on the installed native version of every backend with a Director `listen` row (claude, codex, opencode, hermes, pi), and record each run's mechanism, version and check date in SPEC §6.7 Role eligibility and in the coding-agents.md Basis cells <!-- completed: 2026-11-17T12:59 -->
- [x] Before release, run a live stalled-head recovery check on a member whose composer holds retained CAFleet text (a `left_in_composer` or `uncertain` blocker on a verified-composer run). Confirm that `input_delivery_stalled` reaches the Director after 3 minutes with the `review_stalled_input` instruction, that the Director submits the retained literal with its `press_argv` (`member press ... Enter`), and that the held composer clears and the waiting head and later queued inputs deliver in order. Record the backend, native version and date <!-- completed: 2026-11-17T12:59 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-17 | Initial draft |
| 2026-11-17 | Review round 1: keep the hold ceiling re-notice for unpolled IDs; runner sweep submits through the control-socket path with run-wide backoff and strict oldest-first order; `message_consumed` publishes no event; live listen re-check on every backend before release; complete removal inventory and SPEC section list |
| 2026-11-17 | Stalled preview head recovery: member press or user clearing, no replacement command; restated `review_stalled_input` instruction |
| 2026-11-17 | Review round 2: clientless `pending_sweep` slot; retried-preview set suppressed from every origin through one durable-field predicate shared with the unpublished-result scan; stalled head delivers after clearing unless already consumed |
| 2026-11-17 | User approval addition: live check and unit test for `input_delivery_stalled` recovery through `member press` |
| 2026-11-17 | Implementation complete: `mise //:test` passes with `RUST_TEST_THREADS=4` (1930 passed); at default parallelism under host load average ~150, up to three timing-bound `runtime::input` tests fail and pass alone, outside the code this design changes |
