# Data model

The broker uses typed SQLite columns with per-connection foreign-key enforcement. The selected file must pass current-format diagnosis before writes.

## Tables

| Table | Responsibility |
|---|---|
| `fleets` | Fleet identity, creation/deletion and root Director reference |
| `members` | Member registration and active/deregistered state |
| `member_placements` | Multiplexer and coding-agent association |
| `messages` | Point-to-point delivery, acknowledgement and preview stamp |
| `runtime_database_identity` | Singleton database-instance identity for runtime namespaces |
| `member_runs` | Launch intent, ownership, lifecycle and recorded coverage |
| `member_interactions` | Observed interactive episodes and visibility closure |
| `member_runtime_events` | Sequenced events with atomic Director inbox association or archival |
| `member_input_operations` | Durable submission receipts and settled replay protection |
| `member_permission_responses` | Immutable attributed decisions tied to input/event/interaction identity |
| `member_permission_response_claims` | One possibly transmitted answer per observed episode |
| `asset_installs` | Installed asset versions by backend and config path |
| `refinery_schema_history` | Embedded migration ledger |

### fleets

`fleet_id` is an autoincrement integer primary key; `name` is nullable text; `created_at` is required text; `deleted_at` is nullable text; `director_member_id` references members with delete restricted. A live fleet requires an active valid Director with placement. Bootstrap inserts fleet, Director and placement and backfills the Director reference before one commit.

### members

Columns are `member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT`, `fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT`, required text `name`, `description`, `status`, `registered_at`, and nullable text `deregistered_at`. Index `idx_members_fleet_status` covers fleet/status. Decode `status` as `active` or `deregistered`; unknown values fail. Public `kind` is `director` exactly when the fleet's Director reference matches, otherwise `member`.

### member_placements

`member_id` is the primary key and references members with cascade delete. Required text columns are `mux_session`, `mux_window_id`, `backend` (default `tmux`), `coding_agent` and `created_at`. `mux_pane_id` is nullable during pending member creation. A concrete placement selects the shared pane-input lock; duplicate entries targeting the same backend/session/pane share ownership.

### messages

Required columns are `message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT`, `from_member_id` and `to_member_id` integer references to members with delete restricted, and text `created_at`, `status_state`, `status_timestamp`, `text`. `notified_at` is nullable text. Indexes cover `(to_member_id, status_timestamp)` and `(from_member_id, status_timestamp)`.

`status_state` decodes as `input_required` or `completed`. Pending notifications require an active ordinary recipient with a concrete pane, an unacknowledged non-self row and null `notified_at`. Exclude the root Director before notification selection. Select the oldest pending message this invocation has not attempted, ordered by creation time/message ID, and revalidate eligibility and placement under the pane lock.

Explicitly unwrapped delivery is at-least-once: write no notification state before input, and stamp `notified_at` only after complete submission while the message remains pending and the pane lock is held. A message ACKed before the stamp counts as notified. A per-attempt failure leaves `notified_at` null and ends the drain with the [not-notified diagnostic](cli-options.md#message-send-delivery). The next send to that recipient or a later wait retries it; partial or complete input may therefore appear twice. One send or wait invocation attempts each pending message at most once. Every line carries the message ID, and poll plus ACK remain authoritative. Delivery leaves `status_state` unchanged: a notified message stays `input_required` until its recipient ACKs it. The public [message record](message-envelope.md#persisted-shape) omits internal `notified_at` state.

The read-only [inbox stream](cli-options.md#message-listen) selects pending records independently of `notified_at` and changes neither notification nor consumption state. Self-sends stamp `notified_at` on insertion, produce no pane input and remain pending for reception and explicit ACK. Native handles and emitted-ID tracking belong to the receiver process/host, with no schema or durable consumer state.

### Runtime records

The [runtime persistence contract](member-runtime.md#persistence-notification-and-read-interfaces) owns exact runtime fields, enum checks, nullability and ordered event payloads. Runtime database identity is initialized once by the next contiguous current-family migration. Run/member/fleet/Director references preserve history after soft deregistration; a partial unique index admits at most one nonterminal run per member, and another admits at most one open interaction per run.

Runtime event insertion validates ownership and commits the transition, event and full Director inbox record atomically. A run/sequence retry must match the original immutable transition; generated IDs and commit time are excluded from that comparison. Recovery after member/fleet/Director closure archives historical observations without inventing an active-sender notification.

Wrapped previews use durable operation keys and receipt settlement before cache retirement; accepted delivery can settle its stamp without typing again. The [input contract](member-runtime.md#input-arbitration-and-the-original-native-answer) owns uncertainty, disposition and replay rejection. Permission responses add an immutable audit and episode claim in the same receipt transaction. The [response storage contract](member-runtime.md#permission-response-storage) owns exact V5 SQL fields, constraints, digest inputs and zero-byte claim release; response key submission establishes no native approval or command result. The public message envelope and ACK transition remain unchanged.

### asset_installs

Required text columns are `coding_agent`, `path`, `cafleet_version` and `installed_at`; the first two form the primary key. Each successful backend asset installation upserts its CLI version and timestamp. Records track assets independently of member registration.

## Message visibility rules

| Operation | Selection or transition |
|---|---|
| Send | Active sender and recipient in the same live fleet; persist one exact body |
| Poll / wait | Unacknowledged incoming deliveries by descending status timestamp, then message ID |
| Show | One message by its global ID |
| ACK | Transition an existing `input_required` row to `completed` and stamp consumption |
| Fan-out | The Director sends individually, retaining each committed ID and reporting each recipient failure |

Consumed questions and deferred approvals are ACKed before the Director ends its turn; the Director retains unfinished actions in host-owned state.

## Query and activity contracts

Roster/list queries filter active members in live fleets. Activity is the maximum sent, received and ACK timestamp; `idle` is elapsed seconds from the latest known activity or null when none exists. Detailed views retain registration/description/placement data. Fleet soft deletion deregisters members and retains messages; member deletion protects the root Director. Placement mutations acquire the same input locks as writers.

## Format and connection invariants

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Schedule schema upgrades between active command invocations; internal message writes reuse the invocation's startup-validated connection. The current identity is `PRAGMA application_id = 1128351288`; the baseline ledger is version 1 in this format family. Require the recorded supported version's expected table/column shapes in addition to the identity. Empty zero-identity files initialize transactionally; incompatible populated or identified files remain byte-preserved. [Storage](../concepts/storage.md) explains operation and the [CLI guard](cli-options.md#schema-version-guard) owns exact errors.
