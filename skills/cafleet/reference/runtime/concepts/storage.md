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
before returning `SQLITE_BUSY`. Member registration uses an `IMMEDIATE`
transaction to serialize writers, including the active-monitor recheck and
member/placement inserts. Polling is read-only. Fleet bootstrap holds its
write transaction across pane creation; its lock can therefore last for the
multiplexer call.

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
| Pending migration with duplicate active monitors | Refuses with the conflicting fleet and member ids; preserves existing rows and schema history |
| Ahead of the bundled head | Refuses to auto-downgrade |
| Unversioned, with tables it does not recognize | Refuses |

Every non-setup command checks the schema version before running and fails
with guidance naming `cafleet setup` — never a raw SQLite error; `doctor`
reports instead of blocking. The exact rules and error strings are in
[CLI options](../spec/cli-options.md#schema-version-guard).

Pending migrations run together in one grouped transaction. A failed index
creation rolls back the entire pending group, including its schema-history
entries. Before migrating, after refusing unversioned or newer schemas,
`setup` checks existing members for duplicate active monitors using the
[index predicate](../spec/data-model.md#members). A fresh database without a
members table skips this check. It reports conflicting fleets and their
member ids in ascending order:

```text
active monitor duplicates prevent migration: fleet <id>: members <ids>; ...
```

No member or pane is selected or removed automatically. If a writer introduces
a duplicate after the diagnostic, index creation still rejects it. Following
a migration failure, a successful recheck that finds duplicates reports the
same diagnostic; otherwise the original migration error is retained.

### Recovering duplicate active monitors {#duplicate-monitor-recovery}

1. Stop new registrations against the affected database. Choose the monitor
   to retain from the reported ids.
2. Use the preceding release that supports the old schema, from a separate
   binary path, with the same `CAFLEET_DATABASE_URL` and backend configuration
   directories. The new binary's `member delete` is blocked by its
   behind-schema guard.
3. The failed new `setup` may have successfully updated assets: its DB and
   assets halves run independently. If so, use the old binary's `setup` for
   the same backend(s) to restore assets compatible with that binary. Then
   use the old binary's `member delete <surplus-id>`, one isolated invocation
   per surplus monitor. The database still has the old schema, so these old
   commands can pass its schema guard.
4. Run the new binary's `setup` again. After the schema upgrade succeeds,
   automatic downgrade to the old schema is refused.

## Assets-install recording

The `asset_installs` table records, per coding agent and install path, the
CLI version that last installed the skills and preset
(where one exists) there — **not** a schema version. The assets
half of `cafleet setup` upserts one row per installed agent — all three
agents on the no-flag form — keyed on the
agent's resolved config path. Every fleet-scoped command validates these
rows before running — the
[stale-assets guard](../spec/cli-options.md#stale-assets-guard) — so the
assets can never silently go stale after a CLI upgrade. `cafleet doctor`
reports the per-agent detail. See
[data model](../spec/data-model.md) for the table schema.

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
