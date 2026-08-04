# Message envelope

The shape of a `Message` envelope as it is persisted in SQLite, returned by the broker layer, and rendered by the CLI.

## Persisted shape

Every message is a flat row of typed columns in `messages` — there is no JSON blob; `messages.text` carries the body and the remaining columns carry the routing and lifecycle fields. See [data-model.md](data-model.md#messages) for the full column schema. The persisted shape is the canonical source of truth; every render the broker produces is a projection of these columns.

## Rendered shape

The broker's read paths return the persisted columns as a flat dict (the typed-column dict). `--json` emits that typed-column envelope verbatim — every persisted column, the body complete and untruncated. Text mode projects the dict into a compact rendered line and truncates the body.

### Text-mode projection

The compact text rendering omits the columns whose values are constant or recoverable from context:

| Field | Text mode |
|---|---|
| `message_id` | the bracketed `[<id>` segment on line 1 |
| `from_member_id` | the <code>&#124; from:&lt;n&gt;</code> segment on line 1 |
| `to_member_id` | omitted (the recipient's own poll already establishes `to == self`) |
| `owner_member_id` | omitted (always equals `to_member_id` for delivery rows; equals broadcaster for summary rows) |
| `status_timestamp` | the bare `<ts>` segment on line 1 |
| `text` | the body line, truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…`, omitted when the body is empty |
| `type` | the <code>&#124; kind:&lt;kind&gt;</code> segment when `!= "unicast"` |
| `created_at` | omitted |
| `status_state` | omitted (unconditional) |
| `origin_message_id` | the <code>&#124; origin:&lt;id&gt;</code> segment when non-NULL |

### JSON output

CLI JSON output is governed by the `--json` flag — the single output switch:

| Mode | Output |
|---|---|
| `--json` | Compact single-line JSON — no whitespace; non-ASCII (e.g. a `…` inside a body) is emitted as UTF-8, not escaped. The complete typed-column envelope, the body untruncated. |
| (text mode) | Two lines per message in the compact rendered shape, the body truncated. |

`message send`, `ack`, and `show` wrap the row as `{"message": {…}}` (`send` adds a sibling `notification_sent`); `poll` returns a bare array of rows; `broadcast` returns `[{"message": <summary row>, "recipients": N, "delivered": k}]`.

#### Example

A poll result with one unicast delivery (id `42`, from `7`, to `3`, body `"build OK"`).

**`cafleet message poll 3 --json`**:

```json
[{"message_id":42,"owner_member_id":3,"from_member_id":7,"to_member_id":3,"type":"unicast","created_at":"2026-05-05T05:42:11.123456+00:00","status_state":"input_required","status_timestamp":"2026-05-05T05:42:11.123456+00:00","origin_message_id":null,"text":"build OK"}]
```

A broadcast summary row carries `kind: "broadcast_summary"` and `origin: <id>` (self-referencing); the `text` body is the broker-computed summary string `"Broadcast sent to N recipients"`. The `message broadcast` response always contains exactly this single summary message plus the wrapper-level `recipients` (the real recipient count `N`) and `delivered` (the count of best-effort inline previews that landed) fields — there is no per-recipient envelope list (see [Output shapes](cli-options.md#output-shapes) for the cross-subcommand summary).

### Text mode

Text mode renders each message as two lines (line 1 is the bracketed envelope, line 2 is the body):

```
[42 | from:7 | 2026-05-05T05:42:11.123456+00:00]
build OK
```

Broadcast summary rows are never empty — the broker writes the human-readable summary `"Broadcast sent to N recipients"` at insert time, so summary rows always render their body line. Body truncation (the `…` suffix at `CAFLEET_MAX_TEXT_LEN` codepoints) is documented in [cli-options.md](cli-options.md#message-body-truncation).

## Flag cross-reference

The controls that govern envelope rendering are documented in [cli-options.md](cli-options.md):

| Control | Default | Effect on the envelope |
|---|---|---|
| [`--json`](cli-options.md#json-output) | off — text mode | Emits compact single-line JSON with the complete, untruncated body |
| [`CAFLEET_MAX_TEXT_LEN`](cli-options.md#message-body-truncation) | `200` | Truncates the text-mode body at that many codepoints, appending `…` |
