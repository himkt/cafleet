# Storage

Use one SQLite file for a fleet's registry and durable messages. The CLI accesses it synchronously with bundled SQLite.

## Backend

The default path is `~/.local/share/cafleet/cafleet_v8.db`. Set `CAFLEET_DATABASE_URL` to a `sqlite:///<path>` URL to select another file. Resolve a relative file path against the invoking working directory, then canonicalize the selected database file for shared pane-lock identity. Spawned panes inherit that URL.

Connections enforce foreign keys and normally use `PRAGMA busy_timeout=5000`. Bounded receiving caps database waits to its remaining operation budget. Fleet bootstrap writes its registry and current Director placement in one transaction; member creation preserves pending placement and compensation for failed pane creation.

## Relational model

Typed columns store every routing field and required enum. The [database schema](https://github.com/himkt/cafleet/blob/main/SPEC.md#8-database-schema) owns the table definitions. A missing required Director, placement, column or valid enum fails at the read boundary.

## Schema management

The current format has its own application identity, an embedded migration ledger and the expected table/column shapes; unrelated auxiliary tables, indexes and triggers stay compatible. An incompatible selected file is preserved unchanged: diagnosis runs before database content writes or setup installs assets.

| Selected state | Setup behavior |
|---|---|
| Empty SQLite file with identity zero and no user tables | Apply the embedded baseline transactionally |
| Valid current format at the bundled head | Proceed idempotently |
| Current format behind the bundled head | Apply known migrations |
| Current format ahead of the bundled head | Fail before database writes, asset installation or asset-install recording |
| Other populated/identified format or malformed current schema | Report incompatible format before writes |

Schema checks run within a consistent read snapshot, and application identity and ledger contents are read on every validation, so a changed version or malformed definition fails at the boundary.

Run `cafleet setup` after installation. The same classifier guards normal commands and feeds doctor's read-only diagnosis. Exact errors are in the [schema-version guard](https://github.com/himkt/cafleet/blob/main/SPEC.md#schema-version-guard).

## Assets-install recording

`asset_installs` records the CLI version for each coding-agent/config-path pair. Setup upserts a record after installing embedded skills and any preset for that backend, and `cafleet doctor` reports the recorded asset version at each backend's resolved path. Operator customizations belong outside setup-owned assets.

## Retention and pane locks

Deregistration and fleet deletion retain message history while active query paths filter the registry state. Input ownership uses stable lockfiles adjacent to the canonical database file, as specified by the [pane input lock](https://github.com/himkt/cafleet/blob/main/SPEC.md#pane-input-lock). Keep those files in place while processes may use the database.

## Member runtime artifacts

A private runtime namespace beside the canonical database separates each database file and instance. Replacement at the same pathname fences old runners; recorded identity and ownership govern every recovery attempt. Private descriptors are removed after launch/compensation, while event outboxes and settled input records retain their recovery roles. The [runner boundary contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#runner-boundary-and-creation-transaction) owns paths, permissions and validation.

Terminal transport continues independently of database contention. Events become durable at SQLite commit; a flushed outbox frame is recoverable pending delivery. Metadata evidence stores no screen-derived text or hidden-text hash; explicit redacted evidence applies the configured policy before every persisted field. The [storage pressure contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#storage-pressure-and-durability-boundary) defines bounded pressure and exclusive recovery.
