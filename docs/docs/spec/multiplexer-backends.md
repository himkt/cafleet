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
broker keystrokes an inline preview into the recipient's pane after
persisting a message — see [Push notifications](#push-notifications).

## Backend matrix {#backend-matrix}

Where the two backends differ, behavior by behavior:

| Behavior | tmux | herdr |
|---|---|---|
| Pane id shape | `%7` | `w1:p1` |
| Error class | `TmuxError` | `HerdrError` |
| Native agent-state capability | absent | `AgentStateAware`, tracking `working` / `blocked` / `done` / `idle` / `unknown` |
| Access mechanism | Shells out to `tmux` | Uses the herdr CLI as a subprocess |
| Pane-spawn cwd | Builtin inheritance from the splitting pane | Explicit `--cwd <dir>` on every `herdr pane split` |
| Delete-time layout reflow | Native auto-fit after a bare `kill-pane` | No native reflow — `kill_pane` rebalances best-effort, scoped to the killed pane's tab |
| `send_prompt` shell form | `Esc` first, then payload `! <stripped>` + `Enter` | `Esc` first, then `herdr pane run <pane> "! <stripped>"` |
| `send_prompt` plain form | `Esc` first, then payload `<stripped>` + `Enter` | `Esc` first, then `herdr pane run <pane> <stripped>` |
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

The four override-set combinations collapse into the first two rows because an
explicit override wins outright, whatever the environment holds.

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

## Shared spawn deadlines {#spawn-deadlines}

Planned bounded creation uses one monotonic 30-second deadline for the entire
spawn callback. Herdr list, split, layout/resize, and run, and tmux split and
layout, receive only the remaining duration. Preparation that does not need
allocated ids happens before the transaction; do not restart the deadline at
each subprocess. Fractional remaining time must not be rounded up into a new
whole-second budget. Timeout handling uses the existing runner's pipe draining,
child kill, and reap behavior.

Ordinary failures of best-effort layout/resize remain nonfatal, but a timeout
or exhausted deadline fails creation. Check the deadline again before pane
ownership is transferred. A timed command reports its actual remaining seconds,
including fractional seconds without unnecessary trailing zeroes. Expiry before
a command starts reports `<backend> spawn deadline exceeded: <argv>`; expiry
before ownership transfer reports `<backend> spawn deadline exceeded`.

A known-pane compensation operation gets a separate five-second budget. Keep
backend ownership until successful spawn return, and retain the
[creation failure order](cli-options.md#creation-failure-compensation): a
Herdr run timeout with a known id attempts backend close before DB rollback;
a split timeout before an id is confirmed reports `PaneCleanup::Unknown` and
performs no guessed close. After ownership transfer, CLI compensation follows
DB failure handling. A failed close keeps the primary error and does not
prevent remaining DB compensation or cause a duplicate pane kill.

Bounded compensation spends that budget directly on `herdr pane close <id>` or
`tmux kill-pane -t <id>`, without a preliminary pane lookup or a subsequent
layout repair. Existing missing-pane tolerance still applies. Ordinary delete
operations retain their current lookup, close, and rebalance behavior.

The 30-second budget covers callback subprocess work; it is not an end-to-end
wall-clock promise for database waits or an unresponsive OS. SQLite's existing
five-second busy timeout remains independent.

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

No DB column backs the native status; supervision does not consume it — the
monitor loop's wake is unconditional and periodic. See
[Monitoring](../concepts/monitoring.md).

## The monitor wake and the fixed direct ping

The monitor loop is one fixed-cadence `scan → wake → sleep` path on both
backends. Its only keystroke target is the monitor member's own pane; it
never calls `send_poll_trigger` and never keystrokes any other pane.

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
monitor member, unchanged on both backends: `Esc`, then the literal payload
`cafleet message poll <member-id> — then
resume your work if something was still running.`, then `Enter` (a
pending-placement target skips the keystroke and succeeds). It cannot carry
arbitrary text.

Anything a member needs from the Director travels as a plain
`cafleet message send` — the same persisted queue and Esc-safeguarded
inline-preview path every fleet message uses.

## Access mechanism

Each backend's access mechanism is in the [backend matrix](#backend-matrix);
the backend's binary is expected on `PATH` — no other dependency.

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

`cafleet member prompt` delivers its keystrokes through the multiplexer
interface's `send_prompt(target_pane_id, text, shell = false)` operation.

Both backends validate fail-fast: text empty after strip →
`send_prompt: text may not be empty`; the **original** text containing `\n` or
`\r` → `send_prompt: text may not contain newlines` (raised as the backend's
native error type, `TmuxError` / `HerdrError`). Both forms use the same Esc
safeguard and failure semantics; the `shell` flag controls only whether the
payload has the `! ` prefix. The per-backend payloads are in the
[backend matrix](#backend-matrix). Both herdr forms mirror
`send_poll_trigger`'s esc-then-run shape.

## Push notifications {#push-notifications}

CAFleet's delivery model is pull-based: recipients discover messages via
`cafleet message poll`. To cut latency, the broker keystrokes a 2-line inline
preview into the recipient's pane immediately after persisting a message, so
the recipient's coding agent consumes it as a fresh user-turn input:

```text
[cafleet msg <message_id> from <sender_id> <ts>]
<text-truncated-to-CAFLEET_MAX_TEXT_LEN>
```

The keystroke is dispatched through the resolved backend's
`send_inline_preview` helper; the contract — one Esc-safeguarded submit of the
whole 2-line payload — is identical on both, and the per-backend realization is
in the [backend matrix](#backend-matrix).

### Inline-preview error propagation {#inline-preview-errors}

`send_inline_preview` is the one keystroke path that propagates its failure as
a `Result` carrying the raw backend error instead of a best-effort boolean.
A missing backend binary fails with exactly `tmux binary not found on PATH` or
`herdr binary not found on PATH`; a subprocess failure after that precheck
carries the backend's existing raw error formatting — the failed command, its
payload argv, and a newline-delimited stderr detail — from whichever Escape,
payload, or Enter operation failed. The string is preserved verbatim all the
way to the caller.

The other trigger keystrokes, `send_poll_trigger` and `send_wake_trigger`,
keep their best-effort boolean contract: they never raise, and any failure
returns `false`.

Callers consume the inline-preview `Result` asymmetrically:

| Caller | On an attempted inline-preview failure |
|---|---|
| Unicast `message send` | Surfaces the raw error as an exit-1 partial failure after persistence — see [CLI options](cli-options.md#message-send-partial-failure) |
| `message broadcast` | Discards the individual error; only the `delivered` count reflects it, and the summary output and exit 0 are unchanged |
| `POST /api/messages/send` | Ignores the notification outcome; the 200 `{message_id, status}` response after persistence is unchanged |

The recipient pane is resolved from `member_placements` by `member_id` alone,
so Member → Director notifications work automatically. The recipient acks via
`cafleet message ack <message_id>` once it has consumed the message.
Body truncation in the preview (`…` at `CAFLEET_MAX_TEXT_LEN` codepoints) is
documented in [CLI options](cli-options.md#message-body-truncation).

A member's Director-bound messages ride this same ordinary path — a plain
`cafleet message send` per event, with no monitor-specific delivery state.

### The `Esc` safeguard {#esc-safeguard}

Every keystroke path presses `Escape`, lets the pane settle ~0.1 s, then types
the payload and `Enter`.

| Keystroke path | Payload | Why |
|---|---|---|
| Inline preview (`message send` / `message broadcast`) | The 2-line preview + `Enter` | A recipient parked on a pending permission-approval prompt has it dismissed before the trailing `Enter` lands |
| `cafleet member ping` | The literal poll command with the resume clause + `Enter` | The manual re-poke for a pane that missed an inline preview |
| `cafleet member prompt` (plain and `--shell` forms) | The text, optionally prefixed with `! `, + `Enter` | The same safeguard and failure semantics protect both forms; the flag changes only the payload prefix |
| Exit-command helper (`send_exit`) | `/exit` + `Enter` | Uses the same safeguard, with pane-gone tolerance covering the leading `Esc` when `ignore_missing` is enabled; creation rollback uses `kill_pane` |
| Monitor-loop wake trigger (`send_wake_trigger`) | The `[cafleet] tick:` wake + `Enter` | It targets the monitor member's pane, which can be parked on a permission prompt (see [Monitoring](../concepts/monitoring.md)) |

### Design principles

- **Queue first**: the message queue remains the sole source of truth; a failed
  push leaves the message available for normal polling, and the persisted row
  is never rolled back.
- **Intentional skips stay silent**: a self-send and a recipient whose
  placement has no pane id suppress the notification without an attempt; both
  succeed with `notification_sent: false`.
- **Attempted failures surface**: an attempted preview that fails — a dead
  pane, an absent multiplexer binary, an unavailable or ambiguous multiplexer
  environment — propagates its raw error to the caller, consumed per the
  [caller table](#inline-preview-errors). The notification is attempted at
  most once; no layer retries it.
- **No multiplexer env var required**: the keystroke targets the pane by id
  (tmux `send-keys -t <pane>`, herdr `pane send-*`), which works from any
  process on the same host as long as the multiplexer's server is reachable.

### Response annotations

Unicast success responses include a top-level `notification_sent` boolean —
`true` only when an attempted preview landed, `false` on the intentional
skips; an attempted failure exits 1 with the partial-failure error instead of
printing the success payload (see
[CLI options](cli-options.md#message-send-partial-failure)). Broadcast
responses expose `recipients` (the real recipient count) and `delivered` (how
many recipient panes were successfully triggered) as top-level wrapper fields.
Neither count is persisted — they live only in the broker return value.
