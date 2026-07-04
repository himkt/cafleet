---
icon: lucide/database
---

# Storage

## Backend

Everything is persisted in a single SQLite database accessed through
SQLAlchemy 2.x with the sync `pysqlite` driver. The schema is a single baseline
created in one pass by `cafleet setup`. There is no separate database daemon to
operate, monitor, or back up — the database is a single file.

The default database path is `~/.local/share/cafleet/cafleet.db` (XDG state
directory), expanded once at config load time. Override with the
`CAFLEET_DATABASE_URL` environment variable, e.g.
`sqlite:////var/lib/cafleet/cafleet.db`; see [config](../api/config.md) for the
full `CAFLEET_*` variable set.

**Concurrency**: `PRAGMA busy_timeout=5000` lets SQLite retry for up to 5 s
before returning `SQLITE_BUSY`; contention is low because CLI operations are
short single-statement transactions and concurrent polling is read-only.

## Relational model

Every routing and indexed field is a typed column; the only JSON `TEXT` blob is
`agents.agent_card_json`. See [data model](../spec/data-model.md) for the full
schema.

## Schema management

The schema is a **single baseline** — one ordered `CREATE` sequence that yields
the final schema directly, with no migration chain, no schema-version table,
and no in-place migration machinery. Operators run `cafleet setup` (or its
schema-only subcommand `cafleet setup db`) once before starting the server; the
create uses `CREATE TABLE IF NOT EXISTS`, so it is idempotent and **additive**:
re-running adds any missing tables and touches nothing else. Pre-existing
tables and every row in them are left untouched, so upgrading preserves
existing data (message history included). Tables whose shape predates the
current baseline are not migrated — deleting the database file and re-running
`cafleet setup` is the last resort for those, and it discards the history.
Without the schema, the first request fails with
`OperationalError: no such table: agents`.

## Skills-install recording

The `skill_installs` table records, per coding-agent home, the CLI version
that last installed the skills there — **not** a schema version. The skills
half of `cafleet setup` (and `cafleet setup skill`) upserts one row per home
after that home's install succeeds. Every fleet-scoped command (`fleet *`,
`member *`, `message *`, `monitor *`) checks the recorded rows before running
and hard-errors when no install is recorded or when any recorded version
differs from the running CLI version — so skills can never silently go stale
after a CLI upgrade. `cafleet doctor` reports the per-home detail. See
[data model](../spec/data-model.md) for the table schema and
[CLI options](../spec/cli-options.md) for the guard's error strings.

## No physical cleanup

Deregistered agents and their tasks remain in the database forever. There is
no background cleanup loop. Active query paths filter `status='active'` so
dead rows are invisible to normal traffic; the WebUI is the only consumer
that surfaces deregistered agents (so their inbox history can be inspected).

## contextId convention

The broker sets `contextId = recipient_agent_id` on every delivery Task, so
recipients discover their inbox by polling for tasks whose `context_id` equals
their own agent id — trading per-conversation grouping for the simple
fire-and-forget inbox discovery that suits coding agents. `contextId` is an
opaque routing key.
