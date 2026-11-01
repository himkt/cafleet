# Overview

CAFleet multiplexes independent coding-agent sessions in tmux or herdr. Use its CLI and skills to delegate research, implementation and review, then collect durable replies while your host continues the task.

## Core terms

| Term | Definition | Reference |
|---|---|---|
| fleet | A namespace identified by a non-secret integer `fleet_id` | [Fleet isolation](#fleet-isolation) |
| root Director | The current session registered by fleet creation; owns assignments and team lifecycle | [Members](members.md) |
| member | An independent coding-agent session registered in a fleet | [Members](members.md) |
| placement | The association between a member and its multiplexer session, window, pane and backend | [Database schema](https://github.com/himkt/cafleet/blob/main/SPEC.md#8-database-schema) |
| broker | The synchronous SQLite data-access layer used by the CLI | [Storage](storage.md) |
| message | One point-to-point delivery, consumed through `input_required → completed` | [Output and formatting](https://github.com/himkt/cafleet/blob/main/SPEC.md#64-output--formatting) |
| inline preview | A literal one-line notification submitted through the member’s recorded transport when input is eligible, carrying the full body or a show pointer | [Push notifications](https://github.com/himkt/cafleet/blob/main/SPEC.md#push-notifications) |
| receive | A finite inbox wait for scripts and manual use that also retries ordinary-member previews | [Message wait](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-wait) |
| inbox stream | A persistent read-only JSONL observer of pending messages with explicit ACK consumption | [Message listen](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-listen) |
| runner | The owned PTY observer and input arbiter for a member run | [Member permission events](members.md#member-permission-events) |
| runtime event | A durable typed terminal/lifecycle observation linked to the Director’s inbox | [Member runtime](https://github.com/himkt/cafleet/blob/main/SPEC.md#69-member-pty-runtime) |
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

The [subcommand summary](https://github.com/himkt/cafleet/blob/main/SPEC.md#subcommand-summary) owns the command registry. The product ships as a CLI with embedded skills/presets and this documentation site.

## Receiving delegated work

The Director receives replies through its native inbox tools and ordinary members receive pane previews; the [fleet guide](../mixed-backend-team.md#manual-lifecycle) explains reception, consumption and recovery.

## Design document workflow

Develop a feature through a reviewed design and an implementation.
The `cafleet` and `cafleet-design-doc` skills coordinate the teams; give your
coding agent one prompt per stage.

### Prompts {#prompts}

Each prompt triggers one `cafleet-design-doc` workflow:

| Stage | Prompt | Outcome | Team |
|---|---|---|---|
| 1. Draft | `Create a design doc for <one-line feature description>.` | Clarification, drafting and review, followed by your approval | Director + Drafter + Reviewer |
| 2. Implement | `Implement design-docs/NNNNNNN-<slug>.` | Implementation and checks, fresh review, then your approval | Director + Programmer + Tester for code, optional Verifier, then a fresh Reviewer |

These stages invoke create and execute respectively. Documentation/configuration
work may use a Programmer without a Tester. Contributor setup is in
[Contributing](../contributing.md).

### Where output lands

Creation writes `design-docs/NNNNNNN-<slug>/design-doc.md` in your repository.
CAFleet assigns a seven-digit, zero-padded number one above the highest existing
number in that repository's `design-docs/`, starting at `0000001`. The slug is a
short kebab-case description, such as `add-search`, giving `0000001-add-search`.
Execution updates the design's task progress while
implementing the approved work.

### Watch the team work

Read durable coordination with message poll/show and inspect the team’s visible panes with `member capture`. The Director receives replies and checks evidence until delegated work resolves; the [manual lifecycle](../mixed-backend-team.md#manual-lifecycle) explains reception.

### Invocation syntax

See your coding-agent's skill documentation for the literal invocation
syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill
discovery).
