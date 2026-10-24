# Data model

The broker uses typed SQLite columns with per-connection foreign-key enforcement. The selected file must pass current-format diagnosis before writes.

## Tables

| Table | Responsibility |
|---|---|
| `fleets` | Fleet identity, creation/deletion and root Director reference |
| `members` | Member registration and active/deregistered state |
| `member_placements` | Multiplexer and coding-agent association |
| `messages` | Point-to-point delivery, acknowledgement and preview stamp |
| `asset_installs` | Installed asset versions by backend and config path |
| `refinery_schema_history` | Embedded migration ledger |

### fleets

`fleet_id` is an autoincrement integer primary key; `name` is nullable text; `created_at` is required text; `deleted_at` is nullable text; `director_member_id` references members with delete restricted. A live fleet requires an active valid Director with placement. Bootstrap inserts fleet, Director and placement and backfills the Director reference before one commit.

### members

Columns are `member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT`, `fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT`, required text `name`, `description`, `status`, `registered_at`, and nullable text `deregistered_at`. Index `idx_members_fleet_status` covers fleet/status. Decode `status` as `active` or `deregistered`; unknown values fail. Public `kind` is `director` exactly when the fleet's Director reference matches, otherwise `member`.

### member_placements

`member_id` is the primary key and references members with cascade delete. Required text columns are `mux_session`, `mux_window_id`, `backend` (default `tmux`), `coding_agent` and `created_at`. `mux_pane_id` is nullable during pending member creation. A concrete placement selects the shared pane-input lock; duplicate entries targeting the same backend/session/pane share ownership.

### messages

Required columns are `message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT`, `from_member_id` and `to_member_id` integer references to members with delete restricted, and text `created_at`, `status_state`, `status_timestamp`, `text`. `notified_at` and `preview_started_at` are nullable text. Indexes cover `(to_member_id, status_timestamp)` and `(from_member_id, status_timestamp)`.

`status_state` decodes as `input_required` or `completed`. Pending previews require an active recipient with a concrete pane, the Director included, an unacknowledged non-self row and null `notified_at`. Select automatic input from rows with null `preview_started_at`, ordered by creation time/message ID; retain claimed rows as diagnostics while later messages remain eligible. Persist `preview_started_at` immediately before possible input under the pane lock. Retry its release after a confirmed no-input failure within the absolute completion budget, retaining `not_sent` and the storage cause on failure. Preserve unresolved claims across process restarts so their exact messages remain suspended. Stamp `notified_at` only after successful complete input while holding the pane lock and revalidating eligibility. Delivery leaves `status_state` unchanged: a delivered message stays `input_required` until the recipient ACKs it. Every message drained in one lock hold follows the same claim and stamp rules. The public [message record](message-envelope.md#persisted-shape) omits both internal notification fields.

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

Roster/list queries filter active members in live fleets. Activity is the maximum sent, received and ACK timestamp; `idle` is elapsed seconds from the latest known activity or null when none exists. `unconfirmed_previews` selects, per member and ordered by creation time/message ID, the incoming `input_required` messages with non-null `preview_started_at` and null `notified_at`. Detailed views retain registration/description/placement data. Fleet soft deletion deregisters members and retains messages; member deletion protects the root Director. Placement mutations acquire the same input locks as writers.

## Format and connection invariants

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Schedule schema upgrades between active command invocations; internal message writes reuse the invocation's startup-validated connection. The current identity is `PRAGMA application_id = 1128351288`; the baseline ledger is version 1 in this format family. Require the recorded supported version's expected table/column shapes in addition to the identity. Empty zero-identity files initialize transactionally; incompatible populated or identified files remain byte-preserved. [Storage](../concepts/storage.md) explains operation and the [CLI guard](cli-options.md#schema-version-guard) owns exact errors.
