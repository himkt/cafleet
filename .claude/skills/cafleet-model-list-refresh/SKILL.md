---
name: cafleet-model-list-refresh
description: >-
  Refresh the CAFleet model list at skills/cafleet/reference/model-list.md
  from the approved official pricing sources. Use when a maintainer asks to
  refresh, update, or re-verify the model list, its token prices, or its
  freshness, or when a Director reports the list was last refreshed more than
  30 days ago. Maintainer-invoked only — never run automatically during a
  member spawn.
---

# CAFleet Model List Refresh

Refresh the repository's model list — the per-backend model tables of
`skills/cafleet/reference/model-list.md` — from the approved official
sources, under explicit maintainer review. The repository file is the release
source: a running Director reads only its deployed skill replica, so a refresh
reaches Directors exclusively through the release/deployment transaction below.

## Approved sources (exhaustive allowlist)

Fetch **only** these four official pages. Search results, third-party price
sites, and social posts are never model-list authority.

| Source | Backend it feeds | URL |
|---|---|---|
| Anthropic pricing | `claude` | `https://platform.claude.com/docs/en/about-claude/pricing.md` |
| OpenAI pricing | `codex` (prices) | `https://developers.openai.com/api/docs/pricing` |
| Codex model availability | `codex` (availability) | `https://learn.chatgpt.com/docs/models.md` |
| OpenCode Zen models and pricing | `opencode` | `https://opencode.ai/docs/zen.md` |

## Refresh procedure

1. **Fetch** all three approved sources. If any source cannot be fetched or
   parsed, **stop**: leave the model list unchanged, report the error, and
   let it become stale rather than fabricating values.
2. **Extract pricing facts only**: the currently effective standard input and
   output prices (USD per MTok) and availability/deprecation qualifiers. Do
   not copy long provider text into the repository, and do not present the
   pricing pages as benchmark evidence — they are pricing/availability
   sources only.
3. **Refresh the `claude` and `codex` tables** from their provider pricing
   pages, and **refresh the `opencode` table** with the dedicated procedure
   below — the Zen catalog behaves differently from the provider pages, so
   follow that procedure exactly. For `codex`, pricing and availability come
   from different pages: price each model from the OpenAI pricing page, but
   take Codex availability from the Codex models page
   (`https://learn.chatgpt.com/docs/models.md`) — the table carries only the
   models Codex currently offers in its model picker. A model absent there
   or demoted to a **legacy model** (reachable only via `codex -m` /
   `config.toml`) is removed from the table even when the pricing page
   still prices it; legacy models remain manual pass-through `--model`
   values.
4. **Reapply the capability classes explicitly.** The `Class` descriptions
   and the most-to-least-capable row ordering are reviewed maintainer
   judgment, not provider benchmark claims; changing a class or the ordering
   requires reviewed policy approval in the same pull request. Every row
   carries both prices from its approved source. Re-derive the *Monitor and
   reviewer defaults* table from the refreshed backend tables, and mirror its
   values into each backend overlay's `{monitor_model}` / `{reviewer_model}`
   rows (`skills/cafleet/reference/coding-agent/<name>-overlay.md`).
5. **Propose, then apply atomically.** Generate a concise proposed diff and
   require explicit maintainer approval before rewriting the tables in
   `skills/cafleet/reference/model-list.md`. Preserve the prescribed preamble
   verbatim; update the preamble's *last refreshed* date only after approval.
   That date is part of the page contract — the Director's staleness check
   reads it, so every refresh keeps it present and current. A failed refresh
   makes **no** edit.

## OpenCode Zen procedure (the `opencode` table)

The `opencode` backend is priced by the OpenCode Zen page, and its catalog
behaves differently from the two provider pages: model IDs differ from
display names, the page lists many models while cafleet curates a few, and
free stealth/preview models appear and disappear. On **every** refresh, even
when the claude/codex prices are unchanged:

1. **Price from the Zen page only.** Zen's own USD-per-MTok rates are the
   billing rates for the `opencode` backend. Never price a Zen model from the
   upstream vendor's page (Anthropic, OpenAI, DeepSeek, …) — the Zen rate is
   the one cafleet members are billed at.
2. **Copy the Zen model ID exactly; never slugify the display name.** The
   page shows a display name ("GLM 5.2", "Kimi K2.7 Code") and a distinct
   model ID (`glm-5.2`, `kimi-k2.7-code`). The row's `--model` value is
   `opencode/<zen-model-id>` — the literal `opencode/` prefix followed by the
   ID copied verbatim from the page. A hand-derived slug that does not match
   the Zen ID fails at spawn time.
3. **Re-verify every curated row against the page.** For each existing row,
   confirm the model is still listed on Zen and its price is current. A
   delisted model's row is removed in the same refresh.
4. **Handle free models by their published price.** A model Zen offers free
   for a limited time is priced `0.00` in both columns. When the free offer
   ends: update the row to the newly published Zen price, or remove the row
   if the model disappears or has no published price.
5. **Keep the table a curated subset.** Do not mirror the full Zen catalog —
   keep a handful of reviewed models spanning the price range. Adding or
   removing a curated model is a reviewed policy change, approved in the same
   pull request like any class or ordering change.

## Cadence and staleness

Refresh the model list at least every 30 days and whenever the user asks for
a refresh. A stale list disables cost efficiency mode — the Director relays
an operator choice for those spawns — until a maintainer refreshes the model
list, commits the repository source, and completes the release/deployment
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

This skill owns model-list maintenance; the CAFleet Director owns per-spawn
selection by reading the list; `cafleet member create` remains the execution
boundary.
