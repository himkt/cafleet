# Broadcast — `cafleet message broadcast`

Send a message to every active recipient in the fleet (except the sender and the built-in Administrator). Returns a single `broadcast_summary` envelope to the caller.

```bash
cafleet message broadcast --fleet-id <fleet-id> --agent-id <my-agent-id> \
  --text "Build failed on main branch"
```

| Flag | Required | Notes |
|---|---|---|
| `--text <body>` | exactly one of `--text` / `--text-file` | Inline message body fanned out to every active recipient (except the sender and the built-in Administrator). |
| `--text-file <path>` | exactly one of `--text` / `--text-file` | Same body read from a UTF-8 file (or `-` for stdin); use it for long or multi-line bodies that would hit the shell's `ARG_MAX`. |
| `--full` | no | Renders the single `broadcast_summary` task in full; never adds per-recipient rows or a `recipient_ids` list. See [`reference/output-flags.md`](output-flags.md). |

The broker writes one delivery row per recipient (each visible via that recipient's `cafleet message poll`) plus one `broadcast_summary` row addressed back to the broadcaster, and returns only that summary task plus two top-level fields: `recipients` (the real recipient count `N`) and `delivered` (how many recipient panes the inline-preview keystroke reached — see [`reference/recovery.md`](recovery.md) for the miss-handling chain). The two diverge when any preview fails to land. Default echo is one line:

```
broadcast id=<id> recipients=<N> delivered=<k>
```

Recipients ack their own delivery row exactly like a unicast message; the summary row is a sender-side artifact and is not acked by recipients:

```bash
cafleet message ack --fleet-id <fleet-id> --agent-id <my-agent-id> --task-id <task-id>
```

For the row schema, the `"Broadcast sent to N recipients"` summary string, and `origin_task_id` grouping/threading, see [`docs/spec/data-model.md`](../../../docs/spec/data-model.md#broadcast-grouping) and [`docs/spec/message-envelope.md`](../../../docs/spec/message-envelope.md).
