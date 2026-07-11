---
icon: lucide/database
---

# Storage

## Backend

Everything is persisted in a single SQLite database accessed through
SQLAlchemy 2.x with the sync `pysqlite` driver. Schema changes are managed
by Alembic, bundled inside the `cafleet` wheel and applied via
`cafleet setup` (or its schema-only subcommand `cafleet setup db`). There is
no separate database daemon to operate, monitor, or back up — the database is
a single file.

The default database path is `~/.local/share/cafleet/cafleet_v3.db` (XDG state
directory), expanded once at config load time. Override with the
`CAFLEET_DATABASE_URL` environment variable, e.g.
`sqlite:////var/lib/cafleet/cafleet_v3.db`; see [config](../api/config.md) for the
full `CAFLEET_*` variable set.

**Concurrency**: `PRAGMA busy_timeout=5000` lets SQLite retry for up to 5 s
before returning `SQLITE_BUSY`; contention is low because CLI operations are
short single-statement transactions and concurrent polling is read-only.

## Relational model

Every routing and indexed field is a typed column; the only JSON `TEXT` blob is
`members.member_card_json`. See [data model](../spec/data-model.md) for the full
schema.

## Schema management

The schema is managed by a **chain of Alembic migrations**; the current
revision is recorded in the `alembic_version` table. Operators run
`cafleet setup` (or its schema-only subcommand `cafleet setup db`) once before
starting the server; it migrates the database in place to the bundled head
revision, preserving existing data (message history included), so it is
idempotent and safe to re-run after every upgrade. It refuses to
auto-downgrade a database that is ahead of the bundled head, and refuses an
unversioned database with tables it does not recognize. Without the schema,
the first request fails with `OperationalError: no such table: members`.

## Skills-install recording

The `skill_installs` table records, per
coding-agent home, the CLI version that last installed the skills there —
**not** a schema version. The skills
half of `cafleet setup` (and `cafleet setup skill`) upserts one row per home
after that home's install succeeds. Every fleet-scoped command (`fleet *`,
`member *`, `message *`, `monitor *`) checks the recorded rows before running
and hard-errors when no install is recorded or when any recorded version
differs from the running CLI version — so skills can never silently go stale
after a CLI upgrade. `cafleet doctor` reports the per-home detail. See
[data model](../spec/data-model.md) for the table schema and
[CLI options](../spec/cli-options.md) for the guard's error strings.

## No physical cleanup

Deregistered members and their tasks remain in the database forever. There is
no background cleanup loop. Active query paths filter `status='active'` so
dead rows are invisible to normal traffic; the WebUI is the only consumer
that surfaces deregistered members (so their inbox history can be inspected).

## contextId convention

The broker sets `contextId = recipient_member_id` on every delivery Task, so
recipients discover their inbox by polling for tasks whose `context_id` equals
their own member id — trading per-conversation grouping for the simple
fire-and-forget inbox discovery that suits coding agents. `contextId` is an
opaque routing key.
