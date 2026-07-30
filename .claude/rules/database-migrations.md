# Database Migrations

Database migrations live in `cafleet/migrations/` as hand-written SQL files named `V<N>__<slug>.sql`, numbered contiguously from 1 with exactly one baseline (`V1__baseline.sql`). The chain is embedded in the binary at build time and applied by refinery, which records applied versions in the `refinery_schema_history` ledger.

## Write a migration by hand

1. Create `cafleet/migrations/V<N>__<slug>.sql`, where `<N>` is the current head version + 1 and `<slug>` is a short snake_case description.
2. Write plain SQLite SQL. Keep every migration data-preserving:
   - For a column rename, use `ALTER TABLE <table> RENAME COLUMN <old> TO <new>` (SQLite ≥ 3.25, in place, FK-safe). A recreate that `DROP TABLE`s a parent fails under FK enforcement on a populated DB.
   - Never drop-and-recreate a populated table when an in-place `ALTER TABLE` form exists.
3. Apply it with the schema-only setup invocation: `cafleet setup --skip claude --skip codex --skip opencode` (idempotent; migrates to head).

refinery has no down migrations — a schema change that must be reversible ships its reversal as the next numbered migration.

## Update the chain guard

After adding a migration, update the chain-guard test in `cafleet/tests/` — it asserts the chain is contiguous from 1, has exactly one baseline, and names the expected head version. Bump the expected head and add the new file's version. That snapshot keeps the chain sequential and linear.
