---
icon: lucide/bell
---

# tmux push notifications

CAFleet uses a pull-based delivery model by default: recipients discover
messages via `cafleet message poll`. To reduce latency, the broker can also
push a poll trigger into a recipient's tmux pane immediately after persisting
a message.

## Send + push notification

```mermaid
sequenceDiagram
    autonumber
    participant Sender as Sender agent
    participant Send as broker.send_message
    participant DB as SQLite tasks
    participant Notify as broker._try_notify_recipient
    participant Mux as TmuxMultiplexer.send_inline_preview
    participant Pane as Recipient pane
    participant Recv as Recipient agent

    Sender->>Send: cafleet message send --to <r> --text <body>
    Send->>DB: INSERT INTO tasks (type='unicast', status='input_required')
    Send->>Notify: _try_notify_recipient(recipient_id, sender_id, task_dict)
    Notify->>DB: SELECT placement.tmux_pane_id WHERE agent_id=<r>
    DB-->>Notify: pane_id
    Notify->>Mux: send_inline_preview(pane_id, task_id_8, sender_8, ts, text)
    Mux->>Pane: keystroke "[cafleet msg <id8> from <s8> <ts>]\n<text>"
    Pane-->>Recv: text appears as fresh user-turn input
    Recv->>Recv: process inline preview
    Recv->>DB: cafleet message ack --task-id <id> → UPDATE status='completed'
```

After `broker` saves a delivery task, it looks up the recipient's
`agent_placements` row. Every agent spawned by `cafleet member create` has a
placement row, and every session's root Director also gets one at
`cafleet session create` time (its placement carries `director_agent_id=NULL`
to indicate "no parent"). Because `_try_notify_recipient` resolves a pane by
`agent_id` alone, Member → Director notifications work automatically once
the root Director has a placement row. If the recipient has a non-null
`tmux_pane_id` and is not the sender, the broker keystrokes an inline
preview of the message itself into the recipient's pane via
`TmuxMultiplexer.send_inline_preview` (imported and instantiated per-call so
per-test `monkeypatch.setattr` on the class method is honored):

```text
[cafleet msg <task_id_8> from <sender_8> <ts>]
<text-truncated-to-CAFLEET_MAX_TEXT_LEN>
```

The recipient's coding agent processes the keystroked text as a fresh
user-turn input — no `cafleet message poll` invocation is in the auto-fire
path. The recipient acks via `cafleet message ack --task-id <task_id>` once
it has consumed the message. This replaces the design-0000049-pre keystroke
(the literal `cafleet --session-id <s> message poll --agent-id <r>` + Enter
sequence, which forced the recipient to dump its full unacked-inbox envelope
on every send and dominated per-message token cost).

The `TmuxMultiplexer.send_inline_preview` method is **NOT** a reuse of
`send_freetext_and_submit` (which prepends a literal `4` for
AskUserQuestion option-4 freetext semantics; reusing it would type a stray
`4` into the recipient's input box). The helper combines the literal-text-
plus-Enter pattern from `send_poll_trigger` with the codex bracketed-paste
delay so all three backends (`claude`, `codex`, `opencode`) see the
preview as a single user-turn message.

If the recipient's TUI is in a non-input state, the keystroked preview
lands wherever the cursor is (same failure mode as the legacy auto-fire
poll). The fallback chain is `cafleet member list --agent-id <d>
--activity` (Director observes the recipient's `last_recv` column went
stale), then `cafleet member ping --agent-id <d> --member-id <r>` (manual
re-poke that injects the legacy poll command + Enter so the recipient
catches up via a normal `message poll` round-trip).

## Design principles

- **Best-effort**: The message queue remains the sole source of truth. Push
  notification is an optimization — if it fails, the message is still
  available for normal polling.
- **Self-send skip**: When sender == recipient, the notification is
  suppressed.
- **Silent failure**: Missing placements, null `tmux_pane_id`, dead panes,
  and absent `tmux` binary all result in `False` — no exceptions propagate
  to the caller.
- **No `TMUX` env var required**: `tmux send-keys -t <pane>` works from any
  process on the same host as long as the tmux server socket is accessible.

**Response annotations**: Unicast responses include a top-level
`notification_sent` boolean. Broadcast responses expose
`notifications_sent_count` as a top-level wrapper field (returned alongside
the `broadcast_summary` task), reflecting how many recipient panes were
successfully triggered. Post-Surface-14 the `tasks` schema has no metadata
blob — the count is NOT persisted on the summary row; it lives only in the
broker return value.

**Manual entry-point**: `TmuxMultiplexer.send_poll_trigger` survives as the
method for the **manual** poll-nudge path only — the sole caller is
`cafleet member ping` (the Director-only manual nudge subcommand, which
converts a `False` return to exit 1 so an operator or monitoring loop sees
the failure). Auto-fire on every `cafleet message send` uses
`TmuxMultiplexer.send_inline_preview`; the `member ping` path keeps the
"type the poll command + Enter" behavior because that primitive is exactly
what an operator wants when the recipient missed an inline preview and
needs to drain whatever has accumulated.

## CLI message body truncation

Every `cafleet message *` subcommand that emits a user-supplied delivery
body (`send`, `poll`, `ack`, `cancel`, `show`) truncates the `text` body to
the first `CAFLEET_MAX_TEXT_LEN` (default `200`) Unicode codepoints with a
literal `…` suffix in both text and `--json` output by default. Empty
bodies and bodies whose codepoint length is at most `CAFLEET_MAX_TEXT_LEN`
(default `200`) pass through unchanged with no marker. A per-subcommand
`--full` flag restores the un-truncated body. Truncation runs in
`cafleet/src/cafleet/output.py` via `truncate_text` and `truncate_task_text`
helpers and is wired into the message subcommands through the shared
`_client_command` decorator before either the text formatter or
`format_json` runs, so `--full` and `--json` compose orthogonally.

`cafleet message broadcast` is different —
`broker.broadcast_message` returns a single envelope list containing a
`broadcast_summary` task whose top-level `text` column is the broker-
generated summary string (e.g. `Broadcast sent to N recipients`), not the
original body. Truncating that summary would hide the recipient count, so
`message_broadcast` is wired with `truncates_task_text=False`. The `--full`
Click option is preserved on `message broadcast` for flag-surface
consistency across all six subcommands but is a no-op there.

The truncation applies to CLI emit sites only. FastAPI `/api/*` responses
are unchanged — the WebUI is human-facing and renders full bodies. `member
capture` content, `agent.description`, `skills[].description`, and
`agent_card_json` sub-fields are also untouched in this release.
