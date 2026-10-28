# Delivery-dismissed approval recovery

**Status**: Approved
**Progress**: 17/17 tasks complete
**Last Updated**: 2026-10-28

## Overview

A legacy unwrapped member's Escape-first input (a preview, `member prompt` or `member exec`) can cancel an open Claude Code prompt, which Claude Code reports to the model as a user rejection before the CAFleet input arrives (issue #499). This design binds a rule for recognizing and recovering from such a rejection to member command execution and Director dispatch in the Claude backend section, states affirmatively that the root Director and wrapped members receive no Escape from CAFleet, and locks both guarantees in with regression tests.

## Success Criteria

- [x] `skills/cafleet/reference/coding-agents.md` § `claude` → *Note → applies at* carries a row that states the rejection-recovery rule affirmatively, scopes each affected Claude prompt kind to the launch policy that exposes it, and binds the rule to member command execution, Director dispatch and the unwrapped preview contract.
- [x] The same table carries a row stating that the root Director and wrapped members receive no Escape from CAFleet, bound to Director wake and wrapped previews.
- [x] `skills/cafleet/roles/member.md` § Command execution specifies the backend-neutral member rule for all three Escape-first inputs: only a rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause is delivery-caused and the member re-issues the call or re-asks the question; a resume-clause-only preview leaves the rejection standing; a rejection followed by other submitted input is governed by that input.
- [x] `skills/cafleet/reference/prompt-routing.md` § Director-side dispatch and `skills/cafleet/roles/director.md` § Member Prompt require a pane check before `member exec` / `member prompt` to an explicitly unwrapped member.
- [x] `skills/cafleet/reference/supervision.md` § Director wake states that a tool rejection or dismissed decision in the Director's own pane never originates from CAFleet input. No Director-wake "re-issue" step is added.
- [x] `docs/docs/spec/coding-agent-backends.md` § Claude and `docs/docs/spec/multiplexer-backends.md` § Accepted trade-offs describe the same behavior for operators, with no cross-link between the agent reference and the operator page.
- [x] A Rust regression test proves that an ordinary member's drain, with messages pending in both directions, writes pane input only to that member's pane and never names the root Director's pane.
- [x] A Rust regression test proves that a wrapped preview submitted while a Claude permission form is visible is held with zero child bytes and leaves the native request, its token and its generation intact, for every bundled Claude permission-form fixture recorded at the inspection host's 36×120 geometry.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass.

---

## Background

The incident in issue #499 ran on Claude Code 2.1.302 with released cafleet 0.29.0, where every preview, the Director's included, led with Escape. The halted session was the Director: a member delivery reached the Director's pane while the Director's own `mise //cafleet:test` call awaited approval. The repository's `main` already changes both halves of that behavior:

| Recipient | Current input in `main` | Escape from CAFleet |
|---|---|---|
| Root Director | Broker records through native `message listen` stdout or manual broker reads; zero automatic pane input. `member prompt` / `exec` / `press` reject the Director's own pane. | no |
| Wrapped (runner-managed) member — every newly created member | Runner input into a verified empty composer; a candidate/open interaction, draft, work or unknown coverage holds the preview and rejects explicit prompt/exec input before bytes. | no |
| Explicitly unwrapped legacy member — rows created by cafleet 0.29.0 or earlier | Escape, settle, literal line, settle, Enter for previews, `member prompt` and `member exec`. Only the preview line carries the conditional dismissal clause. | yes |

The remaining exposure is the legacy unwrapped member, and which prompts it can have open depends on its launch policy:

| Member | Launch policy | Prompts Escape can cancel |
|---|---|---|
| Legacy unwrapped Claude member | `--permission-mode dontAsk`, the only Claude member policy before runner-managed members (cafleet 0.29.0 has no `--permission-mode` option) | An open `AskUserQuestion`, which needs no tool permission. `dontAsk` allows or denies every Bash, Edit/Write, WebFetch and MCP tool call without an approval prompt. |
| Wrapped Claude member under an explicit interactive mode (`--permission-mode default` / `manual`) | As selected | Tool approvals appear, but CAFleet sends no Escape and holds previews while they are open. |

