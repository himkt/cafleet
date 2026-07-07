# CAFleet CLI — Fuller Command Catalog

Read this file for the broker CLI surface beyond the core identity / poll / send / ack lifecycle in [`SKILL.md`](../SKILL.md): environment variables, global options, coding-agent backends, cancel / show / roster introspection / doctor / fleet delete, the typical workflow, the message lifecycle, and error handling. Exhaustive per-subcommand flags, exit codes, and error strings live in [`cli-options.md`](../../../docs/spec/cli-options.md).

## Environment variables

CLI env vars (all `CAFLEET_`-prefixed): `CAFLEET_DATABASE_URL` (SQLite URL; default `~/.local/share/cafleet/cafleet.db`, use an absolute path when overriding — `~` is not expanded), `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` (`cafleet server` defaults `127.0.0.1` / `8000`), `CAFLEET_MAX_TEXT_LEN` (body-truncation limit, default `200` — see [`reference/output-flags.md`](output-flags.md)).

## Global Options

`--json` and `--version` are top-level options (precede the subcommand name); `--agent-id` and `--fleet-id` are per-subcommand options (after the subcommand name). Putting one in the wrong position fails with `No such option`.

```bash
cafleet --json member list --fleet-id <fleet-id> --all
cafleet --json message poll --fleet-id <fleet-id> --agent-id <my-agent-id>
```

`cafleet --version` prints `cafleet <version>` and exits 0 without `--fleet-id`.

## Coding-agent backends

Three backends — `claude` (default), `codex`, `opencode` — chosen per member at `member create` time via `--coding-agent`. `--model <m>` pins the LLM and `--role {member,monitor}` selects an ordinary vs the fleet's dedicated **monitoring member**; both flags, the model-name-to-backend inference, the per-backend available-model tables, and the spawn-argv detail live in [`reference/director.md`](director.md) (and [`roles/monitor.md`](../roles/monitor.md) plus [`reference/supervision.md`](supervision.md) for the monitor). All three honor the leading-`!` input shortcut, so `member exec` and inline previews work uniformly. Per-backend deltas: [`claude`](coding-agent/claude.md) / [`codex`](coding-agent/codex.md) / [`opencode`](coding-agent/opencode.md).

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

`member list --all` returns every active agent of the fleet (root Director, Administrator, monitoring member, ordinary members, placementless rows); `member show --member-id <target-agent-id>` fetches one. Both are registry reads — no tmux required.

```bash
cafleet member list --fleet-id <fleet-id> --all
cafleet member show --fleet-id <fleet-id> --member-id <target-agent-id>
```

`member list --all` renders the `N agents:` table with a `kind` column (`director` / `administrator` / `monitor` / `member`) and `-` placement cells for placementless rows. `member show` default output is the one-line row `<id> <name> <status>`; `--full` gives the labeled block (`description` truncated to 60 codepoints, `kind`, `skills`, placement sub-block) — text mode only. See [`reference/output-flags.md`](output-flags.md).

## Doctor

Print the resolved multiplexer backend and the calling pane's session/window/pane identifiers for diagnosing placement without raw multiplexer commands. Does NOT require `--fleet-id`; requires a supported multiplexer to be detected (tmux or herdr).

```bash
cafleet doctor
cafleet --json doctor
```

## Deregister

`cafleet member delete` is the single teardown for any agent: it closes the pane when one exists and soft-deletes the registration; a placementless or pending-placement target is a pure registry soft-delete (no tmux required).

```bash
cafleet member delete --fleet-id <fleet-id> --member-id <target-agent-id>
```

The root Director and the built-in Administrator cannot be deregistered (both exit 1 — see [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages)). Use `cafleet fleet delete --fleet-id <fleet-id>` for fleet teardown.

## Fleet Delete

```bash
cafleet fleet delete --fleet-id <fleet-id>
# → Deleted fleet <fleet-id>. Deregistered N agents.
```

Soft-deletes the fleet in one transaction (stamps `deleted_at`, deregisters every active agent, deletes placement rows; tasks preserved; idempotent). It does **not** close member panes — run `cafleet member delete` per member first, in the [`reference/recovery.md`](recovery.md) Shutdown order. Full behavior: [`cli-options.md`](../../../docs/spec/cli-options.md#fleet-delete).

## Typical Workflow

0. **Verify pane env** (Director): run `cafleet doctor` to confirm a supported multiplexer (tmux or herdr) is detected — the canonical pane-identity probe, before `cafleet fleet create` and any `cafleet member create`.

1. **Create a fleet** (if none exists):
   ```bash
   cafleet fleet create --name "my-project"
   # text: line 1 <fleet-id>, line 2 <root-director-agent-id>; --json for the nested shape
   ```
   Must run inside a tmux or herdr session (else exits 1 with `Error: cafleet fleet create must be run inside a tmux or herdr session`, writes nothing).

2. **Discover, send, poll, ack** per the command sections above; use `cafleet --json …` when parsing output. Director-side create/capture/exec/ping/nudge: [`reference/director.md`](director.md); shutdown ordering: [`reference/recovery.md`](recovery.md).

## Message Lifecycle

Messages are tasks with three states: **input_required** (delivered, awaiting ACK) → **completed** (ACKed), or **canceled** (sender retracted before ACK). For broadcast threading (the `origin_task_id` self-reference shape), see [`reference/broadcast.md`](broadcast.md).

## Error Handling

Errors print to stderr and exit non-zero; `cafleet --json <cmd>` emits them machine-parseably. The most common: missing `--fleet-id` (`Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.`, exit 1), missing `--agent-id` (`Error: Missing option '--agent-id'.`, exit 2), and `member *` commands outside a supported multiplexer session (exit 1). Full catalogue: [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages).
