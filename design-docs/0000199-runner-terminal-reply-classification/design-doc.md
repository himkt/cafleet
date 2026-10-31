# Runner terminal-reply classification and uncertain-cause recording

**Status**: Approved
**Progress**: 14/43 tasks complete
**Last Updated**: 2026-10-31

## Overview

The wrapped-member runner counts every byte read from the outer terminal as manual input, so a terminal reply (cursor-position report, OSC color reply, focus event) arriving during the 500 ms settle abandons a profile delivery before its submit key (GitHub issue #526). This design classifies outer-terminal bytes so only manual bytes advance the manual-input generation for every consumer, and persists an enumerated `uncertain_cause` on each receipt the runner records as `uncertain` so the outcome can be attributed afterwards.

## Success Criteria

- [ ] A complete allowlisted terminal reply read from the outer terminal is forwarded to the child byte-for-byte and in order, and leaves `human_generation` unchanged.
- [ ] A profile delivery whose 500 ms settle receives only terminal replies writes its submit key and its receipt reaches `accepted`.
- [ ] A keystroke during the settle still stops the remaining stages, and the receipt is `uncertain` with `uncertain_cause = manual_input`.
- [ ] A read ending inside a possible reply prefix keeps the transport non-quiescent until the sequence completes or 100 ms pass since its last byte; an unresolved prefix (including a lone Escape key) then counts as manual input.
- [ ] Every runner-recorded `in_progress → uncertain` transition stores one `uncertain_cause` value from the enumerated set, and `record_input_state` refuses that transition without a cause.
- [ ] `input show` / `input reconcile` JSON append `uncertain_cause`; wrapped prompt/exec uncertain errors name the cause; an uncertain `input_delivery_result` event carries `cause: {"kind": UNCERTAIN_CAUSE}` derived from the durable receipt.
- [ ] `V8__uncertain_cause.sql` is the contiguous successor of V7 and preserves every row; the chain guard, head-shape guard and a successor test pass.
- [ ] `SPEC.md`, the affected `docs/` page and `skills/cafleet/reference/{cli,member-runtime}.md` describe the new contracts before code lands; `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build` pass.

---

## Background

`Transport::drain_outer` (tick path) and `Transport::quiesce_for_input` (drain path) both pass each outer read to `Transport::retain_human`, which adds the read length to `human_generation` before queueing the bytes for the child PTY. `InputCoordinator::progress_with_manual` mirrors each delta into `manual_generation` through `observe_manual`, which also clears `current_request`. Consumers of that generation:

| Consumer | Effect of a generation change |
|---|---|
| Staged composer settle check (`InputPhase::AwaitBeforeWrite` with `settle_until`) | `composer.manual_generation != Some(self.manual_generation)` stops the remaining stages through `stop_composer_stages`, recording `uncertain` |
| Drain proof (`DrainedInput`, `validate_observation_drain`, `fresh_drain`) | A changed generation invalidates the proof, so no programmatic write proceeds on it |
| Permission-response token | `observe_manual` clears `current_request`, revoking response authority |
| `member press` accounting | Pressed keys arrive through the outer pane and are accounted as manual input |

Today a child-issued query reply or a focus report is indistinguishable from a keystroke. In the issue's run the preview receipt went `uncertain` 44 ms after creation with no runtime event, and the receipt carries no record of why. The existing `delivery_reason` column cannot hold the cause: SPEC pins it to null for profile delivery and requires the selection reason to persist unchanged after a positive write.

---

## Specification

### 1. Terminal-reply classification

#### 1.1 Placement and invariants

A new `ReplyClassifier` in `cafleet/src/runtime/runner/transport/replies.rs` (declared `mod replies;` in `transport.rs`, beside `mod drain;`) is owned by `Transport`. `retain_human` feeds each positive outer read to the classifier and advances `human_generation` by the number of bytes the classifier resolves as manual.

- **Forwarding is unchanged.** Every read byte is queued for the PTY immediately, unchanged and in order. The classifier never holds, reorders, drops or synthesizes bytes; it stores only parse state and a pending byte count. Forwarding therefore stays unframed, with no holding and no timeout.
- **Classification is accounting only.** It parses escape sequences and applies its own 100 ms bound (1.5) solely to decide how many forwarded bytes count toward `human_generation`. SPEC's terminal-transport Framing row is rewritten accordingly (Step 1): forwarding stays unframed, and accounting classification is specified in the new `Terminal reply classification` subsection.
- **One definition of manual input.** `human_generation` is the cumulative count of manual bytes. Every consumer in the Background table uses it unchanged, so terminal replies affect none of them.
- `TransportProgress.human_bytes` keeps reporting bytes read, so tick pacing is unchanged.

#### 1.2 Classifier interface

```rust
pub(super) struct ReplyClassifier { /* parse state, pending count, last-byte instant, paste flag */ }

pub(super) struct Classified {
    pub(super) manual: usize,   // bytes resolved as manual input by this call
}

impl ReplyClassifier {
    /// Classifies one read; bytes still inside a possible reply stay pending.
    pub(super) fn feed(&mut self, bytes: &[u8], now: Instant) -> Classified;
    /// Resolves a prefix whose last byte is at least REPLY_PREFIX_BOUND old as manual.
    pub(super) fn expire(&mut self, now: Instant) -> Classified;
    /// Resolves any pending prefix as manual (outer EOF).
    pub(super) fn finish(&mut self) -> Classified;
    pub(super) fn pending(&self) -> bool;
}

const REPLY_PREFIX_BOUND: Duration = Duration::from_millis(100);
const CSI_PARAMETER_LIMIT: usize = 64;
const STRING_BODY_LIMIT: usize = 4096;
```

Every byte fed is resolved exactly once: as manual (counted), as part of a complete reply (not counted) or still pending. The `manual` counts of `feed`, `expire` and `finish` sum to the bytes resolved as manual. Methods take `now` explicitly so classifier tests drive time with literal instants; 1.5 specifies how transport-level tests age a pending prefix.

#### 1.3 Reply grammar (allowlist)

A sequence is a terminal reply only when it is complete and matches one row below. All bytes are 7-bit; `ESC` is `0x1b`, `CSI` is `ESC [`, `ST` is `ESC \`, `BEL` is `0x07`. Spaces in the byte forms are for readability only. Two productions are shared by every row:

- `Pd`: one or more ASCII digits (`0x30`–`0x39`).
- `L(min,max)`: between `min` and `max` `Pd` values joined by single `;` bytes, with no empty parameter. For example, `L(1,3)` matches `8`, `8;24` and `8;24;80`, and rejects the empty string, `8;` and `;24`.

| Reply | Byte forms | Notes |
|---|---|---|
| Cursor position report | `CSI L(2,2) R`, `CSI ? L(2,3) R` | Collides with modified F3; accepted, see 1.5 |
| Device status | `CSI 0 n`, `CSI 3 n`, `CSI ? L(1,4) n` | The `?` form covers private status reports such as the color-scheme report `CSI ? 997 ; 1 n` |
| Device attributes | `CSI ? L(1,32) c`, `CSI > L(1,4) c` | Primary and secondary; the tertiary reply is a DCS |
| Mode report (DECRPM) | `CSI ? L(2,2) $ y`, `CSI L(2,2) $ y` | |
| Window report | `CSI L(1,3) t` | Covers single-parameter state reports (`CSI 1 t`, `CSI 2 t`) and size reports (`CSI 8 ; 24 ; 80 t`) |
| Kitty keyboard flags | `CSI ? L(1,1) u` | `CSI L(1,3) u` without `?` is a key event and stays manual |
| Focus | `CSI I`, `CSI O` | No parameters or intermediates |
| OSC reply | `ESC ]` body `BEL`, or `ESC ]` body `ST` | Any body within `STRING_BODY_LIMIT` |
| DCS reply | `ESC P` body `ST` | XTVERSION, XTGETTCAP, DECRQSS and tertiary device-attribute replies |
| APC reply | `ESC _` body `ST` | Kitty graphics-protocol query responses; the same termination rules as DCS |

Everything else counts as manual input, including:

| Input | Classification |
|---|---|
| Printable text, UTF-8, C0 controls (Enter, Tab, Ctrl-keys), DEL | Manual immediately |
| Any CSI not in the allowlist: arrow/function keys, `CSI L(1,2) ~`, kitty key events `CSI L(1,3) u` | Manual on its final byte |
| Mouse reports: SGR `CSI < … M` / `CSI < … m`, X10 `CSI M Cb Cx Cy` | Manual, because they are human pointer input (a click or scroll), even though no composer text changes |
| Bracketed paste `CSI 200 ~` … `CSI 201 ~`, including every byte between the markers | Manual, even when the pasted body contains allowlisted forms |
| SS3 `ESC O x` (application-mode keys), Alt-modified `ESC x` for any `x` other than `[`, `]`, `P`, `_` | Manual once the second byte arrives |
| SOS `ESC X` and PM `ESC ^` strings | Manual, through the Alt-modified row; terminals send no SOS/PM replies |
| 8-bit C1 bytes (`0x9b`, `0x9d`, `0x90`, `0x9f`) | Manual (ordinary bytes in UTF-8 mode) |
| A CSI whose parameter and intermediate bytes exceed `CSI_PARAMETER_LIMIT` | Manual at the byte that exceeds the limit |
| An OSC, DCS or APC body that exceeds `STRING_BODY_LIMIT` | Manual at the byte that exceeds the limit; a larger reply (for example an OSC 52 clipboard read) therefore counts as manual |

#### 1.4 Parse-state rules

The classifier is in exactly one state: ground, escape (after `ESC`), CSI, string body (OSC, DCS or APC), string escape (after `ESC` inside a body), or paste.

| State | Valid continuation bytes | Completes on |
|---|---|---|
| Escape | `[` → CSI; `]`, `P`, `_` → string body | — |
| CSI | Parameter bytes `0x30`–`0x3F`, then intermediate bytes `0x20`–`0x2F` | A final byte `0x40`–`0x7E`; the complete sequence is a reply when it matches an allowlist row, otherwise manual |
| String body | `0x20`–`0x7E` and bytes `≥ 0x80`; `BEL` for OSC only; `ESC` → string escape | `BEL` (OSC only) |
| String escape | `\` | `\` (ST) |

General rule: **any byte that is not a valid continuation for the current state resolves every pending byte as manual.** That byte is then classified again from ground: an `ESC` starts a new sequence, and any other byte counts as manual. This rule decides each boundary case:

| Input | Resolution |
|---|---|
| `ESC [ 1 ; ESC [ I` | `ESC [ 1 ;` is 4 manual bytes; `ESC [ I` is a focus reply |
| `ESC ESC [ I` (Alt-Escape or a stray Escape before a reply) | The first `ESC` is 1 manual byte; `ESC [ I` is a focus reply |
| A C0 control (including CAN `0x18` and SUB `0x1a`), DEL or a byte `≥ 0x80` inside a pending CSI | The pending bytes and that byte count as manual |
| `BEL` inside a DCS or APC body | Not a valid continuation: the pending bytes and the `BEL` count as manual |
| `ESC` inside a string body followed by any byte other than `\` | The pending bytes up to and including that `ESC` count as manual, and the following byte is classified again from ground |
| An `ESC x` key where `x` is not a valid escape continuation | Both bytes count as manual |

The paste state follows a manual `CSI 200 ~`: every byte counts as manual, and `CSI 201 ~` counts as manual and returns to ground. A sequence split across reads stays pending across `feed` calls, within one tick or across ticks.

#### 1.5 Pending prefix and the 100 ms bound

- `Transport` calls `classifier.expire(Instant::now())` at the start of `drain_outer` and at the start of every `quiesce_for_input` round, before reading, and adds the resolved manual count to `human_generation`.
- While `classifier.pending()`:
  - `quiesce_for_input` returns `InputDrain::Pending` with drain blocker `terminal reply pending` at the point where it would otherwise seal a proof.
  - `validate_observation_drain` fails with `input transport is no longer quiescent`.
  - No programmatic stage, permission answer or deferred write proceeds, because each requires a fresh drain proof.
- Outer EOF calls `classifier.finish()` before EOF is reported, so a pending prefix counts as manual.
- A lone Escape key is pending for at most 100 ms and then counts as manual. A staged delivery whose settle sees that key therefore stops with `manual_input` once the bound resolves it.
- The bound runs inside the active attempt's existing five-second deadline and adds no deadline of its own.
- The bound is measured from the prefix's last byte, so each new byte restarts it. `CSI_PARAMETER_LIMIT` and `STRING_BODY_LIMIT` cap how long a continuously fed prefix can stay pending.

**Testing the bound.** `ReplyClassifier` has a `#[cfg(test)] impl` block with `fn backdate(&mut self, by: Duration)`, which moves the pending prefix's last-byte instant earlier by `by` (checked subtraction). `Transport` has a matching `#[cfg(test)] pub(super) fn backdate_reply_prefix(&mut self, by: Duration)`. Transport, drain and input-coordinator tests call it with `REPLY_PREFIX_BOUND` before the next tick or drain, so the production `expire(Instant::now())` call resolves the prefix deterministically, with no sleeps. The runner's existing test clocks (`input_clock::test_support::ScriptClock`, the writer fixture `Clock`) model deadline conversion and event timestamps, not transport time, so they are not reused here.

**Accepted collision class.** Any human input whose bytes exactly equal an allowlisted reply and arrive within the bound counts as a reply, not as manual input. The only realistic member is a modified F3 key (`CSI 1 ; Pmod R`), which matches the cursor-position-report form. The other members need an exact multi-key sequence typed within 100 ms: Alt-`[` then `I`/`O` (a focus report), Alt-`]` … Ctrl-G (an OSC reply), and Alt-Shift-P or Alt-`_` … Alt-`\` (a DCS or APC reply). A colliding key is still forwarded to the child unchanged. The only effect is that it does not interrupt a staged delivery or revoke automatic authority.

**Rejected alternative: query-correlated classification.** One alternative counts a reply as non-manual only when the child's observed output contains a matching outstanding query: `CSI 6n` for a cursor-position report, `CSI c` / `CSI > c` for device attributes, an OSC `?` query for an OSC reply, or DECSET 1004 for focus reports. That removes most collisions. The user chose the grammar-only allowlist because correlation is less reliable in exactly the failure this issue reports:

- The multiplexer, not the child, can trigger replies. For example, tmux and herdr forward focus events under their own focus settings, and a query can be answered after a resize or alternate-screen switch the runner did not pair with it.
- Correlation would need bookkeeping of outstanding queries across the child-output and outer-input paths, plus expiry for queries that were never answered.
- Every correlation miss reintroduces an abandoned submit stage.
- The residual collisions above only spare a rare keystroke from interrupting a delivery, and its bytes still reach the child.

#### 1.6 New blocker reason

`terminal reply pending` joins the permission blocker reason table (SPEC §6.3, `skills/cafleet/reference/cli.md`): "An outer-terminal reply prefix awaits completion within its 100 ms classification bound."

### 2. Uncertain-cause recording

#### 2.1 Value domain

`UncertainCause` is a new `stored_enum!` in `cafleet/src/broker/runtime/records.rs` (column `member_input_operations.uncertain_cause`):

| Value | Recorded when |
|---|---|
| `manual_accounting_failed` | `manual_accounting_failed` is set when a staged stage stops |
| `manual_input` | The manual generation changed since the literal stage when a staged stage stops |
| `interaction_candidate` | A staged stage stops on `ordinary_blocker` = `Interaction` with eligibility `Hold(Candidate)` |
| `interaction_open` | A staged stage stops on `ordinary_blocker` = `Interaction` for any other reason (visible/obscured interaction, `Hold(Interaction)`, retained transition) |
| `observer_unavailable` | A staged stage stops on `ordinary_blocker` = `ObserverUnavailable` (`Hold(ObserverUnavailable \| Gap)` or an observation diagnostic) |
| `short_write` | A programmatic write returned a positive count smaller than its stage (`ProgrammaticWrite::Partial`) |
| `write_failed` | The attempt failed after a possible child byte: a PTY write error, a post-write checkpoint, manual-accounting or deadline failure, a failed deferred attempt, or a failed permission-response attempt |
| `runner_stopped` | Runner shutdown (`fence`) ended an active deferred attempt that may have written a child byte |

At the staged settle check, the first matching row among the first five, in table order, wins. Accounting failure comes first because the generation comparison is unreliable once accounting failed.

#### 2.2 Runner transition sites

Every runner call in `cafleet/src/runtime/runner/input.rs` that moves an input from `in_progress` to `uncertain` passes a cause:

| Call site | Cause |
|---|---|
| `stop_composer_stages`, called from the `AwaitBeforeWrite` settle check | Ordered selection from 2.1 rows 1–5 |
| `stop_composer_stages`, called after `written_stages > 0` with an `Interaction` blocker | `interaction_candidate` or `interaction_open` |
| Write-outcome persistence (`SubmissionStage::Partial \| Unknown → Uncertain`) | `short_write` for `Ok(ProgrammaticWrite::Partial)`, otherwise `write_failed` |
| `record_deferred_uncertain`, called from `retire_failed_deferred` | `write_failed` |
| `record_deferred_uncertain`, called from `fence` when the active deferred attempt may have written | `runner_stopped` |
| `retain_permission_failure` (stage not `ZeroBytes`) | `write_failed` |

`stop_composer_stages` and `record_deferred_uncertain` each gain a `cause: UncertainCause` parameter supplied by their caller. A helper `fn interaction_cause(status: &ObservationStatus) -> UncertainCause` shares the candidate test (`status.eligibility == InputEligibility::Hold(GateReason::Candidate)`) with the two existing defer-reason sites that choose between `interaction_candidate` and `interaction_open`.

#### 2.3 Storage API

- New `pub(crate) fn record_input_uncertain(conn, owner, input_id, cause: UncertainCause) -> Result<InputOperation, CafleetError>` in `cafleet/src/broker/runtime/inputs.rs`. It keeps every check `record_input_state` makes. It runs inside `owned_transaction` and loads the row through `owned_input`, which checks run ownership. It requires the loaded state to be `in_progress`, failing with `input state changed` otherwise. It then runs `UPDATE member_input_operations SET state='uncertain',updated_at=?1,uncertain_cause=?2 WHERE input_id=?3 AND state='in_progress'` and requires exactly one changed row, failing with `input CAS failed` otherwise. It returns `get_input` of the updated row.
- `record_input_state` no longer accepts `InProgress → Uncertain` and fails with its existing `invalid input transition`. Test-support callers that create uncertain fixtures switch to `record_input_uncertain`.
- `InputOperation` gains `uncertain_cause: Option<UncertainCause>`, read in `map_input`.
- `validate_input` requires `uncertain_cause` to be null unless `state` is `uncertain` or `settled`.
- Settlement (`settle_input_in_transaction`, reconcile) preserves the cause.
- Rows that never became uncertain, pre-V8 rows and `in_progress` rows left by runner loss keep a null cause.

#### 2.4 Migration `V8__uncertain_cause.sql`

```sql
ALTER TABLE member_input_operations ADD COLUMN uncertain_cause TEXT
  CHECK(uncertain_cause IS NULL
     OR (uncertain_cause IN ('manual_input','manual_accounting_failed','interaction_candidate',
                             'interaction_open','observer_unavailable','short_write','write_failed',
                             'runner_stopped')
         AND state IN ('uncertain','settled')));
```

The migration preserves the application and instance identities, every row, ID, sequence, foreign key and audit relationship; existing rows receive null. It needs no table rebuild. The production head-shape guard (`schema_matches_version`) compares against `reference_schema(version)`, which is built by running the embedded migrations in memory, so it covers V8 with no code change. Only the hand-written `member_input_operations` column string in the `db/mod.rs` tests gains `uncertain_cause`.

#### 2.5 Surfaces

| Surface | Change |
|---|---|
| `input show` / `input reconcile` JSON | Append `uncertain_cause` (string or null) after `payload_digest_version`. Text line unchanged. |
| Runner `SubmissionResult` for uncertain receipts | `input_client::durable_outcome` returns `reason = uncertain_cause.as_str()` for `in_progress`/`uncertain` rows when non-null, otherwise null. `validate_reply_against_durable` requires an uncertain reply's `reason` to equal the receipt's cause. |
| Wrapped `member prompt` / `member exec` uncertain error | Existing format gains the cause through `result.reason`: `exec delivery to member 110 is uncertain: manual_input (method=profile)` |
| `permission respond` uncertain reply | The live reply (through `durable_outcome`) and the historical/repeated reply (through `input_client::projection`) agree. Both report the receipt's recorded cause (`write_failed`) as `REASON` when it is non-null. When the receipt records no cause (pre-V8 rows, `in_progress` rows left by runner loss), `projection` keeps `prior transmission remains uncertain`. |
| `input_delivery_result` event | For an uncertain result whose receipt has a non-null cause, payload `cause` is `{"kind":"<uncertain_cause>"}`. Without a recorded cause, `cause` is absent. Every publisher derives it from the durable receipt: live `publish_result`, `service.rs` shutdown/startup publication, `recovery/replay.rs` and `recovery/exceptional.rs`. `unpublished_delivery_results` returns `(input_id, stage, Option<UncertainCause>)`. The event reader validates that `cause` matches the receipt. |
| `message send` output | Unchanged. The `delivery` projection keeps `status,method,reason`, and the cause is read from `input show INPUT_ID --json`. |

The live permission reply goes through `durable_outcome`, which returns null for an uncertain receipt without a cause. That case does not arise for live replies, because every live uncertain transition records a cause.

### 3. Out of scope

- Native profile changes or backports for any backend.
- Issue #527, handled separately.
- Real-PTY smoke runs. In-file unit tests drive every classification and cause path with literal bytes and backdated prefixes, so a native CLI adds no coverage this change needs.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] §6.9 *Terminal transport and lifecycle*: rewrite the Framing row so it says forwarding stays unframed, unchanged and in order with no holding or timeout, and point to the `Terminal reply classification` subsection for accounting <!-- completed: 2026-10-31T00:59 -->
- [x] §6.9 *Terminal transport and lifecycle*: replace "Every read advances the manual-input generation by its byte count" with the classification contract: only bytes classified as manual advance the generation. Add a `Terminal reply classification` subsection carrying the shared productions and allowlist table (1.3), the manual-input table, the parse-state table and general rule (1.4), the 100 ms pending bound, the EOF rule and the accepted collision class (1.5) <!-- completed: 2026-10-31T00:59 -->
- [x] §6.9 *Measured submission plans and settle intervals*: steps 3–4 and the following paragraph say "manual input" (terminal replies excluded); the uncertain paragraph states that the receipt records its `uncertain_cause` <!-- completed: 2026-10-31T00:59 -->
- [x] §6.9 *Bounded input draining*: `human_generation` is the cumulative manual-byte count; a pending reply prefix keeps the drain `Pending` with blocker `terminal reply pending` <!-- completed: 2026-10-31T00:59 -->
- [x] §6.9 *Explicit manual key input* and the permission-token paragraphs: the runner counts manual bytes of positive outer reads (terminal replies excluded) <!-- completed: 2026-10-31T00:59 -->
- [x] §6.3 permission blocker reason table: add the `terminal reply pending` row <!-- completed: 2026-10-31T00:59 -->
- [x] §6.3 `input show`/`input reconcile` key order: append `uncertain_cause`; wrapped prompt/exec uncertain errors include the cause <!-- completed: 2026-10-31T00:59 -->
- [x] §6.9 *Delivery reporting*: add an `Uncertain cause` subsection with the value table (2.1) and transition-site rules (2.2); the `uncertain` status row names the cause <!-- completed: 2026-10-31T00:59 -->
- [x] §6.9 *Input delivery result event*: the `cause` row becomes `{"kind": UNCERTAIN_CAUSE}`, present only for an uncertain result whose receipt records a cause and derived from the durable receipt by every publisher <!-- completed: 2026-10-31T00:59 -->
- [x] Runtime records: the input-operation row lists `uncertain_cause` and its nullability rule; the `cafleet_v8.db` chain sentence names V8; add a `V8 storage` paragraph <!-- completed: 2026-10-31T00:59 -->
- [x] §8: add `### V8 uncertain cause migration` with the DDL from 2.4; §9 testing strategy mentions terminal-reply classification and uncertain-cause recording <!-- completed: 2026-10-31T00:59 -->

### Step 2: docs/

- [x] `docs/docs/concepts/member-permission-events.md` § *Deliver generated input*: terminal replies and focus reports during the settle leave profile delivery intact, human input stops it as uncertain, and `input show INPUT_ID --json` reports the recorded `uncertain_cause`; § *Preserve a native request*: "human input" invalidating the token excludes terminal replies <!-- completed: 2026-10-31T01:00 -->

### Step 3: skills/

- [x] `skills/cafleet/reference/cli.md`: `input show` key order with `uncertain_cause`, the cause value domain, the uncertain error/status rows, the `input_delivery_result` `cause` sentence and the `terminal reply pending` blocker row <!-- completed: 2026-10-31T01:00 -->
- [x] `skills/cafleet/reference/member-runtime.md`: the manual-accounting paragraph says the runner counts manual bytes and ignores terminal replies; the inspection paragraph names `uncertain_cause` as attribution evidence before reconciliation <!-- completed: 2026-10-31T01:00 -->

### Step 4: Reply classifier

- [ ] Create `cafleet/src/runtime/runner/transport/replies.rs` with `ReplyClassifier`, `Classified`, `REPLY_PREFIX_BOUND`, `CSI_PARAMETER_LIMIT`, `STRING_BODY_LIMIT` and the `#[cfg(test)]` `backdate` method per 1.2–1.5; declare `mod replies;` in `transport.rs` <!-- completed: -->
- [ ] In-file tests: each allowlisted form, including APC and the single-parameter window report `CSI 1 t`, yields zero manual bytes; `L(min,max)` arity limits hold (`CSI 8;24;80;1 t`, `CSI 5 R` and `CSI 8; t` count as manual); each manual-table input counts its full length; modified-F3 `CSI 1;5R` classifies as a reply <!-- completed: -->
- [ ] In-file tests: bracketed paste containing `ESC[1;1R` and `ESC[I` counts every byte; a mixed read `ab ESC[I cd` counts 4; an overlong CSI and an OSC body over `STRING_BODY_LIMIT` count as manual; 8-bit C1 bytes count as manual <!-- completed: -->
- [ ] In-file tests for the general invalid-continuation rule, one per boundary-table row: `ESC [ 1 ; ESC [ I` counts 4; `ESC ESC [ I` counts 1; CAN, SUB, other C0 controls, DEL and a byte `≥ 0x80` inside a CSI count the prefix and that byte; `BEL` inside a DCS or APC counts as manual while `BEL` ends an OSC reply; `ESC` followed by a non-`\` byte inside an OSC counts as manual <!-- completed: -->
- [ ] In-file tests: a reply split at every byte boundary across `feed` calls stays pending and counts zero on completion; a lone `ESC` and `ESC [ 12 ;` stay pending before 100 ms and resolve as manual at the bound through `expire`; `finish` resolves a pending prefix as manual <!-- completed: -->

### Step 5: Transport integration

- [ ] `Transport` owns a `ReplyClassifier`; `retain_human` feeds it and advances `human_generation` by the manual count only; `drain_outer` and each `quiesce_for_input` round call `expire` first; outer EOF calls `finish` before reporting EOF; add the `#[cfg(test)]` `backdate_reply_prefix` method <!-- completed: -->
- [ ] `quiesce_for_input` returns `Pending("terminal reply pending")` while `pending()`; `validate_observation_drain` requires `!pending()` <!-- completed: -->
- [ ] `transport.rs` tests: extend `runner_transport_forwards_one_outer_terminal_query_answer_without_parser_duplicates` to assert generation 0; a focus/OSC/CPR burst is forwarded unchanged with generation 0; `runner_transport_passes_escape_sequences_unframed` forwards the trailing `ESC [` immediately and counts it as manual only after `backdate_reply_prefix(REPLY_PREFIX_BOUND)` and the next tick; outer EOF with a pending prefix counts it; existing patterned-byte tests keep their generation assertions <!-- completed: -->
- [ ] `transport/drain.rs` tests: a pending prefix makes the drain `Pending` with the new blocker and no proof; after completion a quiet drain seals a proof with an unchanged generation; a proof taken before a reply prefix fails validation while it is pending; after `backdate_reply_prefix(REPLY_PREFIX_BOUND)` the next drain resolves a lone `ESC` as manual <!-- completed: -->

### Step 6: Storage and migration

- [ ] Add `cafleet/migrations/V8__uncertain_cause.sql` (2.4); extend the hand-written `member_input_operations` column string in the `db/mod.rs` tests with `uncertain_cause` <!-- completed: -->
- [ ] `db/mod.rs` tests: `runtime_v8_uncertain_cause_is_the_contiguous_successor_to_v7` migrates a V7 head with populated input rows and checks preserved rows and the new column; a constraint test rejects an unknown value and a cause on a `ready`, `in_progress` or `accepted` row; `runtime_v8_head_guard_refuses_a_v7_shape_recorded_as_v8` follows `runtime_v7_head_guard_refuses_a_v6_shape_recorded_as_v7` <!-- completed: -->
- [ ] Add `UncertainCause` to `records.rs`, the `uncertain_cause` field to `InputOperation`, the `map_input` read and the `validate_input` rule <!-- completed: -->
- [ ] Add `record_input_uncertain` (2.3); remove `InProgress → Uncertain` from `record_input_state`; move every `record_input_state` call whose `next` is `InputState::Uncertain` (found with grep across `cafleet/src`) to `record_input_uncertain` <!-- completed: -->
- [ ] `inputs.rs` tests: the cause persists and survives each reconcile disposition; `record_input_state` rejects `InProgress → Uncertain`; `unpublished_delivery_results` returns the cause <!-- completed: -->

### Step 7: Runner cause recording

- [ ] Add `interaction_cause` and the ordered settle-check selection; pass a cause through `stop_composer_stages` at both call sites <!-- completed: -->
- [ ] Record `short_write` / `write_failed` at the write-outcome site and `write_failed` in `retain_permission_failure`; give `record_deferred_uncertain` a cause parameter, with `write_failed` from `retire_failed_deferred` and `runner_stopped` from `fence` <!-- completed: -->
- [ ] `input.rs` tests: a staged profile delivery that receives `ESC[I`, `ESC[24;80R` or an OSC reply during the settle writes the submit key and ends `accepted` <!-- completed: -->
- [ ] `input.rs` tests: a keystroke during the settle ends `uncertain` with `manual_input` and reply reason `manual_input`; a lone Escape during the settle ends `uncertain` with `manual_input` after `backdate_reply_prefix(REPLY_PREFIX_BOUND)` and the next drain <!-- completed: -->
- [ ] `input.rs` tests, one per remaining cause:
  - accounting failure (`manual_generation = u64::MAX`), candidate, open interaction and observer unavailable;
  - short write (PTY `max_write`) and failed write;
  - failed deferred attempt and failed permission response;
  - `fence` during a written deferred attempt (`runner_stopped`). <!-- completed: -->

### Step 8: Surfaces

- [ ] `presentation.rs`: append `uncertain_cause` to the input JSON; extend the key-order test <!-- completed: -->
- [ ] `input_client.rs`: `durable_outcome` reason for uncertain rows and the `validate_reply_against_durable` reason check; in-file tests for both <!-- completed: -->
- [ ] `cli/member.rs`: test that a wrapped uncertain prompt/exec error names the cause and the method <!-- completed: -->
- [ ] `cli/runtime.rs`: test that an uncertain `permission respond` reply reports `write_failed` as its reason <!-- completed: -->
- [ ] `input_client.rs` `projection`: return the recorded cause as the uncertain reason when non-null, otherwise `prior transmission remains uncertain`; in-file tests show that a live and a repeated reply for the same uncertain permission receipt report the same reason, and that a cause-less receipt keeps the fixed string <!-- completed: -->
- [ ] `events.rs`: derive the uncertain `cause` from the receipt in `input_delivery_result_cause` callers; validate payload `cause` against the receipt; in-file tests for presence, absence and mismatch rejection <!-- completed: -->
- [ ] Pass the receipt cause through every publisher: `input.rs` `publish_result`, `service.rs`, `recovery/replay.rs`, `recovery/exceptional.rs`; extend their existing publication tests with a cause assertion <!-- completed: -->

### Step 9: Verification

- [ ] Run `mise //cafleet:test` and `mise //cafleet:lint`; both pass <!-- completed: -->
- [ ] Run `mise //docs:build`; it passes <!-- completed: -->
- [ ] Grep `SPEC.md`, `docs/` and `skills/` for "every read advances", "bytes actually read" and the `input show` key list, and confirm each matches the new contract <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-31 | Initial draft |
| 2026-10-31 | Review round 1: shared parameter productions, APC replies, invalid-continuation rule and string body limit, general collision class and rejected query-correlated alternative, `runner_stopped` cause from `fence`, ownership-preserving `record_input_uncertain`, head-guard and fixture-caller corrections, permission `projection` reason, deterministic prefix aging for tests |
