---
icon: lucide/code
---

# multiplexer

The terminal-multiplexer abstraction: pane discovery, window/pane splitting,
keystroke delivery, and capture used by the spawn and push-notification paths.
The `Multiplexer` Protocol is backend-neutral; `tmux` and `herdr` are the
shipped backends, each selected per runtime environment by
`resolve_multiplexer()`. The optional `AgentStateAware` Protocol adds native
agent-state reads that only `herdr` implements. See
[Multiplexer backends](../spec/multiplexer-backends.md) for backend
selection and auto-detection.

`kill_pane` closes the target pane; a backend may additionally restore the
window layout after the close as a backend-internal detail — herdr rebalances
the remaining member column (and restores the Director pane to full width
after the last member), while tmux relies on the multiplexer's native
auto-fit. See
[Delete-time pane layout](../spec/multiplexer-backends.md#delete-time-pane-layout).

::: cafleet.multiplexer.base