Claude Code reports an Escape-cancelled prompt to the model as `The user doesn't want to proceed with this tool use. The tool use was rejected … STOP what you are doing and wait for the user to tell you how to proceed.` and only afterwards receives the CAFleet input. `coding-agents.md` § `claude` binds no note to this; `member.md` tells the member to follow the preview's dismissal clause "only when that legacy transport actually dismissed the prompt" without saying how to recognize that case, and says nothing about rejections caused by `member prompt` / `member exec`.

The issue's Director-side proposals (a Director-wake re-issue rule, generalizing a wake "step 6") target an event that cannot occur in `main`: the root Director receives no pane input, and the current Director wake procedure has five steps. This design states that structural guarantee instead of a rule for absent behavior. The issue's proposal 3 (avoid the Escape while an approval is pending) is already the wrapped-member behavior.

Release of `main` to users follows the normal release CI workflow and is outside this design. No CLI option, preview text, schema, error string or output layout changes; SPEC.md changes only where its Accepted trade-offs bullet mirrors the edited operator bullet.

---

## Specification

### Rejection-recovery rule

The rule covers every Escape-first input an explicitly unwrapped member receives: a preview line, Director `member prompt` text and `! COMMAND` input from `member exec`. The member cannot observe its own transport, so the dismissal clause is the sole discriminator: only an explicitly unwrapped preview ends with it, and only that line makes a rejection delivery-caused. The member classifies a rejection by the next input that follows it:

| Next input after the rejection | Member behavior |
|---|---|
| A `[cafleet msg …]` line ending with the Escape-dismissal clause `[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.` | The rejection is delivery-caused. Act on the delivery and ACK it, then re-issue the rejected call or re-ask the dismissed question if still needed. |
| A `[cafleet msg …]` line ending with the resume clause `[cafleet] Resume your work if something was still running.` (a wrapped preview) | The rejection stands: the preview reached an empty composer after the prompt closed. Act on the delivery and ACK it; treat the rejected call as declined. |
| Other submitted input: Director `member prompt` text, `! COMMAND` input from `member exec`, or operator text | That input governs. Follow it, and re-issue the rejected call or re-ask the question only when that input directs it. |
| No further input | Wait for the next input. |

An explicit operator or Director instruction to stop always holds. Re-issue means one fresh call with the same intent under the member's ordinary command-execution rules, including routing a host-denied command to the Director.

The third row is safe on both transports: wrapped `member prompt` / `member exec` input is rejected while an interaction is open, so it never dismisses a prompt, and before such dispatch to an explicitly unwrapped member the Director captures the pane. When the capture shows an open tool approval or question, the Director resolves it through the user relay first, or reaches the member with `message send`, whose preview carries the dismissal clause. A Director input that follows a dismissal it caused states whether the member re-issues the call.

Covered Claude prompt kinds are tool approvals for Bash, Edit/Write, WebFetch and MCP tools, and `AskUserQuestion`. Under the `dontAsk` policy that every legacy unwrapped Claude member runs, `AskUserQuestion` is the prompt such a member can have open; the rule treats a tool approval the same way.

The rule lives in two layers, following the coding-agent overlay contract:

| Layer | File | Content |
|---|---|---|
| Base (backend-neutral) | `skills/cafleet/roles/member.md` § Command execution; `skills/cafleet/reference/prompt-routing.md` § Director-side dispatch; `skills/cafleet/roles/director.md` § Member Prompt | The classification table and member behavior, and the Director's pre-dispatch pane check, phrased for any tool approval or decision prompt |
| Overlay (Claude) | `skills/cafleet/reference/coding-agents.md` § `claude` → *Note → applies at* | The Claude prompt kinds and their launch-policy scoping, the Claude rejection text, and the bindings |

### `coding-agents.md` § `claude` → *Note → applies at* — new rows

Append these two rows after the existing last row (the `dontAsk` row), before the `Evidence:` line:

