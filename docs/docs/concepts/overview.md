# Overview

CAFleet is a message broker and member registry for coding agents. All CLI
commands and the admin WebUI access SQLite directly through a shared broker
package — no HTTP server is needed for member operations. Members are organized
into **fleets** identified by a non-secret `fleet_id` created via
`cafleet fleet create`. Members sharing the same fleet can discover and message
each other; members in different fleets are invisible to one another.

## Core terms

| Term | Definition | Links to |
|---|---|---|
| fleet | isolated namespace partitioning members; identified by a non-secret integer `fleet_id` | [Fleet isolation](fleet-isolation.md) |
| root Director | the member created by `fleet create`; the only member that may own other members | [Member lifecycle](member-lifecycle.md) |
| member | a registry entry spawned by the Director via `cafleet member create`, bound to a multiplexer pane (tmux or herdr) | [Member lifecycle](member-lifecycle.md) |
| placement | the row linking a member to its multiplexer session/window/pane and backend | [Data model](../spec/data-model.md) |
| broker | the data-access layer all CLI commands and the WebUI share; writes SQLite directly | Overview (this page) |
| message | one delivered message; lifecycle `input_required → completed` | [Message envelope](../spec/message-envelope.md) |
| inline preview | the 2-line message preview the broker keystrokes into the recipient's pane | [Multiplexer backends](../spec/multiplexer-backends.md#push-notifications) |
| poll / ack | how a recipient fetches and then confirms consumption of a message | [CLI options](../spec/cli-options.md) |
| coding-agent backend | the binary in a member pane: `claude`, `codex`, or `opencode` | [Coding agents](coding-agents.md) |
| monitor member | the dedicated watcher member spawned first via `cafleet member create --role monitor`; it hosts the fleet's wake loop, classifies member panes on each wake, and contacts the Director only when attention is needed | [Monitoring](monitoring.md) |

## CLI

The `cafleet` CLI has seven entry points — four top-level commands and three
command groups:

| Entry point | Scope | Subcommands |
|---|---|---|
| `setup` | one-time onboarding: brings the database to the current schema and installs the coding-agent assets | — |
| `doctor` | environment check: a three-section diagnosis covering the multiplexer, the database schema, and the coding-agent installs | — |
| `server` | serves the admin WebUI | — |
| `monitor` | the supervision scheduler, run as `cafleet monitor FLEET_ID` | — |
| `fleet` | fleet lifecycle | `create`, `list`, `show`, `delete` |
| `member` | member lifecycle + keystroke interaction | `create`, `delete`, `show`, `list`, `prompt`, `ping`, `capture` |
| `message` | the message broker | `send`, `broadcast`, `poll`, `ack`, `show` |

`member` is the single home for the member lifecycle — spawn, teardown,
introspection (`show`, `list`), and keystroke interaction (the `kind` column in
list output carries the three-value union distinguishing the root `director`,
the fleet's `monitor` member, and ordinary `member` rows). The canonical CLI
surface — every subcommand, option, and option source — lives at
[CLI options](../spec/cli-options.md).

## WebUI

A browser-based dashboard served as a SPA at `/`, with no login: a fleet
picker, then a Discord-style unified timeline per fleet — a member sidebar,
unicast and broadcast messages, and a bottom input parsing `@<member> text` and
`@all text`. Every send goes out as the fleet's root Director — the operator
never registers as a member to use the dashboard. The full API surface and
per-member routes live at [WebUI API](../spec/webui-api.md).

## Monitoring

A fleet is supervised by its **monitor member** — a dedicated watcher the
Director spawns first with `cafleet member create --role monitor`. The monitor
member runs the `cafleet monitor` loop as a background task in its own pane;
once per wake interval the loop keystrokes a wake into the monitor member's
pane naming every ordinary member and its pending-message count. The monitor
member classifies each pane on wake and contacts the Director only when
something needs attention; the Director owns every supervision action.
See [Monitoring](monitoring.md).

## Design-document orchestration

CAFleet ships design-document skills that coordinate a Director and members
entirely through `cafleet message send`, so every inter-member message is
persisted and auditable. See [Quickstart](../quickstart.md).
