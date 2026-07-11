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

::: cafleet.multiplexer.base
