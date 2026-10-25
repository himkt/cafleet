# OpenCode full model catalog and model-name backend routing

**Status**: Complete
**Progress**: 20/20 tasks complete
**Last Updated**: 2026-10-24

## Overview

The `opencode` Model catalog becomes a mirror of every model the OpenCode Console currently lists, including its Claude and GPT models, and the `cafleet-model-list-refresh` skill maintains it that way. The Director's backend routing is restated so that a member referenced only by a Claude or GPT model name runs on Claude Code or Codex, while an explicit OpenCode request runs on OpenCode with the `opencode/` form of that model.

## Success Criteria

- [x] The `## opencode` → `### Model catalog` table in `skills/cafleet/reference/coding-agents.md` holds one row per OpenCode Console model ID listed on the refresh date, excluding models whose published deprecation date has passed, in a single table ordered most → least capable with a `Class` on every row.
- [x] `.claude/skills/cafleet-model-list-refresh/SKILL.md` § OpenCode Console procedure specifies the full-catalog mirror (no curated-subset rule remains).
- [x] `skills/cafleet/roles/director.md` resolves "opus member" / "gpt6.1-sol member" to `--coding-agent claude --model claude-opus-5-5` / `--coding-agent codex --model gpt-6.1-sol` on every fleet backend, including an `opencode` fleet.
- [x] `skills/cafleet/roles/director.md` resolves "spawn a claude member with opencode" to `--coding-agent opencode --model opencode/claude-opus-5-5`.
- [x] The opencode Role defaults (`opencode/deepseek-v4.1-flash-free`, `opencode/glm-5.3-flash-free`) are unchanged and still present in the catalog.
- [x] `docs/docs/concepts/coding-agents.md` and `docs/docs/spec/coding-agent-backends.md` describe the same routing and catalog facts for operators; the stale codex example values are replaced.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass, including a new docs-sync guard on the opencode catalog shape.

---

## Background

| Surface | Current state | Problem |
|---|---|---|
| `coding-agents.md` § opencode Model catalog | 8 hand-picked rows (`glm-5.2`, `kimi-k2.7-code`, `muse-spark-1.2`, `deepseek-v4.1-flash-free`, `glm-5.3-flash-free`, `qwen3.5-plus`, `glm-5.3-flash`, `big-pickle`), introduced as "A curated subset" | Most Console models, including every Claude and GPT model, are missing; cost efficiency mode on an opencode fleet compares against a partial list. |
| Refresh skill § OpenCode Console procedure, step 5 | "Keep the table a curated subset. Do not mirror the full Console catalog" | Each refresh deliberately keeps the catalog partial. |
| `director.md` § Model selection | "Choose the backend first — the fleet's backend unless the user names one" | Read literally, an "opus member" request on an opencode fleet stays on opencode once the opencode catalog lists `opencode/claude-opus-5-5`. |
| `director.md` § Model-name-to-backend inference | Exact-shape table: slash → opencode, `gpt-*` → codex, Claude alias/`claude-*` → claude, anything else → ask | No precedence statement relative to the fleet backend; no loose-spelling handling (`gpt6.1-sol`); no rule for "with opencode" + a family/alias; opencode-only bare names (`glm-5.3`) are always asked. |
| `docs/docs/spec/coding-agent-backends.md` § Model selection | Codex examples `gpt-5.6-sol` (default), `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5` | Contradicts the current codex catalog (`gpt-6-astra`, `gpt-6.1-sol`, `gpt-6-luna`). |

On 2026-10-24 the OpenCode Console page lists 84 model IDs: 24 GPT (`gpt-*`), 14 Claude (`claude-*`) and 46 others, 11 of them free for a limited time. Its *Deprecated models* table assigns a passed deprecation date to eight of those IDs, which remain in the active model and pricing tables:

| Model ID | Deprecation date |
|---|---|
| `gpt-5.2-codex`, `gpt-5.1-codex`, `gpt-5.1-codex-max`, `gpt-5.1-codex-mini`, `gpt-5-codex` | 2026-07-23 |
| `glm-5` | 2026-05-14 |
| `minimax-m2.5`, `kimi-k2.5` | 2026-08-05 |

The populated catalog therefore holds 76 rows on that date.

---

## Specification

### 1. OpenCode catalog contents

