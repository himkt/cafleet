# Composer submit reliability

**Status**: Approved
**Progress**: 0/65 tasks complete
**Last Updated**: 2026-11-14

## Overview

Pushed previews and `member prompt` / `member exec` input are sometimes typed into a member's composer without being submitted (GitHub issue #603). This design removes each of the four causes: pointer input during the settle, native paste thresholds, PTY short writes, and the Hermes deletion composer clear. It also stops one stopped delivery from blocking later deliveries: the runner records a typed-but-unsubmitted literal as a terminal `left_in_composer` outcome and re-delivers the preview automatically.

## Success Criteria

- [ ] SGR or X10 pointer reports (motion, wheel, click) that reach the outer pane during the settle leave a profile delivery `submitted` on every bundled backend, on tmux and herdr (the issue's `herdr_mouse.py` / `matrix.py` matrix).
- [ ] A pointer report still revokes a permission-response token, still stops a gated answer plan between stages, and stops a multi-chunk literal stage between chunks.
- [ ] On a verified-composer run, keyboard or paste input during the settle produces `settled/left_in_composer` with `stop_cause=manual_input`, not `uncertain`. Later previews to that member are never `held` because of it. The stopped preview is re-delivered automatically once the composer is verified empty, unless its message was ACKed first.
- [ ] `message send` prints a stderr diagnostic for every attempted message whose delivery is left in the composer or held by its own uncertain or abandoned receipt. The Director receives one `input_delivery_result` event for every stopped staged write, deferred or immediate.
- [ ] Previews above the profile's measured `plain_literal_limit` (codex 1024 bytes; hermes the measured scalar boundary) are submitted: by bracketed paste when the profile declares it and the child has enabled mode 2004, otherwise by the pointer form. The concrete checks are the § Measurement acceptance sizes, codex 1037 bytes and hermes 2057 characters. An oversized `member prompt` / `member exec` with no safe mechanism is refused with zero bytes.
- [ ] A submit key that does not take effect is reported as `left_in_composer` with `stop_cause=submit_unconfirmed` after a 2-second post-submit verification, not as `submitted`.
- [ ] A 4000-scalar Japanese preview (about 12 KB) is `submitted` on claude, opencode and pi: positive short writes continue until the literal is complete, within the five-second attempt deadline.
- [ ] `member delete` / `fleet delete` of a Hermes member whose composer holds a draft of up to 8192 scalars completes its work settlement.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass. Every new test is an in-file unit test. The native fixtures and acceptance records below are captured on the evidence versions.

---

## Background

Evidence versions (issue #603): cafleet 0.35.0, tmux 3.6a, herdr 0.9.5, claude 2.1.319, codex 0.167.0, opencode 2.2.5, pi 1.5.0, hermes 2026.9.24. These versions are installed under `~/.local/share/mise/installs`. A multiplexer server started with an older `PATH` can resolve older installs, so every capture first verifies the launched version from a pane of the capturing server (Step 13). Reproduction scripts are in `/tmp/cafleet-repro-scripts/` (`matrix.py`, `herdr_mouse.py`, `probe_size.py`, `codex_size.py`).

| Cause | Today's code path | Effect |
|---|---|---|
| A | The reply classifier counts mouse reports as manual input, so they advance `human_generation`. `arm_pending` (`src/runtime/input.rs`) compares that generation at the settle check and stops with `manual_input` | `uncertain`. The delivery drain (`src/delivery.rs`) breaks with `Held` on any earlier uncertain preview receipt, and the runner FIFO stops until `input reconcile` |
| B | Codex (above 1024 bytes) and Hermes (above about 2048 scalars) treat a fast plain literal as a paste, so the submit key inserts a newline or is swallowed | Receipt `submitted`; the literal stays in the composer; the next preview queues as `composer_busy` and is delivered after 3 minutes by `idle_timeout` fallback on top of the retained text |
| C | `Transport::write_programmatic` makes one `try_write`; a positive count below the stage length becomes `uncertain/short_write` | Previews above about 9–10 KB on claude, opencode and pi |
| D | Hermes settlement clears the composer with one batch of `Escape` + 60 `Backspace` (`CLEAR_KEYS`, `src/coding_agent/hermes.rs`) and then requires it to be empty | `the Hermes composer still holds text after clearing` for a longer draft |

The runner's `vt100` screen already tracks the child's bracketed-paste mode (DEC private mode 2004, `Screen::bracketed_paste`), so the runner can tell whether the child accepts a bracketed paste at write time.

---

## Specification

### Terms

| Term | Meaning |
|---|---|
| Keyboard input | Outer-terminal bytes classified as manual input: keys, controls, human bracketed paste and every other non-reply, non-pointer form. Counted by `human_generation` |
| Pointer input | Complete SGR (`CSI < … M`, `CSI < … m`) and X10 (`CSI M Cb Cx Cy`) mouse reports outside a human bracketed paste. Counted by the new `pointer_generation` |
| Literal stage | The operation prefix and the literal, optionally framed as a bracketed paste, written as one or more chunks |
| Complete prefix | A written prefix of the literal stage that ends on a UTF-8 scalar boundary of the literal and outside both paste markers. A bracketed literal whose opening marker was written also has its closing marker written (§ Close stage) |
| Verified-composer run | A run whose selected profile declares empty-composer observation and an executable plan for the operation (`prompt` for previews and prompts, `exec` for native commands). The runner and every client derive it from the run's recorded backend and native version |
| Stopped staged write | An ordinary input (`preview`, `prompt`, `exec`) that wrote at least one byte and ended without a confirmed submission |
| `left_in_composer` | A terminal disposition on a verified-composer run: every write result was recorded, the written bytes form a complete prefix, and either the submit key was not written or its effect was not confirmed. Its durable `left_stage` records what was written (§ Stop outcomes) |
| Held composer | A runner state set when it settles a `left_in_composer` input with a `left_stage` other than `escape_only`, or advances its FIFO past an `uncertain` head. The input that set it is the blocking input. A fresh observation of a verified empty composer clears it |
| Preview attempt | One receipt for one preview of one message in one run. The attempt number starts at 1 |

### Constants

| Name | Value | Use |
|---|---|---|
| Settle interval | 500 ms (unchanged) | After the last literal chunk, before the submit key |
| Post-submit verification window | 2 s, reserved in full before the submit-key stage | After the submit-key stage, on verified-composer runs |
| Paste close reserve | 250 ms | The chunk loop stops 250 ms before the attempt deadline, so a bracketed paste can always be closed |
| Attempt deadline | 5 s (unchanged) | Covers all literal chunks, the settle, the submit key and the verification |
| Stalled-head bound | 3 minutes | A queue head waiting only on a held composer publishes `input_delivery_stalled` |
| Hermes clear round | Exactly 16 batches of 64 keys (`Escape` + 63 `Backspace` first, then 15 × 64 `Backspace`), sent back to back, then one pause and one capture | Clears up to 1023 scalars per round |
| Hermes clearable draft | 8192 scalars | The largest draft the clear loop is specified to clear: 9 rounds, 144 batches, within 10 s of the 20 s settlement budget |

### A. Pointer input and keyboard input during a staged write

#### Classification

The terminal reply classifier resolves each byte into exactly one of three classes. Only the pointer class is new.

| Class | Counter | Forms |
|---|---|---|
| Reply | none | The existing allowlist (unchanged) |
| Pointer | `pointer_generation` | A complete SGR report `CSI < L(3,3) M` or `CSI < L(3,3) m`. An X10 report: `CSI M` with no parameter or intermediate bytes, followed by exactly three bytes in `0x20`–`0xFF` |
| Manual | `human_generation` | Everything else, including mouse forms inside a human bracketed paste |

The X10 form adds one classifier state, `x10`, entered on the final byte `M` of a parameterless CSI. It consumes exactly three bytes. A byte below `0x20`, or the 100 ms pending bound expiring first, resolves the pending bytes as manual, as for every other prefix. Pointer input passes to the child unchanged; tick pacing still counts every byte read.

#### Where each counter applies

| Consumer | Keyboard (`human_generation`) | Pointer (`pointer_generation`) |
|---|---|---|
| Checks between chunks of a multi-chunk literal stage | yes | yes |
| The settle check and the check before the submit key | yes | no |
| Ordinary-input drain proofs (`preview`, `prompt`, `exec` stage validation) | yes | no |
| Permission-response token minting and revocation, and the checks between gated answer stages | yes | yes |
| Permission-response drain proofs | yes | yes |
| `member press` accounting | yes (press sends keys only) | — |

Pointer bytes forwarded between chunks would land inside CAFleet's own open bracketed paste as literal text, or a click would move the cursor so the remaining chunks land mid-literal. That is why the checks between chunks count pointer input. After the last chunk, a forwarded pointer report cannot change what the submit key submits.

`DrainedInput` and the drain result carry both counters. `validate_ordinary` compares `human_generation` only; `validate_permission` compares both. The permission token binds both counters. A permission response stopped by pointer input records `manual_input`, as for keyboard input.

#### Stop outcomes for ordinary input

A check before a stage that finds a blocker stops the remaining stages. On a verified-composer run, when every write result of the attempt was recorded and the written bytes form a complete prefix, the runner records `settled/left_in_composer` itself. It does so in the same transaction that clears the in-memory write authority, and no `input reconcile` is needed. Otherwise it records `uncertain`. On a run that is not a verified-composer run, every stop records `uncertain` with the same `stop_cause`. Those runs also keep today's member-wide hold and FIFO stop (§ Runs without composer verification).

| `stop_cause` | Ordinary outcome on a verified-composer run | Recorded when |
|---|---|---|
| `manual_accounting_failed` | `left_in_composer` | Manual-input accounting failed at a check |
| `manual_input` | `left_in_composer` | `human_generation` changed since the literal stage began (or since the Escape stage, for fallback), at any check |
| `pointer_input` | `left_in_composer` | `pointer_generation` changed since the literal stage began, at a check between literal chunks |
| `interaction_candidate` | `left_in_composer` | A blocking interaction held as a candidate at a check |
| `interaction_open` | `left_in_composer` | Any other blocking interaction at a check |
| `observer_unavailable` | `left_in_composer` | Observer unavailable, gap or diagnostic at a check |
| `deadline_expired` | `left_in_composer` | The chunk loop reached its bound (attempt deadline minus the paste close reserve) at a complete prefix, or the remaining budget before the submit-key stage is below the verification window plus `STAGE_WRITE_MARGIN` |
| `submit_unconfirmed` | `left_in_composer` | The post-submit verification window ended without confirmation (§ B) |
| `short_write` | `uncertain` | A stop between literal chunks, for any cause above or at the chunk-loop bound, left the written prefix inside a UTF-8 scalar or inside a paste marker |
| `write_failed` | `uncertain` | A PTY write error, unrecorded possible I/O, a failed post-write checkpoint, a failed or short close stage, a positive short submit key, a failed deferred attempt or a failed permission-response attempt |
| `runner_stopped` | `uncertain` | Runner shutdown ended an active attempt that may have written a child byte |

At a check, the first matching row among the six check causes (`manual_accounting_failed` through `observer_unavailable`), in table order, wins. The check applies the `short_write` override after choosing the cause. Permission responses keep `uncertain` for every cause; their claims and reconciliation are unchanged.

The runner records what a `left_in_composer` input wrote in the durable `left_stage` column, in the same transaction as the disposition:

| `left_stage` | Written bytes | Diagnostic wording |
|---|---|---|
| `escape_only` | Only the fallback Escape stage (`1b`); a stop during the first fallback settle | `Escape was sent to member MEMBER_ID but the input was not typed` |
| `partial` | A complete prefix shorter than the whole literal stage; no submit key | `partly typed into member MEMBER_ID's composer and not submitted` |
| `literal_complete` | The whole literal stage, including a closed paste; no submit key | `typed into member MEMBER_ID's composer but not submitted` |
| `submitted` | The whole literal stage and the submit key, with the effect not confirmed | `typed into member MEMBER_ID's composer and its submit key sent, but the submission was not confirmed` |

The stage vocabulary of submission replies and `input_delivery_result` becomes `zero_bytes`, `escape_only`, `partial`, `literal_complete`, `submitted`, `unknown`. An in-progress or uncertain input keeps reporting `partial` for any written prefix, as today. An `escape_only` stop typed no text, so nothing typed is re-delivered. The preview's re-delivery attempt is an ordinary delivery of the preview under the same verified-empty rule as every other re-delivery (§ Re-delivery).

#### Close stage

A stop after a bracketed literal's complete opening marker, and before its closing marker, writes the closing marker `1b5b3230317e` as a mandatory close stage within the paste close reserve. The close stage is written even when the written prefix ends inside a scalar, so that the child leaves paste mode. The outcome is then `uncertain/short_write`. A stop whose prefix ends inside the opening marker writes no close stage and records `uncertain/short_write`. A failed or short close write records `uncertain/write_failed`. The runner records the outcome only after the close stage completes.

#### Keyboard input during the settle

The runner stops before the submit key (as today), records `left_in_composer` / `manual_input` with `left_stage` `literal_complete`, and leaves the literal and the human's text in the composer for the human. A preview is then re-delivered (§ Re-delivery). `member prompt` and `member exec` are never retried automatically: they exit 1 with the left-in-composer error (§ CLI output), and later inputs proceed.

### Re-delivery and per-message holds

#### Preview attempt keys

Preview operation keys become `preview:<run_id>:<message_id>:<attempt>`. A new column `preview_attempt` stores the attempt number, and the row is unique on `(run_id, message_id, preview_attempt)`. The payload digest preimage is unchanged (`run_id,kind,message_id,literal`), so every attempt of one message carries the same digest under a distinct key.

#### Latest-attempt rule

Every sender, `message wait` rotation and runner decision reads the latest attempt of a message in the current run and applies the first matching row:

| Latest attempt | Verified-composer run | Other runs |
|---|---|---|
| None | Start attempt 1 | Start attempt 1 |
| `ready` with deferred metadata | Queued; skip this message for this invocation | The same |
| `ready` without deferred metadata (`held` / `zero_bytes_ready`) | Reuse the same attempt and key; re-evaluate eligibility under it as SPEC §6.9 Durable input receipts allows; never start N+1 | The same |
| `in_progress` or `uncertain` | Per-message hold: print its held diagnostic and continue with the next pending message | End the drain (today's member-wide hold) |
| `accepted` or `settled/submitted` | Stamp or verify ACK and settle (unchanged) | The same |
| `settled/not_submitted` | Start attempt N+1 automatically | The same |
| `settled/abandoned_uncertain` | Per-message hold; the Director ended its retries | End the drain |
| `settled/left_in_composer` | Invariant violation: the runner creates the successor in the same transaction. Fail with `preview input INPUT_ID was left in the composer without a re-delivery attempt` | Cannot occur (`left_in_composer` exists only on verified-composer runs) |

The `settled/not_submitted` row starts an automatic N+1 after any `not_submitted` disposition, including a Director `input reconcile` of an attempt that may have written bytes. The Director's evidence-based disposition, or the runner's zero-byte cancellation (`delivery_budget_exhausted`, `pane_input_busy`, `target_changed`, `plain_limit_exceeded`, …), establishes that the message was not delivered, so the next send or `message wait` delivers it again under a fresh key. This replaces two current contracts:

- SPEC §6.9 Durable input receipts, "Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation". For a preview, the next send or wait starts the next attempt key; prompt and exec keep the sentence.
- The drain in `src/delivery.rs`, which holds on any settled non-`submitted` disposition. With it, SPEC §6.9 Runner-owned deferred delivery's "A preview settled not submitted remains an unnotified broker message and may be retried explicitly under a new run's preview key" becomes "is delivered again under the same run's next attempt key".

Only the per-message holds of this table join the drain loop's skip set: an `in_progress`, `uncertain` or `abandoned_uncertain` latest attempt on a verified-composer run, or the runner's `uncertain` / `left_in_composer` reply to this invocation's attempt on such a run. The drain then continues with the next oldest pending message. Every other `Held` end still ends the drain, as today:

- a runner `SubmissionStatus::Held` or `Rejected` result (no deliverable pane, observer unavailable, ownership or storage unavailable);
- a placement or run change;
- every hold on a run that is not a verified-composer run.

On a verified-composer run the runner FIFO also advances past an `uncertain` head and sets the held composer, with that head as the blocking input. Every later write requires a verified empty composer (rule 3), so a composer holding CAFleet's partial bytes keeps later input queued, not held. Rule 1 of § Delivery method and interaction priority checks only the operation's own key for an uncertain prior attempt.

#### Held composer

While the held composer is set, every queue head is exempt from the rule-4 `idle_timeout` bound, whether it is a `redelivery` head or ordinary input queued behind an `uncertain` head's bytes. No fallback submission therefore mixes new bytes with CAFleet's left bytes. Clearing or submitting the composer, so that a fresh observation verifies it empty, clears the held composer and resumes profile delivery. A detector that never again verifies the composer empty would leave the head waiting; the stalled-head event (§ Stalled heads) gives the Director a bounded signal and its recovery. Rule 4 applies unchanged whenever the held composer is clear, for example to a human draft.

#### Re-delivery

On a verified-composer run, the runner settles preview attempt N as `left_in_composer` and, in the same immediate transaction, inserts attempt N+1 as `ready` with `deferred_at` and the new `defer_reason` `redelivery`. It places the canonical base preview at the head of its FIFO, which preserves message order. The re-delivery head:

- waits only for a verified empty composer and is then delivered by profile delivery. The rendering, framing and method are selected afresh at zero bytes;
- sets the held composer, so it is exempt from the rule-4 `idle_timeout` bound and never typed on top of the retained text. Later previews and inputs wait behind it as `earlier_input` until the composer is cleared or submitted;
- is cancelled as `not_submitted` / `message_consumed` once its message is ACKed, and as `not_submitted` / `target_inactive` on target removal. The runner checks the head's message and target on every queue turn (each trigger and each 100 ms timer pass of § Runner-owned deferred delivery), not only immediately before writing; this applies to every queued preview head;
- is settled `not_submitted` / `runner_lost` by runner loss; a new run then delivers the message under its own key.

The number of attempts is not capped. Each re-delivery waits for a verified empty composer, so a loop advances only as fast as someone clears or submits each left literal.

A `left_in_composer` result whose `left_stage` is `literal_complete` or `submitted`, and whose cause is `observer_unavailable`, `deadline_expired` or `submit_unconfirmed`, leaves exactly the input's whole literal in a composer no human is expected to clear. Its `input_delivery_result` carries a `director_action` with the exact recovery command `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Enter` (§ `input_delivery_result`). Every other `left_in_composer` result carries `director_action` null:

- interaction stops, because Enter would answer the open form instead of the composer;
- `manual_input` and `manual_accounting_failed`, because a human may be at the pane;
- `partial` and `escape_only` stages, because the composer holds a truncated prefix or no CAFleet text.

The runner itself never sends a second submit key and never clears the composer.

#### Stalled heads

When a queue head has waited the stalled-head bound only on a held composer, the runner publishes one `input_delivery_stalled` event for that head. Time behind an interaction, or before the held composer was set, does not count. The event carries no literal and is deduplicated by run and head input ID. It names the waiting head and the blocking input and carries a `director_action` with the recovery:

1. Capture the member's pane and inspect the blocking input with `cafleet input show BLOCKING_INPUT_ID --json`.
2. When the composer holds the blocking input's whole literal and `member show` reports no current interaction, submit it with `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Enter`. Otherwise ask the user to clear or submit the composer on the member's native surface.
3. Reconcile an `uncertain` blocking input with `input reconcile` after that evidence.
4. When the composer is cleared but never verifies empty, or the waiting message no longer needs delivery, ACK the waiting preview's message with `cafleet message ack MESSAGE_ID`; the runner cancels the head as `not_submitted` / `message_consumed` on the next queue turn.

A composer that never verifies empty after it was cleared is a composer-rule defect of the selected profile: later heads stall in turn, each with its own event, until the profile is corrected.

#### Runs without composer verification

A run that is not a verified-composer run (unknown coverage, a failed version probe, or no executable plan for the operation) keeps today's protection:

- every stop records `uncertain` with its `stop_cause`;
- no post-submit verification runs, and the submit key records `accepted` as today;
- nothing is re-delivered automatically;
- the runner FIFO stops at an `uncertain` head;
- the drain ends with a member-wide hold until `input reconcile`.

Pointer classification, short-write continuation and the selection table of § B still apply on these runs.

### B. Paste thresholds, bracketed framing and post-submit verification

#### Profile declaration

Each `capabilities.composer.submission.prompt` and `.exec` object gains two required keys:

```json
{
  "prefix_hex": "",
  "submit_hex": "0d",
  "literal_framing": "bracketed_paste",
  "plain_literal_limit": {"unit": "bytes", "max": 1024},
  "fixture_ids": ["…"]
}
```

| Key | Values | Meaning |
|---|---|---|
| `literal_framing` | `plain` or `bracketed_paste` | `bracketed_paste` only where fixtures show that a bracketed literal followed by `submit_hex` submits at every measured size up to the maximum preview |
| `plain_literal_limit` | `null` or `{"unit": "bytes" \| "scalars", "max": N}` with N ≥ 1 | The largest plain literal (prefix plus literal) measured to submit. `null` when the maximum preview submits plainly |

Bracketed framing writes the prefix plainly, then `1b5b3230307e`, the literal and `1b5b3230317e`, as one literal stage. The encoded ordinary-submission limit (65536 bytes) counts the 12 marker bytes. Unknown keys still fail catalog loading. The build guard requires every bundled profile with an executable composer plan to declare empty-composer observation, and requires each plan's fixture list to cite the framing, limit, post-submit and unsubmitted-literal fixtures (§ Measurement).

#### Selection at zero bytes

Before the first byte, and again on every zero-byte reselection, the runner renders the line for the selected method and applies the first matching row:

| Condition | Literal stage |
|---|---|
| `literal_framing` is `bracketed_paste` and the child screen has mode 2004 enabled | Bracketed |
| `plain_literal_limit` is `null`, or the rendered prefix plus literal is within it | Plain |
| Preview over the limit | Re-render the preview in the pointer form `[cafleet msg …] Body is N characters; run cafleet message show MESSAGE_ID --json to read it. [cafleet] Resume …` and write it plainly |
| `member prompt` / `member exec` over the limit, at its immediate attempt | Zero-byte refusal with the pre-write error (§ CLI output); no receipt is reserved |
| `member prompt` / `member exec` over the limit, at a queued drain | Settle the queued receipt `not_submitted` with `delivery_reason` `plain_limit_exceeded`, publish its `input_delivery_result`, and advance the FIFO |

Fallback delivery uses the same table for its literal stage. A run without a selected profile (unknown coverage) writes plainly with no limit, as today. The framing and the inline/pointer form are outside the digest preimage, like the method and the appended clause. They are pinned once a byte is written. The 4000-scalar inline limit is unchanged.

#### Post-submit verification

On a verified-composer run, the submit-key stage starts only when the remaining attempt budget is at least the full verification window plus `STAGE_WRITE_MARGIN`. Otherwise the attempt stops before the submit key with zero submit bytes, as `left_in_composer` / `deadline_expired` with `left_stage` `literal_complete`.

The same zero-byte budget check, `stage_settle() * settles + verification window + STAGE_WRITE_MARGIN`, gates the first byte of every attempt on a verified-composer run:

| Attempt | Check today | Zero-byte result when the remaining budget is below the check |
|---|---|---|
| Deferred queue head (`src/runtime/input.rs`, the `stage_settle() * settles + STAGE_WRITE_MARGIN` pre-check) | Settles only; the window is added | The head keeps waiting with zero bytes under obstacle `delivery_budget_exhausted` and retries on the timer |
| Immediate attempt (`message send`, `message wait`, `member prompt`, `member exec`) | None; the check is new | The runner accepts the input into its queue with zero bytes and the new `defer_reason` `delivery_budget_exhausted`. The CLI reports `queued`, and the runner retries it with its own five-second attempt budget |

After the complete submit-key stage, the runner keeps the pane lock and the active input. It confirms the submission when, within the window, a fresh observation shows either of these:

- a verified empty composer (`InputEligibility::ComposerEmpty`, not visible, no retained draft or evidence, no diagnostic), or
- a newly opened detected interaction (not a candidate) whose episode began after the submit-key stage.

Confirmation records `accepted` and then follows the existing settlement. When the window ends without confirmation, the runner records `left_in_composer` / `submit_unconfirmed` with `left_stage` `submitted`, and a preview is re-delivered. Keyboard input during the window is forwarded to the child. A key typed there leaves the composer non-empty, so it produces `submit_unconfirmed` and later a duplicate delivery (§ Accepted trade-offs).

### C. Short-write continuation

A positive short write in the literal stage keeps stage `partial`, retains the unwritten suffix in memory and returns to `AwaitBeforeWrite` with no settle. Each later chunk:

1. Runs a fresh ordinary drain and the checks between chunks (keyboard, pointer, interaction, observer, accounting), using the stop outcomes above.
2. Writes the remaining bytes through a newly consumed write guard. A `WouldBlock` or `Interrupted` with zero bytes yields within the same deadline, as today.
3. Adds its actual count to the retained cumulative count.

The 500 ms settle starts when the last chunk, including a closing paste marker, completes. The chunk loop stops at the attempt deadline minus the paste close reserve. A stop there records `left_in_composer` / `deadline_expired` at a complete prefix, and `uncertain/short_write` otherwise (§ Close stage). The submit-key stage is one write: a positive short submit key is `uncertain/write_failed`.

### D. Hermes deletion composer clear

#### Composer region

One SGR-carrying capture is read per check. Its rows are indexed identically in the SGR-carrying text and in its ANSI-stripped form. The borders are located on the stripped rows:

- the bottom border is the last row consisting only of `─` across the pane width;
- the top border is the nearest row above it that begins and ends with `─` across the pane width, with an embedded title allowed (rows 32 and 34 in `hermes-2026.9.24-composer-prompt-literal-tmux-120-1`);
- the region is the rows strictly between the two borders.

The composer is empty when the region is exactly one row whose stripped text starts with `❯ `, and when the SGR-carrying capture of that same row shows its remaining cells as blank or italic placeholder text, which is the existing `composer_row_empty` SGR parse applied to that row. A region of several rows, blank rows included, is non-empty. The progress comparison compares the region's stripped text between captures. The emptiness check (`composer_empty`, replacing `composer_row_empty`'s bottom-most `❯ ` row read) and the progress comparison both use this region.

When the 2057-character and 8192-scalar fixtures show a layout these border rules do not bound, the implementer adjusts the region definition to the fixtures and records the adjusted rules in `SPEC.md` § Hermes work settlement.

#### Clear loop

The settlement's clear step becomes a loop, bounded by the 20-second settlement budget:

1. Capture the region. If the composer is empty, continue with the command.
2. Send one clear round: exactly 16 batches sent back to back with no pause between them, the first `Escape` + 63 `Backspace` and the other 15 each 64 `Backspace`. The round size is fixed because the settlement does not count the draft's scalars, and the measurement records that Backspace on an empty Hermes composer has no effect. A measured native clear sequence replaces the round only when a fixture proves it (below).
3. Pause once (`POLL_INTERVAL`, 200 ms), recapture, and compare the region's stripped text with the previous capture. An unchanged region fails with the existing `the Hermes composer still holds text after clearing` cause. Repeat from step 1.

A round removes up to 63 + 15 × 64 = 1023 scalars, so an 8192-scalar draft needs 9 rounds, or 144 batches. The loop is specified to clear it within 10 s on tmux and herdr: 144 `send-keys` invocations of at most 50 ms each, plus 9 pauses and captures. That leaves at least 10 s of the 20 s budget for `/stop` and `/exit`, whose submit loops take at most three 1 s windows each. Their own clear steps find an empty composer after one capture. Budget expiry fails with the existing settlement error.

The existing `CLEAR_KEYS` comment records an earlier finding that the Hermes composer has no line-kill key, so the measurement expects `Ctrl+U` to fail. It records `Ctrl+U` and `Ctrl+A` then `Ctrl+K` as evidence. A sequence replaces the Backspace round only when a fixture shows that it empties a multi-row composer of 8192 scalars without ending the client.

### CLI output, events and errors

#### Delivery projection

`submission.status` and `delivery.status` each gain `left_in_composer`. The receipt projection table gains this row:

| Receipt | `submission.status` | Stage without cache | `delivery.status` | Method and reason |
|---|---|---|---|---|
| `settled/left_in_composer` | `left_in_composer` | The recorded `left_stage` | `left_in_composer` | Recorded selection |

Queue reasons gain `redelivery` and `delivery_budget_exhausted`; not-submitted outcome reasons gain `plain_limit_exceeded`. `unresolved_input_ids` is unchanged: a `left_in_composer` receipt is settled and is not listed, and its successor is listed while queued.

#### `stop_cause` and `left_stage`

`uncertain_cause` is renamed `stop_cause` everywhere: the column, the `StopCause` enum, the `input show` / `input reconcile` key (same position) and the documentation. It is required for `left_in_composer`, required on the transition to `uncertain`, and retained through settlement. The `input show` / `input reconcile` row appends `left_stage` after `stop_cause`; it is null unless the disposition is `left_in_composer`.

#### `message send` and `message wait`

The exit status is unchanged (0 after commit). The JSON `input_id` and `delivery` report the receipt of this invocation's attempt for the new message. For a left-in-composer preview, that is attempt N with `delivery.status` `left_in_composer`; its successor appears only in the diagnostic.

Each attempted message whose outcome is one of the three rows below prints one stderr diagnostic after `Message NEW_ID was persisted. `. Submitted and queued messages print none.

| Outcome | Diagnostic |
|---|---|
| Left in composer | The string for its stage, listed below the table; `INPUT_ID` is attempt N and `NEXT_ID` its `redelivery` successor |
| Held by its own uncertain receipt | `message MESSAGE_ID was not notified to member MEMBER_ID: input INPUT_ID is uncertain (STOP_CAUSE); inspect it with cafleet input show INPUT_ID --json and reconcile it with cafleet input reconcile.` |
| Held after `abandoned_uncertain` | `message MESSAGE_ID was not notified to member MEMBER_ID: input INPUT_ID was abandoned as uncertain; send its content again as a new message if it is still needed.` |

The exact left-in-composer strings, one per `left_stage`, are:

- `message MESSAGE_ID was partly typed into member MEMBER_ID's composer and not submitted: STOP_CAUSE (input_id=INPUT_ID, redelivery input_id=NEXT_ID); the runner re-delivers it once the composer is empty.`
- `message MESSAGE_ID was typed into member MEMBER_ID's composer but not submitted: STOP_CAUSE (input_id=INPUT_ID, redelivery input_id=NEXT_ID); the runner re-delivers it once the composer is empty.`
- `message MESSAGE_ID was typed into member MEMBER_ID's composer and its submit key sent, but the submission was not confirmed: STOP_CAUSE (input_id=INPUT_ID, redelivery input_id=NEXT_ID); the runner re-delivers it once the composer is empty.`
- `message MESSAGE_ID was not typed: Escape was sent to member MEMBER_ID but the input was not typed: STOP_CAUSE (input_id=INPUT_ID, redelivery input_id=NEXT_ID); the runner re-delivers it once the composer is empty.`

`message wait` records the same strings in `notification_errors`. `notification_held` keeps its other causes (no deliverable pane, deregistration, placement or run change). "An unresolved earlier preview receipt" remains a `notification_held` cause only on runs that are not verified-composer runs.

#### `member prompt` / `member exec`

| Condition | Error body (exit 1) |
|---|---|
| Prompt left in composer | `prompt delivery to member ID was not submitted: STOP_CAUSE (method=METHOD); the typed text stays in the member's composer. Inspect the pane before retrying.` |
| Exec left in composer | `native command delivery to member ID was not submitted: STOP_CAUSE (method=METHOD); the typed text stays in the member's composer. Inspect the pane before retrying.` |
| Left with stage `escape_only` | `prompt delivery to member ID was not submitted: STOP_CAUSE (method=fallback); Escape was sent but no text was typed. Inspect the pane before retrying.`, with the `native command delivery …` form for exec |
| Over the limit, profile framing `plain` | The existing pre-write form `prompt was not dispatched to member ID: CAUSE` / `native command was not dispatched to member ID: CAUSE`, with CAUSE `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the BACKEND profile has no bracketed-paste delivery` |
| Over the limit, profile framing `bracketed_paste`, mode 2004 off | The same pre-write form with CAUSE `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the native CLI has not enabled bracketed paste` |

UNIT is `bytes` for a `bytes` limit and `characters` for a `scalars` limit. The left-in-composer errors include `input_id=INPUT_ID`. A queued prompt or exec that reaches the over-limit refusal at drain time has already returned `queued`; its outcome is the `not_submitted` / `plain_limit_exceeded` receipt and its `input_delivery_result`. The uncertain errors are unchanged except that their example cause becomes `write_failed`. Both over-limit CAUSE strings are added verbatim to SPEC §6.3 Error Messages.

#### `input_delivery_result`

The runner emits one event for the first submitted, not-submitted, uncertain or left-in-composer result of a deferred input, and now also for every immediate stopped staged write (`left_in_composer` or `uncertain`). `cause` is `{"kind": STOP_CAUSE}` whenever the receipt records a `stop_cause`. Immediate `submitted` results still publish no event. The `delivery` projection covers `left_in_composer` and the `not_submitted` / `plain_limit_exceeded` refusal.

The runner derives `director_action` from the durable receipt, so live publication and recovery publication agree. Only a `left_in_composer` result whose `left_stage` is `literal_complete` or `submitted` and whose cause is `observer_unavailable`, `deadline_expired` or `submit_unconfirmed` (§ Re-delivery) carries:

```json
{"kind":"review_left_input",
 "instruction":"Capture the member's pane and confirm with member show that no interaction is current. When its composer holds exactly this input's whole text, submit it with press_argv; otherwise ask the user to clear or submit the composer on the member's native surface.",
 "press_argv":["cafleet","member","press","MEMBER_ID","--from-member-id","DIRECTOR_ID","Enter"]}
```

`press_argv` is built from the validated stored identities and appends `--database-url URL` when the fleet uses a non-default database. Every other `input_delivery_result`, including every interaction stop, keeps `director_action` null.

#### `input_delivery_stalled`

A new runtime event kind, published per § Stalled heads under the runtime-event contract. It creates no interaction row and carries no literal payload:

| Field | Value |
|---|---|
| `kind` | `input_delivery_stalled` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_lifecycle` |
| `decision_outcome` | `"unknown"` |
| `evidence` | The run's ordered evidence object with null screen-derived fields |
| `input_id` | The waiting head's input ID |
| `message_id` | The head's preview message ID, or null for prompt/exec |
| `blocking_input_id` | The held composer's blocking input ID |
| `waiting_since` | UTC RFC 3339 time when the head began waiting only on the held composer |
| `delivery` | The head receipt's ordered `status,method,reason` projection (`queued`) |
| `director_action` | `{"kind":"review_stalled_input","instruction":INSTRUCTION,"inspect_argv":["cafleet","input","show","BLOCKING_INPUT_ID","--json"],"press_argv":PRESS_ARGV}` |

INSTRUCTION states the four recovery steps of § Stalled heads. PRESS_ARGV is the `review_left_input` argv when the blocking input is `left_in_composer` with `left_stage` `literal_complete` or `submitted` and a cause that carries `review_left_input`, and null otherwise. The event is published once per head input; orderly shutdown and recovery do not republish it.

### Accepted trade-offs (SPEC §6.5)

- A re-delivery can duplicate a line: the human, or the Director's `press_argv` Enter, submitted the left text, and the composer then empties and the successor is delivered. Recipients dedupe by message ID.
- While the held composer is set, every queue head (a `redelivery` head, or input queued behind an `uncertain` head's bytes) is exempt from the `idle_timeout` bound. Later previews, prompts and execs to that member therefore wait until the composer is cleared or submitted and verifies empty. No fallback submission mixes new bytes with CAFleet's left bytes.
- A composer that never verifies empty again (a native UI change, or a layout the composer rule misreads) leaves the head waiting instead of being delivered by fallback. The bounded signal is `input_delivery_stalled` after 3 minutes. The recovery is to press Enter on a whole left literal, reconcile an uncertain blocking input, or ACK the waiting message, which the runner cancels on its next queue turn.
- A key typed during the post-submit verification window leaves the composer non-empty, which records `submit_unconfirmed` and leads to a duplicate delivery.

### Schema (`assets/migrations/V1__baseline.sql`, edited in place)

```sql
disposition TEXT CHECK(disposition IS NULL OR disposition IN
    ('submitted','not_submitted','abandoned_uncertain','left_in_composer')),
preview_attempt INTEGER CHECK((kind = 'preview') = (preview_attempt IS NOT NULL)
    AND (preview_attempt IS NULL OR preview_attempt >= 1)),
stop_cause TEXT CHECK(stop_cause IS NULL
    OR (stop_cause IN ('manual_input','manual_accounting_failed','pointer_input',
                       'interaction_candidate','interaction_open','observer_unavailable',
                       'deadline_expired','submit_unconfirmed','short_write','write_failed',
                       'runner_stopped')
        AND state IN ('uncertain','settled'))),
CHECK(disposition IS NOT 'left_in_composer'
    OR (kind IN ('preview','prompt','exec') AND stop_cause IS NOT NULL AND stop_cause IN
        ('manual_input','manual_accounting_failed','pointer_input','interaction_candidate',
         'interaction_open','observer_unavailable','deadline_expired','submit_unconfirmed'))),
left_stage TEXT CHECK(left_stage IS NULL
    OR left_stage IN ('escape_only','partial','literal_complete','submitted')),
CHECK((disposition IS 'left_in_composer') = (left_stage IS NOT NULL)),
CHECK(state != 'uncertain' OR stop_cause IS NULL OR stop_cause != 'submit_unconfirmed'),
UNIQUE(run_id, message_id, preview_attempt),
```

`member_runtime_events.kind` admits the new event kind:

```sql
kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured',
    'interaction_closed','automatic_refusal_observed','automatic_review_unavailable',
    'input_delivery_result','input_delivery_stalled','runner_degraded','runner_lost',
    'runner_exited')),
