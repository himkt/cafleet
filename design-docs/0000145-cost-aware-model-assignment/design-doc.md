# Cost-Aware CAFleet Model Assignment

**Status**: Complete
**Progress**: 44/44 tasks complete
**Last Updated**: 2026-07-20

## Overview

Add an opt-in Director responsibility that chooses the least-cost CAFleet member model that satisfies a task's required capability profile and can replace an underpowered member with a stronger eligible model. The feature is activated only when the user's request contains the exact phrase `cost efficiency mode`; monitoring members remain the cheapest monitor-capable model and reviewers remain the most capable eligible model. A local, versioned model list will replace the current prose-only model table as the source of truth for model availability, reviewed capability policy, and token-price estimates. The feature ships as documentation and skills only: the Director reads the model list and decides; the cafleet Python package and its tests are unchanged.

## Success Criteria

- [x] A Director selects a backend and model from the local model list for an ordinary member when `cost efficiency mode` is present, by estimating the task's difficulty from the member's spawn prompt and choosing the cheapest listed model that can finish the task reliably.
- [x] Monitor selection chooses the cheapest listed model that can run the monitoring protocol reliably; reviewer selection chooses the most capable listed model.
- [x] An explicit Director or user `--coding-agent`, `--model`, or `--effort` remains an override and is recorded rather than silently replaced.
- [x] The committed model list records the official source links, the last-refreshed date, and standard input/output prices; capability classes are reviewed maintainer judgment.
- [x] A refresh skill updates the model list using only the supplied official Anthropic, OpenAI, and OpenCode Zen documentation sources — with a dedicated procedure for the Zen-specific pitfalls (exact model IDs, the `opencode/` prefix, curated subset, limited-time free models) — requires maintainer review, and never silently changes a selection policy.
- [x] A stale model list disables cost efficiency mode, and a task no listed model fits fails closed: the Director relays an operator choice instead of spawning a guessed model.
- [x] Evidence that a member is underpowered produces a bounded escalation to a strictly more capable listed replacement, or a user decision when no such replacement is available.
- [x] The feature ships as documentation and skills only: the cafleet Python package and its tests are unchanged.

---

## Background

Today `skills/cafleet/reference/director.md` contains a human-readable list of common Claude, Codex, and OpenCode models plus coarse intelligence labels. `cafleet member create` accepts pass-through `--model` and `--effort` values, but neither the CLI nor the Director workflow can compare model cost, assess task capability, or record why a model was chosen. That structure cannot support cost minimization constrained by reliable completion.

