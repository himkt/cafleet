# Multiplexer backends

cafleet hosts every coding-agent member inside a **terminal-multiplexer pane**.
The multiplexer is abstracted behind the `Multiplexer` Protocol, so the
spawn, keystroke-delivery, capture, and teardown paths are backend-neutral. Two backends ship today: **tmux** and
**herdr** ([herdr.dev](https://herdr.dev)). Both satisfy the same Protocol, so
every `member *` path behaves identically regardless of which one is active.

Pane ids are treated as **opaque strings** end to end — tmux ids look like `%7`,
herdr ids look like `w1:p1`; cafleet stores and passes them verbatim and never
parses them.

The pane is also cafleet's only push channel: message delivery stays pull-based
(recipients drain the persisted queue with `cafleet message poll`), and the
broker keystrokes an inline preview into the recipient's pane once that pane
is at rest — see [Push notifications](#push-notifications).

## Backend matrix {#backend-matrix}

Each backend invokes its CLI from PATH. Their behavior differs as follows:

| Behavior | tmux | herdr |
|---|---|---|
| Pane id shape | `%7` | `w1:p1` |
| Error class | `TmuxError` | `HerdrError` |
| Native agent-state capability | absent | `AgentStateAware`, tracking `working` / `blocked` / `done` / `idle` / `unknown` |
| Access mechanism | Shells out to `tmux` | Uses the herdr CLI as a subprocess |
| Pane-spawn cwd | Builtin inheritance from the splitting pane | Explicit `--cwd <dir>` on every `herdr pane split` |
| Delete-time layout reflow | Native auto-fit after a bare `kill-pane` | No native reflow — `kill_pane` rebalances best-effort, scoped to the killed pane's tab |
| `send_prompt` | `Esc` first, then payload `<stripped>` + `Enter` | `Esc` first, then `herdr pane run <pane> <stripped>` |
| `send_automatic_prompt`, ping and wake | Literal payload, submit delay, `Enter`; no Escape | `herdr pane run <pane> <payload>`; no Escape |
| Inline-preview keystroke | Literal `send-keys`, submit delay, `Enter`; no Escape | `pane send-text`, submit delay, `pane send-keys ... enter`; no Escape |

`TmuxError` and `HerdrError` are both subclasses of `MultiplexerError`. Each
behavior's full contract stays under its own heading below.

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

On success, ownership transfers immediately to the CLI's creation guard. A
fleet callback installs that guard and returns the id without another
fallible operation between those steps. Guard `finish`/`rollback` explicitly
disarms ownership; `Drop` is a last defense for unhandled ownership, so an
explicit cleanup attempt cannot cause a second kill. CLI registration and
transaction compensation follow the
[creation failure order](cli-options.md#creation-failure-compensation).
Creation compensation uses `kill_pane`, because `send_exit` submits a command
to the pane and does not guarantee shell or pane termination. Normal
`member delete` and notification keystrokes keep their existing behavior.

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

## Native agent-state (herdr only) {#native-agent-state}

herdr natively tracks each agent's lifecycle state
(`working`/`blocked`/`done`/`idle`/`unknown`), exposed through a separate
optional capability Protocol, `AgentStateAware`, that only the herdr backend
implements — the base `Multiplexer` Protocol stays clean and tmux implements
nothing new.

No DB column backs the native status, and nothing in cafleet consumes it: the
monitor loop's wake follows its schedule and shared automatic safety gate, and
[pane-state classification](#pane-state-classification) reads captured text
only, so it behaves the same on both backends. See
[Monitoring](../concepts/monitoring.md).

## The monitor wake and the fixed direct ping

The monitor loop is one fixed-cadence path on both backends. Each tick it
sends two kinds of keystroke: the [held deliveries](#held-delivery) owed to
any member pane, and — once per wake interval — the wake into the monitor
member's own pane. It never calls `send_poll_trigger`.

`send_wake_trigger` receives the fleet's wake roster plus a Director
descriptor and emits a **pure trigger** — the member list with
pending-delivery counts, the `Director:` segment, the pointer to the monitor
role protocol, and the resume clause; nothing else. Each rendered entry is
`<member-id> (<name>; coding_agent=<agent>; unacked=<pending-count>)`, joined
by `, `, ordered by `member_id` ascending, excluding the Director and the
monitor member itself; `<pending-count>` counts that member's
`input_required` unicast deliveries. The `Director:` segment is always
present, in the same field grammar as an entry. Every `<name>` is passed
through `sanitize_wake_field`; an entry (roster or Director) whose
`coding_agent` is not a supported backend name fails the wake closed with
`member <id> has invalid coding_agent '<agent>'` and no keystroke is sent.
tmux and herdr emit the payload byte-identically:

```text
[cafleet] tick: fleet <fleet-id> — health-check your <N> members: <entries>. Director: <id> (<name>; coding_agent=<agent>; unacked=<n>). Follow your monitor role protocol. Resume your work if something was still running.
```

With `N == 1` the noun is singular (`health-check your 1 member: …`); with
`N == 0` the clause becomes `no members to health-check.` and there is no
`<entries>` segment — the `Director:` segment stays.

Example:

```text
[cafleet] tick: fleet 3 — health-check your 2 members: 6 (drafter; coding_agent=claude; unacked=2), 7 (reviewer; coding_agent=codex; unacked=0). Director: 4 (Director; coding_agent=claude; unacked=1). Follow your monitor role protocol. Resume your work if something was still running.
```

The Director's `<name>` renders as stored — `fleet create` registers the root
Director as `Director` — with no case transformation.

`cafleet member ping` is a fixed manual primitive of the Director and the
monitor member, identical on both backends: the literal payload
`cafleet message poll <member-id> — then
resume your work if something was still running.`, then `Enter`. It cannot
carry arbitrary text, and it is gated on the target's pane state — see
[CLI options](cli-options.md#member-ping).

Anything a member needs from the Director travels as a plain
`cafleet message send` — the same persisted queue and held inline-preview
path every fleet message uses.

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
rebalance silently skips on unexpected layout shapes. Any `HerdrError` during
the rebalance is swallowed: a layout failure never fails `member delete` — the
pane is closed and the member deregistered regardless.

## Prompt dispatch (`send_prompt`) {#prompt-dispatch}

`cafleet member prompt` uses `send_prompt(target_pane_id, text)`: one line of
text, typed `Esc`-first and submitted with `Enter`. Exec dispatch and resume
use `send_automatic_prompt(target_pane_id, text)` after the shared automatic
delivery checks. Automatic submission starts with the payload, without Escape.

Both backends validate fail-fast: text empty after strip →
`send_prompt: text may not be empty`; the **original** text containing `\n` or
`\r` → `send_prompt: text may not contain newlines` (raised as the backend's
native error type, `TmuxError` / `HerdrError`). The per-backend payloads are
in the [backend matrix](#backend-matrix). The automatic API applies the same
validation and concrete backend errors. Its herdr realization runs the validated
payload with `pane run`; tmux sends it literally, waits the existing submit
delay, then sends `Enter`.

## Push notifications {#push-notifications}

CAFleet's delivery model is pull-based: recipients discover messages via
`cafleet message poll`. To cut latency, the broker keystrokes a 2-line inline
preview into the recipient's pane, so the recipient's coding agent consumes it
as a fresh user-turn input:

```text
[cafleet msg <message_id> from <sender_id> <ts>]
<text-truncated-to-CAFLEET_MAX_TEXT_LEN>
```

The keystroke is dispatched through the resolved backend's
`send_inline_preview` helper; the contract — one automatic submit of the
whole payload — is identical on both, and the per-backend realization is
in the [backend matrix](#backend-matrix). The recipient pane is resolved from
`member_placements` by `member_id` alone, so Member → Director notifications
work automatically. The recipient acks via `cafleet message ack <message_id>`
once it has consumed the message. Body truncation in the preview (`…` at
`CAFLEET_MAX_TEXT_LEN` codepoints) is documented in
[CLI options](cli-options.md#message-body-truncation).

### Held delivery {#held-delivery}

A **broker delivery** is a keystroke the broker owes a pane: a message
preview, an exec dispatch, or an exec resume. The broker sends it only when
the pane can take it, and holds it otherwise. The command that creates the
work makes one delivery attempt, and the monitor loop repeats the attempt on
every tick until the keystroke lands. Both run the same delivery step, so a
delivery into an empty idle composer stays immediate and held work is retried.

| Term | Meaning |
|---|---|
| Eligible delivery | A delivery with two fresh `finished` observations and a valid pane claim |
| Base payload | For a preview, the two lines above: the `[cafleet msg …]` header and the truncated text |
| Owed work | For a fleet: a [pending preview](data-model.md#messages), or an exec not yet closed, for any of its members |

Automatic transport submits the payload without Escape, retaining the existing
payload-to-submit delay where the backend uses separate operations.

The delivery step performs at most one keystroke into one member's pane:

1. **Exec in flight.** If an exec is dispatched or running in the pane, hold.
2. **Select one item**, in this priority: a finished exec not yet resumed,
   else the oldest queued exec, else the oldest pending preview. With no
   item, nothing is owed.
3. **Capture and classify** the last 40 lines of the pane — see
   [Pane-state classification](#pane-state-classification). A capture error
   holds.
4. **Resume by observation**, when the item is a finished exec — see
   [member exec](cli-options.md#member-exec).
5. **Require `finished`**, meaning affirmative empty-composer and idle evidence.
   Every other state holds indefinitely, regardless of item age.
6. **Claim and recheck**: acquire the [pane claim](#pane-claim), capture again,
   and require `finished` again. A refused claim or unsafe/failed final capture
   holds without stamping the item.
7. **Validate and submit**: in an immediate SQLite transaction, validate claim
   ownership and lifetime and conditionally stamp the still-owed item. Commit,
   then immediately submit. An invalid claim or refused item stamp produces
   zero input. A failed send clears the item stamp and leaves the item owed.

Ping and scheduled/requested monitor wakes use the same observation and claim
sequence, including in-flight exec exclusion. Ping skips immediately when held.
A held wake remains due; it preserves the wake request and last-success stamp
while heartbeats and the delivery pass continue. Repeated due ticks coalesce.
Explicit `member prompt`, pane bootstrap and shutdown retain direct-control
semantics.

### Pane claim {#pane-claim}

A cafleet process claims a pane by conditionally stamping `keystroke_at` when
the previous claim is at least 3 seconds old. Automatic callers retain the exact
stored timestamp and a monotonic acquisition time measured immediately before
the database update, so database waits count toward the lifetime.

After the final capture, an immediate SQLite transaction requires the stored
timestamp to match the receipt, monotonic elapsed time to be strictly below
the spacing window, and fresh UTC to remain within that window without moving
backwards. It conditionally stamps the owed item in the same transaction;
ping and wake validate without an item stamp. Database errors retain their
concrete diagnostics. A refused or expired receipt produces zero input and
leaves the claim row intact until normal expiry. A retry starts with fresh
observations and a new receipt.

The claim spaces CAFleet writers in time. It is not a lock over the entire
transport or operator keystrokes; expiry after validation can allow a second
writer to overlap a slow submission.

| Keystroke | On a refused claim |
|---|---|
| Broker delivery | Holds; the next attempt retries |
| `member ping` | Skips with reason `busy`; exit 0 |
| `member prompt` | Exit 1 — see [member prompt](cli-options.md#member-prompt) |
| Monitor wake | The wake stays due for the next tick |

### Preview payloads {#preview-payloads}

Every preview consists of the header and truncated text shown above. Message
newlines retain their existing folding and sanitization. Exec resume, ping and
wake retain their established payloads and resume clauses.

### Pane-state classification {#pane-state-classification}

The broker classifies a pane from the ANSI-stripped text of its last 40 lines.
Each supported agent has a parser for its complete current bottom interface,
grounded in sanitized real captures with agent/version, multiplexer, dimensions
and expected-state provenance. Blank lines and continuation rows are preserved.

| State, in precedence order | Required evidence | Automatic action |
|---|---|---|
| `awaiting_user` | Current permission or decision interface | Hold |
| `working` | Current active-turn indicator, including queued composer text | Hold |
| `draft` | Recognized current composer containing entered content or nonempty continuation rows | Hold |
| `finished` | Complete recognized empty composer, recognized idle context and no stronger cue | Final recheck |
| unclassified | Unsupported, incomplete, truncated, ambiguous or unrecognized layout | Hold |

Every composer row participates in the empty test, including a blank first row
with later content and distinguishable whitespace-only input. Claude's proven
empty composer has one input row; OpenCode's has exactly three bare composer
rows. Additional blank rows in a complete recognized composer are draft;
clipped or incomplete boundaries remain unclassified. Placeholder text
qualifies only within a recognized layout. Historical prompt lines establish
no current idleness. Missing boundaries, unexpected footers, attachments or
decorations whose emptiness is unproven hold. Native agent status alone proves
no composer emptiness. An unproven agent layout remains unclassified until its
required real-state evidence supports recognition; UI changes may reduce
notification liveness until the parser is updated.
Claude viewport overlays such as `Jump to bottom (click)` and horizontal
numbered new-message separators keep the layout unclassified after permission
and active-turn cues have been evaluated, even when the visible composer and
footer otherwise match an empty idle layout. Recognize the separator by its
horizontal rules surrounding the count and label, including the form without
a click label.

### Keystroke error propagation {#keystroke-errors}

`send_inline_preview`, `send_automatic_prompt` and `send_prompt` propagate a failure as a `Result`
carrying the raw backend error instead of a best-effort boolean.
A missing backend binary fails with exactly `tmux binary not found on PATH` or
`herdr binary not found on PATH`; a subprocess failure after that precheck
carries the backend's raw error formatting — the failed command, its
payload argv, and a newline-delimited stderr detail. Automatic paths report
payload or submission failures; direct control also reports Escape failures.

A failed delivery keystroke is not a command failure: the item's stamp is
cleared, the work stays owed, and the next attempt retries it. Every caller of
the delivery step — `message send`, `message broadcast`,
`POST /api/messages/send`, `member exec`, and the monitor loop — treats it the
same way.

The other trigger keystrokes, `send_poll_trigger` and `send_wake_trigger`,
keep their best-effort boolean contract: they never raise, and any failure
returns `false`.

### Automatic input and direct control {#esc-safeguard}

Automatic previews, exec dispatch/resume, ping and monitor wakes submit without
Escape after the shared safety checks. Explicit `member prompt` and `send_exit`
retain their leading Escape safeguard and settle delay; pane bootstrap remains
a lifecycle action.

Both safe captures and receipt validation establish eligibility only for the
observed terminal representation. An operator or application can change state
after the final capture, including during a payload-to-submit delay. tmux and
herdr offer no atomic check-and-submit operation. Identically rendered typed
text and placeholder text can be indistinguishable in text-only captures.

### Delivery outcomes

The persisted queue remains authoritative: a held or failed preview leaves
the row available for polling and ACK, and an ACKed row is never keystroked.
Conditional stamps arbitrate competing delivery attempts. Partial insertion or
an Enter timeout can leave submission uncertain; diagnostics and existing retry
behavior are preserved. A partially inserted payload normally becomes a draft
and holds later automatic input. Every retry requires new safe observations and
a valid new claim; terminal submission is best effort. The recipient pane comes from its placement, so
member-to-Director messages use the same path. Backend resolution follows
[Backend selection](#backend-selection); keystrokes target the opaque pane id
on the same host with a reachable server. Direct targeting uses tmux
`send-keys -t <pane>` or herdr `pane send-*`. A caller that cannot resolve a
multiplexer skips the delivery attempt and leaves the row owed.

| Delivery outcome | Unicast success field | Afterwards |
|---|---|---|
| Self-send or recipient placement has no pane id | `notification_sent: false`; no attempt | Nothing is owed |
| The sending command keystrokes the preview | `notification_sent: true` | Nothing is owed |
| The preview is held, or its keystroke fails | `notification_sent: false` | The preview stays owed; the monitor loop delivers it |

Broadcast wrapper fields `recipients` and `delivered` count intended recipients
and the previews the broadcast command itself keystroked. Neither count is
persisted. CLI wrappers and the loop-start failure output are owned by
[Output shapes](cli-options.md#output-shapes) and
[message send](cli-options.md#message-send-delivery).
