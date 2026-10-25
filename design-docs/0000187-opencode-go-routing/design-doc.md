# OpenCode Go catalog and `opencode-go/` provider preference

**Status**: Approved
**Progress**: 0/24 tasks complete
**Last Updated**: 2026-10-25

## Overview

The `cafleet-model-list-refresh` skill gains the OpenCode Go page as an approved source and maintains a separate `opencode-go/` table in the `opencode` Model catalog. Once the OpenCode backend is selected for a member and its model resolves to a bare model ID, the Director runs `opencode models` and spawns `opencode-go/<id>` whenever the operator's configured providers list it, otherwise `opencode/<id>`. Cost efficiency mode on OpenCode treats models available under `opencode-go/` as effectively free.

## Success Criteria

- [ ] `.claude/skills/cafleet-model-list-refresh/SKILL.md` lists the OpenCode Go page in its approved-source allowlist and specifies an OpenCode Go procedure that mirrors every Go model into the `opencode-go` table.
- [ ] `skills/cafleet/reference/coding-agents.md` `## opencode` → `### Model catalog` contains a `#### OpenCode Go` sub-heading whose table holds one `opencode-go/<model-id>` row per model on the Go page's *Endpoints* table on the refresh date, followed by notes explaining usage-limit pricing, plans and per-model monthly limits.
- [ ] `skills/cafleet/roles/director.md` resolves "glm-5.3 member" to `--coding-agent opencode --model opencode-go/glm-5.3` when `opencode models` lists `opencode-go/glm-5.3`, and to `opencode/glm-5.3` when it lists only `opencode/glm-5.3`.
- [ ] `director.md` honors provider-prefixed tokens, opencode Role defaults and family-word mappings verbatim (no `opencode-go/` swap), and keeps bare Claude/GPT names on their native backends.
- [ ] `director.md` relays to the operator, without spawning, when `opencode models` fails or lists neither candidate token.
- [ ] `director.md` § Model selection states that cost efficiency mode on OpenCode prefers a listed `opencode-go/` model that can finish the task reliably.
- [ ] The opencode Role defaults (`opencode/deepseek-v4.1-flash-free`, `opencode/glm-5.3-flash-free`) are unchanged.
- [ ] `docs/docs/concepts/coding-agents.md` and `docs/docs/spec/coding-agent-backends.md` describe the Go table and provider preference for operators.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass, including a docs-sync guard on the `opencode-go` table shape.

---

## Background

OpenCode Go is a subscription (Go $10/month, Go Plus $40/month, single-member workspaces only) that serves a set of models under the `opencode-go/` provider. Its page (`https://opencode.ai/v2/docs/console/go`, fetched 2026-10-25) publishes:

| Page part | Content relevant to the catalog |
|---|---|
| *Usage limits* price table | Per model: Input, Output, Cached Read, Cached Write (USD per 1M tokens) and a per-plan Monthly limit (USD/month). Usage counts these per-token amounts against limits of 20% per 5 hours, 50% per week and 100% per month of the model's monthly limit. |
| *Estimated requests* tables | One per plan (Go, Go Plus); the Monthly limit differs per plan (e.g. GLM-5.3-Flash $60 on Go, $180 on Go Plus). |
| *Usage beyond limits* | With Extra Usage enabled, Go falls back to the pay-as-you-go balance after the limits are reached. |
| *Endpoints* table | Columns Model, Model ID, Endpoint and AI SDK Package; 30 rows on the fetch date. |
| *Enable restricted models* / *Privacy* | `deepseek-v4.1-flash`, `deepseek-v4-pro` require the Global region; `muse-spark-1.3-contributor`, `muse-spark-1.2-contributor` require "Allow paid endpoints that train on request data". |

