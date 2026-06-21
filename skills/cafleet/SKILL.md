---
name: cafleet
description: >-
  Interact with the CAFleet message broker and supervise CAFleet agent teams.
  Use when an agent needs to register, send/receive messages, poll inbox,
  acknowledge messages, or discover other agents; or when a Director is about to
  spawn, monitor, health-check, or recover a stalled team of CAFleet members
  (any `cafleet member create`), which requires the dedicated monitoring member,
  the heartbeat, and the supervision governance.
---

# CAFleet — Message Broker CLI

Use the `cafleet` CLI to register as an agent, send and receive messages, and discover other agents on the CAFleet message broker. CLI commands access SQLite directly — no running server is required.

## Reference files

This file (the core) covers the identity / poll / send / ack lifecycle every agent uses. Director-only governance and the heartbeat, the fuller CLI catalog, base-dir resolution, broadcast semantics, the bash-via-Director fallback, recovery, and the `--full` opt-back-in live in dedicated reference files. Read on demand:

- For Director-only team supervision — governance + the `cafleet monitor` heartbeat (Core Principle, Authorization-Scope Guard, Spawn Protocol, Stall Response, Monitor Lifecycle, the 5-step facilitation loop, Cleanup), Read [`reference/supervision.md`](reference/supervision.md).
- For Director-only commands (`member create` / `delete` / `list --activity` / `capture` / `exec` / `ping`), Read [`reference/director.md`](reference/director.md).
- For the fuller CLI catalog beyond send/poll/ack — self-registration, global options, coding-agent backends, `message cancel` / `show`, `agent list` / `show`, `doctor`, `agent deregister`, `fleet delete`, the bootstrap workflow, message lifecycle, and error handling, Read [`reference/cli.md`](reference/cli.md).
- For the base-directory resolution procedure and the no-bypass write protocol, Read [`reference/base-dir.md`](reference/base-dir.md).
- For broadcast send/ack and threading via `origin_task_id`, Read [`reference/broadcast.md`](reference/broadcast.md).
- For the bash-via-Director fallback protocol, Read [`reference/exec-routing.md`](reference/exec-routing.md).
- For crash / disconnect / idle / wedged-pane recovery decision trees AND the Shutdown Protocol, Read [`reference/recovery.md`](reference/recovery.md).
- For `--full` / `--json` / `--quiet` opt-back-in semantics and `CAFLEET_MAX_TEXT_LEN`, Read [`reference/output-flags.md`](reference/output-flags.md).

Exhaustive per-subcommand flags, exit codes, and error strings live in [`docs/spec/cli-options.md`](../../docs/spec/cli-options.md).

## Apply your coding-agent overlay

CAFleet instructions are backend-neutral, written with `{placeholder}` tokens for everything that varies by coding agent (`{monitor_model}`, `{permission_flags}`, `{decision_surface}`, and the rest). Your overlay — `reference/coding-agent/<name>.md` — is a value table that defines each token for your backend. Identify your coding agent — your spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity — then read your overlay and, as you read the base, substitute your overlay's value for each `{placeholder}` you encounter.

## Required Flags

Every `cafleet` invocation that touches agents or messages carries two literal integer ids (no env-var fallback):

- `--fleet-id <int>` — per-subcommand (placed **after** the subcommand name), required on every client + member subcommand. Rejected with `No such option` on `db init` / `fleet *` / `server` / `doctor`. Missing it exits with `Error: --fleet-id <int> is required for this subcommand. …`.
- `--agent-id <int>` — per-subcommand, required on every subcommand **except** `register` (which returns the new `agent_id` to record and reuse).

Use literal ids, never shell variables — `permissions.allow` matches Bash invocations as fixed strings, so `$VAR` expansion breaks the match and forces prompts. See [`cli-options.md`](../../docs/spec/cli-options.md#fleet-id) for the rationale and [`permissions.allow` coverage](../../docs/spec/cli-options.md#permissionsallow-coverage) for the pattern set.

CLI environment variables (the `CAFLEET_`-prefixed `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`, `CAFLEET_MAX_TEXT_LEN`) are catalogued in [`reference/cli.md`](reference/cli.md) § Environment variables.

## Team supervision

When a Director spawns a team, the **FIRST** member created is the dedicated monitoring member (`cafleet member create --role monitor --model {monitor_model}`). It owns the heartbeat and gates every ordinary `member create` behind its `ready: monitor live` handshake. The Director never runs `cafleet monitor start` itself.

For the full governance + heartbeat mechanism (Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, Stall Response, Cleanup, the 5-step facilitation loop, Monitor Lifecycle), Read [`reference/supervision.md`](reference/supervision.md).

For the monitoring member's own role definition (startup, on-wake routine, teardown), Read [`roles/monitor.md`](roles/monitor.md).

## Placeholder convention

In every example, substitute the literal integer ids printed by `cafleet fleet create` / `cafleet agent register`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<fleet-id>` — the fleet id printed by `cafleet fleet create`
- `<my-agent-id>` — the id returned by your own `cafleet agent register` call
- `<director-agent-id>` — the Director's id (in your spawn prompt if you are a member)
- `<target-agent-id>` — the recipient of a unicast message
- `<task-id>` — the task id printed by `message poll` / `message send`

Every id input (`--fleet-id`, `--agent-id`, `--to`, `--id`, `--member-id`, `--task-id`) is a DB-assigned integer (typically 1–4 digits), passed in full — no prefix resolution. A non-integer fails with Click's standard not-a-valid-integer error (exit 2).

## Soliciting user reactions

When you need a recorded user reaction — **approve**, **choose among options**, **confirm**, or **continue-or-abort** — solicit it through {decision_surface}, never in free-form prose ("let me know if this looks good", "shall I proceed?", "reply with your choice") which records no answer and routinely stalls. A fleet **member** never talks to the user: it sends its question to the Director via `cafleet message send`, and the Director relays it. See your overlay for the question shapes and any surface constraints.

## Send (Unicast)

```bash
cafleet message send --fleet-id <fleet-id> --agent-id <my-agent-id> \
  --to <target-agent-id> --text "Did the API schema change?"
```

`--to` (recipient id) and `--text` (body, truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` by default) are required; `--full` / `--quiet` per [`reference/output-flags.md`](reference/output-flags.md). After persisting, the broker keystrokes a 2-line inline preview into the recipient's pane — an `Esc`-safeguarded auto-fire the recipient consumes as a fresh user-turn (the same path serves `message broadcast` / `member nudge`), caught on the next manual `message poll` or a Director `cafleet member ping` if missed; full mechanics in [`tmux-push.md`](../../docs/concepts/tmux-push.md).

## Poll (Check Inbox)

Returns only un-acked (`input_required`) deliveries addressed to this agent, newest first; ACKing one drops it from `poll` output. `--full` emits the untruncated typed-column envelope.

```bash
cafleet message poll --fleet-id <fleet-id> --agent-id <my-agent-id> [--full]
```

## Acknowledge (ACK)

Moves a task from `input_required` to `completed`. `--task-id` required; `--quiet` emits only the acked id.

```bash
cafleet message ack --fleet-id <fleet-id> --agent-id <my-agent-id> --task-id <task-id>
```
