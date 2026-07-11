---
icon: lucide/rocket
---

# CAFleet

Agent Teams reinvented for collaborative coding across multiple coding-agent
backends, with full code transparency.

CAFleet is a message broker and member registry for coding agents, exposing a
unified `cafleet` CLI over a single-file SQLite database.
Fleets partition members into isolated namespaces and the CLI accesses SQLite
directly through a shared `broker` module, so no HTTP server is required; three
coding-agent backends — `claude` (Claude Code), `codex` (OpenAI Codex CLI), and
`opencode` — coexist in the same fleet, for developers and operators running
auditable multi-agent coding teams in tmux or herdr.

- Persistent, auditable messages — [Storage](concepts/storage.md)
- Pluggable multiplexer backends (tmux / herdr) — [Multiplexer backends](spec/multiplexer-backends.md)
- Push notifications — [Multiplexer backends § Push notifications](spec/multiplexer-backends.md#push-notifications)
- Monitoring member — [Monitoring](concepts/monitoring.md)
- Design-doc-driven development (SDD skills) — [Design-doc-driven development](how-to/design-doc-development.md)

[Get started :material-arrow-right:](get-started/){ .md-button .md-button--primary }

## Browse the docs

- [Get Started](get-started/) — install, configure, and quickstart walkthroughs.
- [How-to guides](how-to/) — prompt-first task guides for common workflows.
- [Concepts](concepts/overview.md) — architecture and the ideas behind it.
- [Specification](spec/data-model.md) — data model, message envelope, CLI, multiplexer backends, and coding-agent backends.
- [API Reference](api/broker.md) — Python API generated from source.