The `opencode` Model catalog lists **every model ID the OpenCode Console page lists**, with these exclusions and row rules:

| Case on the Console page | Catalog treatment |
|---|---|
| Listed model, no deprecation date | One row. |
| Listed model with a deprecation date in the future | One row; the note under the table records the date. |
| Listed model whose deprecation date has passed | No row. |
| Model with tiered prices (≤ 272K / > 272K, ≤ 200K / > 200K, ≤ 100K / > 100K tokens) | One row at the base (shortest-prompt) tier; the note under the table lists each long-context rate. |
| Model with a promotional discount (e.g. "50% off") | One row at the currently billed Console rate; the note records the list price and the discount end date when published. |
| Free for a limited time | `0.00` in both price columns, as today. |
| Only one price published as free (e.g. `jev-1.13`: input $0.042, output free) | The published rate in one column and `0.00` in the other. |

**Binding a deprecation date to a model ID.** The Console publishes deprecation dates only in its separate *Deprecated models* table, keyed by display name ("GLM 5", "Kimi K2.5"). Map each deprecated display name to its model ID through the page's *Endpoints* table (the table under `## Endpoints`, header `Model | Model ID | Endpoint | AI SDK Package`), then compare the date with the refresh date. A passed date excludes the row even while the ID still appears in the active pricing table. A deprecated display name with no listed ID is already delisted and needs no action. A deprecated display name that maps to more than one ID stops the refresh as a parse failure (refresh procedure step 1).

Each row's `Model` cell is `opencode/<console-model-id>`, the ID copied verbatim from the Console *Endpoints* table's `Model ID` column. Columns stay `Model | Class | Input $/MTok | Output $/MTok`; no Context or cached-price columns are added.

**Ordering and Class.** The table is a single table ordered most → least capable across the whole catalog, with no vendor-family grouping, and every row carries a short `Class` text. Ordering and classes remain reviewed maintainer judgment (existing contract). One invariant keeps the backends consistent: models that also appear in the `claude` or `codex` catalog keep the same relative order as in those catalogs (`claude-fable-5-1` > `claude-fable-5` > `claude-opus-5-5` > `claude-opus-5` > `claude-sonnet-5-5` > `claude-sonnet-5` > `claude-haiku-5-5`; `gpt-6-astra` > `gpt-6.1-sol` > `gpt-6-luna`). A model whose capability is unverified (stealth/preview, e.g. `big-pickle`) sits at the bottom, with its Class saying so.

**Preamble.** The opencode catalog preamble replaces "A curated subset of the OpenCode Console catalog" with text stating:

- The table lists every model the OpenCode Console lists, except models past their published deprecation date; the `opencode/` prefix is part of the `--model` value.
- Any other `<provider-id>/<model-id>` value remains a manual spawn with explicit `--coding-agent opencode --model` flags (existing sentence).
- `0.00` marks an explicitly free model offered for a limited time (existing sentence).
- A Claude or GPT row here is the OpenCode spawn token for that model; a bare model-name reference still selects the native backend under [Director model selection](../roles/director.md#model-selection).

**Role defaults.** The opencode Role defaults stay `opencode/deepseek-v4.1-flash-free` (reviewer) and `opencode/glm-5.3-flash-free` (other), effort `—`. The refresh skill's maintainer-choice step governs later changes.

**Freshness.** Populating the table is a model-data refresh: the shared "last refreshed" date in the `coding-agents.md` preamble is set to the date the populated table is applied, and the `claude` / `codex` tables are re-verified in the same run per the existing procedure.

### 2. Refresh skill: OpenCode Console procedure

`.claude/skills/cafleet-model-list-refresh/SKILL.md` § OpenCode Console procedure is rewritten as follows (the approved-source allowlist, Steps 1–2 and the release-coupled deployment stay unchanged):

1. **Price from the Console page only** — unchanged, including for Claude and GPT rows: the Console rate is the OpenCode billing rate, never the Anthropic or OpenAI page.
2. **Copy the Console model ID exactly** — unchanged in intent; the mapping source is renamed from the "*Model list* table" to the *Endpoints* table (the table under `## Endpoints`, header `Model | Model ID | Endpoint | AI SDK Package`), which maps each display name to its `Model ID`; the *Pricing* table keys prices by the same display name.
3. **Mirror the Console catalog.** Add a row for every listed model ID that is missing; remove the row of every model ID no longer listed or past its published deprecation date; re-verify every remaining row's price. Bind each *Deprecated models* entry to its model ID through the *Endpoints* table; a passed date excludes the row even while the ID is still listed as active. A deprecated display name with no listed ID is already delisted and needs no action; one mapping to more than one ID stops the refresh.
4. **Apply the row rules** from § 1 (tiers, discounts, free models, future deprecation dates) and keep the table notes in sync with them.
5. **Order and classify every row.** One table ordered most → least capable, a Class on every row, keeping the relative order of models shared with the `claude` / `codex` catalogs. A new row's Class and position, and any change to existing ones, are reviewed policy changes approved in the same pull request.

The intro paragraph of that section changes from "the page lists many models while cafleet curates a few" to state that the table mirrors the page, model IDs differ from display names, and free/preview models appear and disappear. Refresh Step 3's pointer to "the dedicated procedure below" stays. The § Ownership boundary is unchanged.

### 3. Backend resolution for a member spawn

`director.md` § Model selection's first sentence is replaced by this resolution order. The Director resolves the backend per member, from the operator's reference to that member:

1. **Explicit backend.** The operator names the backend for this member — the word OpenCode / Claude Code / Codex attached to the member (e.g. "spawn a claude member with opencode", 「opencode で claude member を spawn して」), or a `--coding-agent` value. That backend wins; its model resolves per § 5.
2. **Model-name inference.** Otherwise, a model, alias or family name in the reference selects the backend through § Model-name-to-backend inference — on every fleet backend, including an `opencode` fleet.
3. **Fleet backend.** Otherwise, the member uses the fleet's (the Director's own) backend and its Role default.