```

The `defer_reason` and `delivery_reason` columns stay free text. The runtime validates `redelivery`, `delivery_budget_exhausted` and `plain_limit_exceeded` with the existing reasons. `uncertain` accepts every cause except `submit_unconfirmed`, because runs that are not verified-composer runs record every check cause as `uncertain`. The setup shape guard's required column list is updated with the `stop_cause` rename and the two new columns, `preview_attempt` and `left_stage`.

### Measurement (bundled profile and fixture contract)

The implementer records new native fixtures on the evidence versions, inside each existing profile interval, so no `min_version` changes. Each fixture goes in `assets/profiles/native/<backend>.json`. Every fixture this design adds uses the new id convention `<backend>-<version>-composer-<op>-<case>-<mux>-<cols>-<n>`, for example `codex-0.167.0-composer-prompt-paste-submitted-herdr-120-1`. Existing fixture ids keep their current patterns. Each composer plan cites its fixtures. For every backend and both operations:

| Case | Required evidence |
|---|---|
| 2004 state | The child's mode 2004 at an empty composer, idle and working |
| Plain boundary | The largest plain literal that submits and the smallest that does not, one unit apart (bytes for codex, scalars for hermes; claude, opencode and pi up to the maximum preview) |
| Bracketed submit | Literal typed and submitted screens for a bracketed literal below the plain boundary, above it, and at the maximum preview (4000 Japanese scalars), each followed by `submit_hex` after the 500 ms settle |
| Post-submit | The screen within 2 s after the submit key, idle and working, observed as a verified empty composer |
| Unsubmitted literal | The screen after an ineffective submit key, or with an over-limit plain literal retained (for example codex's inserted newlines after `[Pasted Content 1024 chars]`, the Hermes long composer), observed as not a verified empty composer |
| Long literal | A 4000-scalar Japanese preview written in chunks, with literal-to-submit and submit-to-empty timings within the 5 s deadline |

`literal_framing` and `plain_literal_limit` are set from these results. Exec framing is measured separately: for example, whether `! ` followed by a bracketed command enters native shell mode. Hermes additionally records the clear-sequence candidates (§ D) and composers of 2057 characters and 8192 scalars.

Live acceptance, on tmux and herdr for every backend, repeats the issue's matrix and records the result in the acceptance record:

- hover, wheel, click and a key `x` at 0.3 s
- codex 1007 and 1037 bytes
- hermes 2027 and 2057 characters
- Japanese 7987 and 12085 bytes
- Hermes deletes with 2057-character and 8192-scalar drafts, with the 8192-scalar clear completing within 10 s

### Documentation surfaces

| Target | Change |
|---|---|
| `SPEC.md` §6.3 `message send` / Delivery outcome, `message wait`, `member prompt`, `member exec`, Runtime presentation, Runtime event and input commands, Error Messages | New diagnostics (one per `left_stage`, both input IDs) and errors (both over-limit CAUSE strings verbatim), `left_in_composer`, `left_stage` with `escape_only` / `partial` / `literal_complete` / `submitted`, the `stop_cause` and `left_stage` keys, `queued` results with `delivery_budget_exhausted`, `input_delivery_stalled` in event show |
| `SPEC.md` §6.5 Push notifications, Held delivery, Accepted trade-offs | Attempt keys, latest-attempt rule, per-message hold, pointer form over the plain limit, the four trade-offs of § Accepted trade-offs |
| `SPEC.md` §6.7 Hermes work settlement | Composer region, clear loop and clear rounds, measured clear sequence |
| `SPEC.md` §6.9 Executable native plans, Measured submission plans and settle intervals, Bundled profiles and native screen fixtures | `literal_framing`, `plain_literal_limit`, selection table, chunked literal, close stage, post-submit verification and its budget reservation, each profile's new declarations and fixtures, empty-composer requirement |
| `SPEC.md` §6.9 Terminal reply classification, Bounded input draining, Request-bound permission responses, Explicit manual key input | Pointer class and `pointer_generation`, the counter consumer table |
| `SPEC.md` §6.9 Durable input receipts | Stop outcomes, verified-composer runs, attempt keys, latest-attempt rule; the revised sentence "Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation" |
| `SPEC.md` §6.9 Delivery method and interaction priority, Runner-owned deferred delivery | Per-key rule 1, the held composer and its rule-4 exemption, ACK and target checks on every queue turn, stalled heads, the immediate-attempt budget check and `delivery_budget_exhausted`, FIFO advance past uncertain on verified-composer runs, `plain_limit_exceeded`; the revised sentence "A preview settled not submitted remains an unnotified broker message and may be retried explicitly under a new run's preview key" |
| `SPEC.md` §6.9 Delivery reporting, Uncertain cause (renamed Stop cause), Input delivery result event, new Input delivery stalled event, Native acceptance and coverage | Projection rows, `left_stage`, stage vocabulary, cause table, event scope, `review_left_input` and `review_stalled_input` actions, acceptance cases |
| `SPEC.md` §8 | DDL above: `member_input_operations` (`disposition`, `preview_attempt`, `stop_cause`, `left_stage`, CHECKs, UNIQUE) and the `member_runtime_events.kind` CHECK |
| `docs/docs/concepts/members.md` | Delivery paragraphs: pointer input no longer stops delivery; typing during delivery leaves the text and the runner re-delivers; later input waits while CAFleet text remains in the composer, and the Director is told when it stalls; long previews use paste or the pointer form; `stop_cause` |
| `docs/docs/concepts/coding-agents.md` | One sentence: a backend whose measured plain-input limit is below the longest preview receives long previews by bracketed paste or the pointer form; `SPEC.md` § Bundled profiles owns the per-backend values |
| `README.md` | No change expected; confirm |
| `assets/skills/cafleet/reference/cli.md` | Delivery selection paragraphs, uncertain/left errors, `input show` fields (`stop_cause`, `left_stage`) and dispositions, preview key form, cause table, stage vocabulary (`escape_only`, `literal_complete`), held reasons, queue reasons (`redelivery`, `delivery_budget_exhausted`) and outcome reasons (`plain_limit_exceeded`), the held composer, `review_left_input`, the `input_delivery_stalled` event and `review_stalled_input` |
| `assets/skills/cafleet/roles/director.md` | Receipt inspection: `stop_cause` and `left_stage`, left-in-composer results need no reconcile, re-delivery is automatic, the `review_left_input` press procedure, the `input_delivery_stalled` recovery (inspect, press, reconcile or ACK) |
| `.claude/rules/` | No change |

### Out of scope

Holding human keystrokes while a literal is written. A cap on re-delivery attempts. Changing the 4000-scalar inline limit. Answer-plan (permission response) outcomes other than pointer accounting.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [ ] §6.9 Terminal reply classification and Bounded input draining: pointer class, X10 state, `pointer_generation`, counter consumer table <!-- completed: -->
- [ ] §6.9 Request-bound permission responses and Explicit manual key input: pointer revokes tokens and stops gated stages; press accounting unchanged <!-- completed: -->
- [ ] §6.9 Executable native plans and Measured submission plans: `literal_framing`, `plain_literal_limit`, selection table, chunked literal, close stage, post-submit verification and budget reservation, constants <!-- completed: -->
- [ ] §6.9 Durable input receipts: verified-composer runs, stop outcomes, `left_stage` (`escape_only`, `partial`, `literal_complete`, `submitted`), attempt keys, latest-attempt rule, the revised not-submitted sentence <!-- completed: -->
- [ ] §6.9 Delivery method and interaction priority and Runner-owned deferred delivery: per-key rule 1, re-delivery, the held composer and its rule-4 exemption, queue-turn ACK checks, stalled heads, the immediate-attempt budget check, FIFO advance past uncertain, `plain_limit_exceeded`, the revised retry sentence <!-- completed: -->
- [ ] §6.9 Delivery reporting, Stop cause (renamed section and table), `left_stage`, stage vocabulary, queue reasons (`redelivery`, `delivery_budget_exhausted`) and `plain_limit_exceeded`, Input delivery result event (`review_left_input`), new Input delivery stalled event (`review_stalled_input`), Native acceptance and coverage <!-- completed: -->
- [ ] §6.9 Bundled profiles and native screen fixtures: per-profile framing/limit declarations, fixture id convention and empty-composer requirement (values filled in Step 13) <!-- completed: -->
- [ ] §6.5 Push notifications, Held delivery and Accepted trade-offs (four new trade-offs); §6.7 Hermes work settlement <!-- completed: -->
- [ ] §6.3 `message send`, Delivery outcome, `message wait`, `member prompt`, `member exec`, Runtime presentation, Runtime event and input commands, Error Messages: per-`left_stage` diagnostics with both input IDs, `left_in_composer` and `escape_only` errors, both over-limit CAUSE strings, `stop_cause` / `left_stage` keys, `queued` with `delivery_budget_exhausted`, `input_delivery_stalled` in event show <!-- completed: -->
- [ ] §8 DDL: `member_input_operations` (`disposition`, `preview_attempt`, `stop_cause`, `left_stage`, CHECKs, UNIQUE) and its semantics, and the `member_runtime_events.kind` CHECK <!-- completed: -->

### Step 2: User documentation

- [ ] Update the delivery paragraphs of `docs/docs/concepts/members.md` <!-- completed: -->
- [ ] Add the property-based paste-limit sentence to `docs/docs/concepts/coding-agents.md` <!-- completed: -->
- [ ] Confirm `README.md` needs no change <!-- completed: -->

### Step 3: Skills

- [ ] Update `assets/skills/cafleet/reference/cli.md` (delivery selection, errors, `input show` with `stop_cause` / `left_stage`, dispositions, stages including `escape_only` / `literal_complete`, key form, cause table, held reasons, queue reasons `redelivery` / `delivery_budget_exhausted`, outcome reason `plain_limit_exceeded`, the held composer, `review_left_input`, the `input_delivery_stalled` event and `review_stalled_input`) <!-- completed: -->
- [ ] Update receipt inspection (`stop_cause`, `left_stage`), reconciliation, the `review_left_input` press procedure and the `input_delivery_stalled` recovery (inspect, press, reconcile or ACK) in `assets/skills/cafleet/roles/director.md` <!-- completed: -->

### Step 4: Schema and storage

- [ ] Edit `assets/migrations/V1__baseline.sql` per § Schema, including the `member_runtime_events.kind` CHECK for `input_delivery_stalled`, and update the setup shape guard for `stop_cause`, `preview_attempt` and `left_stage` <!-- completed: -->
- [ ] Rename `UncertainCause` to `StopCause` with the new values; add `InputDisposition::LeftInComposer`, `preview_attempt`, and storage functions `record_input_left_in_composer` and `settle_left_and_queue_redelivery` (one immediate transaction) in `src/broker/runtime/` <!-- completed: -->
- [ ] Add latest-attempt lookup by `(run_id, message_id)` and the `preview:<run_id>:<message_id>:<attempt>` key builder; replace every `format!("preview:…")` site <!-- completed: -->
- [ ] Add the verified-composer run predicate, derived from the recorded backend and native version, shared by the runner and clients <!-- completed: -->

### Step 5: Pointer classification and counters

- [ ] Add the pointer class and `x10` state to `ReplyClassifier` and `pointer_generation` to `Transport`, `DrainedInput` and drain results (`src/runtime/transport.rs`) <!-- completed: -->
- [ ] Split drain-proof validation into ordinary (keyboard) and permission (keyboard and pointer) checks <!-- completed: -->
- [ ] Bind both counters in permission tokens and gated-stage checks, and count pointer input in the checks between literal chunks (`src/runtime/input.rs`) <!-- completed: -->

### Step 6: Staged write stops

- [ ] Route check and deadline stops of ordinary input to `left_in_composer` or `uncertain` per § Stop outcomes, with the complete-prefix and `short_write` override, the `escape_only` stage and the run predicate; keep permission responses `uncertain` <!-- completed: -->
- [ ] Add the close stage for open bracketed literals, including a prefix ending inside a scalar <!-- completed: -->

### Step 7: Short-write continuation

- [ ] Retain the unwritten suffix on a positive short literal write and continue chunks with fresh checks until complete or the deadline minus the close reserve <!-- completed: -->
- [ ] Start the settle after the last chunk <!-- completed: -->

### Step 8: Bracketed framing and plain limits

- [ ] Add `literal_framing` and `plain_literal_limit` to the composer plan schema, loader validation and build guard (`src/coding_agent/catalog.rs`) <!-- completed: -->
- [ ] Expose the child's mode 2004 from the transport screen to input selection <!-- completed: -->
- [ ] Implement the zero-byte selection table: preview pointer-form re-render, immediate prompt/exec refusal, and the queued `not_submitted` / `plain_limit_exceeded` settlement <!-- completed: -->

### Step 9: Post-submit verification

- [ ] Reserve the full window before the submit-key stage (`deadline_expired` otherwise) and extend the deferred-head pre-check at `src/runtime/input.rs` (`stage_settle() * settles + STAGE_WRITE_MARGIN`) to include it <!-- completed: -->
- [ ] Apply the same budget check before the first byte of an immediate attempt on a verified-composer run, queueing it with zero bytes under `defer_reason` `delivery_budget_exhausted` <!-- completed: -->
- [ ] Hold the active input for the window and confirm by verified empty composer or a newly opened interaction <!-- completed: -->
- [ ] Record `left_in_composer` / `submit_unconfirmed` with `left_stage` `submitted` when the window ends unconfirmed <!-- completed: -->

### Step 10: Re-delivery and per-message holds

- [ ] Settle `left_in_composer` previews and insert the `redelivery` successor at the FIFO head in one transaction (runner) <!-- completed: -->
- [ ] Implement the held composer (set by `left_in_composer` other than `escape_only` and by advancing past `uncertain`, cleared by a verified empty composer) and exempt every head from the rule-4 `idle_timeout` bound while it is set <!-- completed: -->
- [ ] Check the queue head's message ACK and target on every queue turn and cancel it as `not_submitted` / `message_consumed` or `target_inactive` <!-- completed: -->
- [ ] Publish `input_delivery_stalled` once per head after the stalled-head bound on a held composer, with `review_stalled_input` <!-- completed: -->
- [ ] Advance the runner FIFO past an `uncertain` head on verified-composer runs only <!-- completed: -->
- [ ] Apply the latest-attempt rule in `src/delivery.rs` and `src/cli/message.rs`: per-message holds join the skip set, every other `Held` ends the drain, the `ready` without deferred metadata row reuses its key, N+1 after `not_submitted` <!-- completed: -->

### Step 11: CLI output and events

- [ ] Add `left_in_composer` to `SubmissionStatus`, `DeliveryStatus` and the projection, `escape_only` and `literal_complete` to the stage vocabulary, the `left_stage` projection, and rename the `input show` key to `stop_cause` with `left_stage` after it (`src/output/json.rs`, `src/output/text.rs`) <!-- completed: -->
- [ ] Print the scoped `message send` diagnostics with both input IDs and record them in `message wait` `notification_errors` <!-- completed: -->
- [ ] Add the prompt/exec left-in-composer, `escape_only` and both over-limit errors (`src/cli/member.rs`, `src/runtime/input_client.rs`) <!-- completed: -->
- [ ] Publish `input_delivery_result` for immediate stopped staged writes, with `cause` from `stop_cause` and the `review_left_input` action derived from the durable receipt; add the `input_delivery_stalled` event kind (`src/broker/runtime/events.rs`) <!-- completed: -->

### Step 12: Hermes clear loop

- [ ] Implement the composer region and replace `composer_row_empty` with the region-based emptiness check (`src/coding_agent/hermes.rs`) <!-- completed: -->
- [ ] Replace the single clear batch with the round-based loop, one pause per round, and the region progress check <!-- completed: -->
- [ ] Use a measured clear sequence only when Step 13 establishes one <!-- completed: -->

### Step 13: Native measurement and bundled profiles

- [ ] Before each capture, run `--version` from a pane of the capturing multiplexer server and confirm the evidence version; when it differs, point that server's `PATH` at the evidence install under `~/.local/share/mise/installs` <!-- completed: -->
- [ ] Capture the § Measurement fixtures for claude, codex, opencode, pi and hermes on tmux and herdr, using the new id convention <!-- completed: -->
- [ ] Set `literal_framing` and `plain_literal_limit` for every bundled prompt/exec plan and cite the fixtures in `assets/profiles/bundled.json` <!-- completed: -->
- [ ] Record the Hermes clear-sequence candidates and the 2057-character and 8192-scalar composer fixtures <!-- completed: -->
- [ ] Fill the measured values into `SPEC.md` § Bundled profiles and native screen fixtures and § Hermes work settlement <!-- completed: -->

### Step 14: Unit tests (in-file `mod tests`)

- [ ] `src/runtime/transport.rs`: SGR/X10 reports advance `pointer_generation` only; split and expired X10 prefixes; mouse forms inside a human paste stay manual; ordinary proofs survive pointer input; permission proofs do not <!-- completed: -->
- [ ] `src/runtime/input.rs` stops: pointer during settle submits; pointer between chunks records `pointer_input` and closes an open paste; keyboard during settle and between chunks records `left_in_composer`; a prefix ending inside a scalar or paste marker records `short_write`; `escape_only` stop; deadline bound at a complete prefix records `deadline_expired` with `left_stage` `partial`; a stop after the whole literal records `literal_complete` <!-- completed: -->
- [ ] `src/runtime/input.rs` writes: chunked continuation completes; close stage written; bracketed selection gated on 2004; over-limit preview pointer form, immediate prompt refusal, queued prompt `not_submitted` / `plain_limit_exceeded` <!-- completed: -->
- [ ] `src/runtime/input.rs` verification: window reservation stops before the submit key; deferred-head pre-check includes the window; an immediate attempt below the budget is queued as `delivery_budget_exhausted` with zero bytes and later drains; confirmation and `submit_unconfirmed`; every Unsubmitted-literal fixture driven through post-submit verification yields `submit_unconfirmed`; pointer revokes permission tokens <!-- completed: -->
- [ ] `src/runtime/input.rs` queue: re-delivery successor at the FIFO head; with the held composer set, a `redelivery` head and a head behind an `uncertain` head get no `idle_timeout` fallback; a verified empty composer clears it and rule 4 applies again; an ACKed waiting head is cancelled on the next queue turn without a write; `input_delivery_stalled` fires once after the bound and not for time behind an interaction; FIFO advances past uncertain on verified-composer runs; a run without composer verification records `uncertain`, re-delivers nothing and keeps the FIFO stopped <!-- completed: -->
- [ ] `src/broker/runtime/inputs.rs`: schema CHECKs, including a `left_in_composer` row with NULL `stop_cause` or NULL `left_stage` rejected and a `left_stage` on another disposition rejected; atomic settle-and-queue; latest-attempt lookup; `stop_cause` and `left_stage` persistence <!-- completed: -->
- [ ] `src/broker/runtime/events.rs`: an `input_delivery_stalled` event row inserts under the edited `kind` CHECK and round-trips its payload; publication is deduplicated per head input <!-- completed: -->
- [ ] `src/delivery.rs` and `src/cli/message.rs`: per-message holds skip and continue, member-wide holds end the drain, `ready` without deferred metadata reuses its key, attempt N+1 after `not_submitted`, invariant failure on a successor-less `left_in_composer`, member-wide hold on runs without composer verification, scoped diagnostics with both input IDs <!-- completed: -->
- [ ] `src/coding_agent/catalog.rs`: new plan keys validated; build guard requires empty-composer observation and fixture citations <!-- completed: -->
- [ ] `src/coding_agent/hermes.rs`: region emptiness on the 2057-character fixture, with the `❯ ` row judged on its SGR capture (italic placeholder empty, typed text non-empty); progress compares stripped region text; clear loop empties an 8192-scalar draft in 9 rounds of exactly 16 batches (144 batches); fails on no progress and on budget expiry <!-- completed: -->
- [ ] `src/output/json.rs`, `src/broker/runtime/events.rs`, `src/cli/member.rs`: projection with `left_stage`, `stop_cause` key, `escape_only` / `literal_complete`, event scope; `review_left_input` only for `literal_complete` / `submitted` with a qualifying cause and null for interaction stops; `input_delivery_stalled` payload and argv; prompt/exec errors <!-- completed: -->

### Step 15: Verification

- [ ] Run `mise //:test`, `mise //:lint` and `mise //docs:build` <!-- completed: -->
- [ ] Run the live acceptance matrix (§ Measurement) on tmux and herdr and record it <!-- completed: -->
- [ ] Reinstall the CLI and confirm the issue's reproduction outline no longer reproduces any cause <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-14 | Initial draft |
| 2026-11-14 | Review round 1: verified-composer runs, complete-prefix rule, `pointer_input`, `escape_only`, re-delivery exempt from `idle_timeout`, `review_left_input`, window reservation, `plain_limit_exceeded`, Hermes region and rounds, fixture id convention |
| 2026-11-14 | Review round 2: held composer exempts every head from `idle_timeout`, queue-turn ACK cancellation, `input_delivery_stalled`, immediate-attempt budget check, durable `left_stage` with `literal_complete`, `review_left_input` scope, Hermes SGR row judgment and fixed 16-batch rounds |
| 2026-11-14 | Review round 3: `member_runtime_events.kind` CHECK admits `input_delivery_stalled`; both new columns in the shape guard; documentation targets and Step 1/3 tasks aligned with every round-2 fact |
