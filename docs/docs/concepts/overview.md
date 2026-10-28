# Overview

CAFleet multiplexes independent coding-agent sessions in tmux or herdr. Use its CLI and skills to delegate research, implementation and review, then collect durable replies while your host continues the task.

## Core terms

| Term | Definition | Reference |
|---|---|---|
| fleet | A namespace identified by a non-secret integer `fleet_id` | [Fleet isolation](#fleet-isolation) |
| root Director | The current session registered by fleet creation; owns assignments and team lifecycle | [Member lifecycle](member-lifecycle.md) |
| member | An independent coding-agent session registered in a fleet | [Member lifecycle](member-lifecycle.md) |
| placement | The association between a member and its multiplexer session, window, pane and backend | [Data model](../spec/data-model.md) |
| broker | The synchronous SQLite data-access layer used by the CLI | [Storage](storage.md) |
| message | One point-to-point delivery, consumed through `input_required → completed` | [Message envelope](../spec/message-envelope.md) |
| inline preview | A literal one-line notification submitted through the member’s recorded transport when input is eligible, carrying the full body or a show pointer | [Notifications](../spec/multiplexer-backends.md#push-notifications) |
| receive | A finite inbox wait for scripts and manual use that also retries ordinary-member previews | [Message wait](../spec/cli-options.md#message-wait) |
| inbox stream | A persistent read-only JSONL observer of pending messages with explicit ACK consumption | [Message listen](../spec/cli-options.md#message-listen) |
| runner | The owned PTY observer and input arbiter for a member run | [Member permission events](member-permission-events.md) |
| runtime event | A durable typed terminal/lifecycle observation linked to the Director’s inbox | [Member runtime](../spec/member-runtime.md) |
| coding-agent backend | The selected agent binary recorded in a placement | [Coding agents](coding-agents.md) |

## Fleet isolation

Use the literal IDs returned by the CLI. A fleet partitions routing within the local trust model; it supplies no authentication. Members of a fleet share the selected database URL. Cross-fleet sends fail with `members {from} and {to} are not in the same fleet.`; missing destinations fail with `Destination member not found: {to_id}`.

Fleet creation atomically registers the calling Director and its current pane. You can create an ordinary member immediately afterward. The Director owns teardown: deleting the fleet ends each ordinary member and closes its pane before deregistering the fleet, leaving the Director's own pane open.

## Responsibilities

| Owner | Responsibility |
|---|---|
| Host | Continue its task and own native shell/background results |
| Director | Assign work, receive replies, inspect evidence, resolve blockers and close the team |
| CLI | Register sessions, persist messages, stream inbox records, preview ordinary-member messages and submit authorized pane input |
| Multiplexer | Split, read, type into and close panes while preserving layout ownership |
| Member | Perform its assignment, report through the broker and route required unavailable commands |

The [CLI reference](../spec/cli-options.md#subcommand-summary) owns the command registry. The product ships as a CLI with embedded skills/presets and this documentation site.

## Receiving delegated work

The Director owns one native inbox receiver, reconciles full broker records, ACKs consumption and keeps deferred actions in host-owned task state. Ordinary members receive pane previews. Native reception preserves supplied goal controls and user decision gates; the [fleet guide](../how-to/mixed-backend-team.md#manual-lifecycle) explains handle ownership, recovery and manual reception.

## Design document workflow

The bundled design-document workflows preserve clarification, independent review, TDD and explicit user approval. Follow the [design document guide](../how-to/design-doc-development.md).
