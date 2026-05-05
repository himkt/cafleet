# Broadcast — `cafleet message broadcast`

Send a message to every active recipient in the session (except the sender and the built-in Administrator). Returns a single `broadcast_summary` envelope to the caller.

```bash
cafleet --session-id <session-id> message broadcast --agent-id <my-agent-id> \
  --text "Build failed on main branch"
```

| Flag | Required | Notes |
|---|---|---|
| `--text <body>` | yes | Message body fanned out to every active recipient (except the sender and the built-in Administrator). |
| `--full` | no | Re-includes per-recipient envelopes and `recipient_ids` in the response. The default summary suppresses both — `recipient_count` is sufficient for the broadcaster's "did it go out?" check. |

## What the broker does

`broker.broadcast_message` writes one row per recipient as an individual delivery task (each visible to its recipient via `cafleet message poll`) PLUS one `broadcast_summary` row addressed to the broadcaster. The summary's `text` is an empty string; the human-facing summary string (`Broadcast sent to N recipients`) is computed client-side from `recipient_count`. Truncating that summary would hide the recipient count, so the CLI emits the broadcast summary in full regardless of `--full` (the `--full` flag controls only whether per-recipient envelopes and `recipient_ids` are included).

The response carries `notifications_sent_count` indicating how many recipient panes were successfully triggered by the inline-preview keystroke (see `reference/recovery.md` for the failure-mode chain when an inline preview misses).

## Default echo

The default broadcast echo is a one-line summary:

```
broadcast id=<id8> recipients=<count>
```

Per-recipient envelopes and the `recipient_ids` list are elided by default (added by design 0000049 Surface 3 + Surface 4). Pass `--full` to restore the legacy multi-envelope view.

## Threading via `origin_task_id`

Every broadcast generates `recipient_count + 1` rows in `tasks`:

| Row kind | `origin_task_id` value |
|---|---|
| Unicast delivery (regular `message send`) | `NULL` |
| Broadcast delivery row (one per recipient) | The summary task's `task_id` (shared across all delivery rows in this broadcast) |
| Broadcast summary row | Its own `task_id` (self-reference) |

The grouping predicate on the wire is `origin_task_id IS NOT NULL`, which cleanly partitions the timeline into "standalone unicast entry" vs "part of a broadcast group". The summary task's `task_id` is pre-allocated **before** the per-recipient INSERT loop in `broker.broadcast_message` so every delivery row can carry the link from the start.

The rendered envelope's compact form surfaces the threading link as `origin: <id8>` (8-char prefix of `origin_task_id`); the legacy `--full` view surfaces it as `origin_task_id` (full UUID). Recipients that want to thread their ACK with the original broadcast read `origin_task_id` and ack their delivery row's `task_id` as usual — there is no separate "ack the broadcast" call.

## Acknowledging a broadcast delivery

A broadcast recipient acks their own delivery row exactly the same way a unicast recipient acks a message:

```bash
cafleet --session-id <session-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

The summary row is NOT acked by recipients — it is a sender-side artifact, addressed back to the broadcaster, that captures the fan-out outcome. The broadcaster sees their own summary row in their inbox alongside any unicast deliveries to themselves.

## Flag-surface consistency

`--full` is preserved on `message broadcast` for surface consistency with `message {send,poll,ack,cancel,show}`. On those five subcommands `--full` disables body truncation; on `message broadcast` body truncation does not apply (the summary's `text` is empty and the per-recipient envelopes carry the original body untruncated by the time they appear in `--full` output). The flag's only on-the-wire effect for broadcast is "include `recipient_ids` and per-recipient envelopes". See `reference/legacy-flags.md` for the cross-subcommand `--full` semantics table.
