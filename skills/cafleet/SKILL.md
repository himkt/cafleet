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

Open your authoritative role first. Use an available non-shell text reader for prerequisites; when shell is the only text reader, prerequisite file reads may precede ready. Ready is an ordinary member's first operational broker shell command and precedes task work. A monitor follows its distinct pre-launch backend read and startup sequence.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](reference/coding-agents.md) | Resolve your Runtime bindings and bound notes before acting; use your `CODING AGENT:` identity, or your own identity when standalone. |
| 2 | [BASE contract](reference/base-dir.md) | Before scratch, audit or figure writes; members inherit their supplied BASE. |

Load skills through your executing backend's supported loader. Absolute-path loading for Codex/OpenCode includes this core, your backend section, each requested skill's core and your role-required references. An assigned workflow member loads its own role and prerequisites; loading the design-doc umbrella continues the existing workflow without creating a second team.

| Required read | Action boundary |
|---|---|
| [Member role](roles/member.md) | Ordinary-member startup and command authority. |
| [Monitor role](roles/monitor.md) | Monitor startup, loop ownership and wake handling. |
| [Director role](roles/director.md) and [Supervision](reference/supervision.md) | Director setup, before orchestration; the role gates selected-backend and spawn-prompt reads. |
| [Denied-command routing](reference/prompt-routing.md) | Before routing or processing an actual denied-command request. |
| [Recovery](reference/supervision.md#recovery) / [Shutdown](reference/supervision.md#shutdown) | Director reads the applicable section immediately before that action. |
| [Broadcast](#broadcast) | Before fan-out or origin-message threading. |

Use the [command index](#command-index) on demand for exact runtime flags, outputs and errors.

## Resolve your overlay

You have read `reference/coding-agents.md` (Required-reading row #1). Resolve values by the subject of the action:

1. **Select the subject.** For your current instructions, use your `CODING AGENT:` identity (your own identity when standalone) and its Runtime bindings. For a member spawn, select the backend under [Director model-selection policy](roles/director.md#model-selection), then read that backend's Model catalog and Role defaults and validate target effort and launch capabilities against its Runtime bindings. For captured panes, use the observed member's recorded backend and its Pane-state capture cues. Keep the observer's own tools and decision surface.
2. **Materialize values.** Resolve the seven runtime placeholders from Runtime bindings for the relevant operation, and `{monitor_model}` / `{reviewer_model}` from the selected spawn backend's Role defaults. A Director's local long-lived work uses its own execution primitive. Monitor bootstrap and recovery inherit the Director's backend; reviewer selection may use a different backend. Use the documented neutral defaults below only for their explicitly allowed missing/unknown-backend cases. Report a missing required supported-backend section, malformed table, or broken reference as a documentation defect.
3. **Apply notes.** At each instruction named in the selected backend's *Note → applies at* table, follow that note's caveat. An ordinary member resolves its own runtime section; it acquires no model-selection duty.
4. **Self-check at emission.** Emit concrete values in commands and messages. Resolve any remaining literal `{token}` before emitting it.

For example, a Codex Director spawning an OpenCode reviewer reads OpenCode Role defaults and effort capability while retaining Codex decision and execution tools. A Claude monitor observing Codex applies Codex capture cues while its own loop uses Claude execution.

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

In the Director's own commands, substitute the literal ids printed by `cafleet fleet create` / `cafleet member create` — never your own exported shell variables. `permissions.allow` matches Bash invocations as fixed strings, so an ad-hoc `export MEMBER_ID=…; cafleet message poll $MEMBER_ID` breaks the match and forces prompts. See [`cli-options.md`](reference/runtime/spec/cli-options.md#positional-subject-ids) for the rationale and [`permissions.allow` coverage](reference/runtime/spec/cli-options.md#permissionsallow-coverage) for the pattern set.

### Spawned-member identity via `str.format` substitution

`cafleet member create` uses the Rust spawn-placeholder mini-formatter on the resolved spawn prompt (supplied as exactly one of the positional `PROMPT` or `--file <path>`), rendering exactly four placeholders to literals at spawn time. It accepts exact names and doubled literal braces only, not Python format specifications, conversions, or attribute/index access:

- `{fleet_id}` — the member's fleet id.
- `{member_id}` — the member's **own** newly-allocated id (the CLI allocates it during the spawn and substitutes it itself — the Director never needs to know it).
- `{director_member_id}` — the member's Director id.
- `{coding_agent}` — the resolved backend name (`claude` / `codex` / `opencode`).

An author writes the spawn prompt with those brace placeholders; after spawn the member reads its identity as literal labeled lines (e.g. `FLEET ID: 24`, `YOUR MEMBER ID: 88`). **Any literal brace in prompt text must be doubled** (`{{` / `}}`) to survive `.format()`; an unknown placeholder fails with `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` and a malformed brace expression with `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` — both exit 2. No identity environment variable is injected into the pane — the member takes the literal ids from its prompt and passes them explicitly: a poll is `cafleet message poll 88`; a self-attributed send is `cafleet message send --from-member-id 88 --to-member-id <director-member-id> "..."`.

CLI environment variables (the `CAFLEET_`-prefixed `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`, `CAFLEET_MAX_TEXT_LEN`) are catalogued in [runtime Environment variables](reference/runtime/spec/cli-options.md#environment-variables).

## Team supervision

The fleet's **monitor member** is spawned by the `cafleet fleet create` bootstrap itself, before any ordinary `cafleet member create`. At startup it launches the `cafleet monitor` wake loop in its own pane, confirms the loop's `monitor loop started` line, and sends the gate signal `monitor live` to the Director — the message that gates the first ordinary spawn (the CLI's monitor-first guard backstops it). On each wake it classifies the fleet's panes and contacts the Director only when something actually needs attention. A dead monitor is re-spawned mid-run with `cafleet member create --role monitor`.

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

## One-shot command isolation

Every one-shot `cafleet` process is the **only command in its shell-tool invocation**. Run a sequence of CAFleet operations as separate shell-tool calls. Do not place a one-shot CAFleet command beside another command using a newline, `;`, `&&`, a pipe, shell `&`, or any other setup/follow-up command — a compound invocation keeps your shell tool occupied after the CAFleet process exits, so your pane cannot consume an inbound inline preview while the extra command runs.

Leading `NAME=value` assignments that set the environment of the CAFleet process are allowed; they do not start another process. They must immediately precede the CAFleet executable — do not substitute an `env` helper process or append another command. Shell redirection does not authorize another process either; a command that needs a long body uses the positional argument or `--file <path>`, not a pipe.

**Permission-error diagnostic.** A CAFleet command that fails with an operating-system permission error — `Operation not permitted` / `Permission denied`, commonly surfacing as a multiplexer socket or pane-command failure — signals that the invocation likely ran outside your coding agent's command auto-approval scope: a compound invocation does not match single-command allow rules, so the shell tool executes it under the agent's restricted sandbox or permission set. The response is to re-run the CAFleet command as its own isolated invocation, honoring the no-resend rule (§ *Send (Unicast)*) whenever a persisted message id was already reported — never a compound retry.

The sole exception is the long-lived `cafleet monitor` process. Its invocation must still contain only that monitor process, but it may use exactly the background or managed-execution mechanism resolved by your coding-agent overlay — including OpenCode's shell `&` form and tool-managed background modes. The overlay owns that launch syntax; this rule does not duplicate lifecycle mechanics.

Command isolation complements, but does not replace, the Director-side dispatch boundary in [`reference/supervision.md`](reference/supervision.md) § *Asynchronous Wait Rule*: after dispatching work to a member, a Director ends or yields its turn and a notification resumes the workflow in a later turn.

## Send (Unicast)

```bash
cafleet message send --from-member-id <my-member-id> \
  --to-member-id <target-member-id> "Did the API schema change?"
```

`--to-member-id` (recipient id) is required, plus exactly one of the positional `TEXT` (inline body) or `--file <path>` (a UTF-8 file, or `-` for stdin — use it for long or multi-line bodies that would exceed the shell's `ARG_MAX`). The delivered body is truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` in the inline preview and text output. `--json` carries the complete untruncated body per [runtime JSON output](reference/runtime/spec/cli-options.md#json-output). After persisting, the broker keystrokes a 2-line inline preview into the recipient's pane — an `Esc`-safeguarded auto-fire the recipient consumes as a fresh user-turn (the same path serves `message broadcast`), caught on the next manual `message poll` or a Director `cafleet member ping` if missed; full mechanics in [`multiplexer-backends.md`](reference/runtime/spec/multiplexer-backends.md#push-notifications).

When `message send` exits nonzero while stating that `Message <id> was persisted`, preserve that ID and **do not resend** the body. The recipient consumes and ACKs the existing row through its own isolated `message poll` and `message ack` calls. Handle the notification failure within your role authority:

- An ordinary member reports the actual failure and persisted ID to the Director through `message send`; it acquires no pane-repair or ping authority.
- The Director diagnoses or re-engages the recipient through [Recovery](reference/supervision.md#recovery), observing the fresh-capture gate before a non-exempt ping. The monitor uses only its fixed-ping and reporting policy.
- If the report itself fails, preserve any persisted relay ID and report the actual observed failure through the remaining permitted surface. End the attempt without recursive duplicate failure reports. If broker routing is unavailable, state that concrete limitation; claim success only from observed results.

This consumes committed messages without retrying their notifications. Exact intentional-skip and failed-preview outcomes remain in the [runtime send contract](reference/runtime/spec/cli-options.md#message-send).

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

## Broadcast

Send to every active recipient in the sender's fleet except the sender. Supply the literal sender ID and exactly one inline `TEXT` or `--file <path>` body (UTF-8; `-` reads stdin). Use file input for long or multiline bodies.

```bash
cafleet message broadcast --from-member-id <my-member-id> "Build failed on main branch"
```

The broker persists one delivery per recipient plus a sender-side `broadcast_summary` row, and returns only the summary with `recipients` (real recipient count N) and `delivered` (panes reached by the preview). Failed previews can make these counts differ. Text output is:

```text
broadcast id=<id> recipients=<N> delivered=<k>
```

Use trailing `--json` for the full untruncated summary envelope and counts. Each recipient polls and ACKs its own delivery, as for unicast; recipients do not ACK the sender's summary. Preserve existing delivery IDs when handling missed previews under [Send](#send-unicast). A Director broadcasts only when every recipient passes the [fresh-capture gate](reference/supervision.md#the-pre-ping-capture-gate); otherwise defer the whole broadcast or use individually gated unicasts.

The exact `"Broadcast sent to N recipients"` summary, `origin_message_id` grouping and row schema are owned by [Broadcast grouping](reference/runtime/spec/data-model.md#broadcast-grouping) and the [message envelope](reference/runtime/spec/message-envelope.md); flags and outcomes are in [message broadcast](reference/runtime/spec/cli-options.md#message-broadcast).

## Command index

Use the offline runtime owners below for commands beyond Send/Poll/ACK. Command access remains role-specific; an on-demand link supplies syntax, not additional authority.

| Need | Complete owner |
|---|---|
| Subject IDs, every subcommand and JSON availability | [Subcommand summary](reference/runtime/spec/cli-options.md#subcommand-summary) |
| Global `--version`, trailing per-command `--json`, full untruncated JSON versus text truncation | [Global options](reference/runtime/spec/cli-options.md#global-options), [JSON output](reference/runtime/spec/cli-options.md#json-output), [message-body truncation](reference/runtime/spec/cli-options.md#message-body-truncation) |
| Database/server/output environment defaults | [Environment variables](reference/runtime/spec/cli-options.md#environment-variables) |
| One message by positional MESSAGE_ID; full body with `--json` | [Message show](reference/runtime/spec/cli-options.md#message-show) and [envelopes](reference/runtime/spec/message-envelope.md) |
| Active registry entries, detailed member view, placement/pending/idle and timestamps | [Member list](reference/runtime/spec/cli-options.md#member-list), [member show](reference/runtime/spec/cli-options.md#member-show), [output shapes](reference/runtime/spec/cli-options.md#output-shapes) |
| Doctor's complete multiplexer/database/agent diagnosis and gating exit | [Doctor](reference/runtime/spec/cli-options.md#cafleet-doctor) |
| Backend selection, inherited/default/pinned models and effort | [Director model selection](roles/director.md#model-selection), [backend catalog](reference/coding-agents.md), [member create](reference/runtime/spec/cli-options.md#member-create) |
| Monitor loop interval/tick/startup, scan/capture and pending-ping output | [Monitor](reference/runtime/spec/cli-options.md#cafleet-monitor), [Director actions](roles/director.md#fleet-scan), [monitor role](roles/monitor.md) |
| Member deletion, pending/placementless registration and root guard | [Member delete](reference/runtime/spec/cli-options.md#member-delete) |
| Fleet transaction/idempotence/message retention and pane teardown distinction | [Fleet delete](reference/runtime/spec/cli-options.md#fleet-delete); read [Shutdown](reference/supervision.md#shutdown) immediately before teardown |
| Bootstrap, doctor → monitor ready/live → per-member ready, dispatch and closure | [Supervision](reference/supervision.md#spawn-protocol), [manual lifecycle](reference/runtime/how-to/mixed-backend-team.md#manual-lifecycle) |
| ACK state transitions | [Persistence contracts](reference/runtime/spec/data-model.md#query-and-activity-contracts) |
| Exact error strings and exit codes | [Error messages](reference/runtime/spec/cli-options.md#error-messages); application errors remain text on stderr even with `--json` |
