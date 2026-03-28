# A2A Operations Specification

The Broker exposes standard A2A endpoints using the `a2a-sdk` Python library. All operations use JSON-RPC 2.0 over HTTP.

## Authentication

All A2A operations require `Authorization: Bearer <api_key>` header. Missing or invalid API key returns HTTP 401 before JSON-RPC processing.

## SendMessage — Send a Message

The sender specifies routing in `Message.metadata`:

| metadata field | Value | Behavior |
|---|---|---|
| `destination` | `"<agent-uuid>"` | Unicast: create delivery Task for target agent |
| `destination` | `"*"` | Broadcast: create delivery Tasks for all active agents (except sender) |

### Unicast — Send

1. Agent A calls `SendMessage` with `metadata.destination = "agentB-uuid"`
2. Broker creates Task: `contextId=agentB-uuid`, state=`INPUT_REQUIRED`, message content in Artifact
3. Broker returns the Task to Agent A (Agent A now has the `taskId` for tracking)

**Example** (JSON-RPC):

```json
{
  "jsonrpc": "2.0",
  "method": "SendMessage",
  "params": {
    "message": {
      "messageId": "msg-uuid-1",
      "role": "ROLE_USER",
      "parts": [
        {"text": "Did the API schema change?"}
      ],
      "metadata": {
        "destination": "agentB-uuid"
      }
    }
  },
  "id": "req-1"
}
```

**Response** (delivery Task returned):

```json
{
  "jsonrpc": "2.0",
  "result": {
    "task": {
      "id": "task-delivery-uuid",
      "contextId": "agentB-uuid",
      "status": {
        "state": "TASK_STATE_INPUT_REQUIRED",
        "timestamp": "2026-03-28T12:00:00.000Z"
      },
      "artifacts": [
        {
          "artifactId": "msg-content-uuid",
          "name": "message",
          "parts": [{"text": "Did the API schema change?"}],
          "metadata": {
            "fromAgentId": "agentA-uuid",
            "fromAgentName": "Agent A",
            "type": "unicast"
          }
        }
      ],
      "metadata": {
        "fromAgentId": "agentA-uuid",
        "toAgentId": "agentB-uuid",
        "type": "unicast"
      }
    }
  },
  "id": "req-1"
}
```

### Broadcast — Send

1. Agent A calls `SendMessage` with `metadata.destination = "*"`
2. Broker creates N delivery Tasks (one per active agent, excluding sender), each with `contextId = recipient_agent_id`
3. Broker returns a summary Task to Agent A (state=`COMPLETED`) with Artifact listing `recipientCount` and `deliveryTaskIds`

**Response** (summary Task returned):

```json
{
  "jsonrpc": "2.0",
  "result": {
    "task": {
      "id": "task-broadcast-uuid",
      "contextId": "broadcast-ctx-uuid",
      "status": {
        "state": "TASK_STATE_COMPLETED",
        "timestamp": "2026-03-28T12:00:00.000Z"
      },
      "artifacts": [
        {
          "artifactId": "broadcast-receipt-uuid",
          "name": "broadcast_receipt",
          "parts": [
            {
              "data": {
                "recipientCount": 5,
                "deliveryTaskIds": ["task-1", "task-2", "task-3", "task-4", "task-5"]
              },
              "mediaType": "application/json"
            }
          ]
        }
      ],
      "metadata": {
        "fromAgentId": "agentA-uuid",
        "type": "broadcast"
      }
    }
  },
  "id": "req-2"
}
```

### Broadcast — Receive & ACK

Same as unicast — each recipient independently discovers and ACKs their own delivery Task.

## SendMessage — Acknowledge (Multi-Turn)

The recipient ACKs by sending a follow-up message referencing the delivery Task's `taskId`:

```json
{
  "jsonrpc": "2.0",
  "method": "SendMessage",
  "params": {
    "message": {
      "messageId": "ack-uuid-1",
      "role": "ROLE_USER",
      "taskId": "task-delivery-uuid",
      "parts": [
        {"text": "ack"}
      ]
    }
  },
  "id": "req-3"
}
```

