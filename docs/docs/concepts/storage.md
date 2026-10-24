# Storage

Use one SQLite file for a fleet's registry and durable messages. The CLI accesses it synchronously with bundled SQLite.

## Backend

The default path is `~/.local/share/cafleet/cafleet_v8.db`. Set `CAFLEET_DATABASE_URL` to a `sqlite:///<path>` URL to select another file. Resolve a relative file path against the invoking working directory, then canonicalize the selected database file for shared pane-lock identity. Spawned panes inherit that URL.

Connections enforce foreign keys and normally use `PRAGMA busy_timeout=5000`. Bounded receiving caps database waits to its remaining operation budget. Fleet bootstrap writes its registry and current Director placement in one transaction; member creation preserves pending placement and compensation for failed pane creation.

## Relational model

Typed columns store every routing field and required enum. The [data model](../spec/data-model.md) owns the table definitions. A missing required Director, placement, column or valid enum fails at the read boundary.

## Schema management

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

The current format requires `PRAGMA application_id = 1128351288` (`CAF8`), a compatible embedded migration ledger and the expected table/column shapes. An incompatible selected file is preserved unchanged: diagnosis runs before database content writes or either setup half installs assets.

| Selected state | Setup behavior |
|---|---|
| Empty SQLite file with identity zero and no user tables | Apply the embedded baseline transactionally |
| Valid current format at the bundled head | Proceed idempotently |
| Current format behind the bundled head | Apply known migrations |
| Current format ahead of the bundled head | Reject both setup write paths before database writes, asset installation or asset-install recording |
| Other populated/identified format or malformed current schema | Report incompatible format before writes |

Run [`cafleet setup`](../spec/cli-options.md#cafleet-setup) after installation. The same classifier guards normal commands and feeds doctor's read-only diagnosis. Exact errors are in the [schema guard](../spec/cli-options.md#schema-version-guard).

## Assets-install recording

`asset_installs` records the CLI version for each coding-agent/config-path pair. Setup upserts a record after installing embedded skills and any preset for that backend. The [stale-assets guard](../spec/cli-options.md#stale-assets-guard) checks recorded versions before fleet-scoped operations. Operator customizations belong outside setup-owned assets.

## Retention and pane locks

Deregistration and fleet deletion retain message history while active query paths filter the registry state. Input ownership uses stable lockfiles adjacent to the canonical database file, as specified by [pane input](../spec/multiplexer-backends.md#pane-input-lock). Keep those files in place while processes may use the database.
