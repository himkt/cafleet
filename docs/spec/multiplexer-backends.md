---
icon: lucide/layout-grid
---

# Multiplexer backends

cafleet hosts every coding-agent member inside a **terminal-multiplexer pane**.
The multiplexer is abstracted behind the `Multiplexer` Protocol
(`cafleet.multiplexer.base`), so the spawn, keystroke-delivery, capture, and
teardown paths are backend-neutral. Two backends ship today: **tmux** and
**herdr** ([herdr.dev](https://herdr.dev)). Both satisfy the same Protocol, so
every `member *` path behaves identically regardless of which one is active.

Pane ids are treated as **opaque strings** end to end — tmux ids look like `%7`,
herdr ids look like `w1:p1`; cafleet stores and passes them verbatim and never
parses them.

The pane is also cafleet's only push channel: message delivery stays pull-based
(recipients drain the persisted queue with `cafleet message poll`), and the
broker keystrokes a best-effort inline preview into the recipient's pane after
persisting a message — see [Push notifications](#push-notifications).

## Backend selection {#backend-selection}

Every call site resolves its backend through `resolve_multiplexer()`
([API reference](../api/multiplexer.md)) rather than hardcoding one. Resolution
precedence:

1. **Explicit override.** If `CAFLEET_MULTIPLEXER` is set, it must name a
   supported backend (`tmux` or `herdr`); an unknown value fails loudly.
2. **Auto-detect from the environment.** `HERDR_ENV` truthy signals a herdr
   session; `TMUX` set signals a tmux session.
3. **Ambiguity is a hard error.** Both `HERDR_ENV` and `TMUX` set → error
   (set `CAFLEET_MULTIPLEXER` to disambiguate). Neither set → error (run cafleet
   inside a tmux or herdr session, or set `CAFLEET_MULTIPLEXER`). Exactly one
   present → that backend.

Auto-detect (an unset `CAFLEET_MULTIPLEXER`) is the default: absence is a valid,
well-defined state. `cafleet doctor` reports the resolved backend and its
identifiers (see [CLI options](cli-options.md#cafleet-doctor)).

## Error taxonomy

Backend failures share a base `MultiplexerError`, with `TmuxError` and
`HerdrError` as backend-specific subclasses. CLI boundaries catch
`MultiplexerError`, so both backends' failures are handled uniformly while each
backend keeps its own message text.

## Native agent-state (herdr only) {#native-agent-state}

herdr natively tracks each agent's lifecycle state
(`working`/`blocked`/`done`/`idle`/`unknown`), exposed through a separate
optional capability Protocol, `AgentStateAware`, that only the herdr backend
implements — the base `Multiplexer` Protocol stays clean and tmux implements
nothing new.

On the herdr backend the monitor loop point-reads each watched member's native
agent status per tick and flags it due when the status transitions into `done`
— the sole wake-on-status state (`_WAKE_ON_STATUS = ("done",)`) — in addition
to the interval and stall-check triggers. A transition into `blocked` is
recorded but never flags a wake: a blocked member is awaiting a user answer and
must not be woken about. On the tmux backend the capability is absent, so
members come due by interval and stall-check only. No DB column backs the
native status; the
last-seen state lives only in the running loop's memory. See
[Monitoring](../concepts/monitoring.md).

## Access mechanism

The herdr backend uses the **herdr CLI** exclusively (subprocess), mirroring how
the tmux backend shells out to `tmux` — no new Python dependency; the binary is
expected on `PATH`. herdr also exposes a JSON unix-socket API whose only unique
capability is a push event stream; that would require a persistent connection
and a concurrent reader that cafleet's synchronous `scan → wake → sleep`
monitor loop does not have, so the socket stream is a deliberately-deferred
optimization.

## Pane spawn working directory {#pane-spawn-cwd}

A member pane spawned by `cafleet member create` starts in the invoking
process's working directory (the Director's pane cwd). Each backend realizes
this differently:

- **tmux** relies on the multiplexer's builtin cwd inheritance — a new pane
  starts in the splitting pane's working directory.
- **herdr** receives the member-create process's cwd explicitly: every
  `herdr pane split` carries `--cwd <dir>`, where `<dir>` is `os.getcwd()` of
  the invoking process. herdr's own inheritance is not relied upon because
  herdr spawns `/bin/sh` instead of the passwd login shell when `SHELL` is
  unset ([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)).
  An unresolvable cwd fails the spawn loudly with `HerdrError`; there is no
  fallback directory.

## Delete-time pane layout {#delete-time-pane-layout}

Closing a member pane leaves the two backends asymmetric on layout reflow:

- **tmux** issues a bare `kill-pane` and relies on the multiplexer's native
  auto-fit — the remaining panes reclaim the freed space with no explicit
  layout step.
- **herdr** has no native reflow, so `kill_pane` restores the layout itself:
  it reads the target pane's tab (`herdr pane get`) before the close, runs
  `herdr pane close`, then rebalances best-effort, scoped to that tab. With
  ≥ 2 members remaining, the member column is re-equalized to equal heights
  (the same invariant the create path enforces); after the last member is
  deleted, the Director pane is explicitly restored to full tab width when the
  layout read shows a residual right split; a single remaining member needs no
  resize. The rebalance silently skips on unexpected layout shapes and whenever
  the focused-tab layout reports a different tab than the killed pane's — it
  never resizes an unrelated tab. Any `HerdrError` during the rebalance is
  swallowed: a layout failure never fails `member delete` — the pane is closed
  and the member deregistered regardless.

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
`send_inline_preview` helper — tmux realizes it with `send-keys`, herdr with
`pane send-text` + `pane send-keys`; the contract (one Esc-safeguarded submit
of the whole 2-line payload) is identical on both.

```mermaid
%%{init: {'theme': 'default', 'sequence': {'actorFontSize': 18, 'messageFontSize': 16, 'noteFontSize': 16, 'wrap': true, 'width': 180}}}%%
sequenceDiagram
    autonumber
    participant Sender
    participant Broker
    participant DB as SQLite
    participant Pane
    participant Recipient

    Sender->>Broker: cafleet message send --to-member-id <recipient-id> --text <body>
    Broker->>DB: INSERT messages (status=input_required)
    Broker->>DB: SELECT placement.mux_pane_id
    DB-->>Broker: pane_id
    Broker->>Pane: keystroke inline preview
    Pane-->>Recipient: text appears as user-turn input
    Recipient->>DB: message ack → status=completed
```

The recipient pane is resolved from `member_placements` by `member_id` alone,
so Member → Director notifications work automatically. The recipient acks via
`cafleet message ack --message-id <message_id>` once it has consumed the message.
Body truncation in the preview (`…` at `CAFLEET_MAX_TEXT_LEN` codepoints) is
documented in [CLI options](cli-options.md#message-body-truncation).

### The `Esc` safeguard {#esc-safeguard}

The preview keystroke **leads with `Esc`** (`send_inline_preview` is called
with `esc_first=True`): it presses `Escape`, lets the pane settle ~0.1 s, then
types the payload and `Enter`, so a recipient parked on a pending
permission-approval prompt has that prompt dismissed before the trailing
`Enter` lands. The same `Esc`-safeguarded path serves `message send` and
`message broadcast`. Two related keystroke paths differ:

- `cafleet member ping` injects `Esc` → a literal `cafleet message poll`
  command → `Enter` (the `send_poll_trigger` helper, also `esc_first=True`) —
  the manual re-poke for a pane that missed an inline preview.
- The monitor loop's wake nudge targets only the monitoring member's own pane,
  which is never parked on a permission prompt, so it does **not** lead with
  `Esc` (see [Monitoring](../concepts/monitoring.md)).

### Design principles

- **Best-effort**: the message queue remains the sole source of truth; a failed
  push leaves the message available for normal polling.
- **Self-send skip**: when sender == recipient, the notification is suppressed.
- **Silent failure**: missing placements, null `mux_pane_id`, dead panes, and
  an absent multiplexer binary all result in no notification — no exceptions
  propagate to the caller.
- **No multiplexer env var required**: the keystroke targets the pane by id
  (tmux `send-keys -t <pane>`, herdr `pane send-*`), which works from any
  process on the same host as long as the multiplexer's server is reachable.

### Response annotations

Unicast responses include a top-level `notification_sent` boolean. Broadcast
responses expose `recipients` (the real recipient count) and `delivered` (how
many recipient panes were successfully triggered) as top-level wrapper fields.
Neither count is persisted — they live only in the broker return value.
