# CAFleet CLI — Fuller Command Catalog

Read this file for the broker CLI surface beyond the core identity / poll / send / ack lifecycle in [`SKILL.md`](../SKILL.md). Exhaustive per-subcommand flags, exit codes, and error strings live in [`cli-options.md`](../../../docs/docs/spec/cli-options.md).

## Environment variables

CLI env vars (all `CAFLEET_`-prefixed): `CAFLEET_DATABASE_URL` (SQLite URL; default `~/.local/share/cafleet/cafleet_v5.db`, use an absolute path when overriding — `~` is not expanded), `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` (`cafleet server` defaults `127.0.0.1` / `8000`), `CAFLEET_MAX_TEXT_LEN` (text-mode body-truncation limit, default `200` — see § *Output switch* below).

## Global Options

`--version` is the only top-level option (precedes the subcommand name); `--json` is per-subcommand (after the subcommand name — write it trailing, after all other arguments). The subject id is a positional argument placed immediately after the subcommand name. Putting an option in the wrong position fails with the parser's unknown-argument error.

```bash
cafleet member list <fleet-id> --json
cafleet message poll <my-member-id> --json
```

`cafleet --version` prints `cafleet <version>` and exits 0.

## Output switch — `--json`

`--json` is cafleet's single output-control flag. Text output (the default) is the compact human/pane form — message bodies truncated to `CAFLEET_MAX_TEXT_LEN` codepoints (default `200`), suffixed with the single codepoint `…` (U+2026; see [`cli-options.md`](../../../docs/docs/spec/cli-options.md#message-body-truncation)). `--json` switches to compact single-line JSON (no whitespace; non-ASCII emitted as UTF-8) and is always the **complete, untruncated machine form** — full envelopes, full bodies — cheap to pipe into `jq` from a Director loop:

```bash
cafleet message poll <my-member-id> --json
```

