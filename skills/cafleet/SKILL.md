---
description: "Interact with the CAFleet message broker. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents."
---

# CAFleet — Message Broker CLI

Use the `cafleet` CLI to register as an agent, send and receive messages, and discover other agents on the CAFleet message broker. CLI commands access SQLite directly — no running server is required.

## Reference files

This file (the core) covers the identity / poll / send / ack / cancel / show lifecycle every agent uses. Director-only flows, broadcast semantics, the bash-via-Director fallback, recovery, and the `--full` opt-back-in live in dedicated reference files. Read on demand:

- For Director-only commands (`member create` / `delete` / `list --activity` / `capture` / `exec` / `ping`), Read [`reference/director.md`](reference/director.md).
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

CLI env vars (all `CAFLEET_`-prefixed): `CAFLEET_DATABASE_URL` (SQLite URL; default `~/.local/share/cafleet/cafleet.db`, use an absolute path when overriding — `~` is not expanded), `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` (`cafleet server` defaults `127.0.0.1` / `8000`), `CAFLEET_MAX_TEXT_LEN` (body-truncation limit, default `200` — see [`reference/output-flags.md`](reference/output-flags.md)).

## Placeholder convention

In every example, substitute the literal integer ids printed by `cafleet fleet create` / `cafleet agent register`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<fleet-id>` — the fleet id printed by `cafleet fleet create`
- `<my-agent-id>` — the id returned by your own `cafleet agent register` call
- `<director-agent-id>` — the Director's id (in your spawn prompt if you are a member)
- `<target-agent-id>` — the recipient of a unicast message
- `<task-id>` — the task id printed by `message poll` / `message send`

Every id input (`--fleet-id`, `--agent-id`, `--to`, `--id`, `--member-id`, `--task-id`) is a DB-assigned integer (typically 1–4 digits), passed in full — no prefix resolution. A non-integer fails with Click's standard not-a-valid-integer error (exit 2).

## Global Options

`--json` and `--version` are top-level options (precede the subcommand name); `--agent-id` and `--fleet-id` are per-subcommand options (after the subcommand name). Putting one in the wrong position fails with `No such option`.

```bash
cafleet --json agent list --fleet-id <fleet-id>
cafleet --json message poll --fleet-id <fleet-id> --agent-id <my-agent-id>
```

`cafleet --version` prints `cafleet <version>` and exits 0 without `--fleet-id`.

## Coding-agent backends

Three backends — `claude` (default), `codex`, `opencode` — chosen per member at create time via `--coding-agent`. `--model <m>` pins the LLM and `--role {member,monitor}` selects an ordinary vs the fleet's dedicated **monitoring member**; both flags, the model-name-to-backend inference, the per-backend available-model tables, and the spawn-argv detail live in [`reference/director.md`](reference/director.md) (and the `cafleet-agent-team-monitoring` skill for the monitor). All three honor the leading-`!` input shortcut, so `member exec` and inline previews work uniformly. Per-backend deltas: [`claude`](reference/coding-agent/claude.md) / [`codex`](reference/coding-agent/codex.md) / [`opencode`](reference/coding-agent/opencode.md).

## Soliciting user reactions

When you need a recorded user reaction — **approve**, **choose among options**, **confirm**, or **continue-or-abort** — solicit it through {decision_surface}, never in free-form prose ("let me know if this looks good", "shall I proceed?", "reply with your choice") which records no answer and routinely stalls. A fleet **member** never talks to the user: it sends its question to the Director via `cafleet message send`, and the Director relays it. See your overlay for the question shapes and any surface constraints.

## Self-registration recipe

Use `--json` so the output is machine-parseable, and capture `agent_id` for every subsequent call:

```bash
cafleet --json agent register --fleet-id <fleet-id> \
  --name "<short-label>" \
  --description "<one-sentence purpose>"
# → {"agent_id":<id>,"name":"<short-label>","registered_at":"<iso8601>"}
```

- **Name**: short, human-identifiable label (`Claude-A`, `reviewer-bot`, …) — not `test`, `foo`, etc.
- **Description**: one sentence stating who the agent is and what it is for.
- **Capture `agent_id` immediately** — it is required for every subsequent call; losing it forces re-registration. Non-`--json` output prints `Agent registered successfully!` then `  agent_id:  <id>` / `  name:      <name>` (parse the `agent_id:` line).
- Call `cafleet agent deregister --fleet-id <fleet-id> --agent-id <my-agent-id>` at end of fleet so stale registrations do not accumulate.

> **Reserved name — `Administrator`**: every fleet is auto-seeded with one built-in `Administrator` (marked `agent_card_json.cafleet.kind == "builtin-administrator"`, protected against deregister and Director placement). Do NOT register an agent under that name.

## Send (Unicast)

```bash
cafleet message send --fleet-id <fleet-id> --agent-id <my-agent-id> \
  --to <target-agent-id> --text "Did the API schema change?"
