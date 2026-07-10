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

The active backend is auto-detected from the environment, with the
`CAFLEET_MULTIPLEXER` environment variable as the deterministic override for
ambiguous environments. The resolution rules, the error taxonomy, and the
optional per-backend capabilities are specified in
[Multiplexer backends (specification)](../spec/multiplexer-backends.md).

## Push notifications

The pane is also cafleet's only push channel. Message delivery is pull-based —
the persisted queue is the sole source of truth and recipients drain it with
`cafleet message poll` — but after persisting a message the broker keystrokes
a short inline preview into the recipient's pane, so the recipient's coding
agent reacts immediately instead of waiting for its next poll. The push is a
best-effort latency optimization: if the keystroke fails for any reason, the
message still arrives on the next poll. The exact payload, the `Esc`
safeguard, and the failure semantics are specified in
[Push notifications](../spec/multiplexer-backends.md#push-notifications).
