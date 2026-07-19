---
icon: lucide/mail
---

# Message envelope

The shape of a `Message` envelope as it is persisted in SQLite, returned by the broker layer, and rendered by the CLI.

## Persisted shape

Every message is a flat row of typed columns in `messages` — there is no JSON blob; `messages.text` carries the body and the remaining columns carry the routing and lifecycle fields. See [data-model.md](data-model.md#messages) for the full column schema. The persisted shape is the canonical source of truth; every render the broker produces is a projection of these columns.

## Rendered shape

The broker's read paths return the persisted columns as a flat dict (the typed-column dict), and the CLI projects that dict into a compact rendered envelope — by default the rendered envelope omits the columns whose values are constant or recoverable from context. The `--full` flag returns the typed-column dict unmodified.

### Compact rendered envelope (default)

Field decisions:

| Field | Default | `--full` |
|---|---|---|
| `message_id` | rendered as `id` (full integer) | rendered as `message_id` |
| `from_member_id` | rendered as `from` (full integer) | rendered as `from_member_id` |
| `to_member_id` | omitted (the recipient's own poll already establishes `to == self`) | included |
| `owner_member_id` | omitted (always equals `to_member_id` for delivery rows; equals broadcaster for summary rows) | included |
| `status_timestamp` | rendered as `ts` | rendered as `status_timestamp` |
| `text` | included, truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix | included, untruncated |
| `type` | omitted when `"unicast"` (the default); rendered as `kind` when `"broadcast_summary"` | rendered as `type` |
| `created_at` | omitted | included |
| `status_state` | omitted (unconditional) | included |
| `origin_message_id` | rendered as `origin` (full integer) when non-NULL; omitted on unicast deliveries | included |

### JSON output

CLI JSON output is governed by the `--json` flag:

| Mode | Output |
|---|---|
| `--json` | Compact single-line JSON — no whitespace; non-ASCII (e.g. the `…` suffix) is emitted as UTF-8, not escaped. |
| (text mode) | Two lines per message in the compact rendered shape; a variable-length labeled block per message in `--full`. |

#### Examples

A poll result with one unicast delivery (id `42`, from `7`, body `"build OK"`).

**Default (`cafleet message poll --member-id <my-member-id> --json`)**:

```json
[{"id":42,"from":7,"ts":"2026-05-05T05:42:11.123456+00:00","text":"build OK"}]
```

**`--full` (`cafleet message poll --member-id <my-member-id> --full --json`)**:

```json
[
  {
    "message_id": 42,
    "owner_member_id": 3,
    "from_member_id": 7,
    "to_member_id": 3,
    "type": "unicast",
    "created_at": "2026-05-05T05:42:11.123456+00:00",
    "status_state": "input_required",
    "status_timestamp": "2026-05-05T05:42:11.123456+00:00",
    "origin_message_id": null,
    "text": "build OK"
  }
]
```

> Indented here for readability; the actual `--json` output is a single compact line with no whitespace. `--full` only changes which fields are emitted, never the encoding.

A broadcast summary row carries `kind: "broadcast_summary"` (or `type` in `--full`) and `origin: <id>` (self-referencing); the `text` body is the broker-computed summary string `"Broadcast sent to N recipients"`. The `message broadcast` response always contains exactly this single summary message plus the wrapper-level `recipients` (the real recipient count `N`) and `delivered` (the count of best-effort inline previews that landed) fields — there is no per-recipient envelope list. `--full` renders that single summary message in full (verbose envelope / typed-column dict) instead of the one-line summary, but never adds per-recipient envelopes (see [`--full` semantics](cli-options.md#full-semantics) for the cross-subcommand summary).

### Text mode

Text mode renders each message as two lines (line 1 is the bracketed envelope, line 2 is the body):

```
[42 | from:7 | 2026-05-05T05:42:11.123456+00:00]
build OK
```

Optional segments `| kind:<kind>` and `| origin:<id>` are appended to line 1 when the message is a broadcast summary (`type != "unicast"`) or has a non-NULL `origin_message_id`, respectively. The body line is omitted entirely when the resulting body is the empty string.

`--full` switches to a variable-length labeled block — one field per line (`id`, `state`, `from`, `to`, `type`, `text`), with the `to:` line omitted for broadcast-summary rows (`to_member_id IS NULL`) and the `text:` line omitted when the body is the empty string (deliveries explicitly sent with an empty body). So a fresh unicast delivery prints six lines, while a broadcast-summary row with no recipient prints fewer. Broadcast summary rows are never empty — the broker writes the human-readable summary `"Broadcast sent to N recipients"` at insert time, so summary rows always render their `text:` line. Body truncation (the `…` suffix at `CAFLEET_MAX_TEXT_LEN` codepoints) is documented in [cli-options.md](cli-options.md#message-body-truncation).

## Flag cross-reference

The flags that govern envelope rendering are documented in [cli-options.md](cli-options.md):

- [`--json`](cli-options.md#json-output) — emit JSON output (compact).
- [`--full`](cli-options.md#full-semantics) — return the full typed-column envelope and untruncated body.

`CAFLEET_MAX_TEXT_LEN` (default `200`) controls body truncation in the rendered envelope; it is documented under [Message Body Truncation](cli-options.md#message-body-truncation).