```

`--to` (recipient id) and `--text` (body, truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` by default) are required; `--full` / `--quiet` per [`reference/output-flags.md`](reference/output-flags.md). After persisting, the broker keystrokes a 2-line inline preview into the recipient's pane:

```
[cafleet msg <task_id> from <sender_id> <ts>]
<text-truncated-to-CAFLEET_MAX_TEXT_LEN>
```

The preview **leads with `Esc`** (settles ~0.1 s, then types the payload + `Enter`), so a recipient parked on a pending permission-approval prompt has it dismissed before the trailing `Enter` lands — the same `Esc`-safeguarded path serves `message send` / `message broadcast` / `member nudge`. The recipient processes the keystroke as a fresh user-turn input (no `message poll` in the auto-fire path) and acks once consumed; a missed preview is caught on the next manual `message poll` or a Director `cafleet member ping`. Mechanics: [`tmux-push.md`](../../docs/concepts/tmux-push.md).

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

## Cancel (Retract)

Retract a sent message that has not been acknowledged yet (sender-only). `--task-id` required.

```bash
cafleet message cancel --fleet-id <fleet-id> --agent-id <my-agent-id> --task-id <task-id>
```

## Show (Get Task)

Fetch one task by id. `--task-id` required; `--full` for the untruncated envelope.

```bash
cafleet message show --fleet-id <fleet-id> --agent-id <my-agent-id> --task-id <task-id>
```

## List Agents

`agent list` returns all registered agents; `agent show --id <target-agent-id>` fetches one.

```bash
cafleet agent list --fleet-id <fleet-id>
cafleet agent show --fleet-id <fleet-id> --agent-id <my-agent-id> --id <target-agent-id>
```

Default output is one row per agent (`<id> <name> <status>`, `description` truncated to 60 codepoints); `--full` gives the four-line per-agent block (the agent surfaces never carry `agent_card_json`). See [`reference/output-flags.md`](reference/output-flags.md).

## Doctor

Print the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for diagnosing placement without raw tmux. Does NOT require `--fleet-id`; requires `TMUX` and `TMUX_PANE` to be set.

```bash
cafleet doctor
cafleet --json doctor
```

## Deregister

```bash
cafleet agent deregister --fleet-id <fleet-id> --agent-id <my-agent-id>
```

The root Director and the built-in Administrator cannot be deregistered (both exit 1 — see [`cli-options.md`](../../docs/spec/cli-options.md#error-messages)). Use `cafleet fleet delete <fleet-id>` for fleet teardown.

## Fleet Delete

```bash
cafleet fleet delete <fleet-id>
# → Deleted fleet <fleet-id>. Deregistered N agents.
```

Soft-deletes the fleet in one transaction (stamps `deleted_at`, deregisters every active agent, deletes placement rows; tasks preserved; idempotent). It does **not** close member panes — run `cafleet member delete` per member first, in the [`reference/recovery.md`](reference/recovery.md) Shutdown order. Full behavior: [`cli-options.md`](../../docs/spec/cli-options.md#fleet-delete).

## Typical Workflow

0. **Verify pane env** (Director): run `cafleet doctor` to confirm `TMUX` / `TMUX_PANE` are set — the canonical pane-identity probe, before `cafleet fleet create` and any `cafleet member create`.

1. **Create a fleet** (if none exists):
   ```bash
   cafleet fleet create --label "my-project"
   # text: line 1 <fleet-id>, line 2 <root-director-agent-id>; --json for the nested shape
   ```
   Must run inside a tmux session (else exits 1 with `Error: cafleet fleet create must be run inside a tmux session`, writes nothing).

2. **Register, discover, send, poll, ack** per the command sections above; use `cafleet --json …` when parsing output. Director-side spawn/capture/exec/ping: [`reference/director.md`](reference/director.md); shutdown ordering: [`reference/recovery.md`](reference/recovery.md).

## Message Lifecycle

Messages are tasks with three states: **input_required** (delivered, awaiting ACK) → **completed** (ACKed), or **canceled** (sender retracted before ACK). For broadcast threading (the `origin_task_id` self-reference shape), see [`reference/broadcast.md`](reference/broadcast.md).

## Error Handling

Errors print to stderr and exit non-zero; `cafleet --json <cmd>` emits them machine-parseably. The most common: missing `--fleet-id` (exit 1), missing `--agent-id` (`Error: Missing option '--agent-id'.`, exit 2), and `member` commands outside a tmux session (exit 1). Full catalogue: [`cli-options.md`](../../docs/spec/cli-options.md#error-messages).
