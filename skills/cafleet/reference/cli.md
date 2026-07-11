# CAFleet CLI — Fuller Command Catalog

Read this file for the broker CLI surface beyond the core identity / poll / send / ack lifecycle in [`SKILL.md`](../SKILL.md): environment variables, global options, output flags, coding-agent backends, cancel / show / broadcast / roster introspection / doctor / fleet delete, the typical workflow, the message lifecycle, and error handling. Exhaustive per-subcommand flags, exit codes, and error strings live in [`cli-options.md`](../../../docs/spec/cli-options.md).

## Environment variables

CLI env vars (all `CAFLEET_`-prefixed): `CAFLEET_DATABASE_URL` (SQLite URL; default `~/.local/share/cafleet/cafleet_v4.db`, use an absolute path when overriding — `~` is not expanded), `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` (`cafleet server` defaults `127.0.0.1` / `8000`), `CAFLEET_MAX_TEXT_LEN` (body-truncation limit, default `200` — see § *Output flags* below).

## Global Options

`--version` is the only top-level option (precedes the subcommand name); `--json`, `--member-id`, and `--fleet-id` are per-subcommand options (after the subcommand name — write `--json` trailing, after all other flags). Putting one in the wrong position fails with `No such option`.

```bash
cafleet member list --fleet-id <fleet-id> --json
cafleet message poll --fleet-id <fleet-id> --member-id <my-member-id> --json
```

`cafleet --version` prints `cafleet <version>` and exits 0 without `--fleet-id`.

## Output flags — `--full` / `--json`

`--full` and `--json` are cafleet's two **independent, composable** output-control flags over its compact default output. `--full` opts back into untruncated, every-field output — it bypasses the truncation caps. `--json` switches the encoding to structured JSON but is **still truncated** unless combined with `--full` (`--full --json` gives untruncated JSON). A separate `--quiet` flag — on `message send`, `message ack`, and `member ping` — prints only the bare `task_id` (the target member id for `ping`) for shell capture.

### `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch over a single flag covering four overloaded surfaces: `message {send,poll,ack,cancel,show}` → untruncated `text` + the full typed-column envelope; `message broadcast` → the single `broadcast_summary` task rendered verbose (never per-recipient rows or a `recipient_ids` list); `member show` → the labeled block (`kind`, `skills`, placement sub-block) in **text mode only** (JSON is the unprojected broker dict regardless of `--full`); `member create` → the 6-line `Member registered and spawned.` block. Per-surface detail: [`cli-options.md`](../../../docs/spec/cli-options.md#full-semantics).

### `--json` (per-subcommand, machine-parseable)

`--json` switches CLI output from text to JSON: compact single-line encoding (no whitespace; non-ASCII like the `…` suffix emitted as UTF-8), cheap to pipe into `jq` from a Director loop. It is a per-subcommand flag, written trailing — after all other flags:

```bash
cafleet message poll --fleet-id <fleet-id> --member-id <m> --json
cafleet message poll --fleet-id <fleet-id> --member-id <m> --full --json
```

### `CAFLEET_MAX_TEXT_LEN`