Price-table variations on the fetch date: long-context tiers (`grok-4.7`, `gpt-6-luna`, `gpt-5.6-luna`, `claude-haiku-5-5`, `qwen3.7-plus`), a 50% discount on `gemini-4-argon` through 2026-11-30, Off-Peak/Peak rates on `deepseek-v4.1-flash` and `deepseek-v4-pro`, and two free-for-a-limited-time models (`longcat-2.5-preview-free`, `beam-free`). No deprecation dates are published.

Most Go IDs also appear in the Console catalog (`opencode/`), including the Claude/GPT IDs `claude-haiku-5-5`, `gpt-6-luna` and `gpt-5.6-luna`. Some are Go-only (e.g. `kimi-k3.1-pro`, `hy3`, `step-5-preview`).

`opencode models [provider]` lists models from the configured providers in `provider/model` form. A user without a Go subscription sees no `opencode-go/` lines, so the preference reduces to today's `opencode/` behavior.

Design 0000185 established the current routing in `director.md` § Model selection: explicit backend → model-name inference → fleet backend, the five-row inference table, model-name normalization, the explicit-backend mapping and the failure table. This design keeps all of that and adds a provider-choice step after an OpenCode model ID is resolved.

---

## Specification

### 1. `opencode-go` catalog table

`### Model catalog` in the `## opencode` section of `skills/cafleet/reference/coding-agents.md` keeps the Console table and its notes first, then adds:

```markdown
#### OpenCode Go

<preamble>

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode-go/<model-id> | <class> | <input> | <output> |

<notes>
```

The level-4 heading keeps the five canonical level-3 subsections intact (the docs-sync subsection guard counts direct children only) and gives the table its own addressable section (`opencode-go` anchor).

**Rows.** One row per Model ID on the Go page's *Endpoints* table. The `Model` cell is `opencode-go/` + the ID copied verbatim. Columns match the Console table; Cached Read/Write prices are not carried.

| Case on the Go page | Row treatment |
|---|---|
| Listed model, single rate | One row at that rate. |
| Long-context tiers | One row at the base (shortest-prompt) tier; a note table lists each long-context rate. |
| Promotional discount | One row at the currently billed rate; a note table records the list price and the discount end date. |
| Off-Peak / Peak rates | One row at the Peak rate (the conservative planning estimate); a note table lists the Off-Peak rates. |
| Free for a limited time | `0.00` in both price columns. |
| Published deprecation date | Same rule as the Console procedure: keep with a note while in the future, drop once passed. |

**Ordering and Class.** One table ordered most → least capable with a Class on every row. A row whose ID also appears in the Console table reuses that row's Class text, and IDs shared with the Console, `claude` or `codex` tables keep the same relative order as there. Unverified preview models sit at the bottom with a Class that says so (existing rule).

**Preamble** (prose above the table) states:

