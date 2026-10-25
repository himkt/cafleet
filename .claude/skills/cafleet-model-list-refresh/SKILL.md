---
name: cafleet-model-list-refresh
description: >-
  Refresh the CAFleet model list at skills/cafleet/reference/coding-agents.md
  from the approved official pricing and capability sources. Use when a
  maintainer asks to refresh, update, or re-verify the model list, its token
  prices, its context windows, or its freshness, or when a Director reports
  the list was last refreshed more than 30 days ago. Maintainer-invoked only —
  never run automatically during a member spawn.
---

# CAFleet Model List Refresh

Refresh the repository's model catalogs and canonical Role defaults in
`skills/cafleet/reference/coding-agents.md` — from the approved official
sources, under explicit maintainer review. The repository file is the release
source: a running Director reads only its deployed skill replica, so a refresh
reaches Directors exclusively through the release/deployment transaction below.

This skill maintains each backend's Model catalog, its provenance/context notes, its canonical Role defaults table, and shared/catalog-specific freshness metadata. Preserve Runtime bindings, bound runtime notes, and worked resolutions as runtime documentation owned outside model refreshes. The common catalog preamble specifies maintainer-owned refresh at least every 30 days, a last-refreshed date for model data, standard provider USD per MTok as planning estimates, reviewed capability ordering, and Director ownership of selection policy.

## Approved sources (exhaustive allowlist)

Fetch **only** the official pages listed in this table, using each for the
facts identified in its row. Claude Code model configuration is capability authority for the `claude`
backend's context windows and `[1m]` applicability; use the pricing sources
to price models, never the capability page. Search results, third-party price
sites, and social posts are never model-list authority.

| Source | What it feeds | URL |
|---|---|---|
| Anthropic pricing | `claude` prices | `https://platform.claude.com/docs/en/about-claude/pricing.md` |
| OpenAI pricing | `codex` prices | `https://developers.openai.com/api/docs/pricing.md` |
| Codex model availability | `codex` availability | `https://learn.chatgpt.com/docs/models.md` |
| OpenCode Console models and pricing | `opencode` prices and availability | `https://opencode.ai/v2/docs/console/models/` |
| OpenCode Go models and usage-limit pricing | `opencode-go` prices and availability | `https://opencode.ai/v2/docs/console/go` |
| Claude Code model configuration | `claude` context windows and `[1m]` applicability | `https://code.claude.com/docs/en/model-config.md` |
| Nous Portal models | `hermes` catalog IDs, types and Portal-billed prices | `https://portal.nousresearch.com/models` |

## Refresh procedure

1. **Confirm the refresh scope and fetch its approved sources.** A full
   refresh covers every maintained backend catalog; a maintainer-selected
   partial refresh covers only its selected catalogs, including a Hermes-only
   bootstrap. Fetch every source feeding that scope. If any required source
   cannot be fetched or parsed, **stop**: leave the model list unchanged, report the error, and
   let it become stale rather than fabricating values.
2. **Extract pricing facts only**: the currently effective standard input and
   output prices (USD per MTok) and availability/deprecation qualifiers. Do
   not copy long provider text into the repository, and do not present the
   pricing pages as benchmark evidence — they are pricing/availability
   sources only.
3. **Follow the dedicated procedure for each selected catalog.** Provider
   pricing pages feed the catalogs identified in the approved-source table;
   catalogs with a dedicated procedure use that procedure below. For `codex`,
   pricing and availability come from different pages: price each model from the OpenAI
   pricing page, but take Codex availability from the Codex models page
   (`https://learn.chatgpt.com/docs/models.md`) — the table carries only the
   models Codex currently offers in its model picker. A model absent there
   or demoted to a **legacy model** (reachable only via `codex -m` /
   `config.toml`) is removed from the table even when the pricing page
   still prices it; legacy models remain manual pass-through `--model`
   values.
4. **Ask the maintainer for role defaults.** Show the current and available
   catalog tokens for reviewer and other members on each selected backend.
   Ask which model each role should use and which supported effort level
   should accompany it when its backend supports effort. Record the maintainer's
   choices in the proposed diff. OpenCode omits effort because its backend
   does not support it. A refresh keeps the existing role defaults only when
   the maintainer explicitly chooses to keep them.
