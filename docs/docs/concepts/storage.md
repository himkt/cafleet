---
icon: lucide/database
---

# Storage

## Backend

Everything is persisted in a single SQLite database accessed synchronously,
with SQLite bundled into the `cafleet` binary. There is no separate database
daemon to operate, monitor, or back up — the database is a single file.

The default database path is `~/.local/share/cafleet/cafleet_v6.db` (XDG state
directory), expanded once at config load time. Override with the
`CAFLEET_DATABASE_URL` environment variable, e.g.
`sqlite:////var/lib/cafleet/cafleet_v6.db`; see
[CLI options](../spec/cli-options.md) for the full `CAFLEET_*` variable set.

**Concurrency**: `PRAGMA busy_timeout=5000` lets SQLite retry for up to 5 s
before returning `SQLITE_BUSY`; contention is low because CLI operations are
short single-statement transactions and concurrent polling is read-only.

## Relational model

Every routing and indexed field is a typed column; the only JSON `TEXT` blob is
`members.member_card_json`. See [data model](../spec/data-model.md) for the full
schema.

## Schema management

The schema is managed by a **chain of SQL migrations embedded in the binary**;
the applied versions are recorded in the `refinery_schema_history` table.
Operators run
`cafleet setup` (schema-only: `cafleet setup --skip claude --skip codex
--skip opencode`) once before starting the server. Existing data (message
history included) is preserved, so the command is idempotent and safe to
re-run after every upgrade.

| Database state | What `cafleet setup` does |
|---|---|
| Behind the bundled head | Migrates in place to the bundled head revision |
| Already at the bundled head | Nothing to apply |
| Ahead of the bundled head | Refuses to auto-downgrade |
| Unversioned, with tables it does not recognize | Refuses |

Without the schema, the first request fails with
`OperationalError: no such table: members`.

## Assets-install recording

The `asset_installs` table records, per
coding agent, the CLI version that last installed the skills and preset
(where one exists) there — **not** a schema version. The assets
half of `cafleet setup`
upserts one row per target agent after that agent's install succeeds. Every fleet-scoped command (`fleet *`,
`member *`, `message *`, `monitor *`) checks the recorded rows before running
and hard-errors when no install is recorded or when any recorded version
differs from the running CLI version — so the assets can never silently go
stale after a CLI upgrade. `cafleet doctor` reports the per-agent detail. See
[data model](../spec/data-model.md) for the table schema and
[CLI options](../spec/cli-options.md) for the guard's error strings.

## No physical cleanup

Deregistered members and their messages remain in the database forever. There is
no background cleanup loop. Active query paths filter `status='active'` so
dead rows are invisible to normal traffic; the WebUI is the only consumer
that surfaces deregistered members (so their inbox history can be inspected).

## owner_member_id convention

The broker sets `owner_member_id = recipient_member_id` on every delivery
message, so recipients discover their inbox by polling for messages whose
`owner_member_id` equals their own member id — trading per-conversation
grouping for the simple fire-and-forget inbox discovery that suits coding
agents. `owner_member_id` is an opaque routing key.
