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
| Inline-preview keystroke | `send-keys` | `pane send-text` + `pane send-keys` |

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
monitor loop's wake is unconditional and periodic, and
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
monitor member, identical on both backends: `Esc`, then the literal payload
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

`cafleet member prompt`, the exec dispatch line, and the exec resume line all
reach the pane through the multiplexer interface's
`send_prompt(target_pane_id, text)` operation: one line of text, typed
`Esc`-first and submitted with `Enter`.

Both backends validate fail-fast: text empty after strip →
`send_prompt: text may not be empty`; the **original** text containing `\n` or
`\r` → `send_prompt: text may not contain newlines` (raised as the backend's
native error type, `TmuxError` / `HerdrError`). The per-backend payloads are
in the [backend matrix](#backend-matrix). The herdr realization mirrors
`send_poll_trigger`'s esc-then-run shape.

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
`send_inline_preview` helper; the contract — one Esc-safeguarded submit of the
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
delivery into a pane at rest stays immediate and held work always has a wake.

| Term | Meaning |
|---|---|
| Ordinary delivery | A delivery into a pane classified `finished` |
| Forced delivery | A delivery into any other pane, made because the hold age reached the timeout |
| Base payload | For a preview, the two lines above: the `[cafleet msg …]` header and the truncated text |
| Owed work | For a fleet: a [pending preview](data-model.md#messages), or an exec not yet closed, for any of its members |

Both kinds use the same keystroke shape: `Escape`, settle, payload, `Enter`.

The delivery step performs at most one keystroke into one member's pane:

1. **Exec in flight.** If an exec is dispatched or running in the pane, hold.
   No timeout applies while a command is running.
2. **Select one item**, in this priority: a finished exec not yet resumed,
   else the oldest queued exec, else the oldest pending preview. With no
   item, nothing is owed.
3. **Capture and classify** the last 40 lines of the pane — see
   [Pane-state classification](#pane-state-classification). A capture error
   holds.
4. **Resume by observation**, when the item is a finished exec — see
   [member exec](cli-options.md#member-exec).
5. **Decide** per the gate table below.
6. **Fire**: take the [pane claim](#pane-claim), stamp the item, and send the
   keystroke. A refused claim holds. A failed keystroke clears the stamp, so
   the item stays owed and the next attempt retries it.

| Pane state | Hold age below the timeout | Hold age at or above the timeout |
|---|---|---|
| `finished` | Ordinary delivery | Ordinary delivery |
| `awaiting_user` | Hold | Forced delivery; a preview adds the broker note |
| `working` | Hold | Forced delivery; a preview adds the resume clause |
| unclassified | Hold | Forced delivery; base payload only |

**Hold age** is the time since the latest of three moments: the item's own
timestamp (creation for a preview or a queued exec, the end of the command
for a resume), the end of the pane's latest exec, and the pane's last forced
delivery. The two pane-level moments restart the clock for everything queued
behind an event: a long command does not immediately force the keystrokes
that waited for it, and several overdue items do not force back to back into
a prompt the recipient has just re-raised.

**Timeout.** `CAFLEET_DELIVERY_HOLD_TIMEOUT` seconds, default `300`. `0`
disables forcing, so a hold lasts until the pane is at rest. The timeout
bounds every classification miss to a delay.

Four keystrokes reach a pane that is not `finished`:

| Keystroke | Rule | Why |
|---|---|---|
| Forced delivery | The hold timeout | The bound on a hold |
| `member ping` on an unclassified pane | [member ping](cli-options.md#member-ping) | A quiet pane the classifier cannot name is the stalled member a ping exists for |
| `member prompt` | Not gated | The Director's deliberate direct turn |
| Monitor wake | Not gated | It re-engages a monitor member stuck mid-turn. The monitor's work is an idempotent re-scan, and its spawn posture shows no user prompt |

### Pane claim {#pane-claim}

Immediately before a keystroke, a cafleet process claims the target pane by
stamping the pane's `keystroke_at`, in one statement that succeeds only when
the previous claim is at least 3 seconds old. A process whose claim is refused
does not type: another cafleet process is typing into that pane. The window
covers the `Esc` settle and submit delays, so two processes never interleave
their keystrokes, and the loser's capture cannot mistake a half-typed pane for
one at rest.

| Keystroke | On a refused claim |
|---|---|
| Broker delivery | Holds; the next attempt retries |
| `member ping` | Skips with reason `busy`; exit 0 |
| `member prompt` | Exit 1 — see [member prompt](cli-options.md#member-prompt) |
| Monitor wake | The wake stays due for the next tick |

### Preview payloads {#preview-payloads}

| Delivery | Payload |
|---|---|
| Ordinary, or forced on an unclassified pane | The base payload |
| Forced on `working` | The base payload, then `[cafleet] Resume your work if something was still running.` |
| Forced on `awaiting_user` | The base payload, then `[cafleet] This delivery's Escape dismissed a pending prompt in your pane; that rejection did not come from the user. Re-issue the tool call if you still need it.` |

Each extra line is conditional. An ordinary preview lands in a pane at rest,
where nothing was interrupted, so it carries no resume clause. The broker note
is emitted only when the broker observed a pending prompt immediately before
its own `Escape`, so its absence still means the user rejected the call.
Re-issuing the tool call raises the prompt again, so the user's decision is
preserved.

### Pane-state classification {#pane-state-classification}

The broker classifies a pane from the ANSI-stripped text of its last 40 lines.
Each coding agent contributes three cues — regular expressions matched line by
line — and the evaluation order is fixed:

1. Any `awaiting_user` cue matches in its region → `awaiting_user`.
2. Any `working` cue matches in its region → `working`.
3. Any `finished` cue matches in its region → `finished`.
4. Otherwise the pane is unclassified. A pane whose coding agent has no cue
   table is always unclassified.

A cue reads only the bottom of the pane, where the coding agent draws its live
prompt box, activity line, and composer. Blank lines are dropped first; a
region is the last N remaining lines.

| Cue | Region | Reason for the size |
|---|---|---|
| `awaiting_user` | Last 20 lines | A selection list with descriptions is tall |
| `working` | Last 10 lines | The activity line sits directly above the composer |
| `finished` | Last 6 lines | The composer and its status lines only |

The sizes are asymmetric on purpose. A false `awaiting_user` or `working`
only delays a delivery, bounded by the timeout. A false `finished` sends the
`Escape` that held delivery exists to withhold, so that cue reads the smallest
region and is evaluated last: an echoed user turn higher in the transcript
does not match, and a prompt box no `awaiting_user` cue recognises leaves no
composer in the last six lines, so the pane is unclassified and holds.

| Coding agent | `awaiting_user` cue | `working` cue | `finished` cue |
|---|---|---|---|
| `claude` | A numbered option under the selection cursor: <code>^[\s│]*❯\s+\d+\.\s</code> | `esc to interrupt` | The composer prompt line: <code>^[\s│]*[>❯](\s.*)?$</code> |
| `codex` | A numbered option under the cursor, <code>^\s*›\s+\d+\.\s</code>, or a `\[y/n\]` approval line | `esc to interrupt` | The composer prompt line: <code>^\s*[›▌](\s.*)?$</code> |
| `opencode` | The permission popup title: `Permission required` | <code>esc\s+(to\s+)?interrupt</code> | The prompt box line: <code>^\s*┃(\s.*)?$</code> |

`finished` means the composer is visible with no prompt box and no active-work
cue. Text already typed into the composer does not change the result; a
keystroke then appends to it.

### Keystroke error propagation {#keystroke-errors}

`send_inline_preview` and `send_prompt` propagate a failure as a `Result`
carrying the raw backend error instead of a best-effort boolean.
A missing backend binary fails with exactly `tmux binary not found on PATH` or
`herdr binary not found on PATH`; a subprocess failure after that precheck
carries the backend's raw error formatting — the failed command, its
payload argv, and a newline-delimited stderr detail — from whichever Escape,
payload, or Enter operation failed.

A failed delivery keystroke is not a command failure: the item's stamp is
cleared, the work stays owed, and the next attempt retries it. Every caller of
the delivery step — `message send`, `message broadcast`,
`POST /api/messages/send`, `member exec`, and the monitor loop — treats it the
same way.

The other trigger keystrokes, `send_poll_trigger` and `send_wake_trigger`,
keep their best-effort boolean contract: they never raise, and any failure
returns `false`.

### The `Esc` safeguard {#esc-safeguard}

Every keystroke path presses `Escape`, lets the pane settle ~0.1 s, then types
the payload and `Enter`.

| Keystroke path | Payload | Why |
|---|---|---|
| Inline preview (`message send` / `message broadcast`) | The [preview payload](#preview-payloads) + `Enter` | A prompt can open between the capture and the keystroke; the `Escape` rejects it instead of letting the trailing `Enter` confirm it |
| Exec dispatch and exec resume | The fixed dispatch line or resume line + `Enter` | The same capture-to-keystroke window as a preview |
| `cafleet member ping` | The literal poll command with the resume clause + `Enter` | The manual re-poke for a quiet pane |
| `cafleet member prompt` | The text + `Enter` | The dispatch never blindly confirms a pending permission prompt |
| Exit-command helper (`send_exit`) | `/exit` + `Enter` | Uses the same safeguard, with pane-gone tolerance covering the leading `Esc` when `ignore_missing` is enabled; creation rollback uses `kill_pane` |
| Monitor-loop wake trigger (`send_wake_trigger`) | The `[cafleet] tick:` wake + `Enter` | It targets the monitor member's pane, which can be parked on a permission prompt (see [Monitoring](../concepts/monitoring.md)) |

### Delivery outcomes

The persisted queue remains authoritative: a held or failed preview leaves
the row available for polling and ACK, and an ACKed row is never keystroked.
A preview is keystroked exactly once, whether the sending command, the monitor
loop, or both evaluate it — the item stamp and the pane claim together
guarantee it. The recipient pane comes from its placement, so
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
