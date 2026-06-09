---
icon: lucide/database
---

# Storage

## Backend

Everything is persisted in a single SQLite database accessed through
SQLAlchemy 2.x with the sync `pysqlite` driver. Schema changes are managed
by Alembic, bundled inside the `cafleet` wheel and applied via
`cafleet db init`. There is no separate database daemon to operate, monitor,
or back up — the database is a single file.

The default database path is `~/.local/share/cafleet/cafleet.db` (XDG state
directory), expanded once at config load time. Override with the
`CAFLEET_DATABASE_URL` environment variable, e.g.
`sqlite:////var/lib/cafleet/cafleet.db`.

**Concurrency**: `PRAGMA busy_timeout=5000` is set on every connection.
SQLite retries internally for up to 5 seconds before returning `SQLITE_BUSY`.
Expected contention is low — CLI operations are short transactions (single
INSERT or UPDATE), and multiple agents polling concurrently is read-only.

## Relational model

Indexed and routing fields are typed columns; the only JSON `TEXT` blob is
`agents.agent_card_json` (an `AgentCard`-shaped document, not queried by
content). See [data model](../spec/data-model.md) for the full schema —
tables, columns, indexes, foreign keys, and the ER diagram.

## Schema management

The schema is created by a single initial Alembic migration. Operators run
`cafleet db init` once before starting the server; it is idempotent and safe
to re-run after upgrades, refuses to auto-downgrade a database that is ahead
of the bundled head, and refuses an unversioned database with tables it does
not recognize. Without `db init`, the first request fails with
`OperationalError: no such table: agents`.

## No physical cleanup

Deregistered agents and their tasks remain in the database forever. There is
no background cleanup loop. Active query paths filter `status='active'` so
dead rows are invisible to normal traffic; the WebUI is the only consumer
that surfaces deregistered agents (so their inbox history can be inspected).

## contextId convention

The broker sets `contextId = recipient_agent_id` on every delivery Task.
This enables inbox discovery — recipients poll for messages whose
`context_id` equals their own agent id to find everything addressed to them.
This trades per-conversation grouping (the typical contextId use case) for
simple inbox discovery, which suits the fire-and-forget messaging pattern of
coding agents. `contextId` is treated as an opaque routing key.
