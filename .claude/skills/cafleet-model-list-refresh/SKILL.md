---
name: cafleet-model-list-refresh
description: >-
  Refresh the CAFleet model list at skills/cafleet/reference/model-list.md
  from the two approved official pricing sources. Use when a maintainer asks to
  refresh, update, or re-verify the model list, its token prices, or its
  source freshness, or when automatic model selection reports a stale model
  list (MODEL_LIST_STALE). Maintainer-invoked only — never run automatically
  during a member spawn.
---

# CAFleet Model List Refresh

Refresh the repository's model list — the three tables of
`skills/cafleet/reference/model-list.md` — from the approved official
sources, under explicit maintainer review. The repository file is the release
source: a running Director reads only its deployed skill replica, so a refresh
reaches Directors exclusively through the release/deployment transaction below.

## Approved sources (exhaustive allowlist)

Fetch **only** these two official pages. Search results, third-party price
sites, social posts, and scraped gateway prices are never model-list authority.

| Source key | URL |
|---|---|
| `anthropic` | `https://platform.claude.com/docs/en/about-claude/pricing` |
| `openai` | `https://developers.openai.com/api/docs/pricing` |

## Refresh procedure

1. **Fetch** both approved sources. For each, record the URL, the retrieval
   timestamp (UTC), and the SHA-256 content hash of the retrieved snapshot.
   If either source cannot be fetched or parsed, **stop**: leave the Markdown
   model list unchanged, report the error, and let it become stale rather
   than fabricating values.
2. **Extract pricing facts only**: the currently effective standard token
   prices (input, cached-input, cache-write, output, USD per MTok), each
   model's context limit, and availability/deprecation qualifiers. Each model
   row carries one current price set; when a source announces a dated price
   change, refresh again when it takes effect. Do not copy long provider text
   into the repository, and do not present the pricing pages as benchmark
   evidence — they are pricing/availability sources only.
3. **Reapply the capability rubric explicitly.** Capability levels and ranks
   are reviewed maintainer judgment, not provider benchmark claims. The
   maintainer must review every changed capability level, rank, model
   availability, and pricing basis; changing a level or rank requires reviewed
   policy approval in the same pull request. A new model row remains
   `Active: no` until it has all required fields and a reviewed
   classification. Gateway models without an approved actual price keep `—`
   in every price cell and stay manual-only.
4. **Validate before writing**: the fixed table layout (the `Metadata`,
   `Sources`, and `Models` sections in order, each with its exact column
   header), the source allowlist, price units and non-negative values,
   backend/model syntax, unique keys/tokens/ranks, and source freshness.
   `cafleet model select --model-list <abs path> --role monitor --json`
   against the candidate file is a convenient end-to-end validation probe.
5. **Propose, then apply atomically.** Generate a concise proposed diff and
   require explicit maintainer approval before rewriting the tables in
   `skills/cafleet/reference/model-list.md`. Preserve the prescribed preamble
   verbatim; update the `Metadata` table's `generated_at` only after
   successful validation and approval. A failed refresh makes **no** edit.

## Cadence and staleness

Refresh the model list at least every 30 days (`freshness_days`) and whenever
the user asks for a refresh. Stale source data disables automatic cost
selection (`MODEL_LIST_STALE`) until a maintainer refreshes the model list,
commits the repository source, and completes the release/deployment
transaction.

## Release-coupled deployment

There is no model-list-only sync path: a committed source file alone does not
refresh a running Director's asset copy. To deploy a refreshed model list, the
maintainer:

1. Bumps the CAFleet release version.
2. Builds the wheel and `cafleet-assets-v<version>.zip` containing the
   repository `skills/` tree — the model list rides inside
   `skills/cafleet/reference/` like every other reference page.
3. Publishes the release. Each active backend upgrades to that CLI version and
   runs `cafleet setup`, which overwrites its installed `cafleet` skill
   replica, model list included.

## Ownership boundary

This skill owns model-list maintenance; the CAFleet Director owns per-task
selection (`cafleet model select`); `cafleet member create` remains the
execution boundary.
