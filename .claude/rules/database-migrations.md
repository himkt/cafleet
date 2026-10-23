# Database Migrations

The binary embeds the SQL migration registry under `cafleet/migrations/`. The current format has an explicit application identity and a `V1__baseline.sql` baseline; the ledger version and required table/column shapes define compatibility together.

Write future current-family migrations as contiguous hand-written `V<N>__<slug>.sql` files. Preserve valid existing data with in-place schema changes where supported. Setup applies compatible pending changes transactionally and refuses an ahead or incompatible format before either write path proceeds.

The embedded chain guard lives in `cafleet/src/db/mod.rs`. Update its expected registry when the bundled head changes, retaining contiguous numbering and one baseline. Use the embedded registry as the source of truth.
