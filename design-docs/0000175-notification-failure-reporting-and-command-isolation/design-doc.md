# Surface Persisted-Message Notification Failures and Isolate CAFleet Commands

**Status**: Approved
**Progress**: 26/26 tasks complete
**Last Updated**: 2026-08-25

## Overview

Make `cafleet message send` report an explicit partial failure when the broker persisted a unicast message but the attempted pane notification failed, including the message id, the raw actionable cause, and recovery guidance that forbids resending. Prevent the reproduced shell-tool race by adding one backend-neutral CAFleet skill rule that isolates every one-shot `cafleet` command, while retaining the existing backend-specific hosting exception for the long-lived monitor process.

## Success Criteria

- [ ] An attempted unicast pane notification failure leaves exactly one message row persisted as `input_required` and makes `cafleet message send` exit 1.
- [ ] The CLI partial-failure text contains the persisted message id, the raw tmux/herdr error detail, an explicit “do not resend” instruction, and both supported recovery paths (`cafleet member ping` and recipient-side `cafleet message poll`).
- [ ] Notification failure triggers no automatic retry at the multiplexer, broker, CLI, or skill layer.
- [ ] Successful notification, self-send, and a recipient with no pane retain their existing exit-0 text/JSON contracts; the latter two remain intentional notification skips.
- [ ] `message broadcast` keeps its current `recipients`/`delivered` output and exit behavior, and `POST /api/messages/send` keeps its current HTTP response contract.
- [ ] No database column, table, status value, or migration is added; existing databases and queued messages remain readable without conversion.
- [ ] The core `skills/cafleet/SKILL.md` is the sole normative source for one-shot command isolation and applies identically to Claude, Codex, and OpenCode.
- [ ] The long-lived `cafleet monitor` process remains the sole command-isolation exception and continues to use the launch mechanism already selected by its coding-agent overlay.
- [ ] The existing Director asynchronous-wait contract unambiguously makes dispatch a turn boundary: the Director ends or yields instead of sleeping, busy-waiting, or recurring-polling until a member replies.
- [ ] Automated tests cover raw tmux/herdr failure propagation, broker persistence, CLI output/exit behavior, unchanged non-goals, and the skill wording guard.

---

## Background

