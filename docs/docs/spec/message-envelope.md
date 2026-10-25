# Message envelope

A message is a durable point-to-point delivery. The recipient explicitly acknowledges consumption; completion of a delivery is separate from completing its assignment.

## Record

```json
{
  "message_id": 42,
  "from_member_id": 7,
  "to_member_id": 3,
  "created_at": "2026-05-05T05:42:11.123456+00:00",
  "status_state": "input_required",
  "status_timestamp": "2026-05-05T05:42:11.123456+00:00",
  "text": "build OK"
}
```

Routing IDs are integers and body text is exact. ACK changes status to `completed` and stamps its timestamp. Required stored values decode with explicit errors.

## Rendering

Poll returns full records as an array. Show/ACK wrap one record as `message`; send adds `notification_sent`. Wait includes full records in its result; listen wraps each inside a tagged `message` event alongside separate ready/closed lifecycle records. See [messaging](cli-options.md#messaging).

Text prints a compact envelope:

```text
[42 | from:7 | 2026-05-05T05:42:11.123456+00:00]
build OK
```

Text bodies use the configured Unicode codepoint limit and `…`; JSON preserves full content. Pane previews submit a safe single line independently of that text limit, using folded content or a `message show ID --json` pointer. Consume the existing ID even after notification failure.
