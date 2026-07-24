# Database Migrations

Alembic migrations live in `cafleet/src/cafleet/db/alembic/versions/` as a linear chain of zero-padded sequential revisions (`0001`, `0002`, … `000N`), each with a manual `down_revision` link.

## Generate a migration with the mise task

Always generate a new migration with the project's mise task — never invoke `alembic revision` directly:

```bash
mise //cafleet:makemigration "short description of the change"
```

- Run `cafleet setup --skip claude --skip codex --skip opencode` (the schema-only invocation) first so the DB is at head — `--autogenerate` requires it.
- The message becomes the migration docstring and the filename slug.
- `env.py`'s `process_revision_directives` hook mints the next sequential id, so the file lands as `000N_<slug>.py`, matching the chain and the chain-guard snapshot in `tests/db/test_alembic_smoke.py`. Raw `alembic revision` mints a random hex id and breaks both.

## Review and hand-edit the generated migration

`--autogenerate` diffs the model against the DB and **cannot detect a column rename** — it emits `drop_column` + `add_column`, which loses data. Review every generated migration:

- For a column rename, replace the drop+add with `op.execute("ALTER TABLE <table> RENAME COLUMN <old> TO <new>")` (SQLite ≥ 3.25, in place, FK-safe). A batch-recreate that `DROP TABLE`s a parent fails under FK enforcement on a populated DB.
- Keep every migration data-preserving, and write the matching `downgrade()`.

## Update the chain guard

After adding a migration, update the chain-guard tests in `tests/db/test_alembic_smoke.py` (currently `test_four_revision_migration_chain_exists`, asserting the linear chain `0004` → `0003` → `0002` → `0001` → `None`, and `test_alembic_version_table_records_head_0004`, asserting the recorded head) — bump the expected count, rename both tests to match the new chain length and head, and add the new revision id plus its `down_revision` link. That snapshot keeps the chain sequential and linear.
