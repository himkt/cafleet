---
icon: lucide/rocket
---

# CAFleet

Agent Teams reinvented for collaborative coding across multiple coding-agent
backends, with full code transparency.

CAFleet is a message broker and agent registry for coding agents, exposing a
unified `cafleet` CLI and an admin WebUI over a single-file SQLite database.
Fleets partition agents into isolated namespaces and the CLI accesses SQLite
directly through a shared `broker` module, so no HTTP server is required; three
coding-agent backends — `claude` (Claude Code), `codex` (OpenAI Codex CLI), and
`opencode` — coexist in the same fleet, for developers and operators running
auditable multi-agent coding teams in tmux.

[Get started :material-arrow-right:](get-started/){ .md-button .md-button--primary }

## Browse the docs

- [Get Started](get-started/) — install, configure, and quickstart walkthroughs.
- [How-to guides](how-to/) — prompt-first task guides for common workflows.
- [Concepts](concepts/overview.md) — architecture and the ideas behind it.
- [Specification](spec/data-model.md) — data model, message envelope, CLI, and WebUI API reference.
- [API Reference](api/broker.md) — Python API generated from source.
