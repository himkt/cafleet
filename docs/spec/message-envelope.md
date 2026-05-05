# Message Envelope Specification

The shape of a `Task` envelope as it is persisted in SQLite, returned by the broker layer, and rendered by the CLI.

This document covers the **post-design-0049** shape only. The legacy nested envelope (`artifacts[].parts[].text`, `metadata.fromAgentId`, `metadata.toAgentId`, `metadata.type`, `kind`, `history`, `contextId`-as-payload-field) is removed — the broker no longer constructs it, the CLI no longer renders it, and the WebUI no longer consumes it. See [design-docs/0000001-a2a-registry-broker/](../../design-docs/0000001-a2a-registry-broker/) for the historical record of the inherited convention; nothing in the current codebase depends on that shape.

## Persisted shape (typed columns)

After [design 0000049 Surface 14](../../design-docs/0000049-token-reduction/design-doc.md), every message lives in `tasks` as a flat row of typed columns. There is no JSON blob — `tasks.task_json` was dropped and replaced with `tasks.text` plus the columns that already existed.

| Column | Type | Meaning |
|---|---|---|
| `task_id` | `TEXT` (UUID v4) | Primary key. Identifies a single delivery (unicast row, broadcast delivery row, or broadcast summary row). |
| `context_id` | `TEXT` (`agents.agent_id` FK) | The recipient agent for unicast/broadcast deliveries; the broadcaster for `broadcast_summary`; the preserved original `context_id` for ACK/cancel. The inbox-listing index `idx_tasks_context_status_ts` keys on `(context_id, status_timestamp DESC)`. |
| `from_agent_id` | `TEXT` (agent_id) | Sender. Not a foreign key — historical tasks may outlive their sender. |
| `to_agent_id` | `TEXT` (agent_id) | Recipient. Empty string `""` for `broadcast_summary` rows. |
| `type` | `TEXT` | `"unicast"` or `"broadcast_summary"`. Drives render-time decisions (e.g. summary tasks always emit in full; unicast tasks honor `CAFLEET_MAX_TEXT_LEN` truncation). |
| `created_at` | `TEXT` (ISO-8601, microsecond precision) | First-write timestamp; preserved across UPSERT. |
| `status_state` | `TEXT` | TaskState enum value: `input_required` (queued), `completed` (acked), `canceled` (retracted), `failed` (routing error). |
| `status_timestamp` | `TEXT` (ISO-8601, microsecond precision) | Updated on every state change. Used for `ORDER BY DESC` and the `since` filter on `cafleet message poll`. |
| `origin_task_id` | `TEXT` (UUID v4, nullable) | Broadcast grouping link. `NULL` on unicast deliveries; on broadcast delivery rows holds the summary task's `task_id`; on the broadcast summary row itself self-references its own `task_id`. |
| `text` | `TEXT` | Message body. For `broadcast_summary` rows, the broker writes the human-readable summary `"Broadcast sent to N recipients"` (computed in `broker.broadcast_message` at insert time). Added by Alembic revision `0009_drop_task_json_add_text`. |

The persisted shape is the canonical source of truth. Every render the broker produces is a projection of these columns.

See [data-model.md](data-model.md) for the full SQL schema (including indexes, foreign-key declarations, and the `0009_drop_task_json_add_text` migration shape with the operator backup procedure).

## Rendered shape

The broker's Python read paths return the persisted columns as a flat dict (the typed-column dict), and the CLI projects that dict into a compact rendered envelope via `output.render_task` — by default the rendered envelope omits the columns whose values are constant or recoverable from context. The `--full` flag returns the typed-column dict unmodified.

### Compact rendered envelope (default)

```python
def render_task(task: dict, *, full: bool = False) -> dict:
    if full:
        return task  # the typed-column dict
    out = {
        "id": task["task_id"][:8],
        "from": task["from_agent_id"][:8],
        "ts": task["status_timestamp"],
        "text": task["text"],
    }
    if task["type"] != "unicast":
        out["kind"] = task["type"]
    if task.get("origin_task_id"):
        out["origin"] = task["origin_task_id"][:8]
    return out
```

Field decisions:

