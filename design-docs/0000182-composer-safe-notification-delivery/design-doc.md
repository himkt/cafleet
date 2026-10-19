# Composer-safe Automatic Pane Delivery

**Status**: Complete
**Progress**: 14/14 tasks complete
**Last Updated**: 2026-10-18

## Overview

Permit automatic pane input only when fresh captured evidence identifies an empty composer and an idle turn. Apply this policy to every registered pane on both multiplexer backends, keeping work pending while a draft, active turn, permission prompt, or ambiguous display is present. Automatic input uses a submission path without Escape; capture-based checks reduce interference but cannot make capture and submission atomic.

## Success Criteria

- [x] Captures containing single-line, wrapped, or multiline operator drafts produce zero automatic input, including when the pending item is arbitrarily old.
- [x] Member previews, broadcast deliveries, broker notices, exec dispatch/resume, ping, and monitor wakes share the affirmative empty-and-idle eligibility rule for every coding agent in the supported-agent registry.
- [x] Deterministic tests verify a second capture immediately before submission and hold when that capture becomes unsafe or fails.
- [x] Automatic transport traces contain payload and submission operations, with zero Escape operations on either backend.
- [x] Held inbox messages remain available through `message poll`; submission or clearing of a draft permits later delivery once an empty idle composer is observed.
- [x] Current configuration, runtime schema, source, tests, and living documentation consistently describe indefinite safe holding and contain no forced-delivery feature.
- [x] Sanitized real captures support the parser rules, and manual herdr reproduction confirms preservation of existing drafts and ongoing turns under the documented race limitation.

---

## Background