5. **Reapply the capability classes explicitly.** The `Class` descriptions
   and the most-to-least-capable row ordering are reviewed maintainer
   judgment, not provider benchmark claims; changing a class or the ordering
   requires reviewed policy approval in the same pull request. Every row
   carries both prices from its approved source. Set the Role defaults within
   the refresh scope from the maintainer's choices, using local catalog entries or aliases and
   effort levels accepted by the selected backend.
6. **When `claude` is selected, re-verify every row's context window**, even
   when the prices are unchanged. Take each window from the Claude Code model
   configuration page's *Extended context* section and record it in the
   `claude` table's `Context` column. That page is also the authority for
   whether a row's `--model` value carries the `[1m]` suffix, which is a
   Claude Code model-string suffix stripped before the request reaches the
   provider — it is meaningless for the `codex` and `opencode` tables, whose
   values never carry it. Apply the suffix only where it changes the window a
   member actually gets:
   - A model the page lists as always running at 1M on the Anthropic API
     carries **no** suffix — the window is already 1M and the suffix is inert.
   - A model the page gives no 1M variant carries **no** suffix.
   - A model without 1M support carries **no** suffix; appending it there is a
     malformed value.
   - Where the page shows the window is opt-in for the reader's plan, the
     suffix belongs in the page's note rather than the row, so the Director
     applies it per spawn against the operator's plan instead of every
     spawn inheriting a credit-billed window.

   A suffixed value must be single-quoted everywhere it appears in a command,
   because `[1m]` is a glob pattern that fails an unquoted zsh invocation.
7. **Propose, then apply atomically.** Generate a concise proposed diff and
   require explicit maintainer approval before applying the model-data changes in
   `skills/cafleet/reference/coding-agents.md`. Preserve the common catalog
   preamble contract described above. Apply catalogs, provenance/context notes,
   Role defaults, and the applicable freshness metadata together after approval and
   successful validation. Advance the shared *last refreshed* date only when
   every catalog it covers has been verified. A partial refresh preserves
   unrelated catalogs, defaults and freshness, and records the selected
   catalog's verification date without making unrelated data appear fresh.
   A failed refresh makes **no** edit.

## OpenCode Console procedure (the `opencode` table)

The `opencode` catalog mirrors every model listed by the OpenCode Console,
excluding models past their published deprecation date, and uses the Console's
prices. Model IDs differ from display names, and free/preview models appear
and disappear. Re-verify the complete Console catalog whenever it is selected:

1. **Price from the Console page only.** Console's own USD-per-MTok rates are
   the billing rates for the `opencode` backend. Never price a Console model
   from the upstream vendor's page (Anthropic, OpenAI, DeepSeek, …) — the
   Console rate is the one cafleet members are billed at.
2. **Copy the Console model ID exactly.** Use the *Endpoints* table under
   `## Endpoints`, whose header is `Model | Model ID | Endpoint | AI SDK Package`,
   to map each display name ("GLM 5.2", "Kimi K2.7 Code") to its `Model ID`
   (`glm-5.2`, `kimi-k2.7-code`). The *Pricing* table keys prices by the same
   display name. Set the row's `--model` value to `opencode/<console-model-id>`:
   the literal `opencode/` prefix followed by the ID copied verbatim from the
   *Endpoints* table.
3. **Mirror the Console catalog.** Add every missing listed model ID, remove
   every delisted model ID or model past its published deprecation date, and
   re-verify every remaining row's price. Bind each *Deprecated models* entry
   to its model ID through the *Endpoints* display-name mapping; a passed date
   excludes the row even while it appears in active pricing. A deprecated
   display name with no listed ID is already delisted and needs no action.
   A deprecated display name mapping to more than one ID is a parse failure:
   stop the refresh under Refresh procedure step 1.
4. **Apply the row rules and maintain their notes.** Keep one row per model
   ID, using the base (shortest-prompt) tier for tiered prices and listing each
   long-context rate beneath the table. Use the currently billed Console rate
   for promotional discounts; note the list price and discount end date when
   published. Record each published free rate as `0.00` in its own column,
   including models with only one free rate. A model free for a limited time
   has `0.00` in both columns; update its rates when the offer ends or remove
   it if delisted. Keep a model with a future deprecation date and record that
   date beneath the table.
5. **Order and classify every row.** Keep a single table ordered most → least
   capable across the whole catalog, with a short `Class` on every row and the
   same relative order for models shared with the `claude` / `codex` catalogs.
   Place unverified stealth/preview models at the bottom and state that their
   capability is unverified in their Class. A new row's Class and position,
   and changes to existing classes or positions, are reviewed policy changes
   approved in the same pull request.