The initial catalog must be maintained locally from the official pricing and model information specified by the user: [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing) and [OpenAI pricing](https://developers.openai.com/api/docs/pricing). These sources publish standard token prices and pricing qualifiers; the catalog must retain the source timestamp and qualification rather than presenting estimates as an invoice guarantee.

---

## Specification

### Scope and activation

1. The feature applies to every CAFleet-native Director workflow that creates a member through `cafleet member create`, including existing design-doc, research, and future team workflows that load the `cafleet` Director guidance.
2. For an ordinary member, automatic selection is enabled only if the originating user request contains the case-insensitive standalone phrase `cost efficiency mode`. The Director passes that boolean and the original request into its selection step before rendering the spawn prompt.
3. Without the trigger, current workflow-specific model behavior is unchanged. The model list may be consulted for reference, but it must not alter a model choice.

The following precedence is normative, highest first:

1. A user-supplied explicit override wins. `--coding-agent` restricts eligible candidates to that backend; `--model` names a listed model token (or alias) and fixes both backend and model; a pinned backend/model pair whose model does not belong to the pinned backend is relayed to the user instead of spawned; any other token remains permitted through the existing manual `member create` path and is never a cost-mode result. `--effort` never selects or ranks a model: it is passed through only after the chosen backend's existing effort validation.
2. A documented workflow override may win only when it is explicitly marked `manual_override` with a reason in the workflow definition. Legacy overlay placeholders such as `{monitor_model}` and `{reviewer_model}` are policy defaults, not overrides, and are removed by this design.
3. The monitor/reviewer policies below run when neither of the preceding overrides applies. Ordinary-member automatic selection runs only when the trigger is active; otherwise existing unpinned workflow behavior runs unchanged.

A user-pinned model is never deleted and replaced automatically. If credible underpowered evidence arises, the Director relays the evidence and candidate upgrade to the user and waits for an explicit replacement/override decision. A workflow manual override follows the same rule unless its definition explicitly authorizes replacement.

4. The monitoring and reviewer roles are policy exceptions whenever a CAFleet team is spawned:

   | Role | Required policy | Cost-mode behavior |
   |---|---|---|
   | `monitor` | Use the cheapest listed model that can run the monitoring protocol reliably. | Applied regardless of the ordinary-member trigger; preserves the user's "cheapest model" requirement without selecting a model unable to run the monitoring protocol. |
   | `reviewer` | Use the most capable listed model of the chosen backend. | Applied regardless of the ordinary-member trigger; cost is not optimized. |
   | ordinary `member` | Use the cheapest listed model that can finish the task reliably, judged from the member's spawn prompt. | Applied only when `cost efficiency mode` is active. |

`eligible` always means the model is listed for a backend that satisfies its existing CAFleet readiness contract and the model list is fresh. The Director picks the backend first — the fleet's backend unless the user names one — and compares within that backend's table, which is ordered most → least capable. Cost efficiency mode and the monitor/reviewer policies cover all three cafleet backends: `claude`, `codex`, and `opencode` (priced through OpenCode Zen, with the `opencode/` prefix in the model value).

### Local model list

Create one source of truth at `skills/cafleet/reference/model-list.md`. It replaces the prose model lists in `skills/cafleet/reference/director.md`, which will retain only model-name/backend rules and a link to the model list/refresh skill; do not maintain a second, drifting list.

The model list is a catalog-style reference page read by the Director, not machine-parsed data: a preamble stating the maintenance rule, the 30-day freshness cadence, and the *last refreshed* date; a `Sources` section linking the three approved official pricing pages; and one table per backend (`claude`, `codex`, `opencode`) with each model's spawn token, alias (claude), reviewed capability class, and standard input/output USD-per-MTok prices. The `opencode` table is a curated subset of the OpenCode Zen catalog whose model values are `opencode/<zen-model-id>`. Each backend's table is ordered most → least capable; capability classes and the ordering are reviewed maintainer judgment, and changing them requires reviewed policy approval in the same pull request. A *Monitor and reviewer defaults* table names each backend's current monitor and reviewer choice, re-derived from the backend tables on every refresh. Every listed model name and alias is a valid `--model` token; arbitrary pass-through strings remain manual-only.

The repository file is the release source; it is not a runtime default. Each `cafleet setup` release asset copies the entire `skills/cafleet/` directory independently to `~/.claude/skills/cafleet`, `~/.codex/skills/cafleet`, and `~/.config/opencode/skills/cafleet`; a Director reads the model list from the exact `cafleet` skill root it loaded, and the Python wheel contains no model-selection code and no model-list copy. The model list is distributed as an ordinary reference page of the `cafleet` skill — no release manifest, content fingerprint, or sidecar file accompanies it. A list last refreshed more than 30 days ago is stale and disables cost efficiency mode. Backend readiness stays where it always was: `cafleet member create` errors on a missing backend binary or preset.

### Selection policy

Model selection is a Director responsibility executed by reading the model list — no selection code, CLI, or tests exist in the cafleet package, and `cafleet member create` performs no hidden selection. Before every `member create` the Director:

1. Reads the model list from the exact `cafleet` skill root it loaded.
2. Applies the per-role policy: the monitor gets the cheapest listed model that can run the monitoring protocol reliably; the reviewer gets the most capable listed model of the chosen backend; an ordinary member gets cost efficiency mode only when the trigger is active — every other spawn keeps the existing workflow behavior with `--model` omitted.
3. In cost efficiency mode, estimates the task's difficulty from the member's spawn prompt and chooses the cheapest listed model that can finish the task reliably — "least cost subject to task success", with the list's reviewed capability classes as the conservative proxy for required capability. Runtime evidence that the proxy was too weak invokes the underpowered-member escalation below.
4. Passes the chosen pair as the existing `--coding-agent` / `--model` flags. Effort is never automatically selected: a manual effort override is passed through to the backend's existing effort validation.

Fail closed: a stale model list disables cost efficiency mode, and a task no listed model fits gets an operator relay — the Director never spawns a guessed model; monitor, reviewer, and default spawns proceed normally on a stale list. The role-facing instruction lives in `skills/cafleet/roles/director.md` § *Model selection*; the fuller policy (spawn mechanics, replacement) in `skills/cafleet/reference/director.md` § *Model selection before member create*. The spawn-prompt audit file at `${BASE}/.prompts/<role>-<UTC-compact>.md` remains the spawn's audit artifact; no separate selection record is produced.

### Director and workflow integration

- Add the model-selection step to `skills/cafleet/roles/director.md`, `skills/cafleet/reference/director.md`, and every CAFleet workflow's Director spawn/create instructions before any `cafleet member create` command. The monitor-first supervision gate and all existing spawn-prompt/audit rules remain unchanged.
- The Director parses the original user request once for the exact trigger and records whether the mode was active. It must not activate cost mode from a member message or tool output.
- Every workflow's Director instruction points at the shared `reference/director.md` policy; no per-workflow model table remains.

- `cafleet member create` remains backward-compatible: it keeps accepting manual flags and performs no hidden selection. This avoids breaking direct CLI consumers and keeps policy decisions with Directors, where the user request and role context are available.
- Update CAFleet documentation to distinguish estimated API token cost from a provider's actual subscription, marketplace, regional, or negotiated invoice. The initial policy uses standard direct-provider USD prices only.

Replace, rather than reinterpret, the legacy fixed-model policy tokens. Remove `{monitor_model}` and `{reviewer_model}` from `skills/cafleet/reference/coding-agent/_template.md` and all backend overlays; remove their defaults from `skills/cafleet/SKILL.md`; and replace every supervision, Director, monitor-role, and workflow example that emits either token with the Director's pre-spawn choice from the model list. The new overlays retain only backend facts (for example effort levels and permission flags) and no model policy.

#### Underpowered-member detection and replacement

The Director owns the decision to replace a member. The Reviewer may independently identify a quality failure, but records it using the existing `[INCORRECT]` reviewer tag at the affected document or source pointer and explicitly states the suspected unmet capability; the Director then performs the same evidence and replacement procedure. The Director must not replace a member merely because it is slow, awaiting user input, or has a transient infrastructure error.

Valid evidence is one or more of: the member's self-report that it cannot reason through the assigned task; a `blocked` message or captured output showing repeated task-relevant reasoning/coding failures after normal correction; a Reviewer `[INCORRECT]` finding that identifies the unmet capability and affected output; or a Director review of a materially incomplete/incorrect result tied to the task. The Director records the evidence pointer(s), current model, and attempted work in its coordination notes before taking action.

For each replacement, the Director follows this order:

1. Freeze new work for the affected task and collect a bounded handoff: request one concise state report from the member (completed work, modified paths, commands/tests run, blockers, and next step). If it cannot respond promptly, capture its pane and use the capture as the handoff evidence.
2. Pick a strictly more capable model from the failed model's backend table (a row above the failed model) that fits the task's now-demonstrated difficulty; choose the cheapest model within that stronger set. It may cost more than the failed model.
3. Note the trigger, evidence pointers, old/new model, attempt number, and handoff artifact path in the coordination notes. Do not include secrets or full prompt contents.
4. Delete the old member through the standard `cafleet member delete` lifecycle before creating the replacement, preventing concurrent agents from editing the same task. The existing monitor remains live; all normal spawn, audit, and prompt-substitution rules apply to the new member.
5. Spawn the replacement with the original assignment plus the bounded handoff and the same deliverable paths. It resumes the task rather than starting a parallel implementation. The Director routes the original task pointer and asks the Reviewer to re-evaluate the resumed output when the workflow normally reaches review.

The initial member plus at most two replacements are allowed per task. Each replacement must be strictly more capable than its predecessor, and a `(task pointer, model)` pair may never be retried. If the failed member has an unlisted manual model or any explicit user override, the Director does not auto-replace it and instead relays a user decision. If the stronger eligible set is empty, the maximum is reached, or evidence remains ambiguous, the Director likewise fails closed and relays an operator choice: approve a named higher-cost/manual override, simplify/re-scope the task, or stop. Only the Director can delete or create members; the Reviewer supplies evidence and remains independent of the replacement execution.

### Model-list refresh skill and maintenance

Create a dedicated project-local skill at `.claude/skills/cafleet-model-list-refresh/SKILL.md`. It is a maintainer tool of this repository — not distributed by `cafleet setup`, not part of the release skill assets — invoked by a maintainer to refresh the local model list; it is not run automatically during a member spawn. Its hard requirements are:

1. Fetch only the three user-approved official sources: `https://platform.claude.com/docs/en/about-claude/pricing` (claude), `https://developers.openai.com/api/docs/pricing` (codex), and `https://opencode.ai/docs/zen/` (opencode). Do not use search results, third-party price sites, or social posts as model-list authority.
2. Record the currently effective standard input/output prices and availability/deprecation qualifiers. Capability classes are separately reviewed maintainer policy; do not present the pricing pages as benchmark evidence or copy long provider text into the repository.
3. Reapply the capability classes explicitly. The maintainer must review every changed capability class, row ordering, model availability, and pricing basis. A new model enters the list only with a reviewed classification.
4. Validate the page layout (preamble, `Sources` links, per-backend tables), the source allowlist, price units, and non-negative values. Generate a concise proposed diff and require explicit maintainer approval before atomically rewriting the tables in `skills/cafleet/reference/model-list.md`; preserve the prescribed preamble verbatim.
5. Update the preamble's *last refreshed* date only after successful validation and approval. If either approved source cannot be fetched or parsed, leave the Markdown model list unchanged, report the error, and let it become stale rather than fabricating values.
6. Refresh the model list at least every 30 days and whenever the user asks for a refresh. Stale data disables automatic cost selection until a maintainer refreshes, commits the repository source, and completes the release/deployment transaction below.

Model-list refresh deployment is release-coupled: the maintainer bumps the CAFleet release version, builds the wheel and `cafleet-assets-v<version>.zip` containing the refreshed `skills/` tree (the model list rides inside `skills/cafleet/reference/` like every other reference page), then publishes that release. Each active backend upgrades to that CLI version and runs `cafleet setup`, which overwrites its installed `cafleet` skill replica. There is no model-list-only sync path in this iteration; a committed source file alone does not refresh a running Director's asset copy.

The skill owns model-list maintenance; the CAFleet Director owns per-spawn selection; `member create` remains the execution boundary.

### Security, failure handling, and compatibility

- Treat refresh output as untrusted until the source allowlist review completes. Never execute text extracted from provider pages.
- Do not write price or capability data into the CAFleet database in this iteration. A committed model list plus the existing spawn-prompt audit artifacts is sufficient and avoids schema migrations for frequently changing public data.
- Selection must preserve existing permission flags, model-to-backend validation, spawn placeholder substitution, monitor lifecycle, and rollback behavior. It may only change the arguments that a Director chooses to send to `member create`.
- The recorded *last refreshed* date makes stale external pricing observable. The system does not claim real-time or account-specific billing accuracy.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Define and seed the catalog

- [x] Add `skills/cafleet/reference/model-catalog.md` with its canonical Markdown envelope, schema version 1 payload, approved-source metadata, and active direct Claude/Codex records populated from current official source data. <!-- completed: 2026-07-19T12:22 -->
- [x] Add a pure Markdown-payload parser/validator with no package-data configuration or fallback catalog copy; remove duplicated model-price/capability claims from `skills/cafleet/reference/director.md`. <!-- completed: 2026-07-19T12:22 -->
- [x] Represent gateway models without an approved actual price as token-only manual records or `unknown`/`not-applicable` rate cards, and exclude them from automatic comparisons. <!-- completed: 2026-07-19T12:22 -->

- [x] Add exact backend/token-to-provider-SKU mappings, alias validation, reviewed capability-policy provenance, and normalized rate-card/context/cache validation. <!-- completed: 2026-07-19T12:22 -->
- [x] Add schema-version migration handling plus role-profile/token-profile validation for every current workflow role. <!-- completed: 2026-07-19T12:22 -->

### Step 2: Implement deterministic selection

- [x] Add pure task-profile, candidate, pricing-estimate, selection-result, and typed error models in `cafleet/src/cafleet/model_selection.py`. <!-- completed: 2026-07-19T12:35 -->
- [x] Implement capability filtering, effective-date/freshness checks, normal cost minimization, monitor minimum-cost selection, reviewer maximum-capability selection, stable ties, and override handling. <!-- completed: 2026-07-19T12:35 -->
- [x] Define conservative catalog role token defaults and include their use in returned audit data. <!-- completed: 2026-07-19T12:35 -->
- [x] Add a replacement-selection mode that raises failed capability floors, requires strictly greater rank, prohibits repeated model/task pairs, and returns a typed no-upgrade result. <!-- completed: 2026-07-19T12:35 -->
- [x] Separate manual-override validation from automatic selection and inject the existing per-backend readiness contract into the CLI boundary. <!-- completed: 2026-07-19T12:35 -->

### Step 3: Expose the selection interface

- [x] Add `cafleet/src/cafleet/cli/model.py`, require absolute `--catalog skills/cafleet/reference/model-catalog.md`, and register `cafleet model select` in `cafleet/src/cafleet/cli/__init__.py`. <!-- completed: 2026-07-19T12:58 -->
- [x] Provide human-readable and `--json` output, validation errors with candidate exclusion reasons, and no shell-command synthesis. <!-- completed: 2026-07-19T12:58 -->
- [x] Document the exact `cost efficiency mode` trigger, manual override semantics, and estimated-cost limitations in CLI help and documentation. <!-- completed: 2026-07-19T12:58 -->
- [x] Call `ensure_assets_current()` at `model select` execution after Click parses arguments (not in help rendering), and validate the explicit loaded-skill-root catalog path against its release asset manifest/hash. <!-- completed: 2026-07-19T12:58 -->

### Step 4: Integrate Directors and audits

- [x] Update the shared Director role/reference and every CAFleet-native workflow Director instruction to classify the role task, invoke selection before spawning, and pass returned flags to `member create`; remove the legacy monitor/reviewer model placeholders from overlays, defaults, supervision, and workflow examples. <!-- completed: 2026-07-19T12:56 -->
- [x] Preserve the existing monitor-first gate and spawn-prompt artifact flow; add guarded two-phase `.selection/<selection_id>.pending.json` and final-result artifacts under the already resolved task base. <!-- completed: 2026-07-19T12:56 -->
- [x] Implement cost-mode fail-closed behavior and Director-relayed operator choice when no qualifying candidate exists. <!-- completed: 2026-07-19T12:56 -->
- [x] Implement the Director/Reviewer underpowered-member protocol: evidence marker, bounded handoff/capture, old-member deletion, higher-rank replacement, resumptive assignment, and two-replacement cap. <!-- completed: 2026-07-19T12:56 -->

### Step 5: Add the refresh skill

- [x] Create `skills/cafleet-model-catalog-refresh/SKILL.md` with the exact official-source allowlist, repository-source-only editing, retrieval, parsing, review, approval, validation, stale-data, and release-coupled deployment workflow. <!-- completed: 2026-07-19T13:12 -->

- [x] Ensure refresh failures make no catalog edit and that the skill records source hashes/effective dates without reproducing long provider text. <!-- completed: 2026-07-19T13:12 -->
- [x] Add `cafleet-model-catalog-refresh` to `SKILL_DIRS`; update setup output/docs, release archive-layout fixtures, and per-backend install/overwrite tests so the fourth skill and the catalog manifest are distributed atomically. <!-- completed: 2026-07-19T13:12 -->

### Step 6: Test the feature

- [x] Add unit tests for schema validation/migration, catalog freshness, pricing arithmetic, rate-card context limits, unknown costs, capability floors, stable ties, monitor/reviewer policy exceptions, and explicit overrides. <!-- completed: 2026-07-19T13:15 -->
- [x] Add CLI/schema tests for structured success/error/audit envelopes including `catalog_asset`, malformed profiles, stale/missing catalog errors, no-candidate errors, no-trigger compatibility, and missing/current/stale/skipped/mismatched candidate asset replicas while both top-level and subcommand help remain usable. <!-- completed: 2026-07-19T13:15 -->
- [x] Add workflow/Director tests or fixtures proving selected backend/model flags are forwarded unchanged to the existing `member create` path, legacy fixed pins do not bypass selection, and audit artifacts obey base-directory rules including the `<unset>` failure path. <!-- completed: 2026-07-19T13:15 -->
- [x] Test the Markdown parser's sentinel/canonical-JSON contract and prove an installed wheel requires the exact deployed skill-asset path, matching asset manifest, and matching catalog fingerprint rather than carrying or falling back to a packaged catalog copy. <!-- completed: 2026-07-19T13:15 -->

- [x] Add replacement-flow tests for self-report, Reviewer finding, capture fallback, capability-floor increase, old-member deletion before respawn, same-task/model prohibition, and the two-replacement escalation cap. <!-- completed: 2026-07-19T13:15 -->

### Step 7: Verify and document rollout

- [x] Run the full CAFleet test suite plus formatting and type checks; verify all existing manual `member create` tests remain byte-for-byte compatible when selection is not invoked. <!-- completed: 2026-07-19T13:34 --> (1189 passed via `mise //cafleet:test`; `mise //cafleet:lint` and `mise //cafleet:typecheck` clean; the pre-existing `member create` suites pass unchanged — no-selection spawns are byte-identical.)
- [x] Exercise representative dry runs for routine, normal, high-risk, monitor, reviewer, explicit-override, stale-catalog, and unsupported-gateway cases; review the generated decision records. <!-- completed: 2026-07-19T13:34 --> (Verifier report `.verifier/verification-report.md`: Director-sanctioned matrix substitution with named pytest evidence per case, plus live read-only E2E fail-closed probes and catalog-price verification against both approved sources.)
- [x] Publish maintainer guidance for the 30-day refresh cadence and require catalog review in changes that alter a model's capability class or rank. <!-- completed: 2026-07-19T13:34 --> (`skills/cafleet-model-catalog-refresh/SKILL.md` § Cadence and staleness + § Refresh procedure step 3; mirrored in the catalog preamble and `docs/concepts/model-selection.md`.)

### Step 8: Post-approval revision — plain catalog page, project-local refresh skill

- [x] Move the refresh skill to `.claude/skills/cafleet-model-catalog-refresh/SKILL.md` (project-local maintainer tool); remove `cafleet-model-catalog-refresh` from `SKILL_DIRS` and restore the three-skill archive contract in setup and its tests. <!-- completed: 2026-07-19T13:55 -->
- [x] Remove the asset-manifest machinery: delete `skills/cafleet/asset-manifest.json`, the loaded-skill-root/fingerprint validation, the `MODEL_CATALOG_ASSET_MISMATCH` and `MODEL_CANDIDATE_ASSET_UNAVAILABLE` codes, and the per-candidate replica checks; candidate backends are filtered by `ensure_available()` only and the audit record carries `catalog_path`. <!-- completed: 2026-07-19T13:55 -->
- [x] Sync `SPEC.md`, `docs/spec/cli-options.md`, `docs/concepts/model-selection.md`, `docs/contributing.md`, and the test suites to the plain-reference-page contract. <!-- completed: 2026-07-19T13:55 -->

### Step 9: Post-approval revision — three-table model list, catalog→list rename

- [x] Rename `model-catalog.md` → `model-list.md` and the refresh skill to `cafleet-model-list-refresh`; rename the CLI flag to `--model-list`, the error codes to `MODEL_LIST_*`, and the audit keys to `model_list` / `model_list_path`. <!-- completed: 2026-07-20T00:58 -->
- [x] Collapse the payload to the three tables `Metadata` / `Sources` / `Models` with prices and aliases inline per model row; move role and token profiles to reviewed code constants; remove rate cards, the token map, `currency`, schema migrations, and the canonical-token concept. <!-- completed: 2026-07-20T00:58 -->
- [x] Sync the parser/selector/CLI, the seeded model list, SPEC/docs/skills, and the consolidated test suites to the simplified contract. <!-- completed: 2026-07-20T00:58 -->

### Step 10: Post-approval revision — model selection covers claude and codex

- [x] Remove the OpenCode rows (no approved cost source) and the unpriced-component (`—` price cell) machinery from the model list, parser, selector, and tests; every listed row is fully priced. <!-- completed: 2026-07-20T01:47 -->
- [x] State the supported backends affirmatively everywhere: automatic selection (cost efficiency mode and the monitor/reviewer policies) covers `claude` and `codex`, a manual `--model` pin may name any cafleet backend, and an OpenCode member is spawned manually through `member create` with explicit flags. <!-- completed: 2026-07-20T01:47 -->

### Step 11: Post-approval revision — Director-owned selection, no codebase change

- [x] Delete the selection implementation and all its tests: `cafleet/src/cafleet/model_selection.py`, `cafleet/src/cafleet/cli/model.py`, the CLI registration, `tests/model_selection/`, the CLI model tests, and the selection repo tests — the cafleet Python package returns to its pre-design state. <!-- completed: 2026-07-20T02:20 -->
- [x] Reshape `skills/cafleet/reference/model-list.md` into a catalog-style page: preamble with the 30-day cadence and last-refreshed date, `Sources` links to the two official pricing pages, and per-backend tables with spawn token, alias, reviewed capability class, and standard input/output prices. <!-- completed: 2026-07-20T02:20 -->
- [x] Move the selection policy into the skills: `roles/director.md` § *Model selection* (estimate the task's difficulty from the member's spawn prompt, read the model list, choose the cheapest model that can finish the task; cost efficient mode only on the user's explicit trigger) and `reference/director.md` § *Model selection before member create*; sync SPEC/docs/workflow pages and the refresh skill. <!-- completed: 2026-07-20T02:20 -->

### Step 12: Post-approval revision — OpenCode Zen models

- [x] Add an `opencode` section to the model list — a curated subset of the OpenCode Zen catalog with exact `opencode/<zen-model-id>` values (`opencode/glm-5.2`, `opencode/kimi-k2.7-code`, `opencode/qwen3.5-plus`, `opencode/big-pickle`, `opencode/deepseek-v4-flash-free`) and Zen's own prices — and extend the coverage statements to all three backends. <!-- completed: 2026-07-20T02:17 -->
- [x] Add `https://opencode.ai/docs/zen/` to the refresh skill's source allowlist with a dedicated OpenCode Zen procedure: price from the Zen page only, copy the Zen model ID verbatim (never slugify the display name), re-verify every curated row on each refresh, price limited-time free models `0.00` and remove them when the offer ends without a published price, and keep the table a reviewed curated subset. <!-- completed: 2026-07-20T02:17 -->

### Step 13: Post-approval revision — review fixes: one canonical policy, within-backend comparison

- [x] Resolve the review findings by simplification: `roles/director.md` § *Model selection* becomes the single normative policy (reference/director.md and the concepts page point to or summarize it); the selection lead is scoped to the monitor/reviewer/cost-mode spawns while every other spawn keeps `--model` omitted; a pinned backend/model pair is consistency-checked and a mismatch relayed instead of spawned. <!-- completed: 2026-07-20T02:41 -->
- [x] Define the comparison rule — pick the backend first, compare within that backend's table, each table ordered most → least capable (reviewer and replacement are within-backend) — scope staleness to disabling cost efficiency mode only, unify the spelling on `cost efficiency mode`, fix the alias attribution (claude only), and declare the *last refreshed* date part of the page contract in the refresh skill. <!-- completed: 2026-07-20T02:41 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-19 | Initial draft: opt-in cost-aware Director selection, structured local catalog, and official-source refresh skill. |
| 2026-07-19 | Moved the authoritative catalog to `skills/cafleet/reference/model-catalog.md` with a canonical JSON payload and explicit skill-asset path. |
| 2026-07-19 | Defined release-coupled propagation, per-backend skill replicas, asset-manifest fingerprinting, and refresh-skill asset distribution. |
| 2026-07-19 | Implementation complete: 29/29 tasks, all Success Criteria verified, Verifier Phase D PASS, Reviewer approved (round 1), PR #216 opened. |
| 2026-07-19 | Post-approval revision: the asset manifest is a committed repository file updated by the maintainer at release time; the release workflow packages it verbatim and never generates it. |
| 2026-07-19 | Post-approval revision 2: the catalog is a plain `cafleet` reference page with no manifest or fingerprint validation, and the refresh skill is project-local under `.claude/skills/` (out of `SKILL_DIRS` and the release assets). |
| 2026-07-20 | Post-approval revision 3: the machine payload is seven fixed Markdown tables embedded in the reference page (user directive), replacing the sentinel marker and canonical-JSON block; redundant structural fields (`key`, `availability`, provenance type, pricing source) are derived by the parser. |
| 2026-07-20 | Post-approval revision 4 (user directive: simplify as much as possible): the catalog is renamed to the **model list** (`model-list.md`, `--model-list`, `MODEL_LIST_*` codes, `cafleet-model-list-refresh`); the payload collapses to three tables (`Metadata`, `Sources`, `Models`) with per-row prices and aliases; role/token profiles become reviewed code constants; rate cards, the token map, `currency`, schema migrations, and the canonical-token concept are removed. |
| 2026-07-20 | Post-approval revision 5 (user directive): OpenCode has no approved cost source, so its rows and the unpriced-component machinery are removed; automatic selection (cost efficiency mode and the monitor/reviewer policies) covers the `claude` and `codex` backends, a manual `--model` pin may name any cafleet backend, and an OpenCode member is spawned manually through `member create`. |
| 2026-07-20 | Post-approval revision 6 (user directive): the feature carries **no codebase change** — the `cafleet model select` CLI, the `model_selection` module, and all their tests are deleted; model selection is a Director responsibility executed by reading the model list, which becomes a catalog-style page (models, capability classes, input/output prices, and official source links); the cost-efficient instruction lives in `roles/director.md`. |
| 2026-07-20 | Post-approval revision 7 (user directive): OpenCode joins the model list via OpenCode Zen (`https://opencode.ai/docs/zen/`) — a curated five-model `opencode` section with exact `opencode/<zen-model-id>` values and Zen prices; all three backends are covered by cost efficient mode, and the refresh skill gains a dedicated OpenCode Zen procedure for the Zen-specific pitfalls. |
| 2026-07-20 | Post-approval revision 8 (code-review fixes): `roles/director.md` is the single normative policy; the pick-backend-first / within-backend comparison rule replaces the lost global ordering (each table ordered most → least capable); the selection lead is scoped so default spawns keep `--model` omitted; pinned backend/model pairs are consistency-checked with mismatches relayed; staleness disables cost efficiency mode only; the one spelling is `cost efficiency mode`; aliases are claude-only; the *last refreshed* date is declared part of the page contract. |
| 2026-07-20 | Post-approval revision 9 (user directive): the removed `{monitor_model}` / `{reviewer_model}` overlay pins return as a **mention, not policy** — a *Monitor and reviewer defaults* table in the model list names each backend's current monitor/reviewer choice, re-derived from the backend tables on every refresh so it cannot go stale like the fixed overlay tokens. |