```markdown
| Legacy unwrapped members run `dontAsk`, so their open prompt is an `AskUserQuestion`; a Bash, Edit/Write, WebFetch or MCP tool approval follows the same rule. Escape-first input cancels it and Claude Code reports `The user doesn't want to proceed with this tool use.` with a STOP instruction before the CAFleet input arrives: only a following `[cafleet msg …]` line ending with the Escape-dismissal clause makes it delivery-caused, so re-issue the call or re-ask the question; a resume-clause-only preview leaves the rejection standing, and following `member prompt` / `member exec` input governs. | [Member command execution](../roles/member.md#command-execution); [Director dispatch](prompt-routing.md#director-side-dispatch); [unwrapped previews](runtime/spec/multiplexer-backends.md#accepted-trade-offs) |
| The root Director receives broker records through Monitor stdout with zero automatic pane input, and wrapped members receive previews only into a verified empty composer without Escape. A tool rejection or dismissed question in either pane therefore comes from the operator or host, never from CAFleet input. | [Director wake](supervision.md#director-wake); [wrapped previews](member-runtime.md#reconcile-generated-input) |
```

Replace the section's `Evidence:` line (currently `Evidence: Claude Code 2.1.296, checked 2026-10-22; [host continuation](https://code.claude.com/docs/en/goal) and [block cap](https://code.claude.com/docs/en/hooks-guide#stop-hook-hits-the-block-cap). Local probes establish only exercised behavior.`) with exactly:

```markdown
Evidence: Claude Code 2.1.296, checked 2026-10-22; [host continuation](https://code.claude.com/docs/en/goal) and [block cap](https://code.claude.com/docs/en/hooks-guide#stop-hook-hits-the-block-cap). Claude Code 2.1.302 with cafleet 0.29.0 reported an Escape-cancelled tool approval as a user rejection (issue #499). Local probes establish only exercised behavior.
```

Issue #499 reports Claude Code 2.1.302 ("Claude Code 2.1.302, herdr, Claude Director with three Claude members").

### `member.md` changes

1. In § On spawn, replace the last two sentences of the delivered-line paragraph ("Wrapped previews carry the resume clause … actually dismissed the prompt.") with:

   ```markdown
   Wrapped previews end with the resume clause and wait for an empty composer. Explicitly unwrapped previews also carry the Escape-dismissal clause, handled under [delivery-dismissed prompts](#command-execution).
   ```

2. In § Command execution, add this paragraph after the "Report only observed results." paragraph:

   ```markdown
   **Delivery-dismissed prompts.** Explicitly unwrapped previews, `member prompt` text and `member exec` input lead with Escape, which can cancel your open tool approval or question; your host can report that as a user rejection before the CAFleet input arrives. When the next input after a rejection is a `[cafleet msg …]` line ending with the Escape-dismissal clause, the rejection came from the delivery: act on the delivery, then re-issue the rejected call or re-ask the dismissed question if you still need it. A `[cafleet msg …]` line ending with the resume clause only is a wrapped preview that arrived after the prompt closed: the rejection stands, so act on the delivery and treat the call as declined. When the next input is other submitted text or a native command, follow it and re-issue or re-ask only when it directs you to; with no further input, wait. An explicit operator or Director instruction to stop always holds; your backend's bound notes name the affected prompt kinds.
   ```

### `prompt-routing.md` § Director-side dispatch change

Add this paragraph after the paragraph that ends "The host owns environment, permissions, execution and visible output.":

```markdown
Escape-first input to an explicitly unwrapped member cancels an open tool approval or question in its pane. Capture that member's pane before `member exec` or `member prompt`; when the capture shows an open prompt, resolve it through the user relay first, or reach the member with `message send`, whose preview carries the dismissal clause.
```

### `director.md` § Member Prompt change

Append this sentence to the section's paragraph:

```markdown
Before prompting an explicitly unwrapped member, run the open-prompt check in [Director-side dispatch](../reference/prompt-routing.md#director-side-dispatch).
```

§ Member Exec already ends with "Follow [routing](../reference/prompt-routing.md)." and needs no change.

### `supervision.md` § Director wake change

Append one sentence to the intro paragraph, after "…including on startup failure, receiver loss or missing native tools.":

```markdown
A tool rejection or dismissed decision in the Director's own pane therefore comes from the operator or host, never from CAFleet input; follow it as reported.
```

The numbered wake steps are unchanged.

### Operator documentation

`docs/docs/spec/coding-agent-backends.md` § Claude — add this paragraph after the "Native shell probes …" paragraph:

```markdown
CAFleet input cancels no Claude tool approval or `AskUserQuestion` in the root Director's pane, which receives no automatic pane input, or in a runner-managed member's pane, where previews wait for an empty composer and send no Escape. A legacy unwrapped member runs `dontAsk`, so its open prompt is an `AskUserQuestion`; its Escape-first previews, `member prompt` and `member exec` input can cancel it, and Claude Code then reports a user rejection before the input arrives. The member treats a rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause as delivery-caused and re-asks the question or re-issues the call. A runner-managed member's preview ends with the resume clause only and arrives after the prompt closed, so a rejection before it stands. A rejection followed by Director prompt or native-command input follows that input, and the Director checks the pane for an open prompt before such dispatch. Recreating a legacy unwrapped member makes it runner-managed and removes the Escape lead.
```

`docs/docs/spec/multiplexer-backends.md` § Accepted trade-offs — replace the bullet "Where Escape dismisses an ordinary member's question, confirmation or approval prompt, …" with:

```markdown
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the host can report a user rejection before the input arrives; Claude Code reports `The user doesn't want to proceed with this tool use.` The preview's dismissed-prompt clause marks that rejection as delivery-caused, and the member re-issues the required tool call or re-asks the question. `member prompt` and `member exec` input carries no clause; the Director checks the pane for an open prompt before that dispatch.
```

`SPEC.md` § Accepted trade-offs carries the same bullet; replace it with the identical new bullet so the specification matches the operator page.

The page is the symlink target of `skills/cafleet/reference/runtime/spec/multiplexer-backends.md`, so the agent reference link in the new Claude note row resolves to this edited section. `coding-agents.md` and `coding-agent-backends.md` stay independent: neither links the other.

### Regression tests

Both tests lock behavior that `main` already implements, so they pass without production changes. A failure signals a regression to fix in production code; keep the assertions as specified.

#### `a_worker_drain_with_both_directions_pending_never_names_the_root_directors_pane`

Location: `cafleet/src/delivery.rs` tests, beside `a_drain_submits_pending_messages_fifo_with_one_escape_and_keeps_full_rows`, reusing its helpers.

1. `create_fleet(&mut conn, …)` (root Director placed at `%0`), then `register(&mut conn, fleet, "worker", Some("%2"))` for one unwrapped worker.
2. Send worker → Director (`reply`), then Director → worker (`assignment`).
3. `let runner = crate::multiplexer::recording::FakeRunner::with_binary("tmux"); runner.succeed(3);` and run `deliver_pending(&mut conn, &settings, worker, Some(fleet), Deadline::new(), &BTreeSet::new(), || Ok(fake_mux(runner.clone())))`.
4. Assert `result.sent == vec![assignment.message_id]` and `result.end == AttemptEnd::Drained`.
5. Assert `runner.run_argvs()` has three calls, each targets `%2`, exactly one ends in `Escape`, and no argv contains `%0`.
6. Assert the Director's `reply` keeps `notified_at` NULL and `broker::poll_messages(&conn, director)` still returns it.

The Director-drain half needs no new test: `director_delivery_ends_before_resolving_or_locking_a_pane` already covers it per backend.

#### `a_preview_during_a_visible_claude_permission_form_is_held_without_child_bytes`

Location: `cafleet/src/runtime/runner/input.rs` tests, beside `permission_claude_blink_cursor_transit_keeps_generation_and_token_and_admits_respond`.

Fixtures: `claude-2.1.302-default-permission-1.json` (`CLAUDE_PERMISSION_SCREEN`) and `claude-2.1.302-boxed-single-row-permission-1.json`. Both are recorded at 36×120, the geometry `with_claude_inspection` runs, and both are in the detection `fixture_ids` of profile `claude-2.1.302-default-3` in `bundled.json`. The 36×52 `wrapped` and 36×89 `boxed` fixtures are excluded because the host's terminal size differs from their recorded geometry.

For each fixture, inside `with_claude_inspection(|h, _, _| …)`:

1. Feed the form: `claude_blocked_form(h)` for the default fixture; for the boxed single-row fixture, add a test constant `CLAUDE_BOXED_SINGLE_ROW_SCREEN` (`include_str!` of that fixture) and feed `native_screen_bytes(CLAUDE_BOXED_SINGLE_ROW_SCREEN)` into `h.pty.child.bytes`, then call `settled_ids(h)` exactly as `claude_blocked_form` does.
2. `let inspected = inspect(h, ids);` assert `inspected.request_token.is_some()`; record `h.controller.request_generation()`, and snapshot `h.pty.offers` and `h.pty.program`.
3. Send Director → member with `crate::broker::send_message(&mut h.fixture.conn, run.director_member_id, run.member_id, …)` and build a request with key `preview:<run_id>:<message_id>` whose operation is `InputControlOperation::SubmitPrompt(SubmitInput { operation_key, input_kind: storage::InputKind::Preview, message_id: Some(message_id), literal })`.
4. Submit it with `h.coordinator.handle_received(&mut h.fixture.conn, &h.fixture.run, request, &h.controller, h.until)` and match `Some(InputControlReply::Submission(held))`.
5. Assert `(held.status, held.stage, held.input_id) == (SubmissionStatus::Held, SubmissionStage::ZeroBytes, None)` and `h.coordinator.pending.is_none()`.
6. Assert `h.pty.offers` and `h.pty.program` equal their step-2 snapshots (no new byte, in particular no `0x1b`), `storage::find_input(&h.fixture.conn, run.run_id, &key)` is `None`, and the message's `notified_at` is NULL.
7. Assert `h.controller.current_request().is_some()`, `request_generation()` is unchanged, and `inspect(h, ids).request_token == inspected.request_token`.

Existing coverage stays as is and needs no new test: `director_delivery_ends_before_resolving_or_locking_a_pane`, `preview_rotation_selects_only_concrete_ordinary_placements` and its siblings in `broker/panes.rs`, the Director-pane rejections of `member prompt` / `exec` (`cli/member.rs`) and `member press` (`runtime/press.rs`), and the exact accepted-preview bytes in `step5_operations_preview_binds_the_exact_run_message_key_and_durable_recipient`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Agent reference documentation

- [x] Add the two Note → applies at rows and replace the `Evidence:` line in `skills/cafleet/reference/coding-agents.md` § `claude` per Specification <!-- completed: 2026-10-28T13:49 -->
- [x] Replace the delivered-line sentences in `skills/cafleet/roles/member.md` § On spawn and add the **Delivery-dismissed prompts** paragraph to § Command execution <!-- completed: 2026-10-28T13:49 -->
- [x] Add the open-prompt check paragraph to `skills/cafleet/reference/prompt-routing.md` § Director-side dispatch <!-- completed: 2026-10-28T13:49 -->
- [x] Append the open-prompt check sentence to `skills/cafleet/roles/director.md` § Member Prompt <!-- completed: 2026-10-28T13:49 -->
- [x] Append the Director-pane sentence to the intro of `skills/cafleet/reference/supervision.md` § Director wake, leaving the numbered steps unchanged <!-- completed: 2026-10-28T13:49 -->

### Step 2: Operator documentation

- [x] Add the CAFleet-input paragraph to `docs/docs/spec/coding-agent-backends.md` § Claude <!-- completed: 2026-10-28T13:50 -->
- [x] Replace the dismissed-prompt bullet in `docs/docs/spec/multiplexer-backends.md` § Accepted trade-offs <!-- completed: 2026-10-28T13:50 -->
- [x] Confirm `coding-agents.md` and `coding-agent-backends.md` contain no link to each other and that no other page restates the new rule's attributes <!-- completed: 2026-10-28T13:50 -->
- [x] Run `mise //docs:build` and fix any broken link or anchor <!-- completed: 2026-10-28T13:50 -->
- [x] Replace the mirrored dismissed-prompt bullet in `SPEC.md` § Accepted trade-offs with the new operator bullet <!-- completed: 2026-10-28T13:52 -->

### Step 3: Root Director regression test

- [x] Add `a_worker_drain_with_both_directions_pending_never_names_the_root_directors_pane` to `cafleet/src/delivery.rs` tests per Specification <!-- completed: 2026-10-28T13:53 -->
- [x] Run `mise //cafleet:test a_worker_drain_with_both_directions_pending_never_names_the_root_directors_pane` and confirm it passes <!-- completed: 2026-10-28T13:53 -->

### Step 4: Wrapped hold regression test

- [x] Add `a_preview_during_a_visible_claude_permission_form_is_held_without_child_bytes` to `cafleet/src/runtime/runner/input.rs` tests per Specification <!-- completed: 2026-10-28T13:55 -->
- [x] Run `mise //cafleet:test a_preview_during_a_visible_claude_permission_form_is_held_without_child_bytes` and confirm it passes <!-- completed: 2026-10-28T13:55 -->

### Step 5: Validation

- [x] Run `mise //cafleet:test` <!-- completed: 2026-10-28T13:57 -->
- [x] Run `mise //cafleet:lint` <!-- completed: 2026-10-28T13:57 -->
- [x] Run `mise //cafleet:typecheck` <!-- completed: 2026-10-28T13:57 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-28 | Initial draft |
