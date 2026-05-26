---
icon: lucide/rocket
---

# CAFleet

Agent Teams reinvented for collaborative coding across multiple coding-agent
backends, with full code transparency.

CAFleet is a message broker and agent registry for coding agents. It ships a
unified `cafleet` CLI and an admin WebUI on top of a single-file SQLite
database. Sessions partition agents into isolated namespaces; the CLI accesses
SQLite directly through a shared `broker` module, so no HTTP server is required
for agent operations.

## Component overview

```mermaid
flowchart LR
    CLI["CLI (click)"] --> Broker["broker.py<br/>(sync SQLAlchemy)"]
    WebUI["Admin WebUI"] --> Server["server.py<br/>(FastAPI)"]
    Server --> WebUIAPI["webui_api.py"]
    WebUIAPI --> Broker
    Broker --> DB[(SQLite)]
```

CAFleet works with three coding-agent backends — `claude` (Claude Code), `codex`
(OpenAI Codex CLI), and `opencode` — and members from different backends can
coexist in the same session.

[Get started :material-arrow-right:](get-started/){ .md-button .md-button--primary }
