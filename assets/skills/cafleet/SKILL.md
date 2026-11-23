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

Use `cafleet` to register sessions, exchange durable messages and discover members.

## Required reading

Open your authoritative role first, with a non-shell text reader when available. A member sends ready as its first operational broker shell command.

| # | Read | Responsibility |
|---|---|---|
| 1 | Your backend in [coding-agents.md](reference/coding-agents.md) | Runtime bindings and bound notes for your `CODING AGENT:` identity (standalone: your own). |
| 2 | Your [member](roles/member.md) or [Director](roles/director.md) role | Startup, BASE, command authority and workflow prerequisites; Directors read Recovery/Shutdown before those actions. |

Load skills through your backend's supported loader, or by absolute path: this core, your backend section, each requested skill core and role-required references. Assigned members keep their existing workflow when loading its umbrella skill.

## Resolve your overlay

1. **Select the subject.** Your own instructions use your executing backend's Runtime bindings. For a spawn, follow [model selection](roles/director.md#model-selection), read the selected backend's Model catalog and Role defaults, confirm its Member eligibility and validate effort/launch capabilities; your own tools and decision surface stay yours.
2. **Materialize values.** Resolve runtime placeholders from Runtime bindings and the role's model/effort from the spawn backend's Role defaults.
3. **Apply notes.** Follow each Note → applies at caveat at its named instruction.
4. **Self-check at emission.** Resolve every `{token}` before emitting a command or message.

### Documented defaults

Use these neutral defaults only when a backend section omits a token or the backend is unknown.

| Token | Documented default |
|---|---|
| `{decision_surface}` | a Director-relayed operator message; members always relay |
| `{reviewer_model}`, `{other_model}` | the spawning Director's model, inherited |
| `{reviewer_effort}`, `{other_effort}` | omit `--effort` |
| `{permission_flags}` | prose only: "host-scoped native permission policy"; spawn flags require the backend binding |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | read the skill core and your overlay by absolute path |
| `{effort_levels}` | unsupported; omit `--effort` |

## IDs

Pass database-assigned integer IDs literally, preserving host permission-pattern matching; environment variables supply no identity. Angle brackets mark values to substitute: `<my-member-id>` (you), `<target-member-id>`, `<message-id>`. A positional ID derives its fleet and recipient from its row ([positional subject ids](reference/cli.md#positional-subject-ids)); `message send` derives the fleet from `--from-member-id`, and `member create --fleet-id` derives the Director from the fleet.

Members use their literal `FLEET ID:`, `DIRECTOR MEMBER ID:`, `YOUR MEMBER ID:` and `DATABASE URL:` spawn lines; Directors use the IDs creation returns and the [`member create`](reference/cli.md#member-create) placeholders.

When `DATABASE URL:` names a URL rather than `default`, pass `--database-url <url>` on every `cafleet` command, the ready signal included ([database selection](reference/cli.md#database-selection)).

## Soliciting user reactions

Record approvals, choices, confirmations and continue-or-abort decisions through {decision_surface}, following its backend note; a free-form invitation records no decision. Members route questions through the Director.

## One-shot command isolation

Run each one-shot `cafleet` process as the only command of its shell-tool call. Leading `NAME=value` assignments may precede `cafleet`; invoke the executable directly, not through `env`.

On `Operation not permitted` / `Permission denied`, retry a combined invocation as an isolated command, then route a still-required denied command through the Director.

## Send (Unicast)

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <target-member-id> "Assignment or reply"
```

Use `--file PATH` (`--file -` for stdin) for long or multiline text; text output truncates bodies at 200 codepoints and `--json` returns them complete. A send stores the message and returns; the recipient's runner types and retries a `[cafleet msg ID from SENDER TIMESTAMP]` preview carrying the body or a `poll` pointer ([wrapped delivery methods](reference/cli.md#wrapped-delivery-methods)). The root Director receives previews the same way, typed into its pane by the fleet's live member runners ([Director previews](reference/cli.md#director-previews)).

Send each body once, one send per recipient, retaining each message ID and reporting each failed recipient ([send outcomes](reference/cli.md#message-send)).

## Poll

```bash
cafleet message poll <my-member-id> [--json]
```

Poll returns and consumes every `input_required` delivery, newest first, with complete bodies under `--json` ([`message poll`](reference/cli.md#message-poll)). Before any other command, record each message's sender, required action and artifact pointers; your role file sets when to poll. `message show MESSAGE_ID --json` reads a consumed message again. Delivery IDs are distinct from host task-list IDs.

## Command index

[cli.md](reference/cli.md) owns invocations, flags, outputs and errors; its [subcommand summary](reference/cli.md#subcommand-summary) is the command registry. Your role file sets your command authority.
