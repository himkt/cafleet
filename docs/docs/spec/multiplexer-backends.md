# Multiplexer backends

CAFleet hosts coding-agent sessions in tmux or herdr panes. Pane IDs are opaque strings passed verbatim. The adapters preserve pane ownership, layout, literal input and bounded subprocess behavior.

## Backend matrix {#backend-matrix}

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Access | CLI subprocess | CLI subprocess |
| Spawn working directory | Splitting pane inheritance | Explicit `--cwd` |
| Capture for `member capture` / `fleet capture` | `capture-pane -e` | `pane read --format ansi` |
| Unwrapped input | Optional Escape lead, literal text, settle, Enter | Optional Escape lead, literal text, settle, Enter |
| Wrapped input | Pinned PTY runner control socket | Pinned PTY runner control socket |
| Delete layout | Native auto-fit | Best-effort rebalance scoped to the closed pane’s tab |

## Backend selection {#backend-selection}

Every call site resolves its backend through one shared resolver rather than
hardcoding one. Resolution
precedence:

1. **Explicit override.** If `CAFLEET_MULTIPLEXER` is set, it must name a
   supported backend (`tmux` or `herdr`); an unknown value fails loudly.
2. **Auto-detect from the environment.** `HERDR_ENV` truthy signals a herdr
   session; `TMUX` set signals a tmux session.
3. **Ambiguity is a hard error.** Both `HERDR_ENV` and `TMUX` set → error
   (set `CAFLEET_MULTIPLEXER` to disambiguate). Neither set → error (run cafleet
   inside a tmux or herdr session, or set `CAFLEET_MULTIPLEXER`). Exactly one
   present → that backend.

The outcomes of that order:

| `CAFLEET_MULTIPLEXER` | `HERDR_ENV` | `TMUX` | Result |
|---|---|---|---|
| `tmux` or `herdr` | any | any | That backend |
| An unknown value | any | any | Fails loudly |
| unset | truthy | set | Error — set `CAFLEET_MULTIPLEXER` to disambiguate |
| unset | truthy | unset | herdr |
| unset | not truthy | set | tmux |
| unset | not truthy | unset | Error — run cafleet inside a tmux or herdr session, or set `CAFLEET_MULTIPLEXER` |

