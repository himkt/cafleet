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
Operators run plain `cafleet setup` once before starting the server — it is
the migrations-apply path. Existing data (message
history included) is preserved, so the command is idempotent and safe to
re-run after every upgrade.

| Database state | What `cafleet setup` does |
|---|---|
| Behind the bundled head | Migrates in place to the bundled head revision |
| Already at the bundled head | Nothing to apply |
| Ahead of the bundled head | Refuses to auto-downgrade |
| Unversioned, with tables it does not recognize | Refuses |

Every non-setup command — the fleet-scoped groups, `monitor`, and `server` —
checks the schema version before running: a missing or behind-head database
fails with guidance naming `cafleet setup`, and a database newer than the
CLI with an upgrade prompt — never a raw SQLite error. `doctor` reports the
same states instead of blocking. The exact error strings are in
[CLI options](../spec/cli-options.md#schema-version-guard).

## Assets-install recording

The `asset_installs` table records, per coding agent and install path, the
CLI version that last installed the skills and preset
(where one exists) there — **not** a schema version. The assets
half of `cafleet setup` upserts one row per installed agent — all three
agents on the no-flag form — keyed on the
agent's resolved config path. Every fleet-scoped command (`fleet *`,
`member *`, `message *`, `monitor *`) checks each agent's row at its
currently-resolved path before running and hard-errors when no agent has a
row at its resolved path or when a row at a resolved path differs from the
running CLI version — so the assets can never silently go stale after a CLI
upgrade; records at other paths never block a command.
`cafleet doctor` reports the per-agent detail. See
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
