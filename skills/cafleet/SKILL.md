---
name: cafleet
description: >-
  Multiplex coding agents in tmux or herdr and coordinate them through durable
  messages. Proactively use CAFleet when delegating work that calls for
  subagents or agent teams, including parallel research, implementation, and
  independent review. Also use for CAFleet member lifecycle, messaging, and
  pane interaction.
---

# CAFleet — Message Broker CLI

Use `cafleet` to register sessions, send and receive durable messages, and discover members. CLI commands access SQLite directly.

## Required reading

Open your authoritative role first. Use an available non-shell text reader; prerequisite file reads may use shell when it is the only reader. A member sends ready as its first operational broker shell command, before task work.

| # | Read | Responsibility |
|---|---|---|
| 1 | Your backend in [coding-agents.md](reference/coding-agents.md) | Resolve Runtime bindings and bound notes using your `CODING AGENT:` identity, or your own identity when standalone. |
| 2 | Your [member](roles/member.md) or [Director](roles/director.md) role | Follow startup, BASE, command authority and workflow prerequisites; Directors read Recovery/Shutdown immediately before those actions. |

Load skills through your executing backend's supported loader. Codex/OpenCode read this core, their backend section, each requested skill core and role-required references by absolute path. Assigned members continue their existing workflow when loading its umbrella skill.

## Resolve your overlay

Resolve the overlay read above by the subject of the action:

1. **Select the subject.** For your instructions, use your executing backend's Runtime bindings. Before a spawn, follow [model selection](roles/director.md#model-selection), read the selected backend's Model catalog and Role defaults, confirm its Member eligibility and validate effort/launch capabilities. Retain your own tools and decision surface.
2. **Materialize values.** Resolve runtime placeholders from Runtime bindings and the role's model/effort from the selected spawn backend's Role defaults.
3. **Apply notes.** Follow each Note → applies at caveat at its named consuming instruction. Ordinary members resolve their own runtime section without acquiring model-selection duties.
4. **Self-check at emission.** Emit concrete commands and messages; resolve every runtime `{token}` before emission.

### Documented defaults

Use these neutral defaults only when a backend section omits a token or the backend is unknown.

| Token | Documented default |
|---|---|
| `{decision_surface}` | a Director-relayed operator message; members always relay |
| `{reviewer_model}` | the spawning Director's model, inherited |
| `{reviewer_effort}` | omit `--effort` for the backend default |
| `{other_model}` | the spawning Director's model, inherited |
| `{other_effort}` | omit `--effort` for the backend default |
| `{permission_flags}` | prose only: "host-scoped native permission policy"; spawn-flag construction requires the backend binding |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | read the skill core and your overlay by absolute path |
| `{effort_levels}` | unsupported; omit `--effort` |

## IDs

Pass full database-assigned integers literally. Angle-bracket examples denote values to substitute, and environment variables supply no member identity. Literal IDs preserve host permission-pattern matching.

| Position/flag | Subject |
|---|---|
| Positional `MEMBER_ID` | Recipient on message reads; recorded target on member commands; fleet derives from its row |
| Positional `MESSAGE_ID` | Delivery on `message ack/show`; recipient and fleet derive from it |
| Positional `FLEET_ID` | Fleet on `fleet show/delete` and `member list` |
| `--from-member-id`, `--to-member-id` | Sender/recipient on `message send`; fleet derives from sender |
| `--fleet-id` | Fleet joined by `member create`; Director derives from the fleet |

Read your literal `FLEET ID:`, `DIRECTOR MEMBER ID:` and `YOUR MEMBER ID:` spawn lines. Directors use IDs returned by creation. `<my-member-id>` is your own ID; `<target-member-id>` a recipient; `<message-id>` the printed delivery ID.

Spawn prompts contain exactly the CLI's identity placeholders `{fleet_id}`, `{member_id}`, `{director_member_id}` and `{coding_agent}`. The CLI fills the newly allocated member ID and renders `CODING AGENT:` to the selected backend. Supply exactly one positional `PROMPT` or `--file PATH`; double other literal braces as `{{` / `}}`. The [substitution contract](reference/cli.md#member-create) owns syntax errors and exit semantics; [CLI environment variables](reference/cli.md#environment-variables) owns supported environment settings.

## Soliciting user reactions

Record approval, choices, confirmation and continue-or-abort decisions through {decision_surface}. Members send questions through the broker to the Director, who relays them. Follow the backend's decision-surface note; free-form invitations to respond record no decision.

## One-shot command isolation

Run every one-shot `cafleet` process as the only command of its shell-tool invocation. Submit sequences as separate calls, preserving host Bash hygiene. Leading `NAME=value` assignments may immediately precede `cafleet`; use the actual executable rather than an `env` helper. Long bodies use a positional argument or `--file PATH`, with `--file -` for tool-provided stdin.

For `Operation not permitted` / `Permission denied`, check isolation and the host boundary, retry a combined invocation as an isolated command, then route a still-required denied command through the Director.

## Send (Unicast)

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <target-member-id> "Assignment or reply"
```

Supply one positional body or `--file PATH`; use file input for long/multiline text. Previews truncate by `CAFLEET_MAX_TEXT_LEN`; `--json` returns complete payloads.

Previews carry one `[cafleet msg ID from SENDER TIMESTAMP]` line with the full body or a `message show ID --json` pointer; [wrapped delivery methods](reference/cli.md#wrapped-delivery-methods) owns profile/queue/fallback behavior. The root Director uses native reception or manual broker reads without automatic pane input, and its sends report `notification_sent: false` without a pane diagnostic or multiplexer requirement.

Retain the committed message ID and successful record across a held preview or post-commit delivery error. Send each body once; a later send retries pending ordinary-member notifications. Recipients record deferred actions and ACK consumption. Fan out with one send per recipient and report failed recipients explicitly. [Send outcomes](reference/cli.md#message-send) owns exact diagnostics.

## Poll (Check Inbox)

```bash
cafleet message poll <my-member-id> [--json]
```

Poll returns unACKed `input_required` deliveries newest first; `--json` includes complete bodies. The printed delivery ID differs from any host task-list ID. Members poll after ready, before reports and on every wake, then handle queued assignments before ending the turn. Directors poll on every wake and before completion. A later send retries a pending preview.

## Acknowledge (ACK)

```bash
cafleet message ack <message-id>
```

ACK changes a delivery from `input_required` to `completed`, removing it from poll output. Track unfinished actions outside the inbox after consumption; use `message show MESSAGE_ID --json` to recover the full record.

## Command index

Use [cli.md](reference/cli.md) for invocation forms, flags, outputs and errors. Members retain Send/Poll/ACK/show authority; Director actions follow their role.

| Need | Owner |
|---|---|
| Registry and subjects | [Subcommand summary](reference/cli.md#subcommand-summary) |
| Output fields | [Output shapes](reference/cli.md#output-shapes) |
| Finite receive and preview retries | [Message wait](reference/cli.md#message-wait) |
| Persistent reception | [Message listen](reference/cli.md#message-listen) |
| Spawn and compensation | [Member create](reference/cli.md#member-create) |
| Runtime commands | [Member runtime](reference/cli.md#member-runtime) |
| Native command input | [Member exec](reference/cli.md#member-exec) |
| Ordered manual keys | [Member press](reference/cli.md#member-press) |
| Input exclusion and previews | [Pane input and previews](reference/cli.md#pane-input-and-previews) |
| Host continuation and evidence | Backend notes in [coding-agents.md](reference/coding-agents.md) |
| Diagnosis and installation | [Setup and doctor](reference/cli.md#setup-and-doctor) |
