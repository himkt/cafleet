# Overview

CAFleet multiplexes independent coding-agent sessions in tmux or herdr and coordinates them through durable point-to-point messages. The CLI ships with embedded skills/presets; the host continues the user's task and owns native tool results.

## Ownership

| Owner | Responsibility |
|---|---|
| Host | Permissions, task continuation, native process results and user decisions |
| Director | Scoped assignments, full-message reception, evidence review, Git and owned teardown |
| Member | Assigned implementation/research/review and suitable checks |
| Broker | Atomic registration, persistent messages and explicit consumption |
| Multiplexer | Concrete pane creation, input, capture, close and layout |

A fleet is a local routing namespace identified by an integer. IDs supply coordination, without authentication. Every participant uses the same selected SQLite database. Fleet creation atomically registers the calling Director and its current pane; member creation starts a separate agent session.

## Messages

A send persists before attempting an ordinary-member preview. Poll reads pending deliveries and ACK marks consumption. The [message record](../spec/message-envelope.md) preserves exact body/routing/status fields. Sends addressed to the root Director and self-sends stay in durable inboxes for native/manual reads.

An ordinary-member notification leads with Escape and submits one safe line. Busy input or transport failure can leave committed delivery pending. A later send or finite wait retries eligible messages in order. A persisted message ID remains the identity to consume.

The Director owns one native listener and reconciles replay before acting. The listener observes records without ACK or pane effects. Host goal/decision controls remain in force; see [Run a fleet](../how-to/mixed-backend-team.md#receive-results).

## Storage and safety

Setup initializes/migrates compatible databases and refuses incompatible/ahead storage before mutation. Existing data and history remain. Ordinary commands validate the current application shape. Backup and recovery use the selected database plus any live SQLite sidecars; preserve a consistent copy.

Short transactions protect registration, insertion and finalization. External pane I/O happens outside SQLite write transactions. Physical-pane locks use canonical database/backend/session/pane identity, with placement revalidation after ownership. Signals defer during protected side effects, and mandatory storage finalization retains independent completion grace.

Creating/deleting herdr panes preserves automatic sizing/equalization. Targeted capture cleans ANSI/carriage-return redraws by default. Roster activity and idle distinguish recent communication from completion evidence.

## Where to look

[Quickstart](../quickstart.md) owns installation/trust, the [fleet guide](../how-to/mixed-backend-team.md) owns operation/recovery, and the [CLI reference](../spec/cli-options.md) explains command behavior. CLI help owns the exact option registry; migrations own storage shape and source owns rendering. Skills supply agent procedure and [design recipes](../how-to/design-doc-development.md).
