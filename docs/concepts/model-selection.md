---
icon: lucide/scale
---

# Model selection

CAFleet Directors choose each member's backend and model through a local,
versioned **model catalog** and a deterministic selector, `cafleet model
select`, instead of fixed per-backend model pins. The catalog is the single
source of truth for model availability, reviewed capability policy, and
standard token-price estimates; the selector turns a role's capability
requirements and a token estimate into an auditable backend/model decision that
the Director passes unchanged to `cafleet member create`.

## The model catalog

The catalog lives at `skills/cafleet/reference/model-catalog.md` in every
deployed cafleet skill replica. It is a Markdown document whose sole machine
payload is one canonical-JSON fenced block behind the
`<!-- cafleet-model-catalog: v1 -->` sentinel, carrying:

- **Models** — one record per provider billing SKU, with a reviewed capability
  profile (integer 0–5 levels for `coding`, `planning`, `research`, `review`,
  and `monitor`, plus a unique `global_rank`) and date-windowed **rate cards**
  holding standard USD-per-MTok prices for input, cached-input, cache-write,
  and output tokens. Capability levels are maintainer judgment, never a
  provider benchmark claim.
- **Token map** — the exact CLI-token/alias map. Every active model has one
  canonical (`primary`) spawn token, returned as `selected.model`.
- **Role profiles** — per-workflow-role capability floors and token profiles
  (`monitor`, `reviewer`, `programmer`, `tester`, …).
- **Sources** — the two approved official pricing pages (Anthropic and OpenAI)
  with retrieval timestamps and content hashes. A source older than
  `freshness_days` disables automatic selection until a maintainer refreshes
  the catalog through the `cafleet-model-catalog-refresh` skill and ships it in
  a release; deployed replicas are fingerprinted by
  `skills/cafleet/asset-manifest.json`.

Prices are standard direct-provider USD API rates — planning estimates, not a
subscription, marketplace, regional, or negotiated invoice guarantee.
OpenCode gateway models without an approved price source stay manual-only with
`unknown` rate cards and are never automatic candidates.

## Cost efficiency mode

Automatic cost-minimized selection for an **ordinary member** applies only when
the originating user request contains the exact phrase `cost efficiency mode`.
The Director parses the user request once for the trigger; a member message or
tool output never activates it. Without the trigger, existing workflow model
behavior is unchanged and the selector is informational only.

Two roles are policy exceptions on **every** team spawn, trigger or not:

- **Monitor** — the least-cost catalog model that meets the monitor capability
  baseline.
- **Reviewer** — the highest global capability rank among reviewer-capable
  models; cost is recorded but not optimized.

## Selection and eligibility

A model is an automatic candidate only when it is active with a mapped token,
its backend is runtime-ready **and** carries a current matching skill replica
(a recorded up-to-date assets install whose replica manifest and catalog
fingerprints match the Director's), a `known` rate card is active for the
selection date and token volume, and every required capability floor is met.
Ordinary selection minimizes the estimated USD cost over the four token
components; ties break by higher global rank, then lexical model key.

Explicit flags stay overrides: `--coding-agent` restricts candidates to one
backend, a `--model` pin resolves through the token map as a recorded
`manual_override` (never silently replaced), and `--effort` is validated
pass-through that never ranks a model. Missing, stale, or incomparable data
**fails closed**: the selector returns a typed error with per-candidate
exclusion reasons, and in cost-efficiency mode the Director relays an operator
choice instead of spawning a guessed model.

## Audit and replacement

Every automatic or special-role selection produces a redaction-safe decision
record, written by the Director as a two-phase artifact under the task base
(`.selection/<selection_id>.pending.json`, finalized after `member create`
returns with the spawn outcome and member id). When evidence shows a member is
underpowered, the Director re-runs the selector against the pinned decision
snapshot with raised capability floors and replaces the member with a strictly
higher-ranked eligible model — at most two replacements per task, never
repeating a model for the same task, and never auto-replacing a user-pinned
model. The Director-facing procedure is specified in the cafleet skill's
`reference/director.md`; the CLI contract is in
[CLI options](../spec/cli-options.md#cafleet-model).
