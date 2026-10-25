# Multiplexer backends

CAFleet hosts coding-agent sessions in tmux or herdr panes. Pane IDs are opaque strings passed verbatim. The adapters preserve pane ownership, layout, literal input and bounded subprocess behavior.

## Backend matrix {#backend-matrix}

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Access | CLI subprocess | CLI subprocess |
| Spawn working directory | Splitting pane inheritance | Explicit `--cwd` |
| Capture for `member capture` / `fleet capture` | `capture-pane -e` | `pane read --format ansi` |
| Input | Optional Escape lead, literal text, settle, Enter | Optional Escape lead, literal text, settle, Enter |
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

Every notification, prompt and native command uses one process-shared OS file lock. Acquire the guard before placement revalidation; hold it through the Escape lead, literal text, settle, Enter and the notification stamp of the last drained message. Close the non-cloned owned handle on every path.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id` |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Exclusive `std::fs::File::try_lock`, retried every 50 milliseconds while another writer holds it; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database; unlinking a held file would let another process lock a different inode for the same pane |
| Contention | A pane writer waits until its five-second budget minus the 3.5-second input reservation, at most 1.5 seconds. A lock still busy then fails that writer with its lock-busy error |

The 3.5-second input reservation covers a 0.5-second revalidation margin, the 0.5-second Escape settle, a 0.5-second minimum literal budget, the one-second submit settle and one second for Enter. A writer that acquires the lock within its wait therefore still submits; a lock released just before the wait ends still leads to a successful submission. No database statement runs while a writer waits for the lock.

| Writer | Lock still busy when the wait ends |
|---|---|
| `message send` / `message wait` delivery | `not_sent`: `member ID's pane input remained busy; preview MESSAGE_ID was not sent.` |
| `member prompt` | Exit 1: `member ID's pane input remained busy; prompt was not dispatched.` |
| `member exec` | Exit 1: `member ID's pane input remained busy; native command was not dispatched.` |

When a delivery's lock wait ends, one short read selects the oldest pending unclaimed message for the recipient. When none remains, the lock holder (usually another sender draining the same recipient) delivered or settled every message, and the attempt reports nothing. Otherwise the lock-busy diagnostic names that message.

Duplicate placements for one pane share the same lock key. Process exit releases ownership. Read-only capture needs no input lock.

Member deletion locks its concrete placement before killing/deregistering. Fleet deletion acquires active placement keys in sorted order and revalidates the placement set in its transaction. Retry contention every 50 milliseconds against one operation-wide five-second monotonic deadline. A changed set releases all guards and restarts within the same deadline. On timeout release all acquired guards and leave registry/panes unchanged.

Errors are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

## Termination during pane input {#termination-during-pane-input}

Interrupting a host turn can also end the turn's in-flight shell tool, including a running `cafleet` command. The CLI therefore blocks `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span; a signal received meanwhile takes its default action when the span ends. Spans nest and restore the outer signal mask.

| Command | Span |
|---|---|
| `message send`, each `message wait` attempt | From lock acquisition through the last `notified_at` stamp or claim release |
| `member prompt`, `member exec` | From lock acquisition through Enter |
| `member create` | From the pane spawn through the registry commit or its compensation |
| `member delete`, `fleet delete` | From lock acquisition through deregistration and layout cleanup |

Every span is bounded by its existing deadline, so a deferred signal waits at most one span. `message wait` outside an attempt keeps default signal behavior. Multiplexer client processes started inside a span inherit the blocked set and stay bounded by their timeouts. The spawned member session is started by the multiplexer server and does not inherit it.

`SIGKILL` cannot be deferred. A kill inside a span leaves:

| Killed command | State left | Discovery and recovery |
|---|---|---|
| `message send` / `message wait` between claim and stamp | Durable claim; the line may or may not have reached the pane | The claim appears in `unconfirmed_previews`; inspect the pane and prompt the member to poll |
| `member prompt` / `member exec` after typing began | Partial or complete input | `member capture`; re-dispatch only after inspecting the pane |
| `member create` | A pane without a registry row, or a row whose placement stays pending | `member list` and `fleet capture`; `member delete` the row, and close an unregistered pane with the multiplexer's own kill command |
| `member delete` / `fleet delete` | A closed pane whose row remains, or the reverse | `member list`; rerun the delete |

