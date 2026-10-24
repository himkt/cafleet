# Unconditional Message Delivery

**Status**: Approved
**Progress**: 53/53 tasks complete
**Last Updated**: 2026-10-24

## Overview

Remove pane-state classification from every CAFleet pane-input path. `message send` (and the `message wait` retry) to any recipient with a concrete pane, the Director included, always submits Escape, one self-describing line carrying the full message body, and Enter under the per-pane input lock, so the Director wakes on delivered lines instead of running a background `message wait`; `member prompt` and `member exec` use the same Escape-first transport, and `member ping` is removed. This fixes issue #485, where Claude Code panes with a custom `statusLine` or a dim default-foreground suggestion classify as `unknown` and receive no input at all, and gives Codex and OpenCode Directors, which have no documented native background receive, the same receive path as Claude Code.

## Success Criteria

- [x] `cafleet/src/pane_state.rs`, `cafleet/tests/pane_state.rs` and `cafleet/tests/fixtures/pane-readiness/` no longer exist, and no source file references `PaneState`, `classify` (pane variant) or `hold_reason`.
- [x] No pane-input path (`message send`, `message wait`, `member prompt`, `member exec`) runs `capture-pane` / `pane read`. Every delivery and dispatch test passes with capture disabled: the in-process mock runner panics on a capture argv, and the integration fake `tmux` exits non-zero on `capture-pane` unless a `member capture` / `fleet capture` test opts in with `set_pane_capture`.
- [x] A `message send` to any recipient with a concrete pane, ordinary member or Director, produces, on both tmux and herdr, the argv sequence Escape key → settle → literal line → settle → Enter for the first delivered message, and literal line → settle → Enter for each further message drained in the same lock hold.
- [x] A member → Director message follows the same line format, drain, lock wait, `notified_at` stamp and uncertainty handling as a message to an ordinary member. A self-send (sender equals recipient) produces no pane input.
- [x] `message wait` rotation retries pending previews for every other placed member of the fleet, the Director included, and never for its own subject.
- [x] No skill, workflow body, backend note, documentation page or `SPEC.md` section instructs the Director to run `message wait` as a background receive. The only Director use is the bounded foreground `cafleet message wait <director-member-id> --timeout 30 --json` in supervision § Director wake, limited to an active host continuation; the `message wait` CLI contract stays documented for scripts and manual use.
- [x] `member list --json` and `member show --json` carry `unconfirmed_previews`, listing exactly the member's messages with a durable claim and no `notified_at` that are still `input_required`.
- [x] A lock wait that times out after another sender delivered every pending message returns `NoPending`: `message send` prints no diagnostic and `message wait` records no entry.
- [x] A `SIGTERM` sent to a `cafleet message send` during its delivery span takes effect only after the `notified_at` stamp: the message is stamped and the process then exits by the signal.
- [x] Every delivered line matches the format in § Notification line, including the resume clause and the conditional dismissed-prompt note; a folded body of at most 4000 characters is inline, a longer one is replaced by the `cafleet message show` pointer.
- [x] After a confirmed submission the message keeps `status_state = 'input_required'`, has `notified_at` stamped, still appears in `message poll`, and leaves the inbox only through `message ack`.
- [x] One `message send` delivers every pending message for the recipient oldest-first within its five-second budget; messages left when the budget no longer covers another submission stay pending with no diagnostic.
- [x] A pane-input lock held by another writer makes `message send` wait until `deadline − INPUT_RESERVATION` (1.5 seconds). If the lock is still busy then, stderr reports `Message NEW_ID was persisted. member ID's pane input remained busy; preview MESSAGE_ID was not sent.` and the message stays pending. A lock released just before that point still leads to a successful submission.
- [x] `member prompt <director-id>` exits 1 with `cannot prompt the Director's own pane` before any pane I/O.
- [x] `cafleet member ping 1` fails with clap's default unrecognized-subcommand error (exit 2), and no living documentation, skill, rule or test mentions `member ping`.
- [x] `CAFLEET_MAX_TEXT_LEN` no longer affects pane input; it only truncates CLI text output.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass.

---

## Background

Today every pane write first captures the pane and classifies it (`cafleet/src/pane_state.rs`). Automatic delivery (`delivery.rs::attempt_preview`) submits only on `idle`; it holds on `working` / `awaiting_user` and reports a diagnostic on `draft` / `unknown`. `member prompt`, `member exec` and `member ping` reject or skip every non-idle state (`cli/member.rs::dispatch_input`, `ping`).

Issue #485 shows two independent classifier rejections on idle Claude Code panes: extra `statusLine` rows between the composer border and the footer, and a dim default-foreground suggestion that is not recognized as decoration. Either one yields `unknown`, so messages are persisted but never delivered, and the Director cannot prompt or ping the member. The fleet stalls until someone types into the pane. Patching the classifier would leave the same failure class open for every future host UI change on every backend, so the owner chose to remove the classifier from input decisions entirely. The cost is accepted: a delivery's Escape can interrupt a working turn or dismiss an open prompt, and a typed draft can merge with the payload.

The Director receives today only through `message poll` / `message wait`: delivery returns early for a Director recipient (`delivery::preview_snapshot`), and the `message wait` rotation excludes the Director (`broker::panes::pending_previews`). The skills therefore tell the Director to keep `message wait` running as a host-native background task. Only Claude Code supplies a native background task that resumes the Director when the wait returns; Codex and OpenCode have no documented equivalent, so a Director there must loop foreground waits of at most 30 seconds.

No resume or dismissed-prompt text exists in `cafleet/src` today. This design re-introduces the resume clause from design 0000183 verbatim, plus a conditional form of 0000183's dismissed-prompt note, as new constants (§ Notification line). The conditional wording is needed because the binary no longer knows whether a prompt was open.

---

## Specification

### Decisions