Issue [#459](https://github.com/himkt/cafleet/issues/459) reports that member messages and member-exec completion notices are inserted into a Claude Director's unsent draft on herdr. The leading Escape has also interrupted a running Director turn.

`cafleet/src/pane_state.rs` currently accepts prompt-prefixed text as `Finished`; `cafleet/tests/pane_state.rs` even classifies a typed Claude draft as finished. `delivery.rs` additionally overrides non-finished states after a configurable timeout. Ping admits `Unclassified`, monitor wakes bypass capture classification, and every automatic transport starts with Escape. Fixing only the Claude prompt regex would leave these other interference paths intact.

The user selected conservative capture-based delivery across the full supported-agent registry and both backends, indefinite holding, and explicit acknowledgement of the remaining capture/send race. Explicit `member prompt`, pane creation, and shutdown remain direct-control operations with their existing semantics.

---

## Specification

### Safety boundary and observable guarantee

For an automatic write, both the initial observation and a fresh observation after the pane claim must classify as `Finished`. Here `Finished` means affirmative evidence of an empty current composer and an idle turn, rather than merely a line that resembles a prompt. Any recognized active-turn or permission cue overrides empty-composer evidence.

The guarantee is conditional on the captured terminal representation: an unsafe first or final observation results in zero input. An operator can begin typing, or the application can begin working, after the final capture; existing tmux/herdr operations provide no atomic check-and-submit operation. The pane claim provides time-bounded spacing between CAFleet writers, not a lock over the complete transport or operator keystrokes. Revalidate its ownership and lifetime after capture as specified below. Identical rendered placeholder and typed text can also be indistinguishable in a text-only capture. Record these limitations in the runtime delivery documentation and manual acceptance evidence.

Keep the final check adjacent to the first payload operation. Remove automatic Escape and its settle delay; retain the current payload submission delay unless backend-specific evidence justifies a separate change. Input arriving during that delay remains part of the documented race. Strict atomic non-interference, a new notification channel, and an application/multiplexer protocol extension are outside this scope.

### Capture classification

Extend `PaneState` with `Draft`. Keep `AwaitingUser`, `Working`, `Finished`, and `Unclassified`, and add the corresponding `HoldReason::Draft`. All consumers must match the complete enum; adding a state should expose incomplete handling at compile time.

| State | Required evidence | Automatic eligibility |
|---|---|---|
| `AwaitingUser` | Current permission or decision interface | Hold |
| `Working` | Current active-turn indicator, including a working view with queued composer text | Hold |
| `Draft` | Recognized current composer with entered content or observable additional continuation rows, including blank rows | Hold |
| `Finished` | Complete recognized current composer layout, empty content, recognized idle context, and no stronger cue | Eligible for final recheck |
| `Unclassified` | Incomplete, unsupported, ambiguous, or unrecognized layout | Hold |

Use this precedence: `AwaitingUser`, `Working`, `Draft`, `Finished`, `Unclassified`. Restrict cues to the current bottom interface, so historical prompts and transcript examples cannot establish idleness. Parse the composer region with agent-specific layout rules instead of searching independently for any prompt-like line in the last six nonblank lines. Preserve blank lines and continuation rows when recognizing its boundaries; stripping ANSI before parsing remains appropriate.

For each coding agent selected by the registry in `cafleet/src/coding_agent/mod.rs`, define a small parser of the complete bottom layout from sanitized real captures. Its empty test must inspect every composer row and recognize the accompanying idle layout. Known placeholder text qualifies only inside that recognized layout; an arbitrary suffix following a prompt glyph is draft content. A blank first row with nonempty following rows is `Draft`. Historical prompt lines, missing composer boundaries, truncated captures, unexpected footer rows, attachments or other composer decorations whose emptiness is unproven all hold. Whitespace-only entered content also holds whenever the capture can distinguish it from empty padding.

Maintain `classify(coding_agent: &str, content: &str) -> PaneState` as the shared classifier. Agent-native lifecycle status alone cannot prove composer emptiness. Keep the bounded `DELIVERY_CAPTURE_LINES` capture; a composer extending beyond the available window is incomplete and therefore held. A new or changed UI layout can reduce notification liveness until its parser is updated; conservative holding is the specified behavior.

Acceptance fixtures must record agent/version, multiplexer, terminal dimensions, state, and expected classification. Collect empty, single-line draft, wrapped/multiline draft, permission or decision, and working examples for each supported agent before enabling its `Finished` recognition. Existing synthetic placeholders are test inputs, not sufficient evidence of the real interface. If evidence for an agent layout is unavailable, its unproven layout remains `Unclassified` and the release's all-agent acceptance criterion remains incomplete; report that gap rather than inventing an empty rule.

### Shared preflight and final check

Introduce a reusable capture/classification helper in `delivery.rs`, shared by queued delivery, ping, and monitor wake. It returns either safe observation or a typed hold reason; `CaptureFailed(String)` preserves the capture error. Reuse `DELIVERY_CAPTURE_LINES` and `strip_ansi` uniformly.

Every automatic path follows this order:

1. Resolve the live target and coding agent, validate the operation's data, and build its payload before taking the claim.
2. Apply existing in-flight exec exclusion and operation-specific readiness rules.
3. Capture and classify. Continue only for `Finished`.
4. Acquire the pane spacing claim with a current timestamp, retaining the exact stored `keystroke_at` value and the monotonic acquisition time as an in-memory claim receipt. A refused claim means `Busy` and zero input.
5. Capture and classify again after the claim. Continue only for `Finished`; an unsafe or failed observation holds. Metadata values may change between two independently safe observations. Leave the spacing claim to expire normally and keep the item pending.
6. Validate the claim receipt's ownership and lifetime, then conditionally stamp the still-owed item in the same short database transaction. An expired/superseded claim or refused item stamp means zero input. Immediately submit through the automatic transport after committing the transaction.
7. Retain existing success bookkeeping. On transport failure, restore the item's pending notification/dispatch stamp as currently supported and return the concrete error through the existing caller contract.

Add an automatic-claim API in `broker/panes.rs` that returns the receipt for its successful conditional update; keep explicit direct-control callers' existing boolean API. Inject a clock exposing fresh UTC and monotonic time into the automatic orchestration so capture duration is observable in tests. Start the monotonic lifetime measurement immediately before the claim update, making database wait time count toward the budget. Before stamping/sending, require the stored `keystroke_at` to equal the receipt, monotonic elapsed time to be strictly less than `KEYSTROKE_SPACING_SECONDS`, and fresh UTC to remain within the same spacing window. A backwards UTC clock or expiration fails this check. Use an immediate SQLite transaction for the ownership read and owed-item stamp, preventing another claim update between them; ping and wake perform the same validation transaction without an item stamp. Commit before terminal I/O. Database failures retain their concrete error diagnostics.

An invalid receipt returns `Busy` without changing item or wake-success stamps. Leave the existing claim row intact: clearing it could erase another writer's claim. A later attempt starts with a new initial observation and claim; it cannot reuse the old final capture or reacquire and send immediately. In particular, a final capture lasting three seconds or longer holds even when it returns `Finished` and no other writer has appeared.

The observations and receipt validation establish eligibility, not a transaction with the terminal. Another CAFleet writer can claim after the spacing window expires, including while a slow transport is still submitting; the validation-to-send gap and transport-duration overlap remain within this design's bounded best-effort guarantee. A transport can partially insert text before failing, and an Enter timeout can leave submission outcome uncertain. Preserve diagnostics and existing retry behavior; the next attempt still requires both safe observations and a valid new receipt. A partially inserted payload normally becomes a draft and holds further automatic input. This change does not claim exactly-once submission or recovery from ambiguous transport results.

### Integration interfaces

The shared clock lives in `time.rs`; automatic claim APIs live in `broker/panes.rs`; observation and caller orchestration live in `delivery.rs`.

```rust
pub trait Clock {
    fn utc_now(&self) -> DateTime<Utc>;
    fn monotonic_now(&self) -> std::time::Instant;
}
pub struct SystemClock;

pub fn claim_automatic_pane(conn: &mut Connection, member_id: i64, clock: &dyn Clock)
    -> Result<Option<PaneClaim>, CafleetError>;
pub fn validate_automatic_claim(tx: &rusqlite::Transaction<'_>, claim: &PaneClaim, clock: &dyn Clock)
    -> Result<bool, CafleetError>;
pub fn observe_pane(io: &dyn PaneIo, pane_id: &str, coding_agent: &str) -> Result<(), HoldReason>;
pub fn claim_and_recheck(conn: &mut Connection, io: &dyn PaneIo, clock: &dyn Clock, target: &PaneTarget)
    -> Result<Result<PaneClaim, HoldReason>, CafleetError>;
pub enum PingOutcome { Sent, Held(HoldReason) }
pub fn ping_pane(conn: &mut Connection, io: &dyn PaneIo, clock: &dyn Clock, target: &PaneTarget)
    -> Result<PingOutcome, CafleetError>;
```

`PaneClaim` is opaque to callers and retains member identity, the exact stored timestamp, claimed UTC and monotonic acquisition instant. A refused conditional claim returns `None`; a database failure remains an error. `claim_and_recheck` performs receipt acquisition and the final observation after a caller's initial safe observation. Every automatic caller prepares its operation first and excludes an in-flight exec before observing. Conditional stamp helpers accept `&Connection`, allowing the caller's immediate transaction to perform ownership validation and the owed-item stamp without nesting transactions.

Replace the final `now: DateTime<Utc>` argument of `deliver_pane` and `monitor_tick` with `clock: &dyn Clock`; production passes `SystemClock`. Test clocks share controlled UTC and monotonic offsets with capture hooks. `MonitorMux` retains its `PaneIo` relationship. Add `PaneIo::send_automatic_trigger(&self, pane_id: &str, payload: &str) -> bool` and the matching `Multiplexer` method for prepared ping/wake payloads. Keep existing poll/wake conveniences on that same prepared-payload transport and render the wake roster before observations. Step 3 verifies no-Escape transport traces independently.

### Automatic write paths and bookkeeping

| Entry point | Eligibility and held behavior | Success bookkeeping |
|---|---|---|
| Message send/broadcast and persisted broker notices | Use `deliver_pane`; preserve oldest pending preview selection and `notification_sent: false` when held | Stamp `notified_at` only for an eligible attempted send; preserve existing send-error rollback |
| Monitor owed-work pass | Apply the same `deliver_pane` path to every live target including Director | At most one selected item per pane per pass |
| Exec dispatch | Keep queued until both observations are safe; a running exec continues to exclude other automatic input | Stamp `dispatched_at` through the existing conditional update |
| Exec resume | Keep the existing grace period and completion semantics; suppress a redundant resume when an observed `Working` turn already consumed command output | Otherwise stamp `resumed_at` only on safe delivery; a draft or unknown state keeps resume pending |
| `member ping` | Use the shared checks; return `skipped: true` with `draft`, `unclassified`, `awaiting_user`, `working`, `exec_running`, or `busy` as appropriate | Preserve its existing JSON shape and immediate, nonqueued semantics |
| Scheduled or explicitly requested monitor wake | Obtain the monitor member's coding agent, exclude an in-flight exec, and use the shared checks | Stamp `last_wake_at` and clear a wake request only after successful submission; a held wake stays due |

Capture failure remains an error for explicit ping, matching its existing CLI behavior; queued delivery returns a typed hold, and monitor wake logs a capture diagnostic and leaves the wake due. A failed wake send retains existing failure reporting and leaves the wake due. A held wake does not stop heartbeats or the owed-work delivery pass. Multiple ticks while held coalesce into one due wake rather than accumulate payloads. A requested wake bypasses scheduling delay, and still passes the full safety gate.

Retain exec ordering (finished resume, queued dispatch, pending preview), the resume grace period, and all message/ACK contracts. A draft can hold a completed exec's resume and consequently `member exec --wait` for an unbounded period; safe automatic input and explicit user resolution take priority over bounded completion. Reading and ACKing an inbox message through `message poll`/`message ack` can consume a held preview without any pane input.

### Transport separation

Keep `Multiplexer::send_prompt` for explicit `member prompt`. Add `Multiplexer::send_automatic_prompt(pane_id, text)` for exec dispatch/resume and expose that operation through `PaneIo`; `Owed::send` uses it. Preserve single-line/nonempty validation and concrete backend errors for both prompt APIs. A low-level automatic sender assumes the caller has completed the shared safety checks; route all automatic callers through those checks.

Change `send_inline_preview`, `send_poll_trigger`, and `send_wake_entries` to automatic no-Escape behavior. Keep their established payload bytes, multiline folding, sanitization, result types, and submission semantics. Payload validation and wake rendering complete before the final check, with no intervening capture-independent subprocesses between the final check and payload operation.

| Operation | tmux | herdr |
|---|---|---|
| Automatic single-line prompt, ping, wake | Literal `send-keys -l`, existing submit delay, `Enter` | `pane run` with validated payload |
| Inline preview | Literal multiline payload, existing submit delay, `Enter` | `pane send-text`, existing submit delay, `pane send-keys ... enter` |
| Explicit prompt and shutdown | Existing direct-control behavior | Existing direct-control behavior |

Factor backend helpers by the intended operation so the no-Escape automatic path is apparent in the API and tests. Explicit direct-control sends retain their existing safeguard. Pane spawn/bootstrap writes start the agent process and remain lifecycle actions; audit all multiplexer write call sites to verify that every runtime notification/trigger is covered by the automatic path.

### Indefinite holding and configuration removal

Remove `Settings::delivery_hold_timeout`, `CAFLEET_DELIVERY_HOLD_TIMEOUT`, the timeout gate, hold-age calculation, forced-delivery notes, `PaneOutcome::Fired`'s `forced` field, forced-preview logging, and `stamp_forced`. `PaneOutcome::Fired` contains only `item`. Remove obsolete note plumbing from inline-preview interfaces and payload construction, whose production caller supplies forced-delivery notes. Keep unrelated payload resume clauses and schedule-request semantics only where they remain part of a current interface.

Remove `PaneTarget::forced_at` and its query mapping. Add the next contiguous forward migration, following the repository's migration rules, to execute `ALTER TABLE member_placements DROP COLUMN forced_at`; the bundled SQLite supports this operation. The inspected migration head is V9; recheck the head immediately before choosing the new filename. Preserve all retained columns, message rows, exec rows, and pane claims. Keep already-applied migrations unchanged for refinery's checksum chain; their references are historical migration records, like this design document, rather than runtime support for the removed feature. Update the migration-chain guard and verify upgrade of a populated database as well as fresh setup.

Remove corresponding settings tests, forced-delivery tests, docs-sync expectations, environment-variable rows, schema descriptions, and living documentation mentions. Describe current safe holding positively without deprecation notices or references to this design as a restoration plan. Audit repository-wide with symlink traversal where applicable, covering sources, tests, skills, examples, and configuration; historical design documents and immutable migration history are the bounded historical exceptions. Unknown environment variables follow the existing settings lookup behavior; this change adds no special compatibility parser or removed-variable error.

### Verification and operational behavior

Use the existing fake `PaneIo`, fake monitor multiplexer, and command-runner seams. Extend fakes with sequential capture results, an injectable UTC/monotonic clock, and a complete event trace so tests assert both observation order and zero input on every unsafe branch. Deterministically advance time during final capture to the spacing boundary and beyond, then verify zero input and unchanged owed-item/wake stamps. Also replace `keystroke_at` through a second connection while final capture is outstanding and verify that a superseded receipt holds even if the returned capture is empty and idle. Cover a timely unchanged receipt, backwards UTC, and a retry requiring fresh captures. These regressions apply to queued delivery, ping, and wake; document that expiration after validation can still allow transport overlap rather than presenting the receipt as a full-duration lock.

| Test surface | Required evidence |
|---|---|
| Classifier | Real capture fixtures for every registry agent; empty/current versus historical prompt; draft, whitespace where observable, wrapped/multiline and blank-first-line input; active and permission precedence; unknown/truncated layout holds |
| Queued delivery | Every owed-item kind holds for unsafe initial/final captures and capture failures at arbitrarily large ages; expired/superseded receipts hold; safe transitions deliver once; claim refusal, ACK race and stamp rollback retain existing behavior |
| Exec lifecycle | Active exec exclusion, grace, working-turn resume suppression, draft-held resume, and later safe completion |
| Ping and monitor wake | Both observations, in-flight exec and busy/expired/superseded claims; ping skip reasons; held wake leaves schedule/request pending while heartbeats and other deliveries continue |
| Backend command traces | Every automatic operation on both backends sends no Escape; explicit prompt/shutdown retain expected operations; payload and error contracts remain accurate |
| Database and documentation | Fresh and populated upgrade paths, migration-chain guard, retained row integrity, and docs-sync checks |

For manual herdr acceptance, capture sanitized interface evidence and reproduce both original triggers in a Claude Director: a member message and a member-exec completion notice. Hold an unsent single-line and multiline draft, observe unchanged text and pending notification state across repeated ticks, then submit or clear it and confirm later delivery after the turn is idle. Repeat while a Director turn is running and while a native permission or decision interface is displayed, confirming no broker-originated interruption or answer. Record the actual interface observed; native model-selection evidence establishes decision preservation, while deterministic regressions cover command-approval cues. Exercise ping and monitor wakes with drafted target composers as well. Record agent/CLI versions, dimensions, steps, observations, and the capture/send limitation under a hidden BASE-rooted acceptance directory; synthetic tests alone do not satisfy manual acceptance.

Use `mise //cafleet:test` for the Rust and docs-sync suite and `mise //cafleet:lint` for lint validation, running from the repository root. Use focused task arguments during iteration and the full required checks before implementation completion.

The completed verification record is `.acceptance/phase-d-report.md`, with the Director's independent success-criteria assessment in `.acceptance/director-success-criteria.md`. Current source revision `4829b9493bf5828164a54c2a3f481ea27934bfaf` passes all 678 tests and lint. Sanitized native newline matrices and current-artifact herdr observations establish preservation of observable blank input rows for both affected agents, indefinite holding of both original notification triggers across advancing ticks, later safe delivery and complete cleanup. Earlier active-turn, single-line/multiline draft, native decision, ping and requested/scheduled wake observations retain their actual `94e352e0` revision; the reviewed source delta changes only empty-row recognition. Interrupted collectors retain their failures, and successful additive read-only audits establish final empty idle input and stopped isolated fixtures. Native decision evidence is a model-selection interface; deterministic tests cover command-approval cues. Capture/submission remains non-atomic.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Establish capture evidence and classification

- [x] Update living skill/runtime/configuration documentation and docs-sync assertions, including empty-composer semantics, no-Escape automatic transport, pending wakes, and the capture/send race. <!-- completed: 2026-10-17T13:35 -->
- [x] Collect sanitized captures and metadata for every registry agent and required state; document recognizable bottom layouts and ambiguous cases before implementing empty detection. <!-- completed: 2026-10-17T16:45 -->
- [x] Add `Draft` and conservative current-composer parsing in `pane_state.rs`; replace draft-as-finished fixtures and verify state precedence, multiline content, historical cues, and incomplete layouts. <!-- completed: 2026-10-17T16:40 -->

### Step 2: Gate every automatic caller

- [x] Add the shared observation helper, claim receipt, final ownership/lifetime validation transaction, and UTC/monotonic clock seam; verify slow captures, superseding writers, and fresh retry behavior deterministically. <!-- completed: 2026-10-17T13:49 -->
- [x] Apply the sequence to queued previews and exec dispatch/resume while preserving conditional stamps, grace, working-turn suppression, ordering, and failure behavior. <!-- completed: 2026-10-17T13:49 -->
- [x] Apply the same sequence to ping and monitor wakes; verify skip reasons, pending wake requests, schedule stamps, and continuing heartbeat/delivery work. <!-- completed: 2026-10-17T13:49 -->

### Step 3: Separate automatic submission from direct control

- [x] Add automatic single-line prompt transport through `Multiplexer`, backend dispatch, and `PaneIo`; route exec payloads through it. <!-- completed: 2026-10-17T13:59 -->
- [x] Make previews, ping, and wakes send without Escape on both backends; preserve payloads and explicit direct-control behavior with command-trace regressions. <!-- completed: 2026-10-17T13:59 -->
- [x] Audit all pane-write call sites and test safe-to-unsafe final-capture transitions for each automatic caller with zero input asserted. <!-- completed: 2026-10-17T13:59 -->

### Step 4: Remove forced delivery completely

- [x] Remove timeout settings, forced gate/notes/outcome fields/logging, hold-age helpers, forced stamps, and obsolete preview-note plumbing; replace their tests with indefinite-hold behavior checks and complete the repository-wide removal audit. <!-- completed: 2026-10-17T14:16 -->
- [x] Add the next forward migration dropping `forced_at`, update runtime records/queries and the chain guard, and verify populated upgrades preserve retained data. <!-- completed: 2026-10-17T14:16 -->

### Step 5: Validate acceptance

- [x] Run the relevant focused regressions, then `mise //cafleet:test` and `mise //cafleet:lint`, and resolve failures attributable to the change. <!-- completed: 2026-10-17T16:53 -->
- [x] Reproduce both original herdr notification triggers and drafted ping/wake targets, recording sanitized evidence that drafts and ongoing turns survive observed unsafe states. <!-- completed: 2026-10-17T17:18 -->
- [x] Verify every success criterion against recorded evidence, mark any missing real-layout/manual evidence incomplete, and synchronize implementation progress timestamps. <!-- completed: 2026-10-17T17:19 -->

## Changelog

- 2026-10-18: Completed all implementation tasks and success criteria, resolved the first review's observable blank-row finding, and received Reviewer approval in round 2. The user authorized publication; the branch was pushed and [PR #466](https://github.com/himkt/cafleet/pull/466) was opened against main. All 678 tests, lint and the recorded real-layout/manual acceptance checks pass. Design and acceptance artifacts remain local under the user's version-control policy.
