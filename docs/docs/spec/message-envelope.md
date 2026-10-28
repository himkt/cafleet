# Message envelope

A message is a durable point-to-point delivery. The recipient owns its inbox and explicitly acknowledges consumption.

## Persisted shape

The [messages table](data-model.md#messages) stores typed routing/status columns, exact body text and internal `notified_at` state. Required values decode with typed errors.

The ordered public record is:

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

`to_member_id` is always an integer. ACK changes `status_state` to `completed` and stamps `status_timestamp`; completion means the delivery was consumed, independently of the assignment's progress.

## Rendered shape

[Output shapes](cli-options.md#output-shapes) owns each command's wrapper. Poll returns an array of full records; show/ACK wrap one record as `message`; send adds `notification_sent`. Wait returns full poll records inside its finite receive result. [Listen](cli-options.md#message-listen) emits that same full record inside a tagged `message` envelope, with separate `ready`/`closed` lifecycle records and explicit ACK consumption.

### Text-mode projection

| Field | Text rendering |
|---|---|
| `message_id` | Bracketed ID on the first line |
| `from_member_id` | `from:` segment |
| `status_timestamp` | Timestamp on the first line |
| `text` | Body on the next line when nonempty |
| Recipient, creation and status fields | Available in full JSON |

```text
[42 | from:7 | 2026-05-05T05:42:11.123456+00:00]
build OK
```

Text bodies use `CAFLEET_MAX_TEXT_LEN` codepoints plus `…`; JSON preserves the complete exact text. A pane notification line folds the full body under its own [line contract](multiplexer-backends.md#push-notifications), independent of `CAFLEET_MAX_TEXT_LEN`.

## Pane preview forms

A wrapped member receives the exact line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

An explicitly unwrapped member retains the exact legacy line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Both forms preserve the existing body folding, inline limit and long-body `message show` pointer. Wrapped delivery follows the [input receipt contract](member-runtime.md#input-arbitration-and-the-original-native-answer); held input preserves the native request and leaves its notification stamp unset.

## Runtime observation messages

A runtime notification is an ordinary inbox record whose `text` is the complete versioned event JSON. The pinned observed member is its sender, and the Director is its recipient; verify the durable event/message association through `event show` before acting. Its typed source distinguishes runner observation from model-authored speech. Version-2 permission-opening events carry a broker-built review instruction and inspection argv, validated against the stored event/Director association. When the evidence's `operation_rows` reports an operation shown over several displayed rows, the instruction adds a sentence asking the Director to confirm the command against the excerpt, because a row break may be a wrap or a line break. ACK consumes the message independently of the attributed decision, answer submission, event visibility and actual continuation. The [runtime contract](member-runtime.md#persistence-notification-and-read-interfaces) owns evidence policy, atomic association and archived recovery.