The § Model-name-to-backend inference table becomes (ordered precedence, first match wins). The row 2 / row 3 family test and every "match" use the § 4 comparison key:

| # | Model reference | Backend | Flags to pass |
|---|---|---|---|
| 1 | Contains a `/` — provider-prefixed (e.g. `opencode/glm-5.3`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>`, verbatim |
| 2 | Claude reference: its key starts with `claude`, `fable`, `opus`, `sonnet` or `haiku` — `claude`, `opus`, `claude-opus-4-8`, `Opus 5.5` | `claude` | `--coding-agent claude --model <token>` resolved per § 4; the family word `claude` alone uses the claude Role default for the role |
| 3 | GPT reference: its key starts with `gpt` — `gpt`, `gpt-6.1-sol`, `gpt6.1-sol`, `GPT 6 Luna` | `codex` | `--coding-agent codex --model <token>` resolved per § 4; the family word `gpt` alone uses the codex Role default for the role |
| 4 | Any other bare name whose key equals the key of exactly one opencode catalog ID — `glm-5.3`, `kimi-k3` | `opencode` | `--coding-agent opencode --model opencode/<console-model-id>` |
| 5 | Anything else — no key-equal ID, or more than one (e.g. `o3-mini`, `gemini`) | none | Ask the operator for the explicit `--coding-agent` + `--model` pair |

Matching is key **equality**, never prefix or substring: `glm-5.3` matches only `opencode/glm-5.3`, never `opencode/glm-5.3-flash` or `opencode/glm-5.3-flash-free`.

The precedence paragraph under the table is replaced with this text:

> The rows apply as ordered precedence — the first match wins. Row 1 wins over row 2 for `anthropic/claude-sonnet-4-6`, which contains both `claude` and a `/`: a provider-prefixed token is itself an explicit OpenCode request. A bare name infers `opencode` only through row 4, and never for a Claude or GPT name, even when the opencode catalog lists that model.

Worked resolutions (added to `director.md` below the table):

| Operator says | Fleet backend | Resolved flags |
|---|---|---|
| "opus member" | `opencode` | `--coding-agent claude --model claude-opus-5-5` |
| "gpt6.1-sol member" | `opencode` | `--coding-agent codex --model gpt-6.1-sol` |
| "claude member" (reviewer) | `codex` | `--coding-agent claude --model claude-opus-5-5 --effort high` |
| "spawn a claude member with opencode" | any | `--coding-agent opencode --model opencode/claude-opus-5-5` |
| "opencode で opus member" | any | `--coding-agent opencode --model opencode/claude-opus-5-5` |
| "opencode で gpt reviewer" | any | `--coding-agent opencode --model opencode/gpt-6-astra` |
| "opencode で sonnet 4.6 member" | any | `--coding-agent opencode --model opencode/claude-sonnet-4-6` |
| "Claude Code で opus 4.8 member" | any | `--coding-agent claude --model claude-opus-4-8` (manual pass-through) |
| "Codex で reviewer" | any | `--coding-agent codex --model gpt-6-astra --effort high` |
| "codex で opus member" | any | none — relayed as a mismatched pair (§ 6) |
| "glm-5.3 member" | `claude` | `--coding-agent opencode --model opencode/glm-5.3` |
| "gpt-5.4 member" | `opencode` | `--coding-agent codex --model gpt-5.4` (manual pass-through) |
| "a member" (no model, no backend) | `opencode` | `--coding-agent opencode --model opencode/glm-5.3-flash-free` |

Effort follows the selected backend's Role defaults and Runtime bindings as today (OpenCode omits `--effort`).

### 4. Model-name normalization

The Director normalizes loose spellings before matching, so "gpt6.1-sol", "GPT 6.1 Sol" and "Opus 5.5" resolve to catalog tokens:

1. **Comparison key.** Lowercase the reference and every candidate token, and drop every character that is not a letter or digit (`gpt-6.1-sol` → `gpt61sol`; `GPT 6.1 Sol` → `gpt61sol`). Strip the `opencode/` prefix from opencode catalog tokens before keying. A Claude candidate also matches its key with the leading `claude` removed (`claude-opus-5-5` → `opus55`, matching "Opus 5.5").
2. **Native catalog first.** For a Claude or GPT reference (classified by the row 2 / row 3 key-prefix test), a key equal to the key of exactly one `Model` or `Alias` token in the native (`claude` / `codex`) catalog resolves to that row's `Model` token.
3. **Spelling dictionary.** A Claude or GPT reference equal to no native token key but to exactly one Claude/GPT ID key in the opencode catalog resolves to that ID **without** the `opencode/` prefix and stays on the native backend as a manual pass-through `--model` (e.g. "sonnet 4.6" → `--coding-agent claude --model claude-sonnet-4-6`).
4. **Exact shape.** A reference already written as an exact `claude-*` / `gpt-*` token that matches nothing passes through verbatim to its native backend (manual pass-through).
5. **Otherwise ask.** No key-equal token, or more than one token sharing the key, is relayed to the operator via {decision_surface}.

### 5. Explicit backend model mapping

When resolution step 1 selects a backend, the `--model` value resolves from the reference. Rows apply in order; the first match wins.

**OpenCode.**

| Reference given with the OpenCode request | Resolved `--model` |
|---|---|
| No model (e.g. "spawn an opencode reviewer") | The opencode Role default for the role |
| Provider-prefixed token (`opencode/…`, `anthropic/…`) | The token, verbatim |
| Claude family word (`claude`) | `opencode/` + the `claude` backend's Role default for the role |
| GPT family word (`gpt`) | `opencode/` + the `codex` backend's Role default for the role |
| Claude alias (`fable`, `opus`, `sonnet`, `haiku`) | `opencode/` + the `claude` catalog Model that the alias names |
| Any other model name | The opencode catalog ID whose key equals the reference's key |

The last row uses only the § 4 step 1 comparison key, including the leading-`claude` strip, against opencode catalog IDs; §§ 4 steps 2–4 do not apply on this path. It requires exactly one key-equal ID, so "opencode で sonnet 4.6" resolves to `opencode/claude-sonnet-4-6`. With today's Role defaults: `claude` → `opencode/claude-opus-5-5` for every role; `gpt` → `opencode/gpt-6-astra` for reviewers and `opencode/gpt-6.1-sol` for other members; `opus` → `opencode/claude-opus-5-5`. A mapped token (every row except the verbatim one) must exist in the opencode catalog; no match or an ambiguous match is relayed per § 6 instead of spawning.

**Claude Code / Codex.**

| Reference given with the Claude Code or Codex request | Resolved `--model` |
|---|---|
| No model (e.g. "Claude Code で reviewer") | That backend's Role default for the role |
| The backend's family word (`claude` for Claude Code, `gpt` for Codex) | That backend's Role default for the role |
| A reference of the backend's own family (row 2 for Claude Code, row 3 for Codex) | The token § 4 steps 2–4 resolve against that backend (e.g. "Claude Code で opus 4.8" → `claude-opus-4-8`) |
| A reference of another family, or a provider-prefixed token | none — relay as a mismatched pair per § 6 |

An unresolved reference under § 4 step 5 is relayed per § 6.

### 6. Failure handling

| Condition | Director behavior |
|---|---|
| Explicit OpenCode request whose mapped model is not in the opencode catalog (e.g. "opencode で claude-opus-4-1") | Relay via {decision_surface}; no spawn |
| Pinned backend and model that do not belong together (e.g. "codex で opus member") | Relay via {decision_surface} (existing pinned-pair rule) |
| Ambiguous normalization (two tokens share the key) or no match | Relay via {decision_surface} |
| Native backend binary missing (`Error: binary <name> not found on PATH`, exit 1) | Report the failure and relay the operator choice; the Director never substitutes `opencode` for a model-name-routed member |
| Opencode catalog row removed at a later refresh while a member referenced it | No effect on the running member; the next spawn resolves against the current catalog |

### 7. Cost efficiency mode and replacement

Cost efficiency mode compares models within the chosen backend's **whole** catalog; on `opencode` that includes its Claude and GPT rows and the free rows. `director.md` § Model selection adds that clause. § Model replacement keeps its rule that a stronger model is a row above the failed model in the same backend's table. `director.md` § Model selection adds the pinning boundary:

| Operator reference | Pinned | Model replacement |
|---|---|---|
| Names a specific model, alias or version ("opus member", "gpt6.1-sol member", "opencode で sonnet 4.6") | Backend and model | Operator decision required (existing user-pinned rule) |
| Names only a family word or a backend ("claude member", "opencode reviewer", "opencode で gpt reviewer") | Backend only | Director may choose a stronger row within that backend's table |
| Names neither | Nothing | Director may choose a stronger row within the fleet backend's table |

### 8. Documentation surfaces

| File | Edit |
|---|---|
| `docs/docs/concepts/coding-agents.md` § Model choice | Replace "OpenCode's curated models retain their `opencode/` prefix." with: the OpenCode catalog lists every Console model with its `opencode/` prefix. Add a paragraph stating the routing for operators: a bare Claude or GPT model reference runs on Claude Code or Codex on every fleet; naming OpenCode for the member runs OpenCode with the `opencode/` form of that model; a name only OpenCode offers runs OpenCode; anything unresolved is asked. Keep the page free of repository paths. |
| `docs/docs/spec/coding-agent-backends.md` § Model selection | Codex example values become `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-luna` (no "(default)" claim). The opencode example values become `opencode/claude-opus-5-5`, `opencode/gpt-6.1-sol`, `anthropic/claude-sonnet-4-6`. |
| `skills/cafleet/reference/coding-agents.md` | opencode catalog preamble, table, notes per § 1; shared "last refreshed" date. |
| `skills/cafleet/roles/director.md` | § Model selection per §§ 3 and 7; § Model-name-to-backend inference per §§ 3–5 and § 6; § Member Create `--model` row keeps pointing at the inference table. |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | § OpenCode Console procedure per § 2. |

`README.md`, `SPEC.md` and `.claude/rules/` need no edit: the CLI contract (`--model` pass-through, opencode `<provider-id>/<model-id>` validation, error strings) is unchanged, and the overlay rule's ownership split already covers the catalog. The agent-facing and operator-facing homes state these facts independently, without linking each other (`.claude/rules/coding-agent-overlay.md` § Two independent homes).

### 9. Tests

Add `opencode_catalog_rows_are_unique_prefixed_and_priced` to `cafleet/tests/docs_sync.rs`, using the existing `INVENTORY.document(BACKENDS).section("opencode").section("model-catalog").rows()` helpers:

- the header is exactly `Model | Class | Input $/MTok | Output $/MTok`;
- every `Model` cell starts with `opencode/` and is unique;
- every `Class` cell is non-empty;
- every price cell parses as a non-negative decimal with two or more fractional digits.

The existing `backend_bindings_have_unique_values_and_local_model_defaults` test keeps guarding that the opencode Role defaults stay in the catalog. Routing rules are prose consumed by the Director and have no executable test.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Operator documentation

- [x] Update `docs/docs/concepts/coding-agents.md` § Model choice per Specification § 8 <!-- completed: 2026-10-24T12:49 -->
- [x] Update `docs/docs/spec/coding-agent-backends.md` § Model selection example values per Specification § 8 <!-- completed: 2026-10-24T12:49 -->
- [x] Run `mise //docs:build` and fix any broken links <!-- completed: 2026-10-24T12:49 -->

### Step 2: Director routing policy

- [x] Replace the first sentence of `skills/cafleet/roles/director.md` § Model selection with the three-step resolution order (Specification § 3) and add the whole-catalog cost-efficiency clause (§ 7) <!-- completed: 2026-10-24T12:50 -->
- [x] Replace the § Model-name-to-backend inference table and its precedence paragraph with the five-row table and slash-case explanation (§ 3) <!-- completed: 2026-10-24T12:50 -->
- [x] Add the worked-resolutions table, the normalization procedure (§ 4), the explicit backend mapping tables (§ 5), the failure-handling rules (§ 6) and the pinning boundary (§ 7) <!-- completed: 2026-10-24T12:51 -->
- [x] Check every table cell against the two-sentence cap and `{decision_surface}` placeholder usage <!-- completed: 2026-10-24T12:53 -->

### Step 3: Refresh skill

- [x] Rewrite `.claude/skills/cafleet-model-list-refresh/SKILL.md` § OpenCode Console procedure intro and steps 2–5 per Specification § 2 <!-- completed: 2026-10-24T12:55 -->
- [x] Confirm no "curated subset" wording remains anywhere outside `design-docs/` (`grep -rn -i curated` over `skills/`, `.claude/`, `docs/docs/`) <!-- completed: 2026-10-24T13:09 -->

### Step 4: Populate the catalog via the refresh procedure

- [x] Fetch all five approved sources per the refresh skill; stop and report on any fetch/parse failure <!-- completed: 2026-10-24T13:02 -->
- [x] Build the opencode table from the Console page per Specification § 1: every listed ID, past-deprecation IDs (bound through the *Endpoints* table's display-name → `Model ID` mapping) dropped, base-tier prices, notes for long-context rates, discounts and future deprecation dates <!-- completed: 2026-10-24T13:02 -->
- [x] Order the table most → least capable with a Class on every row, preserving the claude/codex relative order and placing unverified models last <!-- completed: 2026-10-24T13:02 -->
- [x] Rewrite the opencode catalog preamble per Specification § 1 <!-- completed: 2026-10-24T13:02 -->
- [x] Re-verify the `claude` and `codex` tables and the claude context windows per the existing refresh procedure <!-- completed: 2026-10-24T13:02 -->
- [x] Ask the maintainer (via the Director) to confirm keeping all six Role defaults and to approve the proposed catalog diff, including the ordering and classes <!-- completed: 2026-10-24T13:04 -->
- [x] Apply the approved diff and set the shared "last refreshed" date in `coding-agents.md` <!-- completed: 2026-10-24T13:09 -->

### Step 5: Docs-sync guard

- [x] Add the `opencode_catalog_rows_are_unique_prefixed_and_priced` test to `cafleet/tests/docs_sync.rs` (Specification § 9) <!-- completed: 2026-10-24T13:18 -->
- [x] Run `mise //cafleet:test opencode_catalog_rows` and confirm it passes against the populated table, then confirm it fails on a temporary local edit that duplicates a row or drops the `opencode/` prefix, and revert that edit <!-- completed: 2026-10-24T13:18 -->

### Step 6: Validation

- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck` <!-- completed: 2026-10-24T13:27 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-10-24T13:14 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-24 | Initial draft |
| 2026-10-24 | Reviewer round 1: deprecation binding and 76-row count, key-prefix family test and key equality, Claude Code / Codex explicit mapping, pinning boundary, catalog population before the docs-sync guard |
| 2026-10-24 | Reviewer round 2: delisted deprecated names need no action; display-name → ID mapping read from the *Endpoints* table |
| 2026-10-24 | Clarify that native Alias matches resolve to the corresponding Model token, keeping normalization consistent with the worked resolutions |
| 2026-10-24 | Complete implementation and validation, obtain independent Reviewer and maintainer approval, and publish pull request #487 |