Auto-detect (an unset `CAFLEET_MULTIPLEXER`) is the default. `cafleet doctor`
reports the resolved backend and its
identifiers (see [CLI options](cli-options.md#cafleet-doctor)).


## Error taxonomy

Backend failures share a base `MultiplexerError`, with the per-backend
subclasses in the [backend matrix](#backend-matrix). CLI boundaries catch
`MultiplexerError`, so both backends' failures are handled uniformly while each
backend keeps its own message text.


## Pane ownership during creation {#pane-creation-ownership}

`split_window` owns a newly created pane until it returns successfully. As
soon as herdr extracts the pane id from the split response, it arms a pane
guard. If the subsequent `pane run` fails, that guard calls
`kill_pane(id, true)` before returning the original run error. A failed close
retains both the pane id and the run error and adds the cleanup failure.
Layout equalization remains best-effort and does not fail an otherwise
successful spawn.

Creation errors carry internal cleanup metadata:

| Metadata | Meaning and caller action |
|---|---|
| `PaneCleanup::Attempted { pane_id, error: None }` | The backend already closed the known pane; the CLI performs the remaining DB compensation without another kill. |
| `PaneCleanup::Attempted { pane_id, error: Some(_) }` | The backend tried to close the known pane and failed; retain that diagnostic, perform remaining DB compensation, and do not retry the pane kill. |
| `PaneCleanup::Unknown` | A failed or malformed split response left no confirmed pane id; report that pane compensation is unconfirmed, and do not guess an id or close another pane. |

On success, ownership transfers immediately to the CLI's creation guard. Guard `finish`/`rollback` explicitly
disarms ownership; `Drop` is a last defense for unhandled ownership, so an
explicit cleanup attempt cannot cause a second kill. CLI registration and
transaction compensation follow the
[creation failure order](cli-options.md#creation-failure-compensation).
Creation compensation closes only the pane owned by that operation.


## Subprocess output and deadlines

The shared subprocess runner captures stdout and stderr without truncation.
With a timeout, it drains both pipes concurrently from spawn using nonblocking
I/O. Each loop checks the monotonic deadline and the direct child's exit status,
then reads at most 64 KiB per stream; polling waits at most 20 ms or the remaining
deadline, whichever is shorter. Interrupted operations retry against the same
deadline. Completion requires both the child's exit and EOF on both streams.
Success returns stdout as lossy UTF-8; a nonzero exit reports stderr as lossy
UTF-8. Calls without a timeout retain the unbounded output-collection path.

At the deadline, the runner kills and reaps the direct child, closes both read
pipes, and reports a timeout. This also applies when the direct child has exited
but a descendant still holds a pipe open. Descendant termination is outside this
contract. FD setup, read, poll, and child-status failures also release the pipes
and kill/reap the child; cleanup failures accompany the primary error instead of
replacing it. The deadline bounds observation and the start of cleanup; an
unresponsive operating system can delay cleanup itself.


## Pane spawn working directory {#pane-spawn-cwd}

A member pane spawned by `cafleet member create` starts in the invoking
process's working directory (the Director's pane cwd); each backend realizes
this differently, per the [backend matrix](#backend-matrix).

On herdr, `<dir>` is the invoking process's current working directory. herdr's own
inheritance is not relied upon because herdr spawns `/bin/sh` instead of the
passwd login shell when `SHELL` is unset
([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)).
An unresolvable cwd fails the spawn loudly with `HerdrError`; there is no
fallback directory.


## Delete-time pane layout {#delete-time-pane-layout}

Closing a member pane leaves the two backends asymmetric on layout reflow, per
the [backend matrix](#backend-matrix).

herdr's `kill_pane` reads the target pane's tab (`herdr pane get`) before the
close, runs `herdr pane close`, then rebalances. The scoping comes from the
layout read itself: the killed pane is gone, so the rebalance picks a pane
still open in that tab (`herdr pane list`) and anchors the geometry read on it
(`herdr pane layout --pane <surviving>`), which returns that tab's layout
regardless of which tab or pane holds focus. When no pane remains in the tab,
there is nothing to rebalance and the step is skipped. With ≥ 2 members
remaining, the member column is re-equalized to equal heights (the same
invariant the create path enforces); after the last member is deleted, the
Director pane is explicitly restored to full tab width when the layout read
shows a residual right split; a single remaining member needs no resize. The
rebalance uses the original operation deadline after registry finalization.
A failed or expired restoration emits a stderr diagnostic; `member delete`
retains its successful close and deregistration result.


## Pane input lock {#pane-input-lock}

Every operation that writes pane input uses one process-shared OS file lock. Acquire the guard before placement revalidation. Wrapped prompt/exec/preview submissions hold it through short database validation, bounded runner submission and durable receipt/stamp settlement; explicitly unwrapped submissions hold it through the Escape lead, literal text, settle, Enter and the notification stamp of the last drained message. Explicit key requests hold it through their outer-pane dispatch and bounded cleanup. Close the non-cloned owned handle on every path.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id` |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Exclusive `std::fs::File::try_lock`, retried every 50 milliseconds while another writer holds it; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database; unlinking a held file would let another process lock a different inode for the same pane |
| Contention | Prompt/exec/preview submission reserves 3.5 seconds, leaving at most 1.5 seconds to wait. Explicit keys use their [dispatch reservation](#explicit-key-dispatch); a lock still busy at its wait bound fails that writer |

The 3.5-second input reservation covers a 0.5-second revalidation margin, the 0.5-second Escape settle, a 0.5-second minimum literal budget, the one-second submit settle and one second for Enter. A writer that acquires the lock within its wait therefore still submits; a lock released just before the wait ends still leads to a successful submission. No database statement runs while a writer waits for the lock.

| Writer | Lock still busy when the wait ends |
|---|---|
| `message send` / `message wait` delivery | `message MESSAGE_ID was not notified to member MEMBER_ID: pane input remained busy; the next send to member MEMBER_ID or the next message wait retries it.` |
| `member prompt` | Exit 1: `member ID's pane input remained busy; prompt was not dispatched.` |
| `member exec` | Exit 1: `member ID's pane input remained busy; native command was not dispatched.` |
| `member press` | Exit 1 with the [primary and zero-dispatch diagnostics](cli-options.md#member-press) |

When a delivery's lock wait ends, one short read selects the oldest pending message this invocation has not attempted for the recipient. When none remains, the attempt reports nothing. Otherwise the lock-busy diagnostic names that message.

Duplicate placements for one pane share the same lock key. Process exit releases ownership. Read-only capture needs no input lock.

Member deletion locks its concrete placement before killing/deregistering. Fleet deletion acquires active placement keys in sorted order and revalidates the placement set in its transaction. Retry contention every 50 milliseconds against one operation-wide five-second monotonic deadline. A changed set releases all guards and restarts within the same deadline. On timeout release all acquired guards and leave registry/panes unchanged.

Errors are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

## Termination during pane input {#termination-during-pane-input}

Interrupting a host turn can also end the turn's in-flight shell tool, including a running `cafleet` command. The CLI therefore blocks `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span; a signal received meanwhile takes its default action when the span ends. Spans nest and restore the outer signal mask.

| Command | Span |
|---|---|
| `message send`, each `message wait` attempt | From lock acquisition through the last `notified_at` stamp |
| `member prompt`, `member exec` | From lock acquisition through Enter |
| `member press` | Begin before acquiring the lock; retain it through the writing process and bounded cleanup, releasing the lock before restoring signals |
| `member create` | From the pane spawn through the registry commit or its compensation |
| `member delete`, `fleet delete` | From lock acquisition through deregistration and layout cleanup |

Every span is bounded by its existing deadline, so a deferred signal waits at most one span. `message wait` outside an attempt keeps default signal behavior. Multiplexer client processes started inside a span inherit the blocked set and stay bounded by their timeouts. The spawned member session is started by the multiplexer server and does not inherit it.

`SIGKILL` cannot be deferred. A kill inside a span leaves:

| Killed command | State left | Discovery and recovery |
|---|---|---|
| `message send` / `message wait` between first keystroke and stamp | `notified_at` stays NULL; the line may or may not have reached the pane | The next send or `message wait` retries it; a duplicate line is possible |
| `member prompt` / `member exec` after typing began | Partial or complete input | `member capture`; re-dispatch only after inspecting the pane |
| `member press` after its writing process starts | Any prefix, including empty or complete, may have been handled | Inspect the pane before an explicitly authorized new action |
| `member create` | A pane without a registry row, or a row whose placement stays pending | `member list` and `fleet capture`; `member delete` the row, and close an unregistered pane with the multiplexer's own kill command |
| `member delete` / `fleet delete` | A closed pane whose row remains, or the reverse | `member list`; rerun the delete |

## Explicit key dispatch {#explicit-key-dispatch}

`member press` maps the complete validated ordered vector from the [CLI contract](cli-options.md#member-press) to one writing invocation: `tmux send-keys -t PANE KEY...` or `herdr pane send-keys PANE KEY...`. The typed registry owns the common grammar and mappings below. Single printable characters preserve case; named keys and modifiers match case-insensitively, and control letters normalize to lowercase.

| Common key | tmux argument | herdr argument |
|---|---|---|
| Single printable ASCII character, U+0021–U+007E | `0xXX` for its ASCII codepoint | Same character, with `+` as `plus`, `-` as `minus`, and `;` as `semicolon` |
| `Space` | `Space` | `space` |
| `Escape` | `Escape` | `esc` |
| `Enter` | `Enter` | `enter` |
| `Tab` | `Tab` | `tab` |
| `Backspace` | `BSpace` | `backspace` |
| `Up`, `Down`, `Left`, `Right` | Canonical capitalized name | Lowercase name |
| `F1` through `F12` | `F1` through `F12` | `f1` through `f12` |
| `Ctrl+a` through `Ctrl+z` | `C-a` through `C-z` | `ctrl+a` through `ctrl+z` |
| `Shift+Tab` | `BTab` | `shift+tab` |

Accept only these productions, with the CLI's count/UTF-8 bounds and complete-vector validation. Tmux hexadecimal character arguments keep punctuation out of its command parser. The dedicated key adapter sends only requested keys, using shell-free argv and no prompt phase or settling sleep.

Acquire the same canonical pane-input lock used by deletion and validate its retained descriptor/path binding. Compare the configured/environment adapter with the recorded backend. Within that selected live context, require the current session/workspace to match the placement, then verify its complete target tuple: tmux reads `#{pane_id}|#{window_id}|#{session_name}` for the target and splits only the first two separators; herdr reads its pane-list JSON and requires exactly one matching workspace/tab/pane. Re-read the active member after metadata, compare its fixed fleet/kind/full placement and validate the held lock immediately before writing. A missing, ambiguous, malformed or changed target fails before the writing invocation.

The placement records no historical socket/server-generation identity. Context validation establishes its complete tuple within the selected current server; it provides no cross-server fallback or proof of historical server/build equivalence. Keep that same adapter/environment across every probe and the write.

Capture one inherited-or-new five-second absolute deadline before invocation database validation. The full availability/current-context/target-metadata/write chain uses absolute `run_until` execution with that supplied bound; relative or untimed helper execution supplies no press path. Retain one second for metadata/dispatch/cleanup when computing the lock-wait deadline, rather than the prompt reservation. Every later check uses the remaining original budget, and insufficient launch/cleanup reserve starts no new writing process.

Within the original deadline, reserve 250 milliseconds for owned process cleanup and end collection/launch at that earlier bound. On an actual error, close read pipes, kill only the still-owned unreaped direct child and poll `try_wait` with sleeps at most 2 milliseconds while the original cleanup deadline has time remaining, checking that deadline after every interruption. Explicit cleanup and unwind fallback use the same original deadline and perform no blocking wait. An explicit cleanup attempt prevents a second Drop retry cycle; unconfirmed termination/reap remains an explicit diagnostic: `cleanup unconfirmed for child PID: original command deadline expired before reap`. Preserve actual kill/status errors and the primary process error; unwind emits cleanup diagnostics to stderr. Existing relative/untimed execution retains its separate behavior. Synchronous calls have no hard scheduler/syscall return-time guarantee.

Both writing adapters acknowledge only successful process completion with empty/whitespace stdout. Unexpected non-whitespace stdout on success is unreadable acknowledgment with uncertain effects; herdr metadata retains its command-specific JSON parsing. Binary-not-found/not-started means zero writing invocations; every started failure, including pane-gone stderr, timeout or unreadable output, retains possible prefix delivery and receives no automatic retry. Backend keyboard modes or key interception can affect actual PTY bytes even after acknowledgment. Use the [CLI result/error contract](cli-options.md#member-press) and observe actual native effects separately.

## Prompt dispatch (`send_prompt`) {#prompt-dispatch}

The sequences below specify the explicitly unwrapped multiplexer path. Runner-managed members use a pinned, healthy control socket and the tested composer profile under the [runtime input contract](member-runtime.md#input-arbitration-and-the-original-native-answer); they send literal text and the native submit key without Escape, hold previews for an observed/candidate interaction and reject unsafe explicit input before writing. Unknown coverage and uncertain reachability preserve manual terminal use while holding generated input.

Validate a nonblank literal line without CR/LF or C0/DEL control characters before starting any process. No input path captures the pane or consults its state. Every submission selects a lead:

| Lead | Input sequence | Used by |
|---|---|---|
| Escape | Escape, 0.5-second settle, literal line, settle, Enter | `member prompt`, `member exec` and the first notification of a drain |
| Direct | Literal line, settle, Enter | Each further notification drained in the same lock hold |

Escape is meant to interrupt a working turn or close an open prompt so the line reaches the composer. A further drained line uses the direct lead because a second Escape would interrupt the turn the first delivered line just started; the host queues or appends it.

| Input | tmux argv | herdr argv |
|---|---|---|
| Escape | `tmux send-keys -t PANE Escape` | `herdr pane send-keys PANE escape` |
| Literal | `tmux send-keys -t PANE -l -- TEXT` | `herdr pane send-text PANE TEXT` |
| Enter | `tmux send-keys -t PANE Enter` | `herdr pane send-keys PANE enter` |

Native shell input starts with `! ` followed by the validated actual command. Composer controls begin the submitted input. Tmux uses the `--` option terminator before its literal payload to preserve leading hyphens. Herdr receives the complete payload as one TEXT argument immediately after the pane ID.

Opencode `mini` receives pushed notifications and direct prompts as submitted user turns. Native `! ` input executes a shell command. Poll returns the full persisted message, and ACK consumes its ID.

Deletion and prompt/exec operations start their five-second budgets after invocation schema and asset guards. Resolve native command bodies before starting their input budget. Each submission has one five-second transport budget:

1. Reserve the one-second settle delay and one second for Enter, plus the 0.5-second Escape settle for the Escape lead. When no time remains beyond the reservation, fail with `insufficient pane input budget for literal input, settle and Enter` and start no input.
2. For the Escape lead, send Escape bounded by one second or the unreserved time, whichever is shorter, then wait the Escape settle. Any Escape failure (launch, missing pane, nonzero exit or timeout) reports no dispatch, because no text was typed.
3. Recompute the literal budget as the remaining time minus the settle delay and Enter. When Escape consumed the slack, fail with the same insufficient-budget error and type no text. Otherwise type the literal line within that budget, wait the settle delay and submit Enter.

A confirmed launch failure or recognized missing pane reports no dispatch. A started text write with an ambiguous result, or any failure after successful literal input, is uncertain. A failure after Enter can mean the command ran. Inspect the pane before authorizing any retry.

The lock excludes CAFleet writers; the user's keyboard and host UI can change at any time. Successful dispatch establishes complete submission only.

## Push notifications {#push-notifications}

Active ordinary recipients with concrete panes receive previews through their recorded transport. The root Director receives through native [message listen](cli-options.md#message-listen) or manual broker reads, with zero automatic pane input. Self-sends persist without pane input.

Wrapped previews use this exact line through eligible runner input:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Explicitly unwrapped previews retain their full legacy line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. Both forms fold CRLF/LF/CR to `⏎`, tabs to `␉`, and other C0/DEL controls to visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline; a longer body becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.`, where N counts stored text. This limit is independent of `CAFLEET_MAX_TEXT_LEN`. Every wrapped line ends with the resume clause; every unwrapped line carries both clauses.

One synchronous drain owns the shared pane lock and five-second attempt budget. Commit short read snapshots before socket or multiplexer I/O.

1. Read recipient/message eligibility, excluding the root Director before preview selection or pane locking. An inactive/ineligible recipient, fleet mismatch or no pending row ends the attempt; pending placement holds it.
2. Acquire the pane input lock, then revalidate activity, member kind, fleet, concrete placement, message status and notification stamp. Changed placement holds delivery.
3. Resolve the recorded transport. A wrapped run requires a fresh pinned identity/epoch, socket health and receipt check; an unwrapped placement resolves the multiplexer and checks availability.
4. Select the oldest unattempted pending message by `(created_at,message_id)` and revalidate the recipient/placement. Each invocation attempts a selected message at most once.
5. For wrapped delivery, query durable key `preview:<run_id>:<message_id>` before reevaluating input. An accepted receipt permits settlement without typing; in-progress or uncertain holds for inspection, and a settled key returns its existing disposition without replay. A new eligible attempt commits its durable in-progress row before child bytes, then records accepted or uncertain submission.
6. Settle wrapped accepted delivery in one transaction that stamps `notified_at` or verifies ACK and marks the receipt settled/submitted; retire its in-memory cache only after verifying settlement. Failure after possible input retains durable uncertainty and the unstamped message for receipt reconciliation.
7. For unwrapped delivery, reserve the lead/settle/Enter time plus the minimum literal budget before input. Use Escape for the first line and direct input for later drained lines, then stamp `notified_at` after full submission; an intervening ACK counts as notified.
8. Continue at step 4 while the budget and input state permit. Wrapped input rechecks current output and composer state for every attempt; a busy/unsafe state holds the next message.

The lock remains held through final submission/settlement. Wrapped delivery follows the [durable input contract](member-runtime.md#input-arbitration-and-the-original-native-answer); only absent receipts or certified ready/zero-byte state permit a new eligible attempt. Explicitly unwrapped delivery remains at-least-once: a failed stamp leaves the message pending, so later send/wait may repeat partial or complete input. Poll and ACK identify consumption independently of either transport.

The [notification diagnostic](cli-options.md#message-send-delivery) applies to failed input attempts and database contention, retaining actual causes and the committed message. Mandatory finalization uses the later of the original deadline and one second after finalization begins, while the transport remains within its original deadline. Other fatal storage errors retain their cause; a send preserves its committed result. A held state remains pending for external retry and reports no successful notification.

Delivery leaves `status_state` unchanged. An inline or pointer preview remains `input_required` until its recipient consumes and ACKs the message ID.

### Held delivery {#held-delivery}

Wrapped delivery additionally remains held for a candidate/open interaction, draft/working/unknown composer, untested profile, degraded/recovering/lost observer or uncertain receipt/reachability. Only a verified empty composer allows a new submission. Held delivery leaves `notified_at` unset, creates no new permission event and grants no native answer. After inspecting the attributed native response, its actual result and continuation, consume/ACK available inbox records and explicitly invoke finite `message wait` to retry previews; the runner and listener perform no autonomous retry.

Pending placement and a placement change under the lock are holds. Lock contention waits for the lock within the attempt budget. A message remains pending while the lock stays busy, the pane is not yet placed, the budget runs out or an attempt fails. The next send to that ordinary recipient, or a [message wait](cli-options.md#message-wait) rotation, retries it. Rotation selects concrete ordinary-member placements, excluding the root Director and the wait's own subject, and attempts at most one member per interval. One invocation attempts each pending message at most once; a later invocation retries a failed one. Successful snapshots can establish ACK, departure or notification by another invocation.

Transport and stamp failures after persistence remain diagnostics alongside a successful send result. No notification outcome erases a committed delivery.

### Accepted trade-offs {#accepted-trade-offs}

The following trade-offs apply to explicitly unwrapped input. What a single Escape does depends on the backend, and a backend may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it. Both trailing clauses are conditional, so they stay accurate either way.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the dismissed-prompt note tells that member the rejection did not come from the user and to re-issue the required tool call.
- Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line.
- A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and ACKs by ID.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- Interrupting a host turn can also end its in-flight shell tool; the CLI defers termination signals across its spans ([termination during pane input](#termination-during-pane-input)).
- Opencode mini captures can expose only the status bar. Inspect retained session records for submitted input and native shell output when capture omits them; use broker poll and ACK to establish message consumption.

### Delivery outcomes

`notification_sent` is `true` exactly when this invocation delivered the newly committed message. Delivering only older messages, or another invocation delivering the new message, leaves it `false` without a diagnostic. The [send contract](cli-options.md#message-send) owns exact output and errors; [message wait](cli-options.md#message-wait) owns diagnostic revalidation and bounded retries.
