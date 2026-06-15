# Broadcast — `cafleet message broadcast`

Send a message to every active recipient in the fleet (except the sender and the built-in Administrator). Returns a single `broadcast_summary` envelope to the caller.

```bash
cafleet message broadcast --fleet-id <fleet-id> --agent-id <my-agent-id> \
  --text "Build failed on main branch"
```

| Flag | Required | Notes |
|---|---|---|
| `--text <body>` | yes | Message body fanned out to every active recipient (except the sender and the built-in Administrator). |
| `--full` | no | Renders the single `broadcast_summary` task in full but never adds per-recipient delivery rows or a `recipient_ids` list — the broker only ever returns that one summary task plus the top-level `notifications_sent_count` wrapper, regardless of the flag. See [`reference/output-flags.md`](output-flags.md) for the cross-subcommand `--full` semantics. |

## What the broker does

`broker.broadcast_message` writes one row per recipient as an individual delivery task (each visible to its recipient via `cafleet message poll`) PLUS one `broadcast_summary` row addressed to the broadcaster. The summary's `text` is the human-readable string `"Broadcast sent to N recipients"`, written by the broker at insert time. The function returns a list containing exactly one envelope: the summary task plus a top-level `notifications_sent_count` field. The per-recipient delivery rows are NOT echoed back to the caller (they are observable only via the recipients' own `cafleet message poll`), and there is no `recipient_ids` list in the response.

The response carries `notifications_sent_count` indicating how many recipient panes were successfully triggered by the inline-preview keystroke (see `reference/recovery.md` for the failure-mode chain when an inline preview misses).

## Default echo

The default broadcast echo is a one-line summary:

```
broadcast id=<id> recipients=<count>
```

## Threading via `origin_task_id`

Every broadcast generates `recipient_count + 1` rows in `tasks`:

| Row kind | `origin_task_id` value |
|---|---|
| Unicast delivery (regular `message send`) | `NULL` |
| Broadcast delivery row (one per recipient) | The summary task's `task_id` (shared across all delivery rows in this broadcast) |
| Broadcast summary row | Its own `task_id` (self-reference) |

The grouping predicate on the wire is `origin_task_id IS NOT NULL`, which cleanly partitions the timeline into "standalone unicast entry" vs "part of a broadcast group". The summary task's `task_id` is pre-allocated **before** the per-recipient INSERT loop in `broker.broadcast_message` so every delivery row can carry the link from the start.

The rendered envelope's compact form surfaces the threading link as `origin: <id>` (the full integer `origin_task_id`); the `--full` view surfaces it as `origin_task_id`. Recipients that want to thread their ACK with the original broadcast read `origin_task_id` and ack their delivery row's `task_id` as usual — there is no separate "ack the broadcast" call.

## Acknowledging a broadcast delivery

A broadcast recipient acks their own delivery row exactly the same way a unicast recipient acks a message:

```bash
cafleet message ack --fleet-id <fleet-id> --agent-id <my-agent-id> --task-id <task-id>
```

The summary row is NOT acked by recipients — it is a sender-side artifact, addressed back to the broadcaster, that captures the fan-out outcome. The broadcaster sees their own summary row in their inbox alongside any unicast deliveries to themselves.
