---
icon: lucide/layout-grid
---

# Multiplexer backends

cafleet hosts every coding-agent member inside a **terminal-multiplexer pane**.
The multiplexer is abstracted behind the `Multiplexer` Protocol
(`cafleet.multiplexer.base`), so the spawn, keystroke-delivery, capture, and
teardown paths are backend-neutral. Two backends ship today: **tmux** and
**herdr** ([herdr.dev](https://herdr.dev), a client-server terminal workspace
manager for AI coding agents). Both satisfy the same Protocol, so every
`member *` path behaves identically regardless of which one is active.

Pane ids are treated as **opaque strings** end to end — tmux ids look like `%7`,
herdr ids look like `w1:p1`; cafleet stores and passes them verbatim and never
parses them.

## Backend selection

Every call site resolves its backend through `resolve_multiplexer()` rather than
hardcoding a backend. Resolution precedence:

1. **Explicit override.** If `CAFLEET_MULTIPLEXER` is set, it must name a
   supported backend (`tmux` or `herdr`); an unknown value fails loudly.
2. **Auto-detect from the environment.** `HERDR_ENV` truthy signals a herdr
   session; `TMUX` set signals a tmux session.
3. **Ambiguity is a hard error.** Both `HERDR_ENV` and `TMUX` set → error
   (set `CAFLEET_MULTIPLEXER` to disambiguate). Neither set → error (run cafleet
   inside a tmux or herdr session, or set `CAFLEET_MULTIPLEXER`). Exactly one
   present → that backend.

Auto-detect (an unset `CAFLEET_MULTIPLEXER`) is the default: absence is a valid,
well-defined state, not a fallback for a missing value. The override is the
deterministic escape hatch when the environment is ambiguous.

`cafleet doctor` reports the resolved backend and its identifiers, so an operator
can confirm which multiplexer is active without inspecting the environment
directly (see [CLI options](../spec/cli-options.md#cafleet-doctor)).

## Error taxonomy

Backend failures share a base `MultiplexerError`, with `TmuxError` and
`HerdrError` as backend-specific subclasses. CLI boundaries catch
`MultiplexerError`, so both backends' failures are handled uniformly while each
backend keeps its own message text.

## Native agent-state (herdr only)

herdr natively tracks each agent's lifecycle state
(`working`/`blocked`/`done`/`idle`/`unknown`). This is exposed through a
**separate optional capability** Protocol, `AgentStateAware`, that only the herdr
backend implements — the base `Multiplexer` Protocol stays clean and tmux
implements nothing new. On the herdr backend the monitor loop point-reads each
watched agent's native status and flags it due when the status transitions into
`done` — the sole wake-on-status state (`_WAKE_ON_STATUS = ("done",)`) — in
addition to the interval and stall-check triggers. A transition into `blocked` is
recorded but never flags a wake (an agent awaiting a user answer must not be woken
about). On the tmux backend the capability is absent, so this native branch never
runs; tmux agents come due by interval and stall-check only. See
[Monitoring](monitoring.md) for the native-status due trigger.

## Access mechanism

The herdr backend uses the **herdr CLI** exclusively (subprocess), mirroring how
the tmux backend shells out to `tmux`. herdr also exposes a newline-delimited
JSON unix-socket API whose only unique capability is a **push** event stream;
that would require a persistent connection and a concurrent reader that
cafleet's synchronous `scan → wake → sleep` monitor loop does not have. The
native agent-state the monitor needs is fully reachable through the CLI as a
cheap per-tick point read, so the socket event stream is a deliberately-deferred
optimization. No new Python dependency is added — the backend shells out to the
`herdr` binary, expected on `PATH` when running inside a herdr environment.
