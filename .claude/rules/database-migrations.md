# Database Migrations

The binary embeds the SQL migration registry under `cafleet/migrations/`. The current format has an explicit application identity and a `V1__baseline.sql` baseline; the ledger version and required table/column shapes define compatibility together.

A deliberate format-family reset uses a new application identity and default database filename, preserves predecessor files, and establishes a new V1 baseline. The `cafleet_v8.db` family has its own V1 baseline. Changes within that family extend the migration chain.

Write future current-family migrations as contiguous hand-written `V<N>__<slug>.sql` files. Preserve valid existing data with in-place schema changes where supported. Setup applies compatible pending changes transactionally and refuses an ahead or incompatible format before either write path proceeds.

The chain guard in `cafleet/src/db/mod.rs` derives the registry from the embedded migrations and asserts contiguous numbering from one baseline, so the embedded registry is the source of truth. Add a successor test for each new `V<N>__<slug>.sql` that migrates the previous head to it and checks the new shape.