## Prompt dispatch (`send_prompt`) {#prompt-dispatch}

Validate a nonblank literal line without CR/LF or C0/DEL control characters before starting any process. No input path captures the pane or consults its state. Every submission selects a lead:

| Lead | Input sequence | Used by |
|---|---|---|
| Escape | Escape, 0.5-second settle, literal line, settle, Enter | `member prompt`, `member exec` and the first notification of a drain |
| Direct | Literal line, settle, Enter | Each further notification drained in the same lock hold |

Escape is meant to interrupt a working turn or close an open prompt so the line reaches the composer. A further drained line uses the direct lead because a second Escape would interrupt the turn the first delivered line just started; the host queues or appends it.

| Input | tmux argv | herdr argv |
|---|---|---|
| Escape | `tmux send-keys -t PANE Escape` | `herdr pane send-keys PANE escape` |
| Literal | `tmux send-keys -t PANE -l -- TEXT` | `herdr pane send-text PANE -- TEXT` |
| Enter | `tmux send-keys -t PANE Enter` | `herdr pane send-keys PANE enter` |

Native shell input starts with `! ` followed by the validated actual command. Composer controls begin the submitted input. The argument terminator `--` before the literal payload keeps leading hyphens as text.

Deletion and explicit pane-input operations start their five-second budgets after invocation schema and asset guards. Resolve native command bodies before starting their input budget. Each submission has one five-second transport budget:

1. Reserve the one-second settle delay and one second for Enter, plus the 0.5-second Escape settle for the Escape lead. When no time remains beyond the reservation, fail with `insufficient pane input budget for literal input, settle and Enter` and start no input.
2. For the Escape lead, send Escape bounded by one second or the unreserved time, whichever is shorter, then wait the Escape settle. Any Escape failure (launch, missing pane, nonzero exit or timeout) reports no dispatch, because no text was typed.
3. Recompute the literal budget as the remaining time minus the settle delay and Enter. When Escape consumed the slack, fail with the same insufficient-budget error and type no text. Otherwise type the literal line within that budget, wait the settle delay and submit Enter.

A confirmed launch failure or recognized missing pane reports no dispatch. A started text write with an ambiguous result, or any failure after successful literal input, is uncertain. A failure after Enter can mean the command ran. Inspect the pane before authorizing any retry.

The lock excludes CAFleet writers; the user's keyboard and host UI can change at any time. Successful dispatch establishes complete submission only.

## Push notifications {#push-notifications}

Active ordinary recipients with concrete panes receive automatic message lines. The fleet's root Director receives through native [message listen](cli-options.md#message-listen) or manual broker reads, with zero automatic pane input. Self-sends persist with no pane input.

Read preview eligibility in short consistent snapshots before and after acquiring the shared input lock, then revalidate ordinary-member kind, activity, fleet, concrete placement, message status and `notified_at` before writing. Exclude the Director before selecting a preview, locking a pane or resolving a multiplexer. The invocation validates its connection before notification operations. One connection-scoped database budget owns input eligibility; commit every read snapshot before multiplexer I/O. Each delivered message becomes one literal line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. The line folds the full message body to one line: CRLF/LF/CR become `⏎`, tabs become `␉`, and other C0/DEL controls become visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline. A longer one is replaced by `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.`, where `N` counts the stored text. The line is independent of `CAFLEET_MAX_TEXT_LEN`. Every line, including drained follow-up lines and pointer lines, carries both trailing clauses. The fixed prefix distinguishes message text from native composer controls.

For example:

