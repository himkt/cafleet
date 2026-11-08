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

Open your authoritative role first. Read prerequisites with an available non-shell text reader, or with shell when it is the only reader. A member sends ready as its first operational broker shell command, before task work.

| # | Read | Responsibility |
|---|---|---|
| 1 | Your backend in [coding-agents.md](reference/coding-agents.md) | Resolve Runtime bindings and bound notes for your `CODING AGENT:` identity, or your own identity when standalone. |
| 2 | Your [member](roles/member.md) or [Director](roles/director.md) role | Follow startup, BASE, command authority and workflow prerequisites; Directors read Recovery/Shutdown immediately before those actions. |

Load skills through your executing backend's supported loader; a backend without one reads this core, its backend section, each requested skill core and role-required references by absolute path. Assigned members continue their existing workflow when loading its umbrella skill.

## Resolve your overlay

Resolve the overlay read above by the subject of the action:

1. **Select the subject.** Your own instructions use your executing backend's Runtime bindings. Before a spawn, follow [model selection](roles/director.md#model-selection), read the selected backend's Model catalog and Role defaults, confirm its Member eligibility and validate effort/launch capabilities, keeping your own tools and decision surface.
2. **Materialize values.** Resolve runtime placeholders from Runtime bindings and the role's model/effort from the selected spawn backend's Role defaults.
3. **Apply notes.** Follow each Note → applies at caveat at its named consuming instruction. Ordinary members resolve their own runtime section and take on no model-selection duties.
4. **Self-check at emission.** Resolve every runtime `{token}` before emitting a concrete command or message.

### Documented defaults

Use these neutral defaults only when a backend section omits a token or the backend is unknown.

| Token | Documented default |
|---|---|
| `{decision_surface}` | a Director-relayed operator message; members always relay |
| `{reviewer_model}`, `{other_model}` | the spawning Director's model, inherited |
| `{reviewer_effort}`, `{other_effort}` | omit `--effort` for the backend default |
| `{permission_flags}` | prose only: "host-scoped native permission policy"; spawn-flag construction requires the backend binding |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | read the skill core and your overlay by absolute path |
| `{effort_levels}` | unsupported; omit `--effort` |

## IDs

Pass full database-assigned integers literally; literal IDs preserve host permission-pattern matching. Angle-bracket examples denote values to substitute (`<my-member-id>` your own ID, `<target-member-id>` a recipient, `<message-id>` the printed delivery ID), and environment variables supply no member identity.

| Position/flag | Subject |
|---|---|
| Positional `MEMBER_ID` | Recipient on message reads; recorded target on member commands; fleet derives from its row |
| Positional `MESSAGE_ID` | Delivery on `message ack/show`; recipient and fleet derive from it |
| Positional `FLEET_ID` | Fleet on `fleet show/delete` and `member list` |
| `--from-member-id`, `--to-member-id` | Sender/recipient on `message send`; fleet derives from sender |
| `--from-member-id` on a Director-only command | The caller, which must be the subject fleet's root Director |
| `--fleet-id` | Fleet joined by `member create`; Director derives from the fleet |

Members read their literal `FLEET ID:`, `DIRECTOR MEMBER ID:`, `YOUR MEMBER ID:` and `DATABASE URL:` spawn lines; Directors use IDs returned by creation. Spawn prompts contain exactly the CLI placeholders `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}` and `{database_url}`: the CLI fills the new member ID, renders `CODING AGENT:` to the selected backend and `DATABASE URL:` as `default` or `sqlite:///<path>`, and `member create` refuses a prompt without `{database_url}` for a non-default database. Supply exactly one positional `PROMPT` or `--file PATH`.

CAFleet reads no `CAFLEET_*` environment variable. When `DATABASE URL:` names a URL rather than `default`, pass `--database-url <url>` on every `cafleet` command, broker commands and the ready signal included (`cafleet message poll 7 --json --database-url <url>`); omitted, the default database applies ([database selection](reference/cli.md#database-selection)).

## Soliciting user reactions

Record approval, choices, confirmation and continue-or-abort decisions through {decision_surface}, following the backend's decision-surface note; a free-form invitation to respond records no decision. Members send questions through the broker to the Director, who relays them.

## One-shot command isolation

Run every one-shot `cafleet` process as the only command of its shell-tool invocation, and submit a sequence as separate calls under host Bash hygiene. Leading `NAME=value` assignments may immediately precede `cafleet`; invoke the actual executable, not an `env` helper. Long bodies use a positional argument or `--file PATH` (`--file -` for tool-provided stdin).

On `Operation not permitted` / `Permission denied`, check isolation and the host boundary, retry a combined invocation as an isolated command, then route a still-required denied command through the Director.

## Send (Unicast)

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <target-member-id> "Assignment or reply"
```

Supply one positional body or `--file PATH`, using file input for long/multiline text. Text output truncates bodies at a fixed 200 codepoints; `--json` returns them complete.

Previews carry one `[cafleet msg ID from SENDER TIMESTAMP]` line with the full body or a `message show ID --json` pointer; [wrapped delivery methods](reference/cli.md#wrapped-delivery-methods) owns profile/queue/fallback behavior. The root Director uses native reception or manual broker reads without automatic pane input, and its sends report `notification_sent: false` without a pane diagnostic or multiplexer requirement.

Retain the committed message ID and successful record across a held preview or post-commit delivery error. Send each body once; a later send or `message wait` retries pending ordinary-member notifications. Recipients record deferred actions and ACK consumption. Fan out one send per recipient and report each failed recipient; [send outcomes](reference/cli.md#message-send) owns exact diagnostics.

## Poll and ACK

```bash
cafleet message poll <my-member-id> [--json]
```

```bash
cafleet message ack <message-id>
```

Poll returns unACKed `input_required` deliveries newest first, with complete bodies under `--json`; your role file sets when to poll. The printed delivery ID differs from any host task-list ID. ACK changes a delivery from `input_required` to `completed`, removing it from poll output. Track unfinished actions outside the inbox after consumption; `message show MESSAGE_ID --json` recovers the full record.

## Command index

Use [cli.md](reference/cli.md) for invocation forms, flags, outputs and errors; its [subcommand summary](reference/cli.md#subcommand-summary) is the command registry, and per-command sections follow beside the shared output-shape, pane-input and setup/doctor sections. Backend notes in [coding-agents.md](reference/coding-agents.md) own host continuation and evidence. Members retain Send/Poll/ACK/show authority; Director actions follow their role.
