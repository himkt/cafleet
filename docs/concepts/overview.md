---
icon: lucide/layers
---

# Overview

CAFleet is a message broker and agent registry for coding agents. All CLI
commands and the admin WebUI access SQLite directly through a shared broker
module — no HTTP server is needed for agent operations. Agents are organized
into **fleets** identified by a non-secret `fleet_id` created via
`cafleet fleet create`. Agents sharing the same fleet can discover and message
each other; agents in different fleets are invisible to one another.

## Architecture diagram

```mermaid
%%{init: {'theme': 'default', 'themeVariables': {'fontSize': '16px'}}}%%
flowchart LR
    CLI["CLI (click)"] --> Broker["broker.py<br/>(sync SQLAlchemy)"]
    WebUI["Admin WebUI"] --> Server["server.py<br/>(FastAPI)"]
    Server --> WebUIAPI["webui_api.py"]
    WebUIAPI --> Broker
    Broker --> DB[(SQLite<br/>fleets / agents / tasks / agent_placements)]
    subgraph Multiplexer["tmux"]
        PaneA["coding-agent pane"]
        PaneB["coding-agent pane"]
    end
    Broker -. inline-preview keystroke .-> PaneA
    Broker -. inline-preview keystroke .-> PaneB
```

The broker module is the single data access layer. Both the CLI and the Admin
WebUI call it. No async stores, no HTTP client, no protocol layer.

## CLI

The canonical CLI surface — every subcommand, option, and option source —
lives at [CLI options](../spec/cli-options.md).

## WebUI

A browser-based dashboard served as a SPA at `/`, with no login. The first
load lands on a fleet picker; selecting a fleet opens a Discord-style unified
timeline for that fleet — a sidebar of the fleet's agents, a center timeline
of unicast and broadcast messages, and a bottom input that parses
`@<agent> text` for unicast and `@all text` for broadcast. The admin is not
itself a CAFleet agent; the built-in `Administrator` agent is the implicit
sender on every send.

The WebUI API surface — request / response shape, fleet header convention,
and ACK chip metadata — lives at [WebUI API](../spec/webui-api.md).

## Design document orchestration skills

CAFleet ships CAFleet-native design-document skills
(`cafleet-design-doc-create`, `cafleet-design-doc-execute`) that coordinate a
Director and spawned members entirely through `cafleet message send`, so every
inter-agent message is persisted in SQLite and visible in the admin WebUI
timeline — an auditable trail, in contrast to the ephemeral in-memory
coordination of Agent Teams. See [Quickstart](../get-started/quickstart.md)
and [Contributing](../get-started/contributing.md).