## OpenCode Go procedure (the `opencode-go` table)

Run this procedure whenever this catalog is selected. Maintain a separate `#### OpenCode Go`
section after the Console table and its notes, within the `opencode` backend's
`### Model catalog`. Point to it from the Console preamble; the shared
last-refreshed date covers both tables.

1. **Price from the Go page only.** Use the Go page's per-token amounts from
   its price table. These amounts count against subscription usage limits;
   never price a Go row from the Console page or a vendor page.
2. **Copy the Go model ID exactly.** Map each display name to its `Model ID`
   through the Go page's *Endpoints* table, whose header is
   `Model | Model ID | Endpoint | AI SDK Package`. Write
   `opencode-go/<model-id>`, with the ID copied verbatim.
3. **Mirror the Go catalog.** Add every missing listed ID, remove every
   delisted ID, and re-verify every remaining row's price. Apply published
   deprecation dates using the Console procedure's display-name binding rule:
   keep a future-dated model with a note and remove one whose date has passed.
4. **Apply the row rules and maintain the notes.** Use the Console table's
   columns: `Model | Class | Input $/MTok | Output $/MTok`, with one row per Go
   *Endpoints* ID. A price-table display name with no *Endpoints* ID or more
   than one ID, or an *Endpoints* ID with no price-table row or without both
   Input and Output prices, is a parse failure: stop under Refresh procedure
   step 1.

   Use the base (shortest-prompt) tier for long-context pricing, the currently
   billed rate for promotional discounts, and the Peak rate when the page
   publishes Off-Peak and Peak rates. Record a free-for-a-limited-time model
   with `0.00` in both columns.

   The preamble states that this table lists every model on the Go page and
   that `opencode-go/` is part of the `--model` value. Explain that these tokens
   require configured providers that include an OpenCode Go subscription and
   that the Director checks availability per spawn under
   [Director model selection](../../../skills/cafleet/roles/director.md#model-selection).

   Place these notes beneath the table in this order:

   - Explain that prices are USD per MTok counted against subscription usage
     limits, rather than pay-as-you-go billing. Record the published plan
     prices and single-member-workspace restriction. Explain each model's
     caps of 20% of its monthly limit per 5 hours, 50% per week and 100% per
     month, and that Extra Usage bills usage beyond the limits to the
     pay-as-you-go balance.
   - List every Go model in a monthly-limit table with the header
     `Model | Go monthly limit $ | Go Plus monthly limit $`. Record a plan's
     absent limit (a `-` or an omitted value) as `—`; the Director never
     budgets against it.
   - List each affected model in the applicable long-context, discount and
     Off-Peak tables. Use the headers
     `Model | Prompt length | Input $/MTok | Output $/MTok`,
     `Model | Discount | List input $/MTok | List output $/MTok | Discount end`,
     and `Model | Off-Peak input $/MTok | Off-Peak output $/MTok | Off-Peak hours`.
     Record every long-context rate, each discount's list prices and end date,
     and each Off-Peak rate and its hours. Use `Not published` for an absent
     discount end date or Off-Peak hours.
   - Identify each restricted model by ID and the privacy setting it requires.
   - End with `Source: [OpenCode Go](https://opencode.ai/v2/docs/console/go).`

   Every note-table `Model` cell is a code-span `opencode-go/<model-id>` token.
5. **Order and classify every row.** Keep one table ordered most → least
   capable, with a Class on every row. Reuse the Console Class for each shared
   ID, and preserve the relative order of IDs shared with the Console,
   `claude` or `codex` tables. Place unverified preview models at the bottom
   and state that their capability is unverified in their Class. A new row's
   Class and position, and changes to existing ones, are reviewed policy
   changes approved in the same pull request.

## Hermes procedure (the `hermes` table)

Use https://portal.nousresearch.com/models as the catalog and billing authority.
A Hermes-only refresh follows this procedure within its explicit scope; preserve
the other catalogs, their defaults and the shared freshness date. Initializing
model data grants no Hermes runtime milestone or automatic selection eligibility.

1. **Capture the complete approved page.** Save the official Models page's
   HTML and record its acquisition date and visible Text, Embeddings, total,
   displayed range and page counts. Select the complete catalog view. If direct
   access fails, use an authorized browser to save that same approved page.
   Keep snapshots and resources in the task's scratch area; repository tests
   use synthetic fixtures. A first rendered table is insufficient.
2. **Establish coverage of every display page.** The supported saved format
   carries the complete ModelsBrowser `rows` prop across JSON-encoded React
   Flight string fragments. Verify the official client resource filters and
   slices that full input in memory, and reconcile all unique IDs with the
   visible type/total counts and pagination. Otherwise fetch and reconcile
   every page through a reviewed complete format before proceeding. A changed
   format or unverifiable pagination stops the refresh for parser review.
3. **Extract facts offline.** From this skill directory, run:

   ```text
   python3 -B scripts/extract_hermes_portal.py /absolute/path/to/saved-models.html
   ```

   The standard-library extractor decodes JSON without evaluating JavaScript,
   concatenates every Flight string fragment and selects the single `rows`
   property. It validates the entire array before emitting one JSON object to
   stdout, containing metadata and all model facts in source order. It performs
   no network, subprocess, profile, catalog or filesystem mutation. Missing or
   malformed packets, incomplete/conflicting arrays, duplicate IDs, missing
   fields, inconsistent types/counts/pages, invalid rates and unreviewed fields
   fail with an error and no partial stdout facts. Run its synthetic offline
   checks from the repository root with `mise //:model-refresh-test`.
4. **Retain exact identity and type.** Select every fact whose `modality` is
   `text` for the coding-agent catalog. Keep Embeddings in completeness evidence
   and outside that catalog. Copy IDs exactly, preserving provider prefixes,
   suffix case, free/region/preview/version variants and every other distinction.
   Display names are labels, so repeated names remain separate rows by exact
   ID. A missing or contradictory ID/type stops the refresh; infer neither from
   a display name or an upstream catalog.
5. **Preserve billed rates and qualifiers.** Use Portal's input/output USD per
   million tokens, with each published zero rate recorded as `0.00`. The fact
   output preserves exact JSON decimal precision, both list rates, discount
   flags/badges and free variants. Retain that precision separately from any
   reviewed table display rounding. Record every promotional rate, list-rate
   comparison, published end date and time/context/tier qualifier. The current
   payload schema publishes no end-date or tier fields, so its metadata says
   `Not published`; independently reconcile this with visible pricing and
   official qualifiers before proposing an update. A newly published qualifier
   requires parser/fixture review, rather than silently dropping it. An absent
   necessary fact stops the refresh with the previous model data intact.
6. **Review eligibility and policy separately.** Price and Text type supply
   catalog facts, rather than evidence of agent tool loops. Give each row a
   reviewed Class, capability ordering and explicit tool-loop eligibility note.
   Keep unverified choices manual until the applicable runtime/tool-use evidence
   supports automatic role selection. Ask the maintainer to select reviewer and
   ordinary-member defaults and supported reasoning effort, including an
   explicit choice to retain existing values where they exist. Preserve exact
   manual Hermes tokens without OpenCode mapping. Portal cost comparisons apply
   only with verified effective Nous Portal routing and an eligible model;
   another provider or an unverified `auto` route requires an operator choice.
7. **Propose the complete scoped transaction.** Present the exact catalog,
   classes/order, eligibility, rates/qualifiers, role defaults, effort and
   applicable verification-date change for explicit maintainer approval.
   Apply only that approved model-data scope atomically through the release
   procedure below after validation. Preserve unrelated data and freshness;
   retain the previous state intact whenever collection, validation or approval
   is incomplete. Runtime setup, installation and support gates remain separate.

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
2. Builds the release binary with the repository `skills/` tree embedded,
   including `skills/cafleet/reference/coding-agents.md`, and publishes the
   per-target release archives.
3. Upgrades the installed CLI to that release and runs `cafleet setup`, which
   installs its embedded skill payload for the selected backends, including
   the unified reference. The binary upgrade and setup complete the deployment.

Installing skills directly from a checkout is a separate contributor operation;
it does not replace the embedded-binary build/release → upgrade → setup path.

## Ownership boundary

This skill owns model catalogs, provenance/context notes, canonical Role defaults,
and freshness metadata in the unified reference. Runtime documentation maintenance
owns Runtime bindings, bound notes, and worked resolutions. The CAFleet
Director owns per-spawn selection under `skills/cafleet/roles/director.md`; `cafleet member create` remains the execution
boundary.
