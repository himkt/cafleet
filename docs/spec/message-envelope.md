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

| Field | Compact text mode | Compact JSON key | `--full` text label | `--full` JSON key |
|---|---|---|---|---|
| `message_id` | the bracketed `[<id>` segment on line 1 | `id` | `id` | `message_id` |
| `from_member_id` | the <code>&#124; from:&lt;n&gt;</code> segment on line 1 | `from` | `from` | `from_member_id` |
| `to_member_id` | omitted (the recipient's own poll already establishes `to == self`) | omitted | `to`, omitted for broadcast-summary rows (`to_member_id IS NULL`) | `to_member_id` |
| `owner_member_id` | omitted (always equals `to_member_id` for delivery rows; equals broadcaster for summary rows) | omitted | — | `owner_member_id` |
| `status_timestamp` | the bare `<ts>` segment on line 1 | `ts` | — | `status_timestamp` |
| `text` | the body line, truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…`, omitted when the body is empty | `text`, truncated | `text`, omitted when the body is empty | `text`, untruncated |
| `type` | the <code>&#124; kind:&lt;kind&gt;</code> segment when `!= "unicast"` | `kind` when `"broadcast_summary"` | `type` | `type` |
| `created_at` | omitted | omitted | — | `created_at` |
| `status_state` | omitted (unconditional) | omitted | `state` | `status_state` |
| `origin_message_id` | the <code>&#124; origin:&lt;id&gt;</code> segment when non-NULL | `origin` when non-NULL | — | `origin_message_id` |

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

A broadcast summary row carries `kind: "broadcast_summary"` (or `type` in `--full`) and `origin: <id>` (self-referencing); the `text` body is the broker-computed summary string `"Broadcast sent to N recipients"`. The `message broadcast` response always contains exactly this single summary message plus the wrapper-level `recipients` (the real recipient count `N`) and `delivered` (the count of best-effort inline previews that landed) fields — there is no per-recipient envelope list. `--full` renders that single summary message in full (verbose envelope / typed-column dict) instead of the one-line summary, but never adds per-recipient envelopes (see [Output shapes](cli-options.md#output-shapes) for the cross-subcommand summary).

### Text mode

Text mode renders each message as two lines (line 1 is the bracketed envelope, line 2 is the body):

```
[42 | from:7 | 2026-05-05T05:42:11.123456+00:00]
build OK
```

`--full` switches to a variable-length labeled block — one field per line, per the table above. So a fresh unicast delivery prints six lines, while a broadcast-summary row with no recipient prints fewer. Broadcast summary rows are never empty — the broker writes the human-readable summary `"Broadcast sent to N recipients"` at insert time, so summary rows always render their `text:` line. Body truncation (the `…` suffix at `CAFLEET_MAX_TEXT_LEN` codepoints) is documented in [cli-options.md](cli-options.md#message-body-truncation).

## Flag cross-reference

The flags that govern envelope rendering are documented in [cli-options.md](cli-options.md):

| Control | Default | Effect on the envelope |
|---|---|---|
| [`--json`](cli-options.md#json-output) | off — text mode | Emits compact single-line JSON |
| [`--full`](cli-options.md#output-shapes) | off — the compact envelope | Returns the full typed-column envelope and an untruncated body |
| [`CAFLEET_MAX_TEXT_LEN`](cli-options.md#message-body-truncation) | `200` | Truncates the body at that many codepoints, appending `…` |