Environment variable controlling body truncation in the rendered envelope; default `200` codepoints, suffix the single codepoint `…` (U+2026). Separate hard-coded caps apply to `member.description` (`60`) and metadata strings (`80`); `--full` bypasses all three. See [`cli-options.md`](../../../docs/spec/cli-options.md#message-body-truncation).

## Coding-agent backends

Three backends — `claude` (default), `codex`, `opencode` — chosen per member at `member create` time via `--coding-agent`. `--model <m>` pins the LLM and `--role {member,monitor}` selects an ordinary vs the fleet's dedicated **monitoring member**; both flags, the model-name-to-backend inference, the per-backend available-model tables, and the spawn-argv detail live in [`reference/director.md`](director.md) (and [`roles/monitor.md`](../roles/monitor.md) plus [`reference/supervision.md`](supervision.md) for the monitor). All three honor the leading-`!` input shortcut, so `member exec` and inline previews work uniformly. Per-backend deltas: [`claude`](coding-agent/claude.md) / [`codex`](coding-agent/codex.md) / [`opencode`](coding-agent/opencode.md).

## Cancel (Retract)

Retract a sent message that has not been acknowledged yet (sender-only). `--task-id` required.

```bash
cafleet message cancel --fleet-id <fleet-id> --member-id <my-member-id> --task-id <task-id>
```

## Show (Get Task)

Fetch one task by id. `--task-id` required; `--full` for the untruncated envelope.

```bash
cafleet message show --fleet-id <fleet-id> --member-id <my-member-id> --task-id <task-id>
```

## Broadcast — `cafleet message broadcast`

Send a message to every active recipient in the fleet (except the sender). Returns a single `broadcast_summary` envelope to the caller.

```bash
cafleet message broadcast --fleet-id <fleet-id> --from-member-id <my-member-id> \
  --text "Build failed on main branch"
```

| Flag | Required | Notes |
|---|---|---|
| `--text <body>` | exactly one of `--text` / `--text-file` | Inline message body fanned out to every active recipient (except the sender). |
| `--text-file <path>` | exactly one of `--text` / `--text-file` | Same body read from a UTF-8 file (or `-` for stdin); use it for long or multi-line bodies that would hit the shell's `ARG_MAX`. |
| `--full` | no | Renders the single `broadcast_summary` task in full; never adds per-recipient rows or a `recipient_ids` list. See § *Output flags* above. |

The broker writes one delivery row per recipient (each visible via that recipient's `cafleet message poll`) plus one `broadcast_summary` row addressed back to the broadcaster, and returns only that summary task plus two top-level fields: `recipients` (the real recipient count `N`) and `delivered` (how many recipient panes the inline-preview keystroke reached — see [`reference/recovery.md`](recovery.md) for the miss-handling chain). The two diverge when any preview fails to land. Default echo is one line:

```
broadcast id=<id> recipients=<N> delivered=<k>
```

Recipients ack their own delivery row exactly like a unicast message; the summary row is a sender-side artifact and is not acked by recipients:

```bash
cafleet message ack --fleet-id <fleet-id> --member-id <my-member-id> --task-id <task-id>
```

For the row schema, the `"Broadcast sent to N recipients"` summary string, and `origin_task_id` grouping/threading, see [`docs/spec/data-model.md`](../../../docs/spec/data-model.md#broadcast-grouping) and [`docs/spec/message-envelope.md`](../../../docs/spec/message-envelope.md).

## List Members

`member list` returns every active registry entry of the fleet (root Director, monitoring member, ordinary members, placementless rows); `member show --member-id <target-member-id>` fetches one. Both are registry reads — no tmux required.

```bash
cafleet member list --fleet-id <fleet-id>
cafleet member show --fleet-id <fleet-id> --member-id <target-member-id>
```

`member list` renders the `N members:` table with `member_id`, `name`, `kind` (`director` / `monitor` / `member`), `backend`, `pane_id`, and `idle` columns — `-` placement cells for placementless rows, `(pending)` for a placed row with no pane yet, and `idle` humanized (`Ns`/`Nm`/`Nh`, `-` when no task activity); `--json` adds the underlying `last_sent` / `last_recv` / `last_ack` timestamps per row. `member show` default output is the one-line row `<id> <name> <status>`; `--full` gives the labeled block (`description` truncated to 60 codepoints, `kind`, `skills`, placement sub-block) — text mode only. See § *Output flags* above.

## Doctor

Print the resolved multiplexer backend and the calling pane's session/window/pane identifiers for diagnosing placement without raw multiplexer commands. Does NOT require `--fleet-id`; requires a supported multiplexer to be detected (tmux or herdr).

```bash
cafleet doctor
cafleet doctor --json
```

## Deregister

`cafleet member delete` is the single teardown for any member: it closes the pane when one exists and soft-deletes the registration; a placementless or pending-placement target is a pure registry soft-delete (no tmux required).

```bash
cafleet member delete --fleet-id <fleet-id> --member-id <target-member-id>
```

The root Director cannot be deregistered (exit 1 — see [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages)). Use `cafleet fleet delete --fleet-id <fleet-id>` for fleet teardown.

## Fleet Delete

```bash
cafleet fleet delete --fleet-id <fleet-id>
# → Deleted fleet <fleet-id>. Deregistered N members.
```

Soft-deletes the fleet in one transaction (stamps `deleted_at`, deregisters every active member, deletes placement rows; tasks preserved; idempotent). It does **not** close member panes — run `cafleet member delete` per member first, in the [`reference/recovery.md`](recovery.md) Shutdown order. Full behavior: [`cli-options.md`](../../../docs/spec/cli-options.md#fleet-delete).

## Typical Workflow

0. **Verify pane env** (Director): run `cafleet doctor` to confirm a supported multiplexer (tmux or herdr) is detected — the canonical pane-identity probe, before `cafleet fleet create` and any `cafleet member create`.

1. **Create a fleet** (if none exists):
   ```bash
   cafleet fleet create --name "my-project"
   # text: '<fleet-id> director=<root-director-member-id>'; append --json for the nested shape
   ```
   Must run inside a tmux or herdr session (else exits 1 with `Error: cafleet fleet create must be run inside a tmux or herdr session`, writes nothing).

2. **Discover, send, poll, ack** per the command sections above; append a trailing `--json` when parsing output. Director-side create/capture/exec/ping: [`reference/director.md`](director.md); shutdown ordering: [`reference/recovery.md`](recovery.md).

## Message Lifecycle

Messages are tasks with three states: **input_required** (delivered, awaiting ACK) → **completed** (ACKed), or **canceled** (sender retracted before ACK). For broadcast threading (the `origin_task_id` self-reference shape), see § *Broadcast* above.

## Error Handling

Errors print to stderr and exit non-zero; `cafleet <cmd> … --json` emits them machine-parseably. The most common: missing `--fleet-id` (`Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.`, exit 1), missing `--member-id` (`Error: Missing option '--member-id'.`, exit 2), and `member *` commands outside a supported multiplexer session (exit 1). Full catalogue: [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages).
