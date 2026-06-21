# CAFleet CLI — Fuller Command Catalog

Read this file for the broker CLI surface beyond the core identity / poll / send / ack lifecycle in [`SKILL.md`](../SKILL.md): environment variables, global options, coding-agent backends, the self-registration recipe, cancel / show / list / doctor / deregister / fleet delete, the typical bootstrap workflow, the message lifecycle, and error handling. Exhaustive per-subcommand flags, exit codes, and error strings live in [`cli-options.md`](../../../docs/spec/cli-options.md).

## Environment variables

CLI env vars (all `CAFLEET_`-prefixed): `CAFLEET_DATABASE_URL` (SQLite URL; default `~/.local/share/cafleet/cafleet.db`, use an absolute path when overriding — `~` is not expanded), `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` (`cafleet server` defaults `127.0.0.1` / `8000`), `CAFLEET_MAX_TEXT_LEN` (body-truncation limit, default `200` — see [`reference/output-flags.md`](output-flags.md)).

## Global Options

`--json` and `--version` are top-level options (precede the subcommand name); `--agent-id` and `--fleet-id` are per-subcommand options (after the subcommand name). Putting one in the wrong position fails with `No such option`.

```bash
cafleet --json agent list --fleet-id <fleet-id>
cafleet --json message poll --fleet-id <fleet-id> --agent-id <my-agent-id>
```

`cafleet --version` prints `cafleet <version>` and exits 0 without `--fleet-id`.

## Coding-agent backends

Three backends — `claude` (default), `codex`, `opencode` — chosen per member at create time via `--coding-agent`. `--model <m>` pins the LLM and `--role {member,monitor}` selects an ordinary vs the fleet's dedicated **monitoring member**; both flags, the model-name-to-backend inference, the per-backend available-model tables, and the spawn-argv detail live in [`reference/director.md`](director.md) (and [`roles/monitor.md`](../roles/monitor.md) plus [`reference/supervision.md`](supervision.md) for the monitor). All three honor the leading-`!` input shortcut, so `member exec` and inline previews work uniformly. Per-backend deltas: [`claude`](coding-agent/claude.md) / [`codex`](coding-agent/codex.md) / [`opencode`](coding-agent/opencode.md).

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

Default output is one row per agent (`<id> <name> <status>`, `description` truncated to 60 codepoints); `--full` gives the four-line per-agent block (the agent surfaces never carry `agent_card_json`). See [`reference/output-flags.md`](output-flags.md).

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

The root Director and the built-in Administrator cannot be deregistered (both exit 1 — see [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages)). Use `cafleet fleet delete <fleet-id>` for fleet teardown.

## Fleet Delete

```bash
cafleet fleet delete <fleet-id>
# → Deleted fleet <fleet-id>. Deregistered N agents.
```

Soft-deletes the fleet in one transaction (stamps `deleted_at`, deregisters every active agent, deletes placement rows; tasks preserved; idempotent). It does **not** close member panes — run `cafleet member delete` per member first, in the [`reference/recovery.md`](recovery.md) Shutdown order. Full behavior: [`cli-options.md`](../../../docs/spec/cli-options.md#fleet-delete).

## Typical Workflow

0. **Verify pane env** (Director): run `cafleet doctor` to confirm `TMUX` / `TMUX_PANE` are set — the canonical pane-identity probe, before `cafleet fleet create` and any `cafleet member create`.

1. **Create a fleet** (if none exists):
   ```bash
   cafleet fleet create --label "my-project"
   # text: line 1 <fleet-id>, line 2 <root-director-agent-id>; --json for the nested shape
   ```
   Must run inside a tmux session (else exits 1 with `Error: cafleet fleet create must be run inside a tmux session`, writes nothing).

2. **Register, discover, send, poll, ack** per the command sections above; use `cafleet --json …` when parsing output. Director-side spawn/capture/exec/ping: [`reference/director.md`](director.md); shutdown ordering: [`reference/recovery.md`](recovery.md).

## Message Lifecycle

Messages are tasks with three states: **input_required** (delivered, awaiting ACK) → **completed** (ACKed), or **canceled** (sender retracted before ACK). For broadcast threading (the `origin_task_id` self-reference shape), see [`reference/broadcast.md`](broadcast.md).

## Error Handling

Errors print to stderr and exit non-zero; `cafleet --json <cmd>` emits them machine-parseably. The most common: missing `--fleet-id` (exit 1), missing `--agent-id` (`Error: Missing option '--agent-id'.`, exit 2), and `member` commands outside a tmux session (exit 1). Full catalogue: [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages).