The [confirmed reproduction for issue #338](https://github.com/himkt/cafleet/issues/338#issuecomment-5386474500) exercised a one-shot CAFleet command inside a longer shell-tool invocation. Follow-up shell work kept the Codex tool invocation active while Herdr tried to inject a pane notification, so the notification could fail after the message had already been written. The durable queue behaved correctly—the message remained available as an `input_required` row—but the sender-facing path collapsed the actionable backend failure to `false` and still printed the normal success result.

The present call chain explains that split:

1. `broker::send_message` inserts the full unicast row and obtains its `message_id`.
2. `InlinePreviewSender::send_inline_preview` delegates to the selected multiplexer.
3. tmux and Herdr wrap every inline-preview error into a boolean `false`; `CliNotifier` also converts multiplexer-resolution failure into silent non-delivery.
4. The broker returns `{message, notification_sent: false}`, and the CLI prints `Message sent.` with exit 0.

Persistence-before-notification is the correct ordering and must remain. The defect is that an attempted notification failure is indistinguishable from intentional notification skips and is therefore not actionable to the coding agent that issued `message send`.

---

## Specification

### Scope and non-goals

| Area | Decision |
|---|---|
| CLI scope | Change only unicast `cafleet message send` when a pane notification was attempted and failed. |
| Intentional skips | Self-send and an active recipient whose placement has no pane id continue to succeed with `notification_sent: false`; neither is a partial failure. |
| Persistence | Preserve the current insert-before-notify ordering and leave a failed-notification message in `input_required`. Never delete, roll back, cancel, fail, or duplicate the row. |
| Retry | Do not retry automatically. The multi-step Escape/payload/Enter sequence may have partially affected the pane, so an automatic retry is unsafe. |
| Broadcast | No per-recipient failure schema or new exit behavior. Keep the current summary plus `recipients` and successful-preview `delivered` counts. |
| WebUI | No HTTP 202/207 response, response-field change, or frontend change. `POST /api/messages/send` remains 200 with `{message_id, status}` after persistence. |
| Data model | No schema migration and no durable notification-attempt record. The message queue remains the source of truth. |
| Broader delivery semantics | End-to-end acknowledgement, atomic pane execution, and the separate hypothesis where a notifier reports success but no Codex turn opens are follow-ups, not part of this design. |
| Monitor lifecycle | Do not redesign monitor launch or liveness. The existing overlay-resolved background/managed-execution mechanisms remain authoritative. |
| Async waiting | Clarify and enforce the existing Director turn-boundary rule. Add no scheduler, timer abstraction, polling framework, or runtime feature. |

Internal refactoring may touch shared notifier and multiplexer interfaces only to retain the error cause and keep existing callers compiling. It must not change broadcast or WebUI output behavior.

### Rust notification-result contract

Replace the boolean only at the inline-preview boundary; the other best-effort multiplexer operations remain unchanged.

```rust
trait Multiplexer {
    // Existing send_poll_trigger and send_wake_trigger contracts stay unchanged.
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), MultiplexerError>;
}

trait InlinePreviewSender {
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), String>;
}

pub struct SendMessageOutcome {
    pub(crate) payload: serde_json::Value,
    pub(crate) notification_error: Option<String>,
}
```

The result type is public so the existing public broker function does not expose a private type, while its fields are crate-visible because only sibling CLI/WebUI call sites consume them. It is not an external wire contract.

`MultiplexerError` remains the existing backend error type. tmux and Herdr must stop applying their boolean best-effort wrappers specifically to `send_inline_preview`; instead, they return the existing `map_run_error` detail from whichever Escape, payload, or Enter operation failed. The string is preserved verbatim; the CLI does not separately append the message body, although an existing backend error that renders the failed payload argv may already contain its sanitized preview.

`CliNotifier::new(settings) -> Self` remains infallible even though it runs before `broker::send_message`. It stores either the resolved multiplexer or the string form of the `resolve_mux` error; it does not return that error from construction. Only a later `CliNotifier::send_inline_preview` call exposes the retained error. The broker reaches that call only after inserting the row and only after selecting a non-self recipient placement with a pane id. Consequently an unavailable or ambiguous multiplexer produces one persisted row followed by the CLI partial failure when notification was required, but the identical environment cannot turn a self-send or no-pane skip into an error.

For a missing executable, the result-returning tmux and Herdr inline-preview methods retain their existing `binary_exists` precheck and directly construct `MultiplexerError::new("tmux binary not found on PATH")` or `MultiplexerError::new("herdr binary not found on PATH")`. They do not call `ensure_available`, because its additional environment/session validation is not part of inline delivery, and they do not fall through to runner-dependent `RunError::BinaryNotFound` formatting. Backend tests must assert these exact strings. Subprocess failures after that precheck continue through the existing `map_run_error` path.

`SendMessageOutcome` is a narrow unicast return type, not a generalized delivery-status taxonomy. Its `payload` retains the exact current JSON value:

```json
{"message":{"message_id":42,"status_state":"input_required"},"notification_sent":false}
```

The abbreviated message above illustrates the envelope only; the real `message` object keeps every existing typed-column field and key order. `notification_error` is Rust-side metadata and is never inserted into that JSON value.

The broker classifies the four cases as follows:

| Recipient/notification state | `payload.notification_sent` | `notification_error` | Row state |
|---|---:|---|---|
| Self-send | `false` | `None` | `input_required` |
| Recipient has no pane id | `false` | `None` | `input_required` |
| Pane notification succeeds | `true` | `None` | `input_required` |
| Pane notification is attempted and fails | `false` | `Some(<raw error>)` | `input_required` |

`broker::send_message` still returns `Ok(SendMessageOutcome)` for the last row because the broker operation that owns durability succeeded. The unicast CLI alone interprets `notification_error` as a sender-facing partial failure. This separation also makes the compatibility behavior explicit:

- `message broadcast` calls the result-returning notifier and increments `delivered` for `Ok(())`; it discards individual `Err` values exactly as it discards `false` today.
- The WebUI send handler reads `outcome.payload["message"]` and intentionally ignores `outcome.notification_error`, preserving its existing 200 response after persistence.
- No new `CafleetError` variant, cause enum, phase enum, or serializable error envelope is introduced.

### CLI partial-error contract

After `broker::send_message` returns, `cli/message.rs` checks `notification_error` before calling the normal success emitter. When present, it returns the existing `CafleetError::App` with this cafleet-authored message:

```text
Message <message-id> was persisted, but pane notification failed: <raw backend error>. Do not resend this message. Recover the recipient pane, then run 'cafleet member ping <recipient-id>' or have the recipient run 'cafleet message poll <recipient-id>'.
```

The top-level error handler supplies the existing `Error: ` prefix. `<raw backend error>` is inserted verbatim and may contain the backend command, its payload argv, and a newline-delimited stderr detail, matching existing `MultiplexerError` formatting. The partial-error formatter adds no separate copy of the sent message text.

| Mode | Exit | stdout | stderr |
|---|---:|---|---|
| Text, attempted notification succeeds | 0 | Existing `Message sent.` plus compact echo | Empty |
| `--json`, attempted notification succeeds | 0 | Existing untruncated `{message, notification_sent}` JSON | Empty |
| Text, self-send or no-pane skip | 0 | Existing success output with no new warning | Empty |
| `--json`, self-send or no-pane skip | 0 | Existing JSON with `notification_sent:false` | Empty |
| Text, attempted notification fails | 1 | Empty | `Error: ` plus the exact partial-error message |
| `--json`, attempted notification fails | 1 | Empty | The same text error as non-JSON mode |

The `--json` failure behavior deliberately follows CAFleet's existing global error contract: `--json` selects successful command output and does not create JSON error envelopes. This is the smallest compatible representation and avoids a one-command-only error taxonomy.

The recovery contract is intentionally no-resend:

1. Treat `<message-id>` as authoritative proof that persistence succeeded.
2. Repair or re-engage the recipient pane.
3. Run `cafleet member ping <recipient-id>` as its own shell-tool invocation, or have the recipient run `cafleet message poll <recipient-id>` as its own shell-tool invocation.
4. Consume and ACK the existing row normally. Do not issue a second `message send` for the same content.

### Global one-shot command isolation rule

Add a clearly headed normative section to `skills/cafleet/SKILL.md`. It is the single source of truth for all installed coding-agent backends and must state these requirements without backend-specific variants:

- Every one-shot `cafleet` process is the only command in its shell-tool invocation.
- Run a sequence of CAFleet operations as separate shell-tool calls.
- Do not place a one-shot CAFleet command beside another command using a newline, `;`, `&&`, a pipe, shell `&`, or any other setup/follow-up command.
- Leading `NAME=value` assignments that set the environment of the CAFleet process are allowed; they do not start another process. They must immediately precede the CAFleet executable. Do not substitute an `env` helper process or append another command.
- Shell redirection does not authorize another process; examples that need a body should prefer the existing positional argument or `--file <path>` rather than a pipe.

The same section must carry a backend-neutral diagnostic note for permission errors: a CAFleet command that fails with an operating-system permission error (“Operation not permitted” / “Permission denied”, commonly surfacing as a multiplexer socket or pane-command failure) signals that the invocation likely ran outside the coding agent’s command auto-approval scope — a compound invocation does not match single-command allow rules, so the shell tool executes it under the agent’s restricted sandbox or permission set. The documented response is to re-run the CAFleet command as its own isolated invocation, honoring the no-resend rule whenever a persisted message id was already reported; retrying the compound form is forbidden. The note names no specific backend, sandbox implementation, or allow-rule syntax.

The sole exception is the long-lived `cafleet monitor` process. Its invocation must still contain only that monitor process, but it may use exactly the background or managed-execution mechanism resolved by the existing coding-agent overlay, including OpenCode's shell `&` form and tool-managed background modes. The core rule points to the overlay for launch syntax and does not duplicate lifecycle mechanics.

All roles already load the same core CAFleet skill, so the rule does not need copies in Claude, Codex, or OpenCode presets. Audit role/reference examples for contradictions: compound one-shot examples must become separate invocations, while monitor examples remain unchanged and are covered by the explicit exception. Pointer text may refer readers back to the core section; it must not restate backend-specific rules.

The existing `## Send` section of that same core skill also gains one normative recovery paragraph. When `message send` exits nonzero while stating that `Message <id> was persisted`, the id proves the send committed: the sender MUST NOT resend the body. It repairs or re-engages the recipient and runs an isolated `cafleet member ping <recipient-id>`, or the recipient runs an isolated `cafleet message poll <recipient-id>`, then consumes and ACKs that existing row. This is the only skill-layer response to the partial failure; it introduces no retry.

#### Existing asynchronous-handoff contract

Command isolation complements, but does not create or replace, the existing Director rule in `skills/cafleet/reference/supervision.md` § *Asynchronous Wait Rule*. Strengthen that section and point to it from the core command-isolation guidance using this normative boundary:

> **An asynchronous handoff is a turn boundary.** After a Director dispatches work to a member, the Director MUST end or yield its active turn. It MUST NOT remain active waiting for completion by running `sleep`, repeated or periodic `cafleet message poll`, a busy-wait loop, or any equivalent timer/polling loop. A CAFleet/monitor notification resumes the workflow in a later turn. A user-requested one-off status check is allowed, but MUST NOT become recurring polling.

This rule applies after spawn dispatch, ordinary assignments, review routes, and other asynchronous member work. It does not prohibit the on-demand inbox poll performed when an inbound notification has already reopened a later turn, and it does not prohibit one user-requested status snapshot. After acting on already-arrived inputs and dispatching new work, the Director returns control again.

Keep this distinct from the long-lived monitor member. The monitor remains CAFleet's existing notification/heartbeat mechanism, owns its execution handle and liveness checks, and wakes the Director when action is needed. This issue neither moves polling into the Director nor adds a second scheduler, timer, polling loop, or runtime service.

### Documentation surfaces

| Surface | Required change |
|---|---|
| `SPEC.md` | Amend the broker/multiplexer overlap, messaging result, CLI send behavior, error table, and inline-preview fail-fast/best-effort split. Record the one-shot command-isolation rule and monitor exception as an agent-operation contract. |
| `docs/docs/spec/cli-options.md` | Pin exit 1, stdout/stderr, exact partial-error wording, `--json` failure behavior, persistence/no-resend semantics, and unchanged skip cases. |
| `docs/docs/spec/multiplexer-backends.md` | Change only inline-preview error propagation to `Result`; retain boolean best effort for poll/wake triggers and document broadcast's compatibility adapter. |
| `docs/docs/concepts/coding-agents.md` | Explain why one-shot CAFleet commands use isolated shell-tool invocations and defer monitor hosting syntax to the existing backend lifecycle guidance. |
| `skills/cafleet/SKILL.md` | Add the sole normative global rule, its prohibited compound forms, allowed leading environment assignments, separate-call guidance, the backend-neutral permission-error diagnostic note, and the narrow monitor exception. In `## Send`, add the single persisted-id/no-resend paragraph and isolated ping/poll recovery. |
| `skills/cafleet/reference/supervision.md` | Strengthen the existing *Asynchronous Wait Rule* with the dispatch turn boundary, prohibited Director-side sleep/recurring-poll loops, notification-based resumption, and one-off status-check allowance. |
| `skills/cafleet-design-doc/create/create.md` and `create/roles/director.md` | Replace the known “periodic `cafleet message poll`”/“own periodic polling” directions with dispatch, end/yield, notification-based resumption, and one later on-demand poll/ACK. Treat every correction round as a new asynchronous handoff and turn boundary. |
| `skills/cafleet-design-doc/interview/interview.md` | Replace “poll ... until” and “wait again with ... poll” directions with the same turn-boundary sequence. |
| `skills/cafleet-design-doc/execute/**` | Audit the execute workflow and roles, including “Wait ... via poll” after Tester and Programmer dispatches, and replace any recurring-wait implication with notification-based later-turn resumption. |
| Remaining `skills/cafleet-design-doc/**` roles/references | Audit for equivalent sleep, timer, busy-wait, recurring-poll, and compound one-shot guidance. Add concise pointers to the core contracts where useful; do not duplicate the normative isolation rule or overlay mechanics. |

### Migration and compatibility

- **Database**: no migration. The `messages` schema and all state values are unchanged; old and new binaries read the same rows.
- **Queued messages**: a notification-failed row is indistinguishable at rest from any other unacked delivery and remains recoverable through `poll`/`ack`.
- **Successful CLI output**: byte-for-byte contract remains unchanged in text and JSON modes.
- **Intentional behavior change**: an attempted unicast notification failure changes from exit 0 plus `notification_sent:false` to exit 1 plus the explicit persisted-message error. Automation must treat this as “send committed, notification incomplete,” not as permission to retry.
- **Rust internals**: `Multiplexer::send_inline_preview`, `InlinePreviewSender`, and `broker::send_message` change signatures. They are internal implementation seams for the binary; no external wire or database migration is provided.
- **Broadcast/WebUI**: their outward contracts remain unchanged through explicit adapters at their call sites.
- **Installed skills**: the working-tree source is embedded at build time and installed identically for all three agents by `cafleet setup`. After upgrading the binary, operators run `cafleet setup` in its own shell-tool invocation to refresh assets; repository development may use `mise //:skill-install` as its own invocation.
- **Async orchestration**: no executable, scheduler, timer, database, or monitor-runtime change implements the turn boundary. It is clarification plus contract enforcement in the existing skill and documentation tests.

### Automated verification

Tests must pin both the new behavior and the deliberately unchanged surfaces:

| Layer | Coverage |
|---|---|
| tmux unit tests | Success retains Escape → literal payload → Enter choreography; missing binary returns exactly `tmux binary not found on PATH`; injected subprocess failure returns the raw `MultiplexerError` instead of `false`. At least one failure assertion includes the exact failing argv and stderr. |
| Herdr unit tests | Success retains Escape → send-text → Enter choreography; missing binary returns exactly `herdr binary not found on PATH`; injected subprocess failure returns the raw `MultiplexerError`. At least one failure assertion covers `pane send-text` or trailing `enter`. |
| Notifier/broker unit tests | A retained `resolve_mux` failure is exposed only when a pane preview is attempted after persistence: it yields one full-text `input_required` row, `notification_sent:false`, and `notification_error`. Under that same unavailable/ambiguous multiplexer setup, self-send and no-pane paths return `None` error and remain successful. All attempted failures perform one notification attempt; successful send remains unchanged. |
| Broadcast regression | One failed preview still returns the existing single summary envelope, recipient count, and delivered-success count without an error list or nonzero CLI exit. |
| CLI integration | A forced tmux shim `send-keys` failure produces exit 1, empty stdout, the persisted id/raw cause/no-resend/ping/poll stderr text, and exactly one persisted row. Repeat with `--json` to pin the same existing text-error channel. Add unavailable/ambiguous `resolve_mux` cases proving attempted notification exits 1 after one insert while self-send and no-pane remain exit 0. |
| WebUI regression | Existing route tests remain green; if adapting the typed broker result requires a focused assertion, pin the unchanged 200 `{message_id,status}` response rather than adding fields. |
| Skill/docs contract | Extend `cafleet/tests/docs_sync.rs` to require the core one-shot isolation phrases, prohibited compound forms, allowed leading environment assignments without an `env` helper process, separate invocations, the backend-neutral permission-error diagnostic note (both quoted error spellings, the allow-rule mismatch cause, and the isolated re-run response), and the overlay-deferred monitor exception. Require the `## Send` persisted-id/committed/no-resend plus isolated ping/poll recovery paragraph. Add a focused asynchronous-wait guard over `skills/cafleet-design-doc/**` and the core supervision reference: require dispatch-as-turn-boundary, later-turn notification resumption, and the user-requested one-off status allowance; reject known “periodic poll,” “poll until,” “wait again with poll,” Director-side `sleep`, busy-wait, scheduler, or timer-loop guidance. |

Run each verification command in its own shell-tool invocation, consistent with the new rule. The final gates are `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //docs:build`, each separately.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Amend contracts before code

- [x] Update `SPEC.md` for the inline-preview result, persisted partial failure, unchanged broadcast/WebUI adapters, CLI error string, and one-shot isolation rule with monitor exception. <!-- completed: 2026-08-25T05:45 -->
- [x] Update `docs/docs/spec/cli-options.md` with the exact text/JSON output and exit contract plus recovery guidance. <!-- completed: 2026-08-25T05:45 -->
- [x] Update `docs/docs/spec/multiplexer-backends.md` with raw inline-preview error propagation and the unchanged boolean contracts for poll/wake triggers. <!-- completed: 2026-08-25T05:45 -->
- [x] Update `docs/docs/concepts/coding-agents.md` with the backend-neutral isolated-invocation guidance and overlay-deferred monitor exception. <!-- completed: 2026-08-25T05:45 -->
- [x] Audit the error/reference tables and remove obsolete claims that all notification failures are silently best-effort for unicast CLI sends. <!-- completed: 2026-08-25T05:45 -->

### Step 2: Preserve raw multiplexer failures

- [x] Change only `Multiplexer::send_inline_preview` and `AnyMultiplexer` dispatch to return `Result<(), MultiplexerError>`. <!-- completed: 2026-08-25T06:09 -->
- [x] Update tmux inline-preview delivery to return exact missing-binary text `tmux binary not found on PATH` and propagate Escape/payload/Enter failures with existing raw error formatting. <!-- completed: 2026-08-25T06:09 -->
- [x] Update Herdr inline-preview delivery to return exact missing-binary text `herdr binary not found on PATH` and propagate Escape/send-text/Enter failures with existing raw error formatting. <!-- completed: 2026-08-25T06:09 -->
- [x] Keep `CliNotifier::new` infallible; retain multiplexer resolution or delivery errors as raw strings and expose them only from an attempted `send_inline_preview`. <!-- completed: 2026-08-25T06:09 -->
- [x] Update focused tmux, Herdr, and notifier unit tests for exact missing-binary strings and deferred resolution failure while proving poll/wake best-effort behavior is unchanged. <!-- completed: 2026-08-25T06:09 -->

### Step 3: Surface the unicast CLI partial failure

- [x] Add public `SendMessageOutcome` with crate-visible payload/error fields, carrying the unchanged payload plus optional notification error for sibling CLI/WebUI consumers. <!-- completed: 2026-08-25T06:22 -->
- [x] Update `broker::send_message` to distinguish skip, success, and attempted failure after persistence without retry or rollback. <!-- completed: 2026-08-25T06:22 -->
- [x] Adapt broadcast to count `Ok(())` previews and discard individual errors without changing its result schema or exit behavior. <!-- completed: 2026-08-25T06:22 -->
- [x] Adapt the WebUI unicast call site to consume `outcome.payload` and preserve its current response. <!-- completed: 2026-08-25T06:22 -->
- [x] Make `cli/message.rs` return the exact `CafleetError::App` partial-failure message before the success emitter. <!-- completed: 2026-08-25T06:22 -->
- [x] Add broker and CLI regression tests for persistence, one attempt, exact error/recovery content, text and `--json` channels, successful sends, intentional skips, and resolution failure that cannot preempt insertion or fail self/no-pane skips. <!-- completed: 2026-08-25T06:22 -->

### Step 4: Add the global skill rule

- [x] Add the sole normative one-shot command-isolation section to `skills/cafleet/SKILL.md`, including separate calls, prohibited compound forms, allowed leading environment assignments, the backend-neutral permission-error diagnostic note, and the narrow monitor exception. <!-- completed: 2026-08-25T06:22 -->
- [x] Add the `## Send` persisted-id/committed/no-resend paragraph with isolated ping/poll recovery and no skill-layer retry. <!-- completed: 2026-08-25T06:22 -->
- [x] Point the exception to the existing coding-agent overlay without duplicating monitor lifecycle mechanics or backend-specific variants. <!-- completed: 2026-08-25T06:22 -->
- [x] Strengthen `skills/cafleet/reference/supervision.md` § *Asynchronous Wait Rule* and add a core pointer: dispatch ends/yields the turn, notifications resume later, one-off user status checks stay bounded, and Director sleep/recurring-poll loops remain forbidden. <!-- completed: 2026-08-25T06:22 -->
- [x] Audit `skills/cafleet-design-doc/**`, explicitly replacing create's periodic polling, interview's poll-until/wait-again wording, and execute's post-Tester/Programmer wait-via-poll wording; split other contradictory compound one-shot examples while preserving monitor launch examples. <!-- completed: 2026-08-25T06:22 -->
- [x] Add/extend `cafleet/tests/docs_sync.rs` guards for isolation (including allowed assignment prefixes but no `env` helper), monitor exception, core Send recovery, and the asynchronous-wait surfaces; reject periodic/poll-until/wait-again, timer, busy-wait, and recurring-poll guidance. <!-- completed: 2026-08-25T06:22 -->

### Step 5: Verify automated behavior

- [x] Run the focused tmux/Herdr multiplexer, broker messaging, CLI message, WebUI compatibility, and docs-sync tests as separate invocations. <!-- completed: 2026-08-25T06:32 -->
- [x] Run `mise //cafleet:test` in its own shell-tool invocation. <!-- completed: 2026-08-25T06:32 -->
- [x] Run `mise //cafleet:lint` in its own shell-tool invocation. <!-- completed: 2026-08-25T06:32 -->
- [x] Run `mise //docs:build` in its own shell-tool invocation. <!-- completed: 2026-08-25T06:32 -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-08-24 | Initial draft based on issue #338 reproduction and the clarified lean scope. |
| 2026-08-24 | Clarified the existing asynchronous-handoff turn boundary and added its wording/contract guard; no scheduler or runtime scope added. |
| 2026-08-24 | Resolved first review: deferred mux-resolution errors until an attempted post-insert notification, pinned missing-binary strings, added core no-resend guidance and dependent-workflow guards, and made Herdr/Codex live verification fully executable with retained evidence. |
| 2026-08-24 | Resolved second review: made the broker outcome crate-consumable, moved live binary/skills under writable workspace paths with inline environment propagation, and restored canonical one-pass spawn-placeholder substitution. |
| 2026-08-25 | User feedback during execution: the isolation section must include a backend-neutral diagnostic note — an OS permission error from a CAFleet command signals a compound invocation that missed the agent’s command allow rules and ran sandboxed; respond by re-running the command isolated (no resend). Guarded in docs_sync. |
| 2026-08-25 | User decision during execution: descoped the live Herdr/Codex verification gate (its Success Criterion, spec section, and two Step 5 tasks removed; partial `.live`/database/evidence artifacts deleted). The automated suites cover every shipped contract; non-goal-drift auditing is owned by the Step 5 Reviewer pass. |