| Field | Default | `--full` |
|---|---|---|
| `task_id` | rendered as `id` (8-char prefix) | rendered as `task_id` (full UUID) |
| `from_agent_id` | rendered as `from` (8-char prefix) | rendered as `from_agent_id` (full UUID) |
| `to_agent_id` | omitted (the recipient's own poll already establishes `to == self`) | included |
| `context_id` | omitted (always equals `to_agent_id` for delivery rows; equals broadcaster for summary rows) | included |
| `status_timestamp` | rendered as `ts` | rendered as `status_timestamp` |
| `text` | included, truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix | included, untruncated |
| `type` | omitted when `"unicast"` (the default); rendered as `kind` when `"broadcast_summary"` | rendered as `type` |
| `created_at` | omitted | included |
| `status_state` | omitted when `"input_required"` (the default for fresh deliveries) | included |
| `origin_task_id` | rendered as `origin` (8-char prefix) when non-NULL; omitted on unicast deliveries | included |

### JSON output

CLI JSON output is governed by two global flags:

| Flag combination | Output |
|---|---|
| `--json` (default) | Compact JSON: `json.dumps(data, separators=(",",":"))` — no whitespace. |
| `--json --pretty` | Indented JSON: `json.dumps(data, indent=2)`. |
| (text mode) | Two lines per task in the compact rendered shape; six lines per task in `--full`. |

#### Examples

A poll result with one unicast delivery (id `abc12345…`, from `xy23ef67…`, body `"build OK"`).

**Default (`cafleet --json message poll --agent-id <r>`)**:

```json
[{"id":"abc12345","from":"xy23ef67","ts":"2026-05-05T05:42:11.123456+00:00","text":"build OK"}]
```

**`--pretty` (`cafleet --json --pretty message poll --agent-id <r>`)**:

```json
[
  {
    "id": "abc12345",
    "from": "xy23ef67",
    "ts": "2026-05-05T05:42:11.123456+00:00",
    "text": "build OK"
  }
]
```

**`--full` (`cafleet --json --pretty message poll --agent-id <r> --full`)**:

```json
[
  {
    "task_id": "abc12345-aaaa-bbbb-cccc-dddddddddddd",
    "context_id": "<recipient-uuid>",
    "from_agent_id": "xy23ef67-aaaa-bbbb-cccc-eeeeeeeeeeee",
    "to_agent_id": "<recipient-uuid>",
    "type": "unicast",
    "created_at": "2026-05-05T05:42:11.123456+00:00",
    "status_state": "input_required",
    "status_timestamp": "2026-05-05T05:42:11.123456+00:00",
    "origin_task_id": null,
    "text": "build OK"
  }
]
```

A broadcast summary row carries `kind: "broadcast_summary"` (or `type` in `--full`) and `origin: <id8>` (self-referencing); the `text` body is the broker-computed summary string `"Broadcast sent to N recipients"` and the CLI suppresses the per-recipient envelope list unless `--full` is passed (see [`--full` semantics](cli-options.md#full-semantics-cross-subcommand-escape-hatch) for the cross-subcommand summary).

### Text mode

Text mode renders each task as two lines:

```
abc12345 from xy23ef67 @2026-05-05T05:42:11.123456+00:00
  build OK
```

`--full` switches to a six-line block that mirrors the `--full` JSON keys one-per-line. Text mode omits the `text:` line entirely only when the resulting body is the empty string (deliveries explicitly sent with an empty body). Broadcast summary rows are NOT empty — the broker writes the human-readable summary `"Broadcast sent to N recipients"` at insert time, so summary rows always render their `text:` line. The 10-codepoint `...` suffix from the pre-design-0049 era is replaced with the single Unicode codepoint `…` (U+2026) at the configured `CAFLEET_MAX_TEXT_LEN` codepoint count (default 200).

## Flag cross-reference

The flags that govern envelope rendering are documented in [cli-options.md](cli-options.md):

- [`--json`](cli-options.md#global-options) — emit JSON output (compact by default).
- [`--pretty`](cli-options.md#global-options) — switch JSON output from compact to indented.
- [`--full`](cli-options.md#full-semantics-cross-subcommand-escape-hatch) — return the full typed-column envelope and untruncated body.
- [`--quiet`](cli-options.md#message-body-truncation) — on `message send` / `message ack` / `member ping`, emit only the new task id (8-char prefix).

`CAFLEET_MAX_TEXT_LEN` (default `200`) controls body truncation in the rendered envelope; it is documented under [Message Body Truncation](cli-options.md#message-body-truncation).

## Changes from the inherited convention

The previous nested envelope shape — modelled after the inherited `Task` / `Message` / `Artifact` / `Part` types — wrapped the body in `artifacts[0].parts[0].text`, the routing identifiers in `metadata.fromAgentId` / `metadata.toAgentId` / `metadata.type`, the constants `kind: "text"` and `artifactId`, the always-empty `history`, and the `contextId`-as-payload-field that always equalled the recipient's own `agent_id`. cafleet has no external consumers that depend on that exact shape, so design 0000049 dropped it entirely. The `tasks.task_json` blob that persisted the legacy shape is dropped in the same single Alembic revision that introduces `tasks.text`.

The historical record of the inherited convention lives in [design-docs/0000001-a2a-registry-broker/](../../design-docs/0000001-a2a-registry-broker/). It is preserved for context only; the current codebase does not depend on the legacy shape, and new code paths MUST consume the typed-column envelope documented above.