**Behavior**: Broker validates that the authenticated agent is the recipient of this Task, then moves it to `TASK_STATE_COMPLETED`. Returns the updated Task.

The ACK message content (parts) is optional — an empty parts array or a simple `"ack"` text suffices. The Broker records the ACK in the Task's history.

## ListTasks — Poll Inbox

Recipients discover unread messages by calling `ListTasks` with their `agent_id` as `contextId`:

```json
{
  "jsonrpc": "2.0",
  "method": "ListTasks",
  "params": {
    "contextId": "agentB-uuid",
    "status": "TASK_STATE_INPUT_REQUIRED",
    "includeArtifacts": true,
    "pageSize": 20
  },
  "id": "req-4"
}
```

**Response**: Standard A2A `ListTasksResponse` with Tasks containing message Artifacts.

The `statusTimestampAfter` parameter can be used for efficient delta polling — only fetch tasks updated since the last poll.

## GetTask — Read Specific Message

Either sender or recipient can read a specific Task by ID:

```json
{
  "jsonrpc": "2.0",
  "method": "GetTask",
  "params": {
    "id": "task-delivery-uuid"
  },
  "id": "req-5"
}
```

**Visibility**: Sender can access tasks they created (known taskId). Recipient can access tasks in their contextId. Others get `TaskNotFoundError`.

## CancelTask — Retract Message

Sender can retract an unread message:

```json
{
  "jsonrpc": "2.0",
  "method": "CancelTask",
  "params": {
    "id": "task-delivery-uuid"
  },
  "id": "req-6"
}
```

**Behavior**: Only the sender can cancel. Only tasks in `INPUT_REQUIRED` state can be canceled. Returns `TaskNotCancelableError` (JSON-RPC code `-32002`) if the task is already completed or canceled.

## Message Lifecycle

### Task State Mapping

| Task State | Message Meaning |
|---|---|
| `TASK_STATE_INPUT_REQUIRED` | Message queued, awaiting recipient pickup (unread) |
| `TASK_STATE_COMPLETED` | Message acknowledged by recipient |
| `TASK_STATE_CANCELED` | Message retracted by sender before ACK |
| `TASK_STATE_FAILED` | Routing error (returned immediately to sender) |

### Unicast Flow

1. Sender calls `SendMessage` with `metadata.destination = "<agent-uuid>"`
2. Broker creates Task (`INPUT_REQUIRED`), returns to sender
3. Recipient calls `ListTasks(contextId=own_id, status=INPUT_REQUIRED)`
4. Recipient reads message from Task artifacts
5. Recipient calls `SendMessage(taskId=existing)` to ACK
6. Broker moves Task to `COMPLETED`

### Broadcast Flow

1. Sender calls `SendMessage` with `metadata.destination = "*"`
2. Broker creates N delivery Tasks (one per active agent excluding sender)
3. Broker returns summary Task (`COMPLETED`) to sender
4. Each recipient independently discovers and ACKs their delivery Task

## Error Cases

| Condition | JSON-RPC Error | Code |
|---|---|---|
| Missing `Authorization` header | HTTP 401 before JSON-RPC processing | N/A |
| Invalid API key | HTTP 401 before JSON-RPC processing | N/A |
| Missing `metadata.destination` | `InvalidParams`: "metadata.destination is required" | `-32602` |
| `destination` is not a valid UUID or `"*"` | `InvalidParams`: "invalid destination format" | `-32602` |
| Destination agent not found | `InvalidParams`: "destination agent not found" | `-32602` |
| Destination agent is deregistered | `InvalidParams`: "destination agent is deregistered" | `-32602` |
| ACK on task not owned by caller | `TaskNotFoundError` | `-32001` |
| ACK on task in terminal state (completed/canceled) | `UnsupportedOperationError`: "task is in terminal state" | `-32004` |
| GetTask/CancelTask on unknown or unauthorized task | `TaskNotFoundError` | `-32001` |