- The table lists every model the OpenCode Go page lists; the `opencode-go/` prefix is part of the `--model` value.
- An `opencode-go/` token is available only to an operator whose configured providers include an OpenCode Go subscription; the Director checks availability per spawn under [Director model selection](../roles/director.md#model-selection).

**Notes** (below the table), in this order:

1. Pricing meaning: prices are per-token amounts in USD per MTok counted against the subscription's usage limits, not pay-as-you-go billing. Plans are Go ($10/month) and Go Plus ($40/month), for single-member workspaces only. Each model's usage is capped at 20% of its monthly limit per 5 hours, 50% per week and 100% per month; with Extra Usage enabled, usage beyond the limits bills the pay-as-you-go balance.
2. Monthly-limit table, one row per Go model.
3. Long-context, discount and Off-Peak note tables, one row per affected model.
4. Restricted models: the IDs that need a privacy setting and the setting each needs.
5. `Source: [OpenCode Go](https://opencode.ai/v2/docs/console/go).`

Every note table's `Model` cell is a code-span `opencode-go/<model-id>` token, as in the Console note tables. Headers:

| Note table | Header | Absent value |
|---|---|---|
| Monthly limits | `Model \| Go monthly limit $ \| Go Plus monthly limit $` | `—` when the page publishes no limit for that plan (shown as `-` or omitted) |
| Long-context rates | `Model \| Prompt length \| Input $/MTok \| Output $/MTok` (Console shape) | — |
| Discounts | `Model \| Discount \| List input $/MTok \| List output $/MTok \| Discount end` (Console shape) | `Not published` for a missing end date |
| Off-Peak rates | `Model \| Off-Peak input $/MTok \| Off-Peak output $/MTok \| Off-Peak hours` | `Not published` for missing hours |

The Console preamble gains one sentence pointing at the OpenCode Go table below it. The shared "last refreshed" date covers both tables. Role defaults stay `opencode/deepseek-v4.1-flash-free` and `opencode/glm-5.3-flash-free`, effort `—`.

### 2. Refresh skill

`.claude/skills/cafleet-model-list-refresh/SKILL.md` changes:

| Part | Change |
|---|---|
| § Approved sources | Add the row `OpenCode Go models and usage-limit pricing` → `opencode-go` prices and availability → `https://opencode.ai/v2/docs/console/go`. Rewrite the intro without a page count: fetch only the pages in this table; every page except Claude Code model configuration is pricing and availability authority; Claude Code model configuration is capability authority for `claude` context windows and `[1m]`, never a price source. |
| Refresh procedure step 1 | "Fetch all five approved sources" → "Fetch every approved source". |
| Refresh procedure step 3 | Refresh the `opencode` table with the Console procedure and the `opencode-go` table with the Go procedure. |
| New § OpenCode Go procedure | Added after § OpenCode Console procedure, steps below. |
| Description front matter, Role-defaults step, Cadence, Release-coupled deployment, Ownership boundary | Unchanged. |

**OpenCode Go procedure (the `opencode-go` table)**, run on every refresh:

1. **Price from the Go page only.** Use the Go page's per-token amounts from its price table; never price a Go row from the Console page or a vendor page.
2. **Copy the Go model ID exactly.** Map each display name to its `Model ID` through the Go page's *Endpoints* table (header `Model | Model ID | Endpoint | AI SDK Package`) and write `opencode-go/<model-id>`.
3. **Mirror the Go catalog.** Add every missing listed ID, remove every delisted ID, re-verify every remaining row's price. Apply deprecation dates with the Console procedure's binding rule if the page publishes any.
4. **Apply the row rules and maintain the notes** from Specification § 1, including the monthly-limit table for both plans and the restricted-models note. Each of these is a parse failure that stops the refresh under Refresh procedure step 1: a display name in the price table with no *Endpoints* ID or mapping to more than one ID, and an *Endpoints* ID with no price-table row or without both an Input and an Output price. A missing Monthly limit for one plan is recorded as `—` in the monthly-limit note (Specification § 1), because the Director never budgets against it.
5. **Order and classify every row** per Specification § 1. A new row's Class and position, and changes to existing ones, are reviewed policy changes approved in the same pull request.

### 3. Provider choice for an OpenCode model ID

A new `#### OpenCode provider choice` subsection in `director.md` (after § Explicit backend model mapping). It applies when an OpenCode spawn's model resolves to a **bare model ID** — inference row 4, or an explicit-OpenCode mapping row that yields an ID (§ 4). It does not apply to tokens that already carry a provider (§ 4 verbatim rows).

Candidate tokens: `opencode-go/<id>` when the Go table lists `<id>`, and `opencode/<id>` when the Console table lists `<id>`. At least one exists by construction, because the ID was resolved against those tables.

1. Run `opencode models` once for this spawn (the Director's own command).
2. Read its output as one `provider/model` token per line, matching candidate tokens exactly and ignoring every other provider.
3. Choose:

| `opencode models` result | `--model` |
|---|---|
| Non-zero exit, or no output | none — relay via {decision_surface} |
| Lists the `opencode-go/<id>` candidate | `opencode-go/<id>` |
| Lists only the `opencode/<id>` candidate | `opencode/<id>` |
| Lists neither candidate (e.g. a Go-only ID without a Go subscription) | none — relay via {decision_surface} |

The operator's explicit provider is never overridden: a reference that carries a provider prefix never reaches this procedure.

### 4. Changes to existing routing rules in `director.md`

**Model-name normalization step 1.** Replace "Strip the `opencode/` prefix from opencode catalog tokens before keying." with: "OpenCode candidates are the distinct bare IDs of the Console and OpenCode Go tables: strip the `opencode/` or `opencode-go/` prefix before keying, and count an ID listed in both tables once." Every consumer of step 1 inherits this de-duplication, so `claude-haiku-5-5` and `gpt-6-luna` (listed in both tables) are one candidate each. Step 3 (spelling dictionary) searches the Claude/GPT IDs among these OpenCode candidates.

**Inference table row 4 and row 5.**

| # | Model reference | Backend | Flags to pass |
|---|---|---|---|
| 4 | Any other bare name whose key equals the key of exactly one OpenCode candidate ID (normalization step 1) — `glm-5.3`, `kimi-k3.1-pro` | `opencode` | `--coding-agent opencode --model <token>`, token from OpenCode provider choice |
| 5 | Anything else — no key-equal ID, or more than one (e.g. `o3-mini`, `gemini`) | none | Ask the operator for the explicit `--coding-agent` + `--model` pair via {decision_surface} |

Rows 1–3 are unchanged. In the precedence paragraph, "even when the opencode catalog lists the model" becomes "even when either OpenCode table lists the model".

**Explicit backend model mapping → OpenCode.**

| Reference given with the OpenCode request | Resolved `--model` |
|---|---|
| No model (e.g. "spawn an opencode reviewer") | The opencode Role default for the role, verbatim |
| Provider-prefixed token (`opencode/…`, `opencode-go/…`, `anthropic/…`) | The token, verbatim |
| Claude family word (`claude`) | `opencode/` + the `claude` backend's Role default for the role, verbatim |
| GPT family word (`gpt`) | `opencode/` + the `codex` backend's Role default for the role, verbatim |
| Claude alias (`fable`, `opus`, `sonnet`, `haiku`) | The `claude` catalog Model the alias names, as an ID through OpenCode provider choice |
| Any other model name | The OpenCode candidate ID whose key equals the reference's key, through OpenCode provider choice |

Verbatim rows produce a complete token and skip provider choice. The two paragraphs under the table are rewritten:

| Current sentence | Replacement |
|---|---|
| "The last row uses only Model-name normalization step 1, including the leading-`claude` strip, against opencode catalog IDs. It requires exactly one key-equal ID, so "sonnet 4.6 with OpenCode" resolves to `opencode/claude-sonnet-4-6`; …" | "The last row uses only Model-name normalization step 1, including the leading-`claude` strip, against the OpenCode candidate IDs. It requires exactly one key-equal ID, so "sonnet 4.6 with OpenCode" resolves to the ID `claude-sonnet-4-6` before provider choice; normalization steps 2–4 apply only to native-backend resolution." |
| "With the current Role defaults, … `opus` maps to `opencode/claude-opus-5-5`." | Keep the `claude` and `gpt` examples; `opus` resolves the ID `claude-opus-5-5` and `haiku` the ID `claude-haiku-5-5`, which provider choice turns into `opencode-go/claude-haiku-5-5` when `opencode models` lists it. |
| "Require every mapped token, including a Role default and excluding the verbatim provider-prefixed row, to exist in the opencode catalog." | "Require every Role-default and family-word token to exist in the Console table, and every resolved ID to be an OpenCode candidate ID; the verbatim provider-prefixed row is exempt." |

The following sentence ("Relay a missing or ambiguous match through Model routing failures before spawning.") stays.

**Model routing failures.** The first row's condition becomes "Explicit OpenCode request whose mapped model is absent from both OpenCode tables (e.g. "claude-opus-4-1 with OpenCode")". The table gains three rows:

| Condition | Director behavior |
|---|---|
| `opencode models` exits non-zero or prints nothing during provider choice, cost efficiency selection or replacement | Relay via {decision_surface} before spawning; never fall back to `opencode/` |
| `opencode models` lists neither candidate token | Relay via {decision_surface} before spawning |
| Cost efficiency selection or replacement on OpenCode finds no listed token that can finish the task reliably | Relay an operator choice via {decision_surface}, as when no listed model fits the task |

A Go usage limit reached by a running member needs no new handling; the existing failure and Model replacement rules apply.

**Worked resolutions.** The table gains a `opencode models lists` column (`—` where provider choice does not run). The existing "glm-5.3 member" row splits into the first two rows below; the other rows are added, and every other existing row keeps its value with `—`.

| Operator says | Fleet backend | `opencode models` lists | Resolved flags |
|---|---|---|---|
| "glm-5.3 member" | `claude` | `opencode/glm-5.3`, `opencode-go/glm-5.3` | `--coding-agent opencode --model opencode-go/glm-5.3` |
| "glm-5.3 member" | `claude` | `opencode/glm-5.3` only | `--coding-agent opencode --model opencode/glm-5.3` |
| "kimi-k3.1-pro member" | any | `opencode-go/kimi-k3.1-pro` | `--coding-agent opencode --model opencode-go/kimi-k3.1-pro` |
| "kimi-k3.1-pro member" | any | no `opencode-go/` lines | none — relay (neither candidate listed) |
| "haiku member with OpenCode" | any | `opencode-go/claude-haiku-5-5` | `--coding-agent opencode --model opencode-go/claude-haiku-5-5` |
| "haiku member" | `opencode` | — | `--coding-agent claude --model claude-haiku-5-5` |
| "opencode/glm-5.3 member" | any | — | `--coding-agent opencode --model opencode/glm-5.3` |
| "claude member with OpenCode" | any | — | `--coding-agent opencode --model opencode/claude-opus-5-5` |
| "a member" (no model, no backend) | `opencode` | — | `--coding-agent opencode --model opencode/glm-5.3-flash-free` |
| "glm-5.3 member with OpenCode" | any | command fails | none — relay |

### 5. Cost efficiency mode and Model replacement

**Receiving sentence.** In `director.md` § Model selection, the sentence "In cost efficiency mode, compare models within the chosen backend's whole catalog, ordered most → least capable; the OpenCode catalog includes its Claude, GPT and free rows and retains the `opencode/` prefix." becomes "In cost efficiency mode, compare models within the chosen backend's whole catalog, ordered most → least capable; the OpenCode catalog comprises its Console table (`opencode/` prefix, including Claude, GPT and free rows) and its OpenCode Go table (`opencode-go/` prefix), ranked by OpenCode cost ranking below." The numbered OpenCode cost ranking follows that sentence. The "Ordinary members in cost efficiency mode" bullet stays; on OpenCode, its "cheapest listed model" is the model this ranking selects.

**OpenCode cost ranking.** Given a set of OpenCode candidate IDs (the whole catalog in cost efficiency mode, or the stronger set in Model replacement):

1. Run `opencode models` once for the spawn; relay a failure as in § 3.
2. If it lists at least one `opencode-go/` token, the Go candidates are the `opencode-go/<id>` tokens it lists for IDs in the set. Treat them as effectively free: choose a Go candidate that can finish the task reliably, preferring it over every Console token, including Console rows priced `0.00`. Among several reliable Go candidates, choose the one with the lowest Go-table rates, since lower amounts consume less of the usage limit.
3. Otherwise, or when no Go candidate can finish the task reliably, choose the cheapest reliable `opencode/<id>` token it lists for IDs in the set, by Console-table prices.
4. If neither step selects a token, relay an operator choice via {decision_surface} (Model routing failures).

The selected token is final; § 3's provider choice is not re-run.

**Model replacement.** `director.md` § Model replacement step 2 gains one sentence: "For an OpenCode member, the stronger set is every ID ranked above the failed ID in each OpenCode table that lists the failed ID, and OpenCode cost ranking chooses within it." A failed ID listed in both tables therefore also draws stronger Go-only rows (e.g. `kimi-k3.1-pro`); a Go-only failed ID draws only from the Go table; an empty set fails closed as today. The caps paragraph gains: "For an OpenCode member, the model in a `(task pointer, model)` pair is the bare ID, so `opencode/X` after a failed `opencode-go/X` is a retry of the same model." The pinning boundary table is unchanged: the provider the Director chose is not an operator pin.

### 6. Documentation surfaces

| File | Edit |
|---|---|
| `docs/docs/concepts/coding-agents.md` § Model choice | Revise the catalog and routing sentences and extend the cost efficiency paragraph as listed below. No repository paths. |
| `docs/docs/spec/coding-agent-backends.md` § Model selection | Add `opencode-go/glm-5.3` to the opencode example values. |
| `skills/cafleet/reference/coding-agents.md` | Console preamble sentence, `#### OpenCode Go` section per § 1, shared "last refreshed" date. |
| `skills/cafleet/roles/director.md` | § Model selection cost-efficiency sentence and OpenCode cost ranking (§ 5); normalization, inference rows 4–5, precedence wording, explicit mapping and its paragraphs, provider choice, failure rows and worked resolutions (§§ 3–4); § Model replacement step 2 and caps (§ 5). |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | Per § 2. |

Concepts page wording:

| Current text | Replacement |
|---|---|
| "The OpenCode catalog lists every Console model with its `opencode/` prefix, excluding models past their published deprecation date." | "The OpenCode catalog lists every Console model with its `opencode/` prefix, excluding models past their published deprecation date, and every OpenCode Go model with its `opencode-go/` prefix." |
| "Naming OpenCode for the member selects OpenCode with the `opencode/` form of that model." | "Naming OpenCode for the member selects OpenCode for that model. When the Director resolves an OpenCode model by name, it checks the operator's configured providers and uses the OpenCode Go form when they offer it, otherwise the Console form." |
| (new sentence after "The Director asks the operator to resolve an unknown or ambiguous reference.") | "It also asks when the provider check fails or offers neither form. A provider-prefixed model, a role default and a family-word request on OpenCode use their written or Console form as is." |
| (new sentence in the cost efficiency paragraph, after "The Director chooses a backend first and compares models within that backend.") | "On OpenCode, models available through the operator's OpenCode Go subscription count as effectively free and are preferred." |

`README.md`, `SPEC.md` and `.claude/rules/` need no edit: the CLI contract (`--model` pass-through, `<provider-id>/<model-id>` validation, error strings) is unchanged, and the overlay rule's five-subsection structure is preserved. The agent-facing and operator-facing homes state these facts independently, without linking each other.

### 7. Tests

In `cafleet/tests/docs_sync.rs`, extract the body of `opencode_catalog_rows_are_unique_prefixed_and_priced` into a helper taking the catalog region and the required prefix, then:

- keep `opencode_catalog_rows_are_unique_prefixed_and_priced` calling it on `section("opencode").section("model-catalog")` with `opencode/`;
- add `opencode_go_catalog_rows_are_unique_prefixed_and_priced` calling it on `section("opencode").section("model-catalog").section("opencode-go")` with `opencode-go/`.

Both assert the header `Model | Class | Input $/MTok | Output $/MTok`, the prefix on every `Model` cell, unique models, a non-empty Class and decimal prices with two or more fractional digits. `backend_sections_hold_the_canonical_subsections_in_order` and `backend_bindings_have_unique_values_and_local_model_defaults` stay unchanged and must keep passing; the latter reads the first table under `model-catalog`, which stays the Console table. Routing rules are Director prose and have no executable test.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Operator documentation

- [ ] Update `docs/docs/concepts/coding-agents.md` § Model choice per Specification § 6 <!-- completed: -->
- [ ] Add the `opencode-go/glm-5.3` example value in `docs/docs/spec/coding-agent-backends.md` § Model selection <!-- completed: -->
- [ ] Run `mise //docs:build` and fix any broken links <!-- completed: -->

### Step 2: Director routing policy

- [ ] Replace the cost-efficiency comparison sentence in `skills/cafleet/roles/director.md` § Model selection and add the OpenCode cost ranking after it (Specification § 5) <!-- completed: -->
- [ ] Update Model-name normalization steps 1 and 3, inference rows 4–5 and the precedence-paragraph wording (Specification § 4) <!-- completed: -->
- [ ] Replace the OpenCode explicit-mapping table and rewrite the two paragraphs under it per the replacement table (Specification § 4) <!-- completed: -->
- [ ] Add the `#### OpenCode provider choice` subsection (Specification § 3) <!-- completed: -->
- [ ] Rewrite the first Model routing failures row, add its three rows, and add the worked-resolutions column and rows (Specification § 4) <!-- completed: -->
- [ ] Add the OpenCode stronger-set and bare-ID cap sentences to § Model replacement (Specification § 5) <!-- completed: -->
- [ ] Confirm `grep -n -i "opencode catalog" skills/cafleet/roles/director.md` finds no sentence that still assumes a single `opencode/` table <!-- completed: -->
- [ ] Check every edited table cell against the two-sentence cap and `{decision_surface}` placeholder usage <!-- completed: -->

### Step 3: Refresh skill

- [ ] Add the Go page to § Approved sources and rewrite its intro without a page count (Specification § 2) <!-- completed: -->
- [ ] Update Refresh procedure steps 1 and 3 <!-- completed: -->
- [ ] Add § OpenCode Go procedure (Specification § 2) <!-- completed: -->

### Step 4: Populate the Go table via the refresh procedure

- [ ] Fetch every approved source per the refresh skill; stop and report on any fetch/parse failure <!-- completed: -->
- [ ] Build the `#### OpenCode Go` table, preamble and notes per Specification § 1, and add the Console preamble sentence pointing at it <!-- completed: -->
- [ ] Re-verify the `claude`, `codex` and `opencode` Console tables per the existing procedure <!-- completed: -->
- [ ] Ask the maintainer (via the Director) to confirm keeping all six Role defaults and to approve the proposed catalog diff, including the Go ordering and classes <!-- completed: -->
- [ ] Apply the approved diff and set the shared "last refreshed" date in `coding-agents.md` <!-- completed: -->

### Step 5: Docs-sync guard

- [ ] Extract the shared catalog-row helper and add `opencode_go_catalog_rows_are_unique_prefixed_and_priced` (Specification § 7) <!-- completed: -->
- [ ] Run `mise //cafleet:test opencode` and confirm both catalog tests pass, then confirm the Go test fails on a temporary local edit that duplicates a Go row or drops the `opencode-go/` prefix, and revert that edit <!-- completed: -->

### Step 6: Validation

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck` <!-- completed: -->
- [ ] Run `mise //docs:build` <!-- completed: -->
- [ ] Confirm `grep -nE -i "five|four|fifth|sixth" .claude/skills/cafleet-model-list-refresh/SKILL.md` finds no source count, and reread the rewritten § Approved sources intro and Refresh procedure step 1 for any remaining ordinal reference to a source <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-25 | Initial draft |
| 2026-10-25 | Reviewer round 1: Go note-table headers and parse-failure cases, replacement wording for every single-table sentence, candidate-ID de-duplication, OpenCode cost ranking shared by cost efficiency mode and Model replacement, concepts-page rewrites |
| 2026-10-25 | Reviewer and user approved; Status set to Approved |