```text
[cafleet msg 57 from 15 2026-10-24T09:00:00.000000+00:00] ready (doc)⏎see the reviewer notes [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

A synchronous send drains the recipient's pending messages oldest-first, ordered by `(created_at, message_id)`, within one lock hold and one five-second budget:

1. Take the eligibility snapshot. An inactive/ineligible recipient, a fleet mismatch or no pending message ends the attempt with nothing to deliver. A placement without a concrete pane holds delivery.
2. Wait for the [pane input lock](#pane-input-lock).
3. Under the lock, re-read the snapshot; a changed placement holds delivery. Resolve the multiplexer and check its availability.
4. Select the oldest pending unclaimed message; the drain ends when none remains.
5. When the remaining budget no longer covers the lead's reservation plus a 0.5-second minimum literal budget, stop. For the first message this fails as `not_sent` with `insufficient pane input budget for literal input, settle and Enter`; later messages stay pending with no diagnostic.
6. Claim the message. A departed recipient or one now serving as root Director ends the drain, and a placement change or an earlier candidate holds it. A message ACKed or stamped meanwhile is skipped.
7. Submit its line, with the Escape lead for the first line and the direct lead afterwards. Stamp `notified_at`, then continue at step 4. A message the recipient ACKed between claim and stamp still counts as delivered.

The lock is held from placement revalidation through the last Enter and the last `notified_at` stamp of the drain.

Claim a selected message with one conditional UPDATE inside a short IMMEDIATE transaction while holding the pane input lock. Revalidate ordinary-member eligibility and placement, preserve oldest-unclaimed ordering and commit before multiplexer input. A zero-row claim settles that obsolete attempt. After a confirmed no-input failure, retry transient claim-release contention within one absolute completion budget and retain `not_sent` plus the storage cause if release still fails. An ambiguous write, process exit or failed stamp leaves the claim durable. Stamp `notified_at` only after complete submission while the message remains pending. Mandatory finalization uses the later of the original deadline and one second after finalization begins. Unresolved claims suspend automatic input for their exact messages; later unclaimed messages remain eligible. The Director discovers ordinary-member claims through `unconfirmed_previews` in [member list](cli-options.md#member-list-output); after inspecting the pane, it prompts the member with `member prompt` to poll its durable inbox, and the member ACKs consumed messages normally. The Director consumes its own pending records through broker reads.

Delivery leaves `status_state` unchanged. A delivered message, inline or pointer, stays `input_required` and in `message poll` output until the recipient ACKs the ID shown in the line's `[cafleet msg ID …]` prefix.

### Held delivery {#held-delivery}

Pending placement and a placement change at claim time are holds. Lock contention waits for the lock within the attempt budget. A message remains pending while the lock stays busy, the pane is not yet placed or the budget runs out. The next send to that ordinary recipient, or a [message wait](cli-options.md#message-wait) rotation, retries it. Rotation selects concrete ordinary-member placements, excluding the root Director and the wait's own subject, and attempts at most one member per interval. After an uncertain delivery, subsequent waits reconstruct diagnostics from the same message snapshots and skip the claimed message while selecting later unclaimed messages for that member. Successful snapshots can establish ACK, departure or notification by another invocation.

Transport and stamp failures after persistence remain diagnostics alongside a successful send result. No notification outcome erases a committed delivery.

### Accepted trade-offs {#accepted-trade-offs}

What a single Escape does depends on the backend, and a backend may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it. Both trailing clauses are conditional, so they stay accurate either way.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the dismissed-prompt note tells that member the rejection did not come from the user and to re-issue the required tool call.
- Escape does not reliably clear typed composer text on every host. A draft in an ordinary member's pane can merge with the delivered line and be submitted with it.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- Interrupting a host turn can also end its in-flight shell tool; the CLI defers termination signals across its spans ([termination during pane input](#termination-during-pane-input)).

### Delivery outcomes

`notification_sent` is `true` exactly when this invocation delivered the newly committed message. Delivering only older messages, or another invocation delivering the new message, leaves it `false` without a diagnostic. The [send contract](cli-options.md#message-send) owns exact output and errors; [message wait](cli-options.md#message-wait) owns diagnostic revalidation and bounded retries.
