# Storage

Use one SQLite file for a fleet's registry and durable messages. The CLI accesses it synchronously with bundled SQLite.

## Backend

The default path is `~/.local/share/cafleet/cafleet_<VERSION>.db`, where `<VERSION>` is the package version (the version field of `cafleet --version`, e.g. `0.31.1`). Pass `--database-url sqlite:///<path>` on a command to select another file; a relative path resolves against the invoking working directory, and the selected file is then canonicalized for shared pane-lock identity. `member create` renders a non-default database into the spawn prompt's `DATABASE URL:` line, and the member passes that URL on every command; for a non-default database, a prompt without the `{database_url}` placeholder is refused ([database selection](https://github.com/himkt/cafleet/blob/main/SPEC.md#database-selection)).

Connections enforce foreign keys and normally use `PRAGMA busy_timeout=5000`; bounded receiving caps database waits to its remaining operation budget. Fleet creation writes its registry and current Director placement in one transaction, and [Members](members.md#registration-and-readiness) covers failed member creation.

## Relational model

Typed columns store every routing field and required enum. The [database schema](https://github.com/himkt/cafleet/blob/main/SPEC.md#8-database-schema) owns the table definitions. A missing required Director, placement, column or valid enum fails at the read boundary.

## Schema management

A compatible file has CAFleet's application identity, a ledger holding only the embedded schema baseline, the cafleet version that initialized it equal to the running CLI's, and the expected table/column shapes; unrelated auxiliary tables, indexes and triggers stay compatible. An incompatible selected file is preserved unchanged: diagnosis runs before any database content write or asset installation.

| Selected state | Setup behavior |
|---|---|
| Missing file, or an empty SQLite file with identity zero and no user tables | Apply the embedded baseline transactionally |
| Valid file of this version | Proceed idempotently |
| Other populated/identified file, another version's file or malformed schema | Report incompatible format before writes |

Run `cafleet setup` after installation. The same classifier guards normal commands and feeds doctor's read-only diagnosis; the [schema-version guard](https://github.com/himkt/cafleet/blob/main/SPEC.md#schema-version-guard) has the exact errors.

### Upgrading cafleet

Each cafleet version starts from a fresh database, so no fleets, messages or runtime history carry across an upgrade:

1. Finish and delete your active fleets before upgrading.
2. After upgrading, run `cafleet setup` once. It creates the version's `~/.local/share/cafleet/cafleet_<VERSION>.db` and records the asset installations there; until then `cafleet doctor` reports the database as absent.
3. Create your fleets anew.

Database files of earlier versions in `~/.local/share/cafleet/` stay in place, byte for byte; delete them by hand when you no longer need them. With an explicit `--database-url`, point it at a new empty path, or delete the old file by hand, then run `cafleet setup`.

## Assets-install recording

`asset_installs` records the CLI version for each coding-agent/config-path pair. Setup upserts it after installing that backend's embedded skills and any preset or plugin, and `cafleet doctor` reports the recorded asset version at each backend's resolved path. Operator customizations belong outside setup-owned assets.

## Retention and pane locks

Deregistration and fleet deletion retain message history while active query paths filter the registry state. Input ownership uses stable lockfiles beside the canonical database file ([pane input lock](https://github.com/himkt/cafleet/blob/main/SPEC.md#pane-input-lock)); keep them in place while processes may use the database.

## Member runtime artifacts

Member runners keep a private runtime namespace beside the canonical database, separate for each database file and instance; replacing the database file at the same pathname fences old runners. The [runner boundary contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#runner-boundary-and-creation-transaction) owns paths, permissions, validation and runtime-record recovery.

Terminal transport continues independently of database contention. Events become durable at SQLite commit; a flushed outbox frame is recoverable pending delivery. Metadata evidence stores no screen-derived text or hidden-text hash; explicit redacted evidence applies the configured policy before every persisted field. The [storage pressure contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#storage-pressure-and-durability-boundary) defines bounded pressure and exclusive recovery.
