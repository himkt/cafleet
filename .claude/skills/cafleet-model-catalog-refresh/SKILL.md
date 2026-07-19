---
name: cafleet-model-catalog-refresh
description: >-
  Refresh the CAFleet model catalog at skills/cafleet/reference/model-catalog.md
  from the two approved official pricing sources. Use when a maintainer asks to
  refresh, update, or re-verify the model catalog, its token prices, or its
  source freshness, or when automatic model selection reports a stale catalog
  (MODEL_CATALOG_STALE). Maintainer-invoked only — never run automatically
  during a member spawn.
---

# CAFleet Model Catalog Refresh

Refresh the repository's model catalog — the sole payload block of
`skills/cafleet/reference/model-catalog.md` — from the approved official
sources, under explicit maintainer review. The repository file is the release
source: a running Director reads only its deployed skill replica, so a refresh
reaches Directors exclusively through the release/deployment transaction below.

## Approved sources (exhaustive allowlist)

Fetch **only** these two official pages. Search results, third-party price
sites, social posts, and scraped gateway prices are never catalog authority.

| Source key | URL |
|---|---|
| `anthropic` | `https://platform.claude.com/docs/en/about-claude/pricing` |
| `openai` | `https://developers.openai.com/api/docs/pricing` |

## Refresh procedure

1. **Fetch** both approved sources. For each, record the URL, the retrieval
   timestamp (UTC), and the SHA-256 content hash of the retrieved snapshot.
   If either source cannot be fetched or parsed, **stop**: leave the Markdown
   catalog unchanged, report the error, and let the catalog become stale rather
   than fabricating values.
2. **Extract pricing facts only**: relevant standard token prices (input,
   cached-input, cache-write, output, USD per MTok), date-limited pricing with
   its effective dates, and availability/deprecation qualifiers. Do not copy
   long provider text into the repository, and do not present the pricing
   pages as benchmark evidence — they are pricing/availability sources only.
3. **Reapply the capability rubric explicitly.** Capability levels and global
   ranks are separately reviewed maintainer policy (`maintainer_judgment`
   provenance). The maintainer must review every changed capability level,
   global rank, model availability, pricing basis, and effective date; changing
   a level or rank requires reviewed policy approval in the same pull request.
   A new model remains `active: false` until it has all required fields and a
   reviewed classification. Gateway models without an approved actual price
   keep `unknown` / `not-applicable` rate cards and stay manual-only.
4. **Validate before writing**: the Markdown envelope (single sentinel marker,
   single canonical-JSON payload block), the schema, the source allowlist,
   price units and non-negative values, date windows, backend/model syntax,
   unique keys/tokens/ranks, and catalog freshness. `cafleet model select
   --catalog <abs path> --role monitor --json` against the candidate file is a
   convenient end-to-end validation probe.
5. **Propose, then apply atomically.** Generate a concise proposed diff and
   require explicit maintainer approval before rewriting the one payload block
   in `skills/cafleet/reference/model-catalog.md`. Preserve the prescribed
   preamble verbatim; update the payload's `generated_at` only after successful
   validation and approval. A failed refresh makes **no** catalog edit.

## Cadence and staleness

Refresh the catalog at least every 30 days (`freshness_days`) and whenever the
user asks for a refresh. Stale source data disables automatic cost selection
(`MODEL_CATALOG_STALE`) until a maintainer refreshes the catalog, commits the
repository source, and completes the release/deployment transaction.

## Release-coupled deployment

There is no catalog-only sync path: a committed source file alone does not
refresh a running Director's asset copy. To deploy a refreshed catalog, the
maintainer:

1. Bumps the CAFleet release version.
2. Builds the wheel and `cafleet-assets-v<version>.zip` containing the
   repository `skills/` tree — the catalog rides inside
   `skills/cafleet/reference/` like every other reference page.
3. Publishes the release. Each active backend upgrades to that CLI version and
   runs `cafleet setup`, which overwrites its installed `cafleet` skill
   replica, catalog included.

## Ownership boundary

This skill owns catalog maintenance; the CAFleet Director owns per-task
selection (`cafleet model select`); `cafleet member create` remains the
execution boundary.
