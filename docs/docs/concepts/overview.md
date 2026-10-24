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
| inline preview | A literal one-line notification submitted Escape-first into the recipient's pane, carrying the full body or a show pointer | [Notifications](../spec/multiplexer-backends.md#push-notifications) |
| receive | A finite inbox wait for scripts and manual use that also retries pending previews | [Message wait](../spec/cli-options.md#message-wait) |
| coding-agent backend | The selected agent binary recorded in a placement | [Coding agents](coding-agents.md) |

## Fleet isolation

Use the literal IDs returned by the CLI. A fleet partitions routing within the local trust model; it supplies no authentication. Members of a fleet share the selected database URL. Cross-fleet sends fail with `members {from} and {to} are not in the same fleet.`; missing destinations fail with `Destination member not found: {to_id}`.

Fleet creation atomically registers the calling Director and its current pane. You can create an ordinary member immediately afterward. The Director owns pane teardown and deletes its members before deregistering the fleet.

## Responsibilities

| Owner | Responsibility |
|---|---|
| Host | Continue its task and own native shell/background results |
| Director | Assign work, receive replies, inspect evidence, resolve blockers and close the team |
| CLI | Register sessions, persist messages, deliver them into recipient panes and submit authorized pane input |
| Multiplexer | Split, read, type into and close panes while preserving layout ownership |
| Member | Perform its assignment, report through the broker and route required unavailable commands |

The [CLI reference](../spec/cli-options.md#subcommand-summary) owns the command registry. The product ships as a CLI with embedded skills/presets and this documentation site.

## Receiving delegated work

The Director ends its turn after dispatching work and wakes when a reply is delivered into its pane as a line. On every wake it polls its inbox, ACKs each consumed delivery, records deferred actions in host-owned task state and checks the roster, panes and artifacts. Read the [fleet guide](../how-to/mixed-backend-team.md#manual-lifecycle) for the complete procedure.

## Design document workflow

The bundled design-document workflows preserve clarification, independent review, TDD and explicit user approval. Follow the [design document guide](../how-to/design-doc-development.md).