The detailed member view (`kind`, `skills`, the placement sub-dict) is likewise `--json`-only — text `member show` is the compact one-line row. Per-surface shapes: [`cli-options.md`](../../../docs/docs/spec/cli-options.md#output-shapes).

## Coding-agent backends

Three backends — `claude`, `codex`, `opencode` — chosen per member at `member create` time via `--coding-agent` (omitted → the member inherits the Director's backend). `--model <m>` pins the LLM and `--effort <level>` forwards a reasoning-effort level (claude: `low`–`max`; codex: `minimal`–`xhigh`; opencode: unsupported — any value exits 2); both flags, the model-name-to-backend inference, the per-backend available-model tables, and the spawn-argv detail live in [`reference/director.md`](director.md). All three honor the leading-`!` input shortcut, so `member prompt --shell` and inline previews work uniformly. Per-backend deltas: [`claude`](coding-agent-overlays.md#claude) / [`codex`](coding-agent-overlays.md#codex) / [`opencode`](coding-agent-overlays.md#opencode).

## Show (Get Message)

Fetch one message by id — the positional `MESSAGE_ID` is the only required argument (the fleet is derived from the message row); `--json` for the untruncated envelope.

```bash
cafleet message show <message-id>
```

## Broadcast — `cafleet message broadcast`

Send a message to every active recipient in the fleet (except the sender). Returns a single `broadcast_summary` envelope to the caller.

```bash
cafleet message broadcast --from-member-id <my-member-id> \
  "Build failed on main branch"
```

| Argument | Required | Notes |
|---|---|---|
| `--from-member-id <int>` | yes | The broadcaster; the fleet is derived from the sender row. |
| positional `TEXT` | exactly one of `TEXT` / `--file` | Inline message body fanned out to every active recipient (except the sender). |
| `--file <path>` | exactly one of `TEXT` / `--file` | Same body read from a UTF-8 file (or `-` for stdin); use it for long or multi-line bodies that would hit the shell's `ARG_MAX`. |

The broker writes one delivery row per recipient (each visible via that recipient's `cafleet message poll`) plus one `broadcast_summary` row addressed back to the broadcaster, and returns only that summary message plus two top-level fields: `recipients` (the real recipient count `N`) and `delivered` (how many recipient panes the inline-preview keystroke reached — see [`reference/recovery.md`](recovery.md) for the miss-handling chain). The two diverge when any preview fails to land. Default echo is one line:

```
broadcast id=<id> recipients=<N> delivered=<k>
```

Recipients ack their own delivery row exactly like a unicast message; the summary row is a sender-side artifact and is not acked by recipients:

```bash
cafleet message ack <message-id>
```

For the row schema, the `"Broadcast sent to N recipients"` summary string, and `origin_message_id` grouping/threading, see [`docs/docs/spec/data-model.md`](../../../docs/docs/spec/data-model.md#broadcast-grouping) and [`docs/docs/spec/message-envelope.md`](../../../docs/docs/spec/message-envelope.md).

## List Members

`member list` returns every active registry entry of the fleet (root Director, ordinary members, placementless rows); `member show` fetches one by its positional `MEMBER_ID`. Both are registry reads — no tmux required.

```bash
cafleet member list <fleet-id>
cafleet member show <target-member-id>
```

`member list` renders the `N members:` table with `member_id`, `name`, `kind` (`director` / `member`), `backend`, `pane_id`, and `idle` columns — `-` placement cells for placementless rows, `(pending)` for a placed row with no pane yet, and `idle` humanized (`Ns`/`Nm`/`Nh`, `-` when no message activity); `--json` adds the underlying `last_sent` / `last_recv` / `last_ack` timestamps per row. `member show` text output is the one-line row `<id> <name> <status>`; the detailed view (`kind`, `skills`, placement sub-dict) is `--json`. See § *Output switch* above.

## Doctor

The full-environment diagnosis: a three-section report — multiplexer (the resolved backend and the calling pane's session/window/pane identifiers), database (recorded schema version vs. the embedded head), and coding agents (per-agent install state at the resolved config paths) — rendered without early abort, replacing raw multiplexer commands for placement diagnosis. Takes no id. Exits non-zero iff any section reports an issue (a not-installed agent never counts as one).

```bash
cafleet doctor
cafleet doctor --json
```

## Monitor

`cafleet monitor` is the supervision scheduler — a two-form command with a positional fleet id:

```bash
cafleet monitor <fleet-id>          # the scheduler loop (launched by the Director as a background task in its own pane)
cafleet monitor scan <fleet-id>     # one-shot batch capture: the Director's pane + every active member's pane, print, exit
```

The loop form takes `--interval N` (the Director wake interval in seconds; omitted → `CAFLEET_MONITOR_WAKE_INTERVAL`, default `600`; `0` disables the wake) and `--tick N` (scan cadence, default `5`). It prints `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` immediately after claiming the runtime row — the line the Director confirms before its first `member create`.

`cafleet monitor scan` is the fleet-wide read — flags, output shape, and gate semantics in [`reference/director.md`](director.md) § *Fleet Scan*.

`cafleet member capture <target-member-id>` is the targeted single-pane read (a pending-placement target is a hard error) — flags and gate semantics in [`reference/director.md`](director.md) § *Member Capture*.

`cafleet member ping` is a Director write primitive. Against a pending-placement member it skips the keystroke and exits 0 — the member polls its inbox on spawn — with the stable `skipped` JSON key on both success paths.

## Deregister

`cafleet member delete` is the single teardown for any member: it closes the pane when one exists and soft-deletes the registration; a placementless or pending-placement target is a pure registry soft-delete (no tmux required).

```bash
cafleet member delete <target-member-id>
```

The root Director cannot be deregistered (exit 1 — see [`cli-options.md`](../../../docs/docs/spec/cli-options.md#error-messages)). Use `cafleet fleet delete <fleet-id>` for fleet teardown.

## Fleet Delete

```bash
cafleet fleet delete <fleet-id>
# → Deleted fleet <fleet-id>. Deregistered N members.
```

Soft-deletes the fleet in one transaction (stamps `deleted_at`, deregisters every active member, deletes placement rows; messages preserved; idempotent). It does **not** close member panes — run `cafleet member delete` per member first, in the [`reference/recovery.md`](recovery.md) Shutdown order. Full behavior: [`cli-options.md`](../../../docs/docs/spec/cli-options.md#fleet-delete).

## Typical Workflow

0. **Verify pane env** (Director): run `cafleet doctor` — the canonical pane-identity probe, before `cafleet fleet create` and any `cafleet member create`. It renders the three-section diagnosis (multiplexer, database, coding agents) and exits non-zero on any rendered issue; a non-zero exit aborts the spawn protocol.

1. **Create a fleet** (if none exists):
   ```bash
   cafleet fleet create --name "my-project" --coding-agent <backend>
   # text: '<fleet-id> director=<root-director-member-id>'; append --json for the nested shape
   ```
   `--coding-agent <backend>` — the substitution rule is [`reference/supervision.md`](supervision.md) § *Spawn Protocol* → *Fleet bootstrap*.
   Must run inside a tmux or herdr session (else exits 1 with `Error: cafleet fleet create must be run inside a tmux or herdr session`, writes nothing).

2. **Discover, send, poll, ack** per the command sections above; append a trailing `--json` when parsing output. Director-side create/capture/prompt/ping: [`reference/director.md`](director.md); shutdown ordering: [`reference/recovery.md`](recovery.md).

## Message Lifecycle

A `messages` row moves through two states: **input_required** (delivered, awaiting ACK) → **completed** (ACKed). For broadcast threading (the `origin_message_id` self-reference shape), see § *Broadcast* above.

## Error Handling

Errors print to stderr and exit non-zero; `cafleet <cmd> … --json` emits them machine-parseably. The most common: a missing positional subject id (the parser's missing-required-argument error, exit 2), an unknown member on `message poll` (`Error: Member <member-id> not found`, exit 1), and `member *` pane commands outside a supported multiplexer session (exit 1). Full catalogue: [`cli-options.md`](../../../docs/docs/spec/cli-options.md#error-messages).