| Topic | Decision | Rationale |
|---|---|---|
| Classification | Delete the pane-state classifier; no input path captures the pane | The classifier is brittle on every backend; any `unknown` silently stalls a fleet |
| Transport | Escape, settle, literal line, settle, Enter | Escape is meant to interrupt a working turn or close an open prompt so the line reaches the composer; delivery does not depend on it succeeding (§ Accepted trade-offs) |
| Payload | Full body inline (folded to one line) plus resume clause and dismissed-prompt note | The recipient acts on the line directly; the clauses explain the interruption |
| Long bodies | Folded body over 4000 characters → `cafleet message show` pointer | Keeps every submitted line bounded |
| Storage | Unchanged: confirmed submission stamps `notified_at`; the message stays `input_required` until the recipient ACKs | Owner's choice (B1/B2); keeps the inbox the durable record |
| Multiple pending | One invocation drains pending messages oldest-first under one lock hold | Avoids one-message-per-send backlog |
| Lock contention | Wait for the lock within the five-second deadline | Serializes writers per pane instead of dropping the attempt |
| Uncertain submission | Unchanged: durable claim, no automatic retry, diagnostic tells the Director to inspect | Prevents duplicate delivery |
| `member prompt` / `member exec` | No classification, Escape-first, Director-authored payload unchanged; both reject the Director's own pane | Same failure class as delivery; Escape into the Director's pane would interrupt the Director's own running turn |
| `member ping` | Removed | Full-body inline delivery makes a poll keystroke redundant |
| Capture cues in `coding-agents.md` | Kept as Director-facing visual guidance for `member capture` / `fleet capture` only | The Director still reads panes; delivery no longer depends on the cues |
| Director recipient | Delivered into the Director's pane exactly like an ordinary member: Escape-first line, drain, lock wait, uncertainty handling, `input_required` until the Director ACKs | `message wait` resumes the Director automatically only on Claude Code; pane delivery works on every backend (§ Director receiving) |
| Director receiving procedure | Remove the finite-receiving obligation from skills, docs and `SPEC.md`; the Director ends its turn after dispatch and wakes on delivered lines. Under an active host continuation, each continued turn ends with one 30-second foreground `message wait`. `message wait` otherwise stays a CLI command for scripts and manual use | One receive path for every backend; the bounded wait keeps continued turns tool-bearing |
| Unconfirmed claims | Expose them as `unconfirmed_previews` in `member list --json` / `member show --json` | Without the Director's `message wait`, nothing else reports a durable claim |
| Delivery failure at the sender | One follow-up send to the same recipient | Its drain or its own line reaches the recipient without a retry daemon |
| Termination signals | Defer `SIGINT` / `SIGTERM` / `SIGHUP` across pane-input and lifecycle spans | An interrupt delivered by Escape cannot split a claim from its stamp or a spawn from its registration |
| Self-send | Unchanged: persisted with no pane input | `PENDING_PREVIEW` already excludes `from_member_id = to_member_id`; Escape into the sender's own pane would interrupt the command's own turn |

### Accepted trade-offs

What a single Escape does depends on the backend and is unverified, because installed-host checks are waived. A backend may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it. Both clauses are already conditional, so they stay accurate either way.

- Where Escape interrupts a working turn, the delivery interrupts it, the Director's turn included. The resume clause tells the recipient to continue.
- Where Escape dismisses an open question, confirmation or approval prompt, the delivery dismisses it. In the Director's pane this includes an open decision-surface question to the operator. The dismissed-prompt note tells the recipient the rejection did not come from the user, and the Director re-asks the question.
- Escape does not reliably clear typed composer text on every host. A draft typed into a member pane, or text the operator typed but did not submit in the Director's pane, can merge with the delivered line and be submitted with it. This is documented, not prevented.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- The Director has no timer-driven stall check. It inspects roster, panes and artifacts when a delivered line, a host continuation or the operator wakes it.
- A delivery failure at the sender is answered by one follow-up send (§ Director receiving, sender follow-up). If the follow-up also fails, the message stays pending and nothing retries it automatically; the fleet waits until the next successful delivery to that recipient or until the operator intervenes.
- Interrupting a host turn can also end the turn's in-flight shell tool, including a running `cafleet` command. The binary defers termination signals across its pane-input and lifecycle spans; a `SIGKILL` inside a span remains unprotected (§ Termination during pane input).

### Pane input transport

`cafleet/src/multiplexer/mod.rs` gains a lead selector and a shared Escape settle delay:

```rust
pub(crate) const ESCAPE_SETTLE_DELAY: f64 = 0.5;
pub(crate) const ESCAPE_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const MIN_LITERAL_BUDGET: Duration = Duration::from_millis(500);
pub(crate) const REVALIDATION_MARGIN: Duration = Duration::from_millis(500);
/// Revalidation margin + Escape settle + literal minimum + SUBMIT_DELAY + Enter: 3.5 s of the 5 s budget.
pub(crate) const INPUT_RESERVATION: Duration = Duration::from_millis(3500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lead {
    Escape,
    Direct,
}
```

`Multiplexer::send_prompt(&self, target_pane_id: &str, text: &str, lead: Lead)` replaces both `send_prompt(pane, text)` and `send_inline_preview(...)`. Delete `send_inline_preview` from the trait, both adapters, `AnyMultiplexer` and the test mock, and delete `inline_preview_payload` (its folding moves to `notification_line`, below).

| Backend | Escape argv | Literal argv | Enter argv |
|---|---|---|---|
| tmux | `tmux send-keys -t PANE Escape` | `tmux send-keys -t PANE -l -- TEXT` | `tmux send-keys -t PANE Enter` |
| herdr | `herdr pane send-keys PANE escape` | `herdr pane send-text PANE -- TEXT` | `herdr pane send-keys PANE enter` |

`submit_prompt` performs, in order:

1. Compute `reserved = SUBMIT_DELAY + 1 s`, plus `ESCAPE_SETTLE_DELAY` when `lead == Lead::Escape`. If `remaining − reserved` is not positive, return `insufficient pane input budget for literal input, settle and Enter` with no input (not started).
2. For `Lead::Escape`, run the Escape argv with timeout `min(ESCAPE_TIMEOUT, remaining − reserved)`, then sleep `ESCAPE_SETTLE_DELAY`. Any Escape failure (launch, pane gone, non-zero exit or timeout) returns a not-started error: no message text was typed.
3. Recompute `literal_budget = remaining_now − (SUBMIT_DELAY + 1 s)`. If it is not positive (Escape consumed the slack), return the same insufficient-budget error, not started. Otherwise run the literal argv bounded by `literal_budget`, sleep `SUBMIT_DELAY` and run Enter. Not-started and after-write failures are classified as they are today.

Input validation (`valid_input`) is unchanged and runs before any process starts.

### Notification line

`cafleet/src/delivery.rs` builds the delivered line:

```rust
pub(crate) const INLINE_BODY_LIMIT: usize = 4000;
const RESUME_CLAUSE: &str = "[cafleet] Resume your work if something was still running.";
const DISMISSED_PROMPT_NOTE: &str = "[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

pub(crate) fn notification_line(message: &MessageRecord) -> String {
    let folded = fold_visible(&message.text);
    let body = if folded.chars().count() <= INLINE_BODY_LIMIT {
        folded
    } else {
        format!(
            "Body is {} characters; run cafleet message show {} --json to read it.",
            message.text.chars().count(),
            message.message_id
        )
    };
    format!(
        "[cafleet msg {} from {} {}] {body} {RESUME_CLAUSE} {DISMISSED_PROMPT_NOTE}",
        message.message_id, message.from_member_id, message.created_at
    )
}
```

`fold_visible` is today's `visible` folding moved verbatim: CRLF / LF / CR → `⏎`, tab → `␉`, other C0 and DEL → `\xNN`. The limit counts Unicode scalar values of the folded body; the pointer's `N` counts those of the stored text. `CAFLEET_MAX_TEXT_LEN` is no longer applied to the line. Every line, including drained follow-up lines and pointer lines, carries both clauses.

Example:

```text
[cafleet msg 57 from 15 2026-10-24T09:00:00.000000+00:00] ready (doc)⏎see the reviewer notes [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

### Delivery procedure

`delivery::attempt_preview` becomes `delivery::deliver_pending(conn, settings, member_id, expected_fleet, deadline, resolve) -> Result<AttemptOutcome, CafleetError>`. `message send` and `message wait` (through `runtime::attempt_preview`, renamed `runtime::deliver_pending`) both call it.

1. Take the preview snapshot as today, without the Director early return (§ Director receiving). Return `NoPending` for an inactive member, a fleet mismatch or no pending message. Return `Held` for a placement without a concrete pane.
2. Acquire the pane-input lock with `PaneLock::acquire_within(database_url, placement, deadline.reserving(INPUT_RESERVATION))` (§ Pane input lock). If the lock is not acquired, return `Failed` (`not_sent`) with `member ID's pane input remained busy; preview MESSAGE_ID was not sent.`, naming the message chosen by a post-timeout revalidation. When `acquire_within` returns `Ok(None)`, one short read (`read_snapshot`) re-reads the oldest pending unclaimed message for the member. None → return `NoPending`, because the lock holder, usually another sender draining the same recipient, delivered or settled every message meanwhile. Otherwise report the lock-busy diagnostic for that message. The wait runs inside `deliver_pending`'s existing `with_database_budget` scope; no statement runs while waiting, and the scope only bounds SQLite busy waits.
3. Re-read the snapshot under the lock; a changed placement returns `Held`. Resolve the multiplexer and run `ensure_available` as today. Run no capture.
4. Loop, starting with `lead = Lead::Escape`:
   1. Read the oldest pending unclaimed message for the member. None → finish with `end = Drained`.
   2. If the remaining budget is less than the transport's `reserved` for `lead` plus `MIN_LITERAL_BUDGET`: when `sent` is empty, finish with `Failed(not_sent)` carrying `insufficient pane input budget for literal input, settle and Enter`; otherwise finish with `end = BudgetExhausted`. No claim exists yet, so nothing is released.
   3. Claim it with `claim_current_preview`. Its skip payload becomes a local enum `ClaimSkip { Gone, Held, Settled }`, replacing the `AttemptOutcome` it carries today. `Gone` (the member is no longer active) finishes with `end = NoPending`. `Held` (placement changed, or an earlier candidate appeared) finishes with `end = Held`. `Settled` (the message was ACKed or stamped meanwhile) continues the loop with `lead` unchanged.
   4. Submit `notification_line(&message)` with `send_prompt(pane, line, lead)`. A not-started failure releases the claim (existing `retry_database_completion` path) and finishes with `Failed(not_sent)`. An after-write failure finishes with `Failed(uncertain)` and leaves the claim durable.
   5. The line was submitted, so set `lead = Lead::Direct`. Stamp `notified_at` through `complete_database_budget` as today. `Ok(true)` and `Ok(false)` both push the message ID onto `sent`, then continue; `Ok(false)` means the recipient ACKed the message between claim and stamp. A stamp error finishes with `Failed(uncertain)`.

`Deadline::reserving(self, reserve: Duration) -> Deadline` returns a deadline earlier by `reserve` (already expired when less than `reserve` remains). `INPUT_RESERVATION` covers the step 4.2 threshold for the first message (`reserved` 2.5 s + `MIN_LITERAL_BUDGET` 0.5 s) plus `REVALIDATION_MARGIN` (0.5 s). The margin covers the work between lock acquisition and that check: the step 3 snapshot re-read, `resolve` / `ensure_available` and the step 4.1 read. Contention therefore reports the lock-busy string, and a lock acquired within the wait submits. If that revalidation work takes longer than its margin, the first message ends `Failed(not_sent)` with `insufficient pane input budget for literal input, settle and Enter`; this boundary diagnostic is documented alongside the lock-busy one.

Later messages use `Lead::Direct` because a second Escape would interrupt the turn the first delivered line just started; the host queues or appends the further lines.

The outcome type in `cafleet/src/receive.rs` records the delivered IDs alongside how the attempt ended:

```rust
pub(crate) struct AttemptOutcome {
    pub sent: Vec<i64>,
    pub end: AttemptEnd,
}

pub(crate) enum AttemptEnd {
    NoPending,
    Drained,
    BudgetExhausted,
    Held,
    Failed(NotificationError),
}
```

