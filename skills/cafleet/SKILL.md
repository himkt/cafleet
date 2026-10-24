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

Use the `cafleet` CLI to register as a member, send and receive messages, and discover other members on the CAFleet message broker. CLI commands access SQLite directly.

## Required reading

Open your authoritative role first. Use an available non-shell text reader for prerequisites; when shell is the only text reader, prerequisite file reads may precede ready. Ready is a member's first operational broker shell command and precedes task work.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](reference/coding-agents.md) | Resolve your Runtime bindings and bound notes before acting; use your `CODING AGENT:` identity, or your own identity when standalone. |
| 2 | [BASE contract](reference/base-dir.md) | Before scratch, audit or figure writes; members inherit their supplied BASE. |

Load skills through your executing backend's supported loader. Absolute-path loading for Codex/OpenCode includes this core, your backend section, each requested skill's core and your role-required references. An assigned workflow member loads its own role and prerequisites; loading the design-doc umbrella continues the existing workflow without creating a second team.

| Required read | Action boundary |
|---|---|
| [Member role](roles/member.md) | Ordinary-member startup and command authority. |
| [Director role](roles/director.md) and [Supervision](reference/supervision.md) | Director setup, before orchestration; the role gates backend and prompt reads. |
| [Command routing](reference/prompt-routing.md) | Before routing a command your harness denied, or running one a member routed to you. |
| [Recovery](reference/supervision.md#recovery) / [Shutdown](reference/supervision.md#shutdown) | Director reads the applicable section immediately before that action. |

Use the [command index](#command-index) on demand for exact runtime flags, outputs and errors.

## Resolve your overlay

You have read `reference/coding-agents.md` (Required-reading row #1). Resolve values by the subject of the action:

1. **Select the subject.** For your current instructions, use your `CODING AGENT:` identity (your own identity when standalone) and its Runtime bindings. For a member spawn, select the backend under [Director model-selection policy](roles/director.md#model-selection), then read that backend's Model catalog and Role defaults and validate target effort and launch capabilities against its Runtime bindings. For captured panes, use the observed member's recorded backend and its Pane-state capture cues. Keep the observer's own tools and decision surface.
2. **Materialize values.** Resolve the five runtime placeholders from Runtime bindings for the relevant operation, and the role's model and effort from the selected spawn backend's Role defaults.
3. **Apply notes.** At each instruction named in the selected backend's *Note → applies at* table, follow that note's caveat. An ordinary member resolves its own runtime section; it acquires no model-selection duty.
4. **Self-check at emission.** Emit concrete values in commands and messages. Resolve any remaining literal `{token}` before emitting it.

For example, a Codex Director spawning an OpenCode reviewer reads OpenCode Role defaults and effort capability while retaining Codex decision and execution tools. A Claude Director inspecting Codex applies the target backend’s capture cues.

### Documented defaults

Used only when your backend's section omits a token or your backend is unknown. Each default is the correct neutral-floor behavior — the form that functions on every backend — not a guess.

| Token | Documented default (section silent / backend unknown) |
|-------|-------------------------------------------------------|
| `{decision_surface}` | a Director-relayed operator message (a member always routes to the Director) |
| `{reviewer_model}` | the spawning Director's own model (inherit the parent) — a safe floor, possibly intelligence-suboptimal |
| `{reviewer_effort}` | omit `--effort` to use the backend default |
| `{other_model}` | the spawning Director's own model (inherit the parent) |
| `{other_effort}` | omit `--effort` to use the backend default |
| `{permission_flags}` | describe the mode neutrally as "workspace-scoped auto-approval" — for prose uses only; spawn-flag construction never falls here |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the skill's `SKILL.md` + your overlay by absolute path |
| `{effort_levels}` | unsupported — omit `--effort` |

## Required ids

Every `cafleet` invocation that touches members or messages names its **subject** as a positional integer id placed immediately after the subcommand name; ids that describe a relationship stay as flags:

- Positional `MEMBER_ID` — **the member in question**: the requester on `message poll`, and the target on `member delete` / `show` / `prompt` / `exec` / `capture`. The fleet is derived from the member row.
- Positional `MESSAGE_ID` — the message on `message ack` / `message show`; recipient and fleet are derived from the message row.
- Positional `FLEET_ID` — the fleet on `fleet show` / `fleet delete` / `member list` / `fleet capture`.
- `--from-member-id <int>` / `--to-member-id <int>` — the two parties of a two-party command: the **sender** and the **recipient** on `message send`. The fleet is derived from the sender row.
- `--fleet-id <int>` — only on `member create`: the fleet the new member joins (the subject of the command is the member being created; the Director is auto-resolved from the fleet row).

In the Director's own commands, substitute the literal ids printed by `cafleet fleet create` / `cafleet member create` — never your own exported shell variables. `permissions.allow` matches Bash invocations as fixed strings, so an ad-hoc `export MEMBER_ID=…; cafleet message poll $MEMBER_ID` breaks the match and forces prompts. See [`cli-options.md`](reference/runtime/spec/cli-options.md#positional-subject-ids) for the rationale and [`permissions.allow` coverage](reference/runtime/spec/cli-options.md#permissionsallow-coverage) for the pattern set.

### Spawned-member identity via `str.format` substitution

`cafleet member create` uses the Rust spawn-placeholder mini-formatter on the resolved spawn prompt (supplied as exactly one of the positional `PROMPT` or `--file <path>`), rendering exactly four placeholders to literals at spawn time. It accepts exact names and doubled literal braces only, not Python format specifications, conversions, or attribute/index access:

- `{fleet_id}` — the member's fleet id.
- `{member_id}` — the member's **own** newly-allocated id (the CLI allocates it during the spawn and substitutes it itself — the Director never needs to know it).
- `{director_member_id}` — the member's Director id.
- `{coding_agent}` — the resolved backend name (`claude` / `codex` / `opencode`).

An author writes the spawn prompt with those brace placeholders; after spawn the member reads its identity as literal labeled lines (e.g. `FLEET ID: 24`, `YOUR MEMBER ID: 88`). **Any literal brace in prompt text must be doubled** (`{{` / `}}`) to survive `.format()`; an unknown placeholder fails with `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` and a malformed brace expression with `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` — both exit 2. No identity environment variable is injected into the pane — the member takes the literal ids from its prompt and passes them explicitly: a poll is `cafleet message poll 88`; a self-attributed send is `cafleet message send --from-member-id 88 --to-member-id <director-member-id> "..."`.

The supported CLI environment variables are catalogued in [runtime Environment variables](reference/runtime/spec/cli-options.md#environment-variables).

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

Every one-shot `cafleet` process is the **only command in its shell-tool invocation**. Run a sequence of CAFleet operations as separate shell-tool calls. Do not place a one-shot CAFleet command beside another command using a newline, `;`, `&&`, a pipe, shell `&`, or any other setup/follow-up command — a compound invocation does not match the host's single-command allow rules.

Leading `NAME=value` assignments that set the environment of the CAFleet process are allowed; they do not start another process. They must immediately precede the CAFleet executable — do not substitute an `env` helper process or append another command. Shell redirection does not authorize another process either; a command that needs a long body uses the positional argument or `--file <path>`, not a pipe.

**Permission-error diagnostic.** A CAFleet command that fails with an operating-system permission error — `Operation not permitted` / `Permission denied`, commonly surfacing as a multiplexer socket or pane-command failure — signals that the invocation likely ran outside your coding agent's command auto-approval scope: a compound invocation does not match single-command allow rules, so the shell tool executes it under the agent's restricted sandbox or permission set. The response is to re-run the CAFleet command as its own isolated invocation. A message whose persisted id was already reported is committed (§ *Send (Unicast)*): send its body once only.

Command isolation complements the Director-side wake procedure in [`reference/supervision.md`](reference/supervision.md#director-wake).

## Send (Unicast)

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <target-member-id> "Assignment or reply"
```

Supply exactly one positional text body or `--file <path>`; `--file -` reads stdin. Use file input for long/multiline bodies. Text previews truncate by `CAFLEET_MAX_TEXT_LEN`; use `--json` for full payloads. [JSON](reference/runtime/spec/cli-options.md#json-output) returns the full record.

[Push notifications](reference/runtime/spec/multiplexer-backends.md#push-notifications) deliver every message Escape-first into its recipient's pane, the Director's included, as one `[cafleet msg ID from SENDER TIMESTAMP]` line carrying the full body or a `cafleet message show ID --json` pointer. The recipient ACKs that ID after acting on it. Holds and post-commit delivery errors preserve the committed ID and successful stdout record; consume that existing row. The next send to the recipient retries a pending notification. [Send outcomes](reference/runtime/spec/cli-options.md#message-send) define exact diagnostics.

Fan out with an individual send per selected recipient. Retain each committed ID and report a failed recipient explicitly.

## Poll (Check Inbox)

Returns only un-acked (`input_required`) deliveries addressed to this member, newest first; ACKing one drops it from `poll` output. The `id:` integer printed by `poll` is the cafleet message id — **distinct from** any harness task-list id (present only where your backend has a task list). `--json` emits the untruncated envelopes. Members poll after ready, before reporting completion and on every wake; the Director also polls on every wake. Handle queued assignments before becoming idle; a later send to the recipient retries a pending notification.

```bash
cafleet message poll <my-member-id> [--json]
```

## Acknowledge (ACK)

Moves a message from `input_required` to `completed`. The positional `MESSAGE_ID` names the delivery; recipient and fleet are derived from the message row.

```bash
cafleet message ack <message-id>
```



## Command index

Use the runtime owner for exact flags, outputs and errors. Ordinary members retain Send/Poll/Wait/ACK/show authority; Director actions follow [Director policy](roles/director.md).

| Need | Owner |
|---|---|
| Current command registry and subjects | [CLI summary](reference/runtime/spec/cli-options.md#subcommand-summary) |
| Full JSON/text outputs | [Output shapes](reference/runtime/spec/cli-options.md#output-shapes) |
| Scripted receive and retries | [Message wait](reference/runtime/spec/cli-options.md#message-wait) |
| Fleet/member creation and compensation | [Lifecycle contracts](reference/runtime/spec/cli-options.md#member-create) |
| Read-only roster pane snapshot | [Fleet capture](reference/runtime/spec/cli-options.md#fleet-capture) |
| Explicit native shell input | [Member exec](reference/runtime/spec/cli-options.md#member-exec), [routing](reference/prompt-routing.md) |
| Pane input, exclusion and previews | [Multiplexer contracts](reference/runtime/spec/multiplexer-backends.md) |
| Host continuation, native waiting and evidence boundaries | Selected backend's bound notes in [coding-agents.md](reference/coding-agents.md) |
| Setup, configuration and diagnosis | [CLI configuration](reference/runtime/spec/cli-options.md#cafleet-setup) |
| Recovery / teardown | [Recovery](reference/supervision.md#recovery) / [Shutdown](reference/supervision.md#shutdown) |
