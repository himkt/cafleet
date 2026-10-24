# Multiplexer backends

CAFleet hosts coding-agent sessions in tmux or herdr panes. Pane IDs are opaque strings passed verbatim. The adapters preserve pane ownership, layout, literal input and bounded subprocess behavior.

## Backend matrix {#backend-matrix}

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Access | CLI subprocess | CLI subprocess |
| Spawn working directory | Splitting pane inheritance | Explicit `--cwd` |
| Capture for readiness | `capture-pane -e` | `pane read --format ansi` |
| Input | Literal text, settle, Enter | Literal text, settle, Enter |
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

Every notification, prompt, ping and native command uses one process-shared OS file lock. Acquire the guard before placement revalidation and styled capture; hold it through focus normalization, literal text, settle, Enter and the notification stamp. Close the non-cloned owned handle on every path.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id` |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Nonblocking exclusive `std::fs::File::try_lock`; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database; unlinking a held file would let another process lock a different inode for the same pane |
| Contention | Hold notifications, skip ping, reject prompt/exec |

Duplicate placements for one pane share the same lock key. Process exit releases ownership. Read-only capture needs no input lock.

Member deletion locks its concrete placement before killing/deregistering. Fleet deletion acquires active placement keys in sorted order and revalidates the placement set in its transaction. Retry contention every 50 milliseconds against one operation-wide five-second monotonic deadline. A changed set releases all guards and restarts within the same deadline. On timeout release all acquired guards and leave registry/panes unchanged.

Errors are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

## Pane-state classification {#pane-state-classification}

Use the recorded coding-agent backend and a fresh styled capture after locking. tmux captures with `-e`; herdr reads with `--format ansi`. Parse SGR spans in the recognized composer before stripping ANSI for presentation. Recognize the terminal composer/footer frame independently of transcript filtering. Parse fenced output forward using its marker and closing width; exclude code from approval/work evidence. Historical quoted frames and an explicit unfinished quoted frame remain unknown.

| Readiness | Evidence | Action |
|---|---|---|
| `idle` | Affirmative empty composer or verified host-styled empty placeholder | Submit |
| `working` | Active turn, streaming, tool/shell execution or interrupt cue | Hold/reject |
| `awaiting_user` | Question, confirmation or approval UI | Hold/reject |
| `draft` | Unsent user text in the composer | Preserve and hold/reject |
| `unknown` | Missing, ambiguous, truncated or unrecognized evidence | Inspect and hold/reject |

Approval and work evidence outrank empty-composer cues. A dim/grey host suggestion or changing placeholder is empty only in its recognized composer decoration style. The same visible text in user-input style is a draft; grey text elsewhere establishes no readiness. Missing or ambiguous style evidence yields unknown. Claude member argv disables prompt suggestions, while the recognizer still handles operator-enabled suggestions.

## Prompt dispatch (`send_prompt`) {#prompt-dispatch}

Validate a nonblank literal line without CR/LF or C0/DEL control characters. After locking and idle revalidation, type the payload once, wait the backend settle interval and submit Enter. Native shell input starts with `! ` followed by the validated actual command. Composer controls begin the submitted input. Place the argument terminator `--` before the literal payload in both multiplexer adapters so leading hyphens remain text.

Deletion and explicit pane-input operations start their five-second budgets after invocation schema and asset guards. Resolve native command bodies before starting their input budget. Each submission has one five-second transport budget. Before literal input, reserve the one-second settle delay and one second for Enter; bound the literal command by the remaining unreserved time. Insufficient time starts no input. A confirmed launch failure or recognized missing pane reports no dispatch. A started text write with an ambiguous result, or any failure after successful literal input, is uncertain. A failure after Enter can mean the command ran. Inspect the pane before authorizing any retry.

The lock excludes CAFleet writers; the user's keyboard and host UI can still change after a capture. Successful dispatch establishes complete submission only.

## Push notifications {#push-notifications}

The Director receives through broker poll/wait reads; sends to the Director persist without injected text. A self-send is already settled. For an ordinary member, a synchronous send attempts at most one oldest pending unacknowledged preview ordered by `(created_at, message_id)`.

Read preview eligibility in short consistent snapshots before and after acquiring the shared input lock, then revalidate activity, fleet, concrete placement, message status and `notified_at` before writing. The invocation validates its connection before notification operations. One connection-scoped database budget owns capture/input eligibility; commit every read snapshot before multiplexer I/O. An idle empty composer accepts one literal line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] TEXT
```

Stored text remains exact. Truncate the preview body by `CAFLEET_MAX_TEXT_LEN`, fold CRLF/LF/CR to `⏎`, render tabs as `␉`, and escape other C0/DEL controls as visible hexadecimal escapes. The fixed prefix distinguishes message text from native composer controls.

Persist `preview_started_at` before the first possible keystroke while holding the input lock. A confirmed no-input failure clears that claim; an ambiguous write, process exit or failed stamp leaves it durable. Stamp `notified_at` only after complete submission and only while the message remains pending. Mandatory stamp finalization uses the later of the original deadline and one second after finalization begins. An unresolved claim produces an uncertain diagnostic in later invocations and suspends automatic input. ACK removes preview eligibility. After inspecting the pane, explicitly prompt or ping an idle member to poll its durable inbox; the member ACKs consumed messages normally.

### Held delivery {#held-delivery}

Working, approval, pending placement and lock contention are ordinary holds. A finite [message wait](cli-options.md#message-wait) or a later send retries eligible ordinary-member previews. Rotation selects concrete ordinary-member placements and attempts at most one member per interval. After an uncertain delivery, every subsequent wait preserves the unresolved durable claim and excludes the member from automatic input while that exact preview remains eligible. Successful snapshots can establish ACK, departure or notification by another invocation.

A first attempted draft or unknown composer yields an immediate diagnostic naming the member and preview ID. Capture and relay retained user drafts before a user decision or authorized replacement. Transport and stamp failures after persistence remain diagnostics alongside a successful send result. No notification outcome erases a committed delivery.

### Delivery outcomes

`notification_sent` refers to the newly committed message. Delivering an older preview leaves it false. The [send contract](cli-options.md#message-send) owns exact output and errors; [finite receiving](cli-options.md#message-wait) owns diagnostic revalidation and bounded retries.