| `end` | `message send` stderr | `message wait` diagnostic bookkeeping |
|---|---|---|
| `NoPending` | — | Remove the member's entry if its message is no longer pending (today's `NoPending` rule) |
| `Drained`, `BudgetExhausted` | — | Remove the member's entry when its `message_id` is in `sent` |
| `Held` | — | Keep entries unchanged (today's `Held` rule) |
| `Failed(error)` | `Message NEW_ID was persisted. ` followed by `error.error` | Insert or replace the member's entry with `error` |

`notification_sent` in the `message send` result is `true` exactly when the newly committed message ID is in `sent`. When another invocation delivered the new message, `notification_sent` stays `false` and stderr carries no diagnostic; the recipient still received the line. Messages left pending after `BudgetExhausted` are retried by the next `message send` to that member or by a `message wait` rotation. The rotation cadence (at most one member attempt per one-second interval, uncertain claims skipped) is unchanged; its candidate set gains the Director (§ Director receiving).

`message wait` changes behavior on a busy pane. Today a held lock returns `Held` at once and adds nothing. Now a rotation attempt waits for the lock for up to 1.5 seconds (five seconds minus `INPUT_RESERVATION`). If the lock never frees, the attempt adds a `not_sent` lock-busy entry to `notification_errors`. The wait, and any drain after it, stays inside the attempt's existing five-second budget. A receive therefore still returns by its deadline plus one in-flight attempt plus at most one second of finalization grace.

### Storage

No migration; the database format family and `V2__preview_attempts.sql` head stay as they are.

| Column | Meaning |
|---|---|
| `preview_started_at` | Set by the claim immediately before possible input under the lock; cleared on a confirmed not-started failure; durable after an uncertain one |
| `notified_at` | Stamped after a confirmed complete submission while the message is still pending |
| `status_state` | `input_required` until the recipient runs `message ack`; delivery never changes it |

A delivered message, inline or pointer, stays in `message poll` output until ACKed. The recipient ACKs the ID shown in the line's `[cafleet msg ID …]` prefix after acting on it.

### Pane input lock

Add `PaneLock::acquire_within(database_url: &str, placement: &Placement, lock_deadline: Deadline) -> Result<Option<PaneLock>, CafleetError>`. It calls `try_acquire`, and on `WouldBlock` retries with `lock_deadline.retry()` (50 ms steps) until `lock_deadline` expires, then returns `Ok(None)`. Unexpected lock errors propagate as today. Every pane writer passes `deadline.reserving(INPUT_RESERVATION)`, so it waits at most 1.5 seconds and keeps 3.5 seconds for revalidation and input. Member and fleet deletion keep their existing loop and five-second wait.

There are two call sites:

| Call site | Lock wait placement |
|---|---|
| `delivery::deliver_pending` | Calls `PaneLock::acquire_within` directly, inside its existing `with_database_budget` scope (§ Delivery procedure step 2) |
| `input::lock_member(conn: &mut Connection, settings, member, deadline) -> Result<Option<LockedMember>, CafleetError>` | Acquires the file lock first, outside any database budget, then reloads and revalidates the placement under `with_database_budget(conn, deadline, …)`. `dispatch_input` calls it directly instead of wrapping it in `with_database_budget` |

| Writer | Lock still busy at `deadline − INPUT_RESERVATION` |
|---|---|
| `message send` / `message wait` delivery | `Failed(not_sent)`: `member ID's pane input remained busy; preview MESSAGE_ID was not sent.` |
| `member prompt` | Exit 1: `member ID's pane input remained busy; prompt was not dispatched.` |
| `member exec` | Exit 1: `member ID's pane input remained busy; native command was not dispatched.` |
| `member delete` / `fleet delete` | Unchanged |

The lock is held from placement revalidation through the last Enter and the last `notified_at` stamp of the drain.

### Director receiving

The Director becomes an ordinary delivery recipient. Two exclusions are deleted:

| Item | Change |
|---|---|
| `delivery::preview_snapshot` | Delete the `fleet.director_member_id == Some(member_id)` early return |
| `broker::panes::pending_previews` | Delete `m.member_id!=f.director_member_id` from the `WHERE` clause. Keep `m.member_id!=?2`, so a `message wait` never types into its own subject's pane |

`fleet create` already records the calling pane as the Director's concrete placement, so no schema or registration change is needed. That pane must be the Director's coding-agent session, which the `--coding-agent` flag already declares. A plain shell registered as Director would receive each delivered line as shell input. The docs state this registration requirement next to `fleet create`.

The Director-side procedure replaces finite receiving in the skills:

1. After dispatching work, the Director continues independent authorized work or ends its turn. Without an active host continuation, ending the turn with outstanding delegated work is the normal waiting state.
2. A delivered `[cafleet msg ID from SENDER TIMESTAMP]` line wakes the Director. The inline body is the full message; a pointer line names the `cafleet message show ID --json` command for the full body.
3. On every wake and before ending a turn, the Director runs `cafleet message poll <director-member-id> --json`, which also surfaces messages whose delivery failed. It records deferred actions in host-owned task state, ACKs each consumed delivery and acts on it.
4. On every wake, the Director runs `cafleet member list <fleet-id> --json` and checks each member's `unconfirmed_previews` and the assignments past their expected reply point, using `cafleet member capture` / `cafleet fleet capture` and artifacts as evidence.
5. For each ID in a member's `unconfirmed_previews`, the Director captures the member pane. When the line is absent, or the capture cannot establish it, the Director runs `cafleet member prompt <member-id> "cafleet message poll <member-id> --json"`. The member ACKs the message normally, which removes it from the list.
6. The Director retains the operator's decision gates. A delivery that dismissed an open decision-surface question calls for re-asking it.

#### Waiting under host continuation

A host continuation is an active Claude Code `/goal` condition or an active Codex thread objective. While one is active, the host starts another Director turn as soon as a turn ends with the goal unmet, and consecutive tool-free Claude Code turns reach the block cap. During an active host continuation, each continued Director turn with outstanding delegated work ends with one bounded foreground receive, `cafleet message wait <director-member-id> --timeout 30 --json`, and then applies steps 3–6. The 30-second timeout paces the loop and keeps every continued turn tool-bearing. This is the only Director use of `message wait` in the skills. A delivery that arrives during the wait still reaches the pane; the wait returns the same persisted message.

#### Unconfirmed claims in the roster

`member list --json` roster records and the `member show --json` member record gain `unconfirmed_previews`, appended after `idle` (roster) or as the last key (show):

```json
"unconfirmed_previews": [41, 44]
```

It lists, ordered by `(created_at, message_id)`, the IDs of messages addressed to that member with `status_state = 'input_required'`, `preview_started_at IS NOT NULL` and `notified_at IS NULL`. These are the durable claims that automatic delivery skips. An empty list is `[]`; the Director row carries the field too. Text output is unchanged. The roster query lives in `broker::members` and the presenters in `presentation.rs`.

#### Sender follow-up

A `message send` whose delivery fails prints `Message N was persisted. …` on stderr. The sender, member or Director, then sends exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`.

| State of message N | Effect of the follow-up |
|---|---|
| Unclaimed (`not_sent`) | The follow-up's drain delivers N first, oldest-first, then the follow-up |
| Durable claim (`uncertain`) | Automatic delivery skips N; the follow-up line wakes the recipient, whose poll lists N |

If the follow-up also prints `Message M was persisted. …`, the sender stops: a member ends its turn, and the Director inspects the target pane and continues with step 5's `member prompt`. A failed follow-up to the Director is the accepted stall in § Accepted trade-offs.

`message wait` keeps its CLI contract, including its own-subject exclusion and the five-second attempt budget. Apart from § Waiting under host continuation, the documentation presents it as a blocking receive for scripts and manual use.

### Termination during pane input

Interrupting a Claude Code or Codex turn can also end the turn's in-flight shell tool, a `cafleet` process included; installed-host checks are waived, so the exact signal is unverified. Every pane write and lifecycle change runs inside a termination-deferral span. Add `cafleet/src/signals.rs`:

```rust
/// Blocks SIGINT, SIGTERM and SIGHUP for the calling thread until dropped;
/// a signal received meanwhile takes its default action when the previous mask is restored.
pub(crate) struct DeferTermination { previous: nix::sys::signal::SigSet }

impl DeferTermination {
    pub(crate) fn begin() -> Result<Self, CafleetError>;
}
```

`begin` calls `pthread_sigmask(SIG_BLOCK, …)` and keeps the previous mask; `Drop` restores it with `SIG_SETMASK`, so nested spans compose. Each span starts before the guarded operation creates any helper thread, so pipe-draining threads inherit the blocked set.

| Command | Span |
|---|---|
| `message send`, each `message wait` attempt | `deliver_pending` from lock acquisition through the last `notified_at` stamp or claim release |
| `member prompt`, `member exec` | From lock acquisition through Enter |
| `member create` | From the pane spawn through the registry commit or its compensation |
| `member delete`, `fleet delete` | From lock acquisition through deregistration and layout cleanup |

Every span is bounded by its existing deadline, so a deferred signal waits at most one span. `message wait` outside an attempt keeps default signal behavior. Multiplexer client processes started inside a span inherit the mask and stay bounded by their timeouts. The spawned member session is started by the multiplexer server and must not inherit the mask; Implementation Step 4 verifies this on tmux and herdr.

`SIGKILL` cannot be deferred. A kill inside a span leaves:

| Killed command | State left | Director discovery and recovery |
|---|---|---|
| `message send` / `message wait` between claim and stamp | Durable claim; the line may or may not have reached the pane | `unconfirmed_previews` on wake; step 5 |
| `member prompt` / `member exec` after typing began | Partial or complete input | `member capture`; re-dispatch only after inspecting the pane |
| `member create` | A pane without a registry row, or a row whose placement stays pending | `member list` and `fleet capture`; `member delete` the row, and close an unregistered pane with the multiplexer's own kill command |
| `member delete` / `fleet delete` | A closed pane whose row remains, or the reverse | `member list`; rerun the delete |

### `member prompt` and `member exec`

`cli/member.rs::dispatch_input` keeps lock → reload → resolve multiplexer → `ensure_available`, drops capture and classification, and calls `send_prompt(pane, text, Lead::Escape)`. Payloads stay Director-authored and receive no appended clauses: `member prompt` sends `TEXT`, `member exec` sends `! COMMAND`. Delete the `input_state` helper.

`member prompt` gains the Director-target guard that `member exec` already has. After loading the member and before any pane I/O, a `MemberKind::Director` target exits 1 with `cannot prompt the Director's own pane`. The classifier blocked this case implicitly, because the Director's pane is `working` while it runs the command. Without the guard, Escape would interrupt the Director's own turn.

| Error | Status |
|---|---|
| `cannot prompt the Director's own pane` | New, exit 1, before pane I/O |
| `member ID is STATE; prompt was not dispatched.` | Removed |
| `member ID is STATE; native command was not dispatched.` | Removed |
| `member ID's pane is receiving another input; retry after it is idle.` | Replaced by the lock-busy strings above |
| `prompt was not dispatched to member ID: CAUSE` / `native command was not dispatched to member ID: CAUSE` | Unchanged; now also covers an Escape failure |
| `prompt delivery to member ID is uncertain: …` / `native command delivery to member ID is uncertain: …` | Unchanged |

### `member ping` removal

Delete the `Ping` clap variant and `fn ping` in `cafleet/src/cli/member.rs`, its `--member-id` / `--quiet` parser cases in `cafleet/src/cli/mod.rs` tests, its tests in `cafleet/tests/cli_member.rs` and `cafleet/tests/pane_input.rs`, and every documentation mention (§ Documentation updates). The absence test is a single `cafleet member ping 1` case asserting clap's default unrecognized-subcommand error with exit 2.

### Removed code

| Item | Location |
|---|---|
| `PaneState`, `classify`, styled-capture parsing | `cafleet/src/pane_state.rs` (file deleted), `pub mod pane_state` in `cafleet/src/lib.rs` |
| Classifier tests and fixtures | `cafleet/tests/pane_state.rs`, `cafleet/tests/fixtures/pane-readiness/` |
| Delivery capture and readiness gate | `delivery.rs` capture call, `DELIVERY_CAPTURE_LINES` if no other user remains, `hold_reason` branch |
| Draft/unknown diagnostic | `member ID is STATE; preview MESSAGE_ID was not sent.` |
| `send_inline_preview`, `inline_preview_payload` | `multiplexer/mod.rs`, `tmux.rs`, `herdr.rs`, test mock |
| `member ping` | § `member ping` removal |
| `CAFLEET_MAX_TEXT_LEN` preview truncation | `delivery.rs` `truncate_text` call |

### Documentation updates

Per `.claude/rules/documentation-maintenance.md`, documentation changes land before code.

| Target | Change |
|---|---|
| `docs/docs/spec/multiplexer-backends.md` | Delete § Pane-state classification. Rewrite § Prompt dispatch for the Escape-first transport and `Lead`. Rewrite § Push notifications for the notification line, the 4000-character limit and pointer, the drain procedure and the ACK requirement. Rewrite § Held delivery: holds are pending placement and claim-time placement change; lock contention waits; remove draft/unknown diagnostics; document the accepted trade-offs. Update the pane-input-lock contention row and the literal-input paragraph ("automatic interruption keys are absent" becomes the Escape lead). In § Push notifications, replace the Director exclusion (line 179) with delivery into the Director's pane like any recipient, keep self-sends settled, and say retries (lines 193 and 199) come from the next send to the recipient or a `message wait` rotation that includes the Director. Line 138: "Every notification, prompt and native command" shares the lock, acquired before placement revalidation (no styled capture). Line 189: after inspecting the pane, `member prompt` the member to poll its durable inbox (no idle requirement), and the Director discovers such claims through `unconfirmed_previews`. Document § Termination during pane input next to the pane input lock |
| `docs/docs/spec/cli-options.md` | Remove the `member ping` rows (subcommand summary, output shapes, error table), § `member ping` and ping mentions in member targeting. Change the `CAFLEET_MAX_TEXT_LEN` description to the CLI text-body limit, and § Message Body Truncation to CLI text only. Rewrite § `message send` / delivery outcome: drain, `notification_sent`, the lock-busy stderr, the removed STATE stderr, and delivery to a Director recipient. In § `message wait`, describe the retry without readiness, the rotation that includes the Director, and the lock wait of up to 1.5 seconds that ends in a `not_sent` lock-busy entry; the existing return bound holds. Replace its Director paragraph (line 551: background task, handle, 30-second foreground loop) with its use as a blocking receive for scripts and manual use. In § `fleet create`, state that the registering pane must be the Director's coding-agent session. In § Member list output and § `member show`, add `unconfirmed_previews` with its selection rule and key position. In § `message send`, state that another invocation's delivery leaves `notification_sent` false without a diagnostic and that a lock-wait timeout with nothing left pending reports nothing. Document the boundary insufficient-budget diagnostic next to the lock-busy one in § `message send` / delivery outcome. Rewrite § `member prompt` (Director-target guard, no readiness, Escape-first, lock-busy error) and § `member exec` (no readiness, Escape-first, lock-busy error), and add the `cannot prompt the Director's own pane` error row |
| `docs/docs/spec/message-envelope.md` | Line 44: the pane line folds the full body under its own contract, independent of `CAFLEET_MAX_TEXT_LEN` |
| `docs/docs/spec/data-model.md` | Pending-preview paragraph: delivery stamps `notified_at` and leaves `status_state` unchanged; drained messages follow the same claim/stamp rules. Query and activity contracts: the `unconfirmed_previews` selection |
| `docs/docs/concepts/overview.md` | Core terms row `inline preview` (line 15): a literal one-line notification submitted Escape-first into the recipient's pane, carrying the full body or a show pointer. The term table governs sitewide, so the other pages adopt this wording. Row `receive` (line 16): a finite inbox wait for scripts and manual use that also retries pending previews. Line 39: the Director ends its turn after dispatch, wakes on delivered lines, and polls, ACKs and records deferred actions on every wake |
| `docs/docs/concepts/coding-agents.md` | Line 42: `member exec` submits Escape-first without idle revalidation |
| `docs/docs/concepts/member-lifecycle.md` | Line 13: every recipient's messages, the Director's included, are delivered into its pane with Escape first, and the recipient ACKs each delivered ID. Line 19: the native dispatch contract owns validation, lock and uncertainty errors |
| `docs/docs/how-to/mixed-backend-team.md` | Line 3: replies are delivered into the Director's pane. Line 60: a pending preview remains pending only while the pane lock stays busy, the pane is not yet placed, or the budget runs out; the next send to that recipient or a `message wait` rotation retries it. § Receive results (lines 62–82): replies arrive as delivered lines in the Director's pane; poll `--json` and ACK on every wake; remove the background-task, handle and 30-second instructions; keep `cafleet message wait 2 --timeout 300 --json` as an optional blocking receive for scripts |
| `docs/docs/how-to/design-doc-development.md` | Line 35: the Director wakes on delivered replies and owns evidence checks and continuation |
| `docs/docs/spec/coding-agent-backends.md` | Line 68 (claude): replace "Retain a finite receive task handle while external work remains." with: while a goal is active, each continued Director turn ends with the bounded 30-second foreground wait. Line 164 (codex): the same sentence for an active thread objective, keeping "retain live tool/session handles". Keep the host-continuation evidence. The capture-cue subsection stays |
| `SPEC.md` | Mirror the cli-options and multiplexer-backends changes in §6.3 and §6.5, including the Director delivery, the `message wait` paragraph (line 663), the `fleet create` registration requirement, `unconfirmed_previews`, lines 1136 and 1187 and the termination-deferral spans. Mirror the coding-agent-backends change in §6.7 (lines 1664 and the codex counterpart). §6.3 line 677: drop "or a ping skip" and "and ping". Other targets: §1 overview (line 7: the Director owns delegation, wake handling, evidence inspection, authorization and cleanup) and goal sentence (drop "readiness"); §3 Module layout `Pane input` row (drop "styled readiness"); §4 dependency graph (drop `pane input -> styled readiness`); §5.3 readiness enum sentence (remove); §6.3 error table rows 915–917 (drop `ping`, add the Director-prompt row) and the line 886 "readiness" mention; §6.5 backend matrix row `Capture for readiness` (becomes capture for `member capture` / `fleet capture` only), § Pane input lock (bounded wait instead of nonblocking contention), § Pane-state classification (delete), § Literal input; §6.6 budget text (drop capture) and lock paragraph (bounded wait, no capture); §7.2 ("ping skips exit 0" removed); §7.3 truncation; §9 Testing strategy (replace composer fixtures with Escape-first and drain coverage); §11 Decisions & clarifications row `Stable input exclusion and readiness`. Sync through the `/update-readme` skill |
| `README.md` | Through the `/update-readme` skill: line 5 says replies are delivered into the Director's pane instead of "collects replies with finite receiving"; edit the rest only if its pitch or command list mentions ping or idle-only delivery |
| `skills/cafleet/SKILL.md` | § Required ids: remove `ping` from the target list. Delete § Finite receiving. § One-shot command isolation (lines 117 and 123): drop the held-preview rationale and the Asynchronous Wait Rule sentence; a compound invocation still breaks host allow-rule matching. § Send (Unicast): previews are delivered Escape-first into every recipient's pane, the Director's included, with the full body or a show pointer, and the recipient ACKs. § Poll (line 139): the Director also polls on every wake; a later send to the recipient retries a pending notification. Command-index rows: `Pane readiness, exclusion and previews` (line 167) becomes `Pane input, exclusion and previews`; `Finite receive results and retries` becomes `Scripted receive and retries`, still linking § message wait |
| `skills/cafleet/roles/director.md` | Remove § Member Ping and the ping mention at line 47. In § member prompt (line 246), replace "Require an idle empty composer" with: prompt leads with Escape, can interrupt a working turn, and rejects the Director's own pane; check a retained draft in a capture before prompting. Required-reading row 3 (line 15): "finite receiving" becomes "Director wake". Replace § Finite receiving (line 260) with § Director wake, a short summary linking `supervision.md#director-wake` |
| `skills/cafleet/roles/member.md` | § Identity and authority: drop the `member ping` sentence. § On spawn: a delivered `[cafleet msg ID …]` line carries the full body or a show pointer; ACK that ID after acting on it. Line 33: a message that arrives while you are busy is delivered into your pane, and a pending one is retried by the next send to you. Add the sender follow-up: when your send prints `Message N was persisted. …`, send exactly one follow-up `message N was not delivered to your pane; poll your inbox` to the same recipient; if it also fails, end your turn |
| `skills/cafleet/reference/prompt-routing.md` | Remove the `member ping` primitive row. Line 13: every writer shares the bounded input lock (no readiness checks). Line 41: the CLI types `! ` plus the command after Escape, lock and revalidation |
| `skills/cafleet/reference/supervision.md` | Replace § Asynchronous Wait Rule (lines 24–30) with § Director wake carrying the procedure in § Director receiving: steps 1–6, waiting under host continuation, the `unconfirmed_previews` check and recovery, and the Director's side of the sender follow-up. Core Principle (line 7): CAFleet delivers each message into its recipient's pane instead of receiving for a finite interval. Communication Model (line 13): ACK every consumed delivery before ending the turn. Spawn Protocol step 6 (line 22): end the turn after dispatch and wake on delivered lines. § Idle Semantics (line 34 onward): captures guide the Director's next action only, delivery and dispatch never wait on them, a retained draft can merge with a delivery, and the `working` row becomes "End the turn; a delivered line or the operator wakes you". § Team-facilitation instructions: "At every receive result" becomes "At every wake", and step 5 ends the turn while delegated work remains |
| `skills/cafleet/reference/coding-agents.md` | In each backend's § Pane-state capture cues, replace the "transport evidence" wording with Director visual inspection of `member capture` / `fleet capture`; delivery and dispatch do not consult the cues. In each backend's Note → applies at, rebind the Director-receiving rows from `supervision.md#asynchronous-wait-rule` to `supervision.md#director-wake`: claude's turn-boundary row replaces "Retain the finite receive task handle while delegated work remains." with "While a goal is active, end each continued turn with the bounded foreground wait in Director wake."; codex's row adds "While the objective is active, each continued turn ends with the bounded foreground wait in Director wake." and keeps its other host-continuation facts; opencode's "retains finite receiving" becomes "wakes on delivered lines". The host-evidence wording the docs-sync backend-notes test matches stays |
| `skills/cafleet-design-doc/create/create.md`, `execute/execute.md`, `interview/interview.md` | Required-reading row 4: "finite receiving" becomes "Director wake". Replace § 1b / 3b / 2b "Own finite receiving" with "Wake on delivered replies": after dispatch end the turn, a delivered line wakes you, then poll `--json`, ACK and act per `supervision.md#director-wake`. Every "retain finite receiving" / "finite receive task handle" / "finite receive result" phrase in these bodies becomes waking on the delivered reply |
| `.claude/rules/bash-tool.md` | Remove the `member ping` bullet |

`.claude/rules/coding-agent-overlay.md` needs no change: the cue subsection stays.

### Tests

| Area | Cases |
|---|---|
| Transport (`tmux.rs`, `herdr.rs`, `multiplexer/mod.rs`) | `Lead::Escape` argv order Escape → literal → Enter with both settles; `Lead::Direct` argv literal → Enter; Escape failure starts no literal and is not-started; insufficient budget for the Escape reservation starts no process; a slow Escape that consumes the slack starts no literal and is not-started with the insufficient-budget error |
| Notification line (`delivery.rs` unit) | Inline single-line body; multiline/tab/control folding; folded length exactly 4000 inline and 4001 pointer with stored-text count and `--json` pointer; both clauses present on inline and pointer lines |
| Delivery (`cafleet/tests/delivery.rs`) | No `capture-pane` call and delivery still succeeds; two pending messages drained oldest-first with exactly one Escape; budget exhaustion leaves later messages pending with no diagnostic; a message ACKed between claim and stamp is in `sent` and the next line uses `Lead::Direct`; member deregistered under the lock ends `NoPending`; lock released within the wait → delivery succeeds; lock released just before `deadline − INPUT_RESERVATION` → the first message is still submitted; lock still busy at `deadline − INPUT_RESERVATION` → lock-busy diagnostic and message still pending; a concurrent drainer delivers the message during the lock wait → `NoPending`, no diagnostic, `notification_sent` false; `notified_at` stamped and `status_state` still `input_required`; uncertain after-write failure keeps the claim and blocks automatic retry; a Director recipient with a concrete pane receives the Escape-first line, and `delivery.rs`'s `a_director_without_a_placement_has_no_preview_attempt` becomes a Director-placement delivery case |
| `message send` (`cafleet/tests/cli_message.rs`) | `notification_sent` true when the new message is drained after an older one; persisted-prefix stderr for the lock-busy case; a member → Director send delivers into the Director's pane with `notification_sent` true; a Director self-send produces no pane input |
| `message wait` (`receive.rs`, `cafleet/tests/notification_retry.rs`, `message_wait.rs`) | Bookkeeping table above with the new `AttemptOutcome`; rotation cadence unchanged; a busy lock for the whole wait window yields one `not_sent` lock-busy entry in `notification_errors`, and the receive still returns within its deadline plus one attempt and finalization grace; a member's wait retries a pending Director preview; a wait never attempts its own subject |
| `pending_previews` (`cafleet/src/broker/panes.rs`, `broker/messaging.rs` unit) | The Director's pending preview is listed; the subject's own previews are not |
| `unconfirmed_previews` (`broker::members` unit, `cafleet/tests/cli_member.rs`) | Empty list by default; a started claim without `notified_at` is listed in `member list --json` and `member show --json` at the specified key position; ACK, stamp or claim release removes it; text output unchanged |
| Termination deferral (`signals.rs` unit, `cafleet/tests/delivery.rs`) | Nested spans restore the outer mask; a `SIGTERM` sent while the fake `tmux` blocks inside the literal write lets the stamp complete, then the process exits by `SIGTERM`; a `SIGTERM` outside any span (e.g. during a `message wait` sleep) terminates at once; a `member create` span covers spawn through registry commit |
| `member prompt` / `member exec` (`cafleet/tests/cli_member.rs`, `cli_exec.rs`, `pane_input.rs`) | No capture; Escape-first argv; lock-busy strings; `member prompt <director-id>` exits 1 with `cannot prompt the Director's own pane` and sends no input; removed STATE errors no longer produced |
| `member ping` absence | `cafleet member ping 1` → clap unrecognized-subcommand error, exit 2 |
| Docs sync (`cafleet/tests/docs_sync.rs`) | Update assertions that quote removed strings, including the `SPEC.md` `member-ping` section check; rewrite `backend_capture_cues_describe_transport_readiness` to assert the cues are described as Director inspection guidance |
| Docs sync, Director receiving (`cafleet/tests/docs_sync.rs`) | Rewrite `director_owns_finite_receiving_until_reply_or_explicit_handoff` as `director_wakes_on_delivered_lines_and_polls_each_wake`: supervision requires `cafleet message poll <director-member-id> --json`, `cafleet fleet capture`, `cafleet member capture`, the wake pattern, ACK before ending the turn, deferred host-owned actions, retained-draft relay, `unconfirmed_previews` and the host-continuation `cafleet message wait <director-member-id> --timeout 30 --json`. In `workflows_preserve_phase_gates_and_use_the_shared_receive_procedure`, require `cafleet message poll` and a wake pattern instead of `cafleet message wait` and the finite-receive pattern. In `message_and_exec_examples_retain_commands_and_protocol_links`, drop the core `cafleet message wait` token requirement and keep the § message wait link. Add a check that, across `skills/`, `cafleet message wait <director-member-id>` appears only in supervision § Director wake and only with `--timeout 30`. In `members_poll_at_startup_report_boundary_and_wake`, require the follow-up body `message N was not delivered to your pane; poll your inbox` |

The shared integration fakes change as follows:

| Fake | Change |
|---|---|
| `cafleet/tests/common/mod.rs` fake `tmux` | Record `send-keys … Escape` like other `send-keys` calls. `capture-pane` exits non-zero with `unexpected capture-pane` for every pane without an opted-in capture (Success Criterion 2); `set_pane_capture` opts a pane in and, with `CAPTURE_FAILS`, remains only for `member capture` / `fleet capture` tests. Add an option that blocks the literal `send-keys -l` until a release file appears, for the termination-deferral test |
| `cafleet/tests/common/receive.rs` | Remove the idle-capture setup in `Fixture::worker` and the `view` helper used for readiness. Its `capture-pane` branch exits non-zero with `unexpected capture-pane` |
| In-process mock runners (`tmux.rs`, `herdr.rs`, `delivery.rs` unit tests) | Panic on a `capture-pane` / `pane read` argv |

Installed-host manual checks are waived by the owner; automated tests are the verification.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation site

- [x] Rewrite `docs/docs/spec/multiplexer-backends.md` per § Documentation updates <!-- completed: 2026-10-24T19:43 -->
- [x] Rewrite `docs/docs/spec/cli-options.md` per § Documentation updates, removing every `member ping` mention <!-- completed: 2026-10-24T19:43 -->
- [x] Update `docs/docs/spec/message-envelope.md` and `docs/docs/spec/data-model.md` <!-- completed: 2026-10-24T19:43 -->
- [x] Update `docs/docs/concepts/overview.md`, `docs/docs/concepts/coding-agents.md`, `docs/docs/concepts/member-lifecycle.md` and `docs/docs/how-to/mixed-backend-team.md` <!-- completed: 2026-10-24T19:43 -->
- [x] Update `docs/docs/how-to/design-doc-development.md` and `docs/docs/spec/coding-agent-backends.md` for Director wake <!-- completed: 2026-10-24T19:43 -->
- [x] Run `mise //docs:build` and fix broken links or anchors (removed `#member-ping`, `#pane-state-classification`) <!-- completed: 2026-10-24T19:45 -->

### Step 2: SPEC, README, skills and rules

- [x] Mirror the contract changes into `SPEC.md` through the `/update-readme` skill, covering every section listed in its § Documentation updates row, and check `README.md` <!-- completed: 2026-10-24T19:50 -->
- [x] Update `skills/cafleet/SKILL.md`, `skills/cafleet/roles/director.md` and `skills/cafleet/roles/member.md` <!-- completed: 2026-10-24T19:50 -->
- [x] Update `skills/cafleet/reference/prompt-routing.md`, `skills/cafleet/reference/supervision.md` (including § Director wake replacing § Asynchronous Wait Rule) and the cue intros in `skills/cafleet/reference/coding-agents.md` <!-- completed: 2026-10-24T19:50 -->
- [x] Rebind each backend's Director-receiving notes in `skills/cafleet/reference/coding-agents.md` to `supervision.md#director-wake` <!-- completed: 2026-10-24T19:50 -->
- [x] Replace finite receiving with the wake procedure in `skills/cafleet-design-doc/create/create.md`, `execute/execute.md` and `interview/interview.md` <!-- completed: 2026-10-24T19:50 -->
- [x] Remove the `member ping` bullet from `.claude/rules/bash-tool.md` <!-- completed: 2026-10-24T19:50 -->
- [x] Confirm no living document still mentions `member ping`, pane-state classification as an input gate, `CAFLEET_MAX_TEXT_LEN` preview truncation, `supervision.md#asynchronous-wait-rule`, or `message wait` as the Director's background or repeated receive <!-- completed: 2026-10-24T19:50 -->

### Step 3: Escape-first transport

- [x] Add `Lead`, `ESCAPE_SETTLE_DELAY`, `ESCAPE_TIMEOUT`, `MIN_LITERAL_BUDGET`, `REVALIDATION_MARGIN` and `INPUT_RESERVATION` to `cafleet/src/multiplexer/mod.rs` <!-- completed: 2026-10-24T19:55 -->
- [x] Extend `submit_prompt` with the capped Escape argv, the Escape reservation, the post-Escape `literal_budget` recomputation and not-started Escape failures <!-- completed: 2026-10-24T19:55 -->
- [x] Change `send_prompt` to take `lead` in the trait, `TmuxMultiplexer`, `HerdrMultiplexer`, `AnyMultiplexer` and the test mock <!-- completed: 2026-10-24T19:55 -->
- [x] Confirm the herdr Escape key token with `herdr pane send-keys --help` and use it in the herdr argv <!-- completed: 2026-10-24T19:55 -->
- [x] Delete `send_inline_preview` and `inline_preview_payload` <!-- completed: 2026-10-24T19:55 -->
- [x] Add the transport tests from § Tests <!-- completed: 2026-10-24T20:15 -->

### Step 4: Notification line, pane lock and termination deferral

- [x] Add `fold_visible`, `INLINE_BODY_LIMIT`, `RESUME_CLAUSE`, `DISMISSED_PROMPT_NOTE` and `notification_line` to `cafleet/src/delivery.rs` <!-- completed: 2026-10-24T19:59 -->
- [x] Add the notification-line unit tests <!-- completed: 2026-10-24T20:15 -->
- [x] Add `Deadline::reserving` and `PaneLock::acquire_within` to `cafleet/src/input.rs` <!-- completed: 2026-10-24T19:59 -->
- [x] Change `lock_member` to `lock_member(conn, settings, member, deadline)`: file lock first, then reload under `with_database_budget` <!-- completed: 2026-10-24T19:59 -->
- [x] Add lock-wait tests (released within the wait, released just before `deadline − INPUT_RESERVATION`, still busy at that point) <!-- completed: 2026-10-24T20:15 -->
- [x] Add `cafleet/src/signals.rs` with `DeferTermination` and open its span at each call site in § Termination during pane input <!-- completed: 2026-10-24T19:59 -->
- [x] Confirm on tmux and herdr that a spawned member session's process does not inherit the blocked signal set <!-- completed: 2026-10-24T20:50 -->
- [x] Add the termination-deferral tests and the blocking-literal fake option <!-- completed: 2026-10-24T20:15 -->

### Step 5: Draining delivery

- [x] Replace `AttemptOutcome` with the `sent` + `AttemptEnd` shape in `cafleet/src/receive.rs` <!-- completed: 2026-10-24T20:15 -->
- [x] Implement `deliver_pending` in `cafleet/src/delivery.rs` per § Delivery procedure, with no capture, the `ClaimSkip` enum and the post-timeout revalidation read <!-- completed: 2026-10-24T20:15 -->
- [x] Rename `runtime::attempt_preview` to `runtime::deliver_pending` and update `BrokerReceiver::attempt` <!-- completed: 2026-10-24T20:15 -->
- [x] Update the `message wait` bookkeeping in `receive::wait` per the outcome table <!-- completed: 2026-10-24T20:15 -->
- [x] Update `message send` in `cafleet/src/cli/message.rs`: `notification_sent` from `sent`, stderr from `Failed` <!-- completed: 2026-10-24T20:15 -->
- [x] Delete the Director early return in `delivery::preview_snapshot` and the `m.member_id!=f.director_member_id` filter in `broker::panes::pending_previews` <!-- completed: 2026-10-24T20:15 -->
- [x] Add the Director delivery, Director self-send and `pending_previews` tests from § Tests <!-- completed: 2026-10-24T20:39 -->
- [x] Add `unconfirmed_previews` to the roster query in `broker::members` and the `member list` / `member show` JSON presenters in `presentation.rs`, with the tests from § Tests <!-- completed: 2026-10-24T20:15 -->
- [x] Update the shared fakes in `cafleet/tests/common/mod.rs` and `cafleet/tests/common/receive.rs` per § Tests <!-- completed: 2026-10-24T20:39 -->
- [x] Update `cafleet/tests/delivery.rs`, `notification_retry.rs`, `message_wait.rs`, `cli_message.rs` and `receive.rs` unit tests per § Tests <!-- completed: 2026-10-24T20:39 -->

### Step 6: `member prompt`, `member exec` and `member ping`

- [x] Remove capture and classification from `dispatch_input`, send with `Lead::Escape`, and delete `input_state` <!-- completed: 2026-10-24T20:39 -->
- [x] Emit the lock-busy errors for prompt and exec <!-- completed: 2026-10-24T20:39 -->
- [x] Add the `cannot prompt the Director's own pane` guard to `member prompt` before pane I/O <!-- completed: 2026-10-24T20:39 -->
- [x] Delete the `Ping` variant, `fn ping` and its parser test cases in `cafleet/src/cli/mod.rs` <!-- completed: 2026-10-24T20:39 -->
- [x] Update `cafleet/tests/cli_member.rs`, `cli_exec.rs` and `pane_input.rs`: remove ping tests, add the ping absence test and the Director-prompt guard test, assert Escape-first argv and no capture <!-- completed: 2026-10-24T20:39 -->

### Step 7: Classifier removal

- [x] Delete `cafleet/src/pane_state.rs` and its `pub mod` line in `cafleet/src/lib.rs` <!-- completed: 2026-10-24T20:39 -->
- [x] Delete `cafleet/tests/pane_state.rs` and `cafleet/tests/fixtures/pane-readiness/` <!-- completed: 2026-10-24T20:39 -->
- [x] Remove `DELIVERY_CAPTURE_LINES` and other helpers left without a caller <!-- completed: 2026-10-24T20:39 -->
- [x] Update `cafleet/tests/docs_sync.rs` assertions, including the capture-cue test <!-- completed: 2026-10-24T20:39 -->
- [x] Rewrite the Director-receiving docs-sync tests and add the `skills/` check per § Tests <!-- completed: 2026-10-24T20:39 -->

### Step 8: Verification

- [x] Run `mise //cafleet:test` <!-- completed: 2026-10-24T20:55 -->
- [x] Run `mise //cafleet:lint` <!-- completed: 2026-10-24T20:55 -->
- [x] Run `mise //cafleet:typecheck` <!-- completed: 2026-10-24T20:55 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-10-24T20:55 -->
- [x] Search the repository for `PaneState`, `hold_reason`, `send_inline_preview`, `member ping`, `poll-trigger` and `asynchronous-wait-rule`, plus a whole-word `ping` search (`grep -rnw ping`), outside `design-docs/`, and resolve every hit <!-- completed: 2026-10-24T20:55 -->
- [x] Check each Success Criteria item against the test evidence <!-- completed: 2026-10-24T20:55 -->
