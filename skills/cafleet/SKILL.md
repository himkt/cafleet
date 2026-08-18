---
name: cafleet
description: >-
  Interact with the CAFleet message broker and supervise CAFleet member teams.
  Use when an agent needs to register as a member, send/receive messages, poll
  inbox, acknowledge messages, or discover other members; or when a Director is
  about to spawn, monitor, health-check, or recover a stalled team of CAFleet
  members (any `cafleet member create`), which requires the monitor-member
  heartbeat and the supervision governance.
---

# CAFleet — Message Broker CLI

Use the `cafleet` CLI to register as a member, send and receive messages, and discover other members on the CAFleet message broker. CLI commands access SQLite directly — no running server is required.

## Required reading

Before your first action other than these Reads, Read every file in the **Load-bearing** table below, in order (row #2 applies only if you write files). Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity. After reading the overlays file, **resolve** only your backend's section before acting — see *Resolve your overlay* below.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay section [`reference/coding-agent-overlays.md#<name>`](reference/coding-agent-overlays.md) — read **and resolve** it (see *Resolve your overlay*) | unresolved `{token}`s, guessed values, ignored backend notes |
| 2 | [`reference/base-dir.md`](reference/base-dir.md) — if you write any scratch / audit / figure file | the no-bypass write protocol and the `<unset>` contract — you mis-root every write or fall back to `/tmp` |

**Load-bearing on trigger — Read at the named moment, before that action:**

| Read | Read before you… | What you lose if you skip it |
|------|------------------|------------------------------|
| [`reference/prompt-routing.md`](reference/prompt-routing.md) | route a Bash-denied command to the Director | the dispatch shape — you stall or fabricate command output |
| [`reference/recovery.md`](reference/recovery.md) | tear down or recover a member / fleet (also the Shutdown Protocol) | the first-out teardown order — you orphan panes / leak the fleet |
| [`reference/cli.md`](reference/cli.md) § *Broadcast* | broadcast to the fleet or thread via `origin_message_id` | the broadcast send/ack semantics — your fan-out misfires or double-acks |

**On-demand — Read only when you need that capability:**

| Read | When |
|------|------|
| [`reference/cli.md`](reference/cli.md) | you need a CLI subcommand beyond send/poll/ack — global options, the `--json` output switch (`CAFLEET_MAX_TEXT_LEN` text truncation), coding-agent backends, `message show` / `broadcast`, `member show` / `member list`, `doctor`, `fleet delete`, the typical workflow |

Director-only governance — [`reference/supervision.md`](reference/supervision.md) (governance + the `cafleet monitor` heartbeat) and [`reference/director.md`](reference/director.md) (`member create` / `member delete` / `member list` / `member capture` / `member prompt` / `member ping`) — is load-bearing for a Director; its gated Required-reading block lives in [`roles/director.md`](roles/director.md), not on this dispatch surface.

Exhaustive per-subcommand flags, exit codes, and error strings live in [`docs/docs/spec/cli-options.md`](../../docs/docs/spec/cli-options.md).

## Resolve your overlay

You have read `reference/coding-agent-overlays.md` (Required-reading row #1). Before your first action, resolve **only your backend's section** of it:

1. **Materialize values.** For every `{placeholder}` token you will use this session, take the concrete value from your backend section's table and use that literal value — never the brace token. Resolution order for each token: (i) your backend section's value; (ii) the documented default below, only if your section omits the token or you cannot identify your backend. Never a literal `{token}`, never an ad-hoc guess — and never a value from another backend's section, which is a resolution defect of the same class as emitting a literal `{token}`.
2. **Apply notes.** When you reach a base instruction named in your backend section's *Note → applies at* table, follow that note's caveat there (e.g. on codex, coordinate via cafleet messages, not a harness task list; on opencode, treat a permission popup as a regression to escalate, not a decision point).
3. **Self-check at emission.** A literal `{token}` in any command you run, any message you send, or anything you show the user is a defect — stop and resolve it before emitting.

These steps close the three failure modes of an unresolved overlay: a literal `{token}` emitted in output, a wrong or guessed value acted on, and a backend note ignored.

### Documented defaults

Used only when your backend's section omits a token or your backend is unknown. Each default is the correct neutral-floor behavior — the form that functions on every backend — not a guess.

| Token | Documented default (section silent / backend unknown) |
|-------|-------------------------------------------------------|
| `{decision_surface}` | a Director-relayed operator message (a member always routes to the Director) |
| `{reviewer_model}` | the spawning Director's own model (inherit the parent) — a safe floor, possibly intelligence-suboptimal |
| `{monitor_model}` | the spawning Director's own model (inherit the parent) — a safe floor, possibly cost-suboptimal |
| `{permission_flags}` | describe the mode neutrally as "workspace-scoped auto-approval" — for prose uses only; spawn-flag construction never falls here |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{task_coord}` | cafleet messages |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the skill's `SKILL.md` + your overlay by absolute path |
| `{effort_levels}` | unsupported — omit `--effort` |

## Required ids

Every `cafleet` invocation that touches members or messages names its **subject** as a positional integer id placed immediately after the subcommand name; ids that describe a relationship stay as flags:

- Positional `MEMBER_ID` — **the member in question**: the requester on `message poll`, and the target on `member delete` / `show` / `prompt` / `ping` / `capture`. The fleet is derived from the member row.
- Positional `MESSAGE_ID` — the message on `message ack` / `message show`; recipient and fleet are derived from the message row.
- Positional `FLEET_ID` — the fleet on `fleet show` / `fleet delete` / `member list` / `monitor`.
- `--from-member-id <int>` / `--to-member-id <int>` — the two parties of a two-party command: the **sender** and the **recipient** on `message send`; `message broadcast` takes the sender only. The fleet is derived from the sender row.
- `--fleet-id <int>` — only on `member create`: the fleet the new member joins (the subject of the command is the member being created; the Director is auto-resolved from the fleet row).

In the Director's own commands, substitute the literal ids printed by `cafleet fleet create` / `cafleet member create` — never your own exported shell variables. `permissions.allow` matches Bash invocations as fixed strings, so an ad-hoc `export MEMBER_ID=…; cafleet message poll $MEMBER_ID` breaks the match and forces prompts. See [`cli-options.md`](../../docs/docs/spec/cli-options.md#positional-subject-ids) for the rationale and [`permissions.allow` coverage](../../docs/docs/spec/cli-options.md#permissionsallow-coverage) for the pattern set.

### Spawned-member identity via `str.format` substitution

`cafleet member create` runs `str.format` over the resolved spawn prompt (supplied as exactly one of the positional `PROMPT` or `--file <path>`), rendering exactly four placeholders to literals at spawn time:

- `{fleet_id}` — the member's fleet id.
- `{member_id}` — the member's **own** newly-allocated id (the CLI allocates it during the spawn and substitutes it itself — the Director never needs to know it).
- `{director_member_id}` — the member's Director id.
- `{coding_agent}` — the resolved backend name (`claude` / `codex` / `opencode`).

An author writes the spawn prompt with those brace placeholders; after spawn the member reads its identity as literal labeled lines (e.g. `FLEET ID: 24`, `YOUR MEMBER ID: 88`). **Any literal brace in prompt text must be doubled** (`{{` / `}}`) to survive `.format()`; an unknown placeholder fails with `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` and a malformed brace expression with `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` — both exit 2. No identity environment variable is injected into the pane — the member takes the literal ids from its prompt and passes them explicitly: a poll is `cafleet message poll 88`; a self-attributed send is `cafleet message send --from-member-id 88 --to-member-id <director-member-id> "..."`.

CLI environment variables (the `CAFLEET_`-prefixed `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`, `CAFLEET_MAX_TEXT_LEN`) are catalogued in [`reference/cli.md`](reference/cli.md) § Environment variables.

## Team supervision

The fleet's **monitor member** is spawned by the `cafleet fleet create` bootstrap itself, before any ordinary `cafleet member create`. At startup it launches the `cafleet monitor` wake loop in its own pane and sends the gate signal `monitor live` to the Director — the message that gates the first ordinary spawn (the CLI's monitor-first guard backstops it). On each wake it classifies the fleet's panes and contacts the Director only when something actually needs attention.

For the full governance + heartbeat mechanism, Read [`reference/supervision.md`](reference/supervision.md); the monitor member's own protocol is [`roles/monitor.md`](roles/monitor.md).

## Placeholder convention

In every example, substitute the literal integer ids printed by `cafleet fleet create` / `cafleet member create`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<fleet-id>` — the fleet id printed by `cafleet fleet create`
- `<my-member-id>` — your own id, read from the literal `YOUR MEMBER ID:` line in your spawn prompt
- `<director-member-id>` — the Director's id (in your spawn prompt if you are a member)
- `<target-member-id>` — the recipient of a unicast message
- `<message-id>` — the message id printed by `message poll` / `message send`

Every id input (the positional `FLEET_ID` / `MEMBER_ID` / `MESSAGE_ID` subjects, `--from-member-id`, `--to-member-id`, `member create`'s `--fleet-id`) is a DB-assigned integer (typically 1–4 digits), passed in full — no prefix resolution. A non-integer fails with the parser's invalid-value error (exit 2).

## Soliciting user reactions

When you need a recorded user reaction — **approve**, **choose among options**, **confirm**, or **continue-or-abort** — solicit it through {decision_surface}, never in free-form prose ("let me know if this looks good", "shall I proceed?", "reply with your choice") which records no answer and routinely stalls. A fleet **member** never talks to the user: it sends its question to the Director via `cafleet message send`, and the Director relays it. See your overlay for the question shapes and any surface constraints.

## Send (Unicast)

```bash
cafleet message send --from-member-id <my-member-id> \
  --to-member-id <target-member-id> "Did the API schema change?"
```

`--to-member-id` (recipient id) is required, plus exactly one of the positional `TEXT` (inline body) or `--file <path>` (a UTF-8 file, or `-` for stdin — use it for long or multi-line bodies that would exceed the shell's `ARG_MAX`). The delivered body is truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` in the inline preview and text output. `--json` carries the complete untruncated body per [`reference/cli.md`](reference/cli.md) § *Output switch*. After persisting, the broker keystrokes a 2-line inline preview into the recipient's pane — an `Esc`-safeguarded auto-fire the recipient consumes as a fresh user-turn (the same path serves `message broadcast`), caught on the next manual `message poll` or a Director `cafleet member ping` if missed; full mechanics in [`multiplexer-backends.md`](../../docs/docs/spec/multiplexer-backends.md#push-notifications).

## Poll (Check Inbox)

Returns only un-acked (`input_required`) deliveries addressed to this member, newest first; ACKing one drops it from `poll` output. The `id:` integer printed by `poll` is the cafleet message id — **distinct from** any harness task-list id (present only where your backend has a task list). `--json` emits the untruncated envelopes. Poll is an on-demand inbox check — run it on wake or when you have a reason to check now, never on a self-scheduled `sleep`-timer loop; the broker re-opens your turn when work arrives.

```bash
cafleet message poll <my-member-id> [--json]
```

## Acknowledge (ACK)

Moves a message from `input_required` to `completed`. The positional `MESSAGE_ID` names the delivery; recipient and fleet are derived from the message row.

```bash
cafleet message ack <message-id>
```
