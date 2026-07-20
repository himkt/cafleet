# Cost-Aware CAFleet Model Assignment

**Status**: Complete
**Progress**: 35/35 tasks complete
**Last Updated**: 2026-07-20

## Overview

Add an opt-in Director responsibility that chooses the least-cost CAFleet member model that satisfies a task's required capability profile and can replace an underpowered member with a stronger eligible model. The feature is activated only when the user's request contains the exact phrase `cost efficiency mode`; monitoring members remain the cheapest monitor-capable model and reviewers remain the most capable eligible model. A local, versioned model list will replace the current prose-only model table as the source of truth for model availability, reviewed capability policy, and token-price estimates.

## Success Criteria

- [x] A Director can deterministically select a backend and model from a local model list for an ordinary member when `cost efficiency mode` is present.
- [x] Selection minimizes estimated USD token cost only among models that meet every required task-capability floor and runtime-availability constraint.
- [x] Monitor selection chooses the least-cost listed model that meets the monitor baseline; reviewer selection chooses the highest-ranked eligible listed model.
- [x] An explicit Director or user `--coding-agent`, `--model`, or `--effort` remains an override and is recorded rather than silently replaced.
- [x] The committed model list records source URLs, retrieval time, freshness state, and the input/output/cached-token prices needed for an estimate; capability levels are reviewed maintainer judgment.
- [x] A refresh skill updates the model list using only the supplied official Anthropic and OpenAI documentation sources, requires maintainer review, and never silently changes a selection policy.
- [x] Missing, stale, unsupported, or incomparable data fails closed for automatic cost selection and produces an actionable decision record.
- [x] Evidence that a member is underpowered produces a bounded, auditable escalation to a strictly more capable eligible replacement, or a user decision when no such replacement is available.
- [x] Unit and CLI tests cover eligibility, pricing arithmetic, special roles, overrides, stale model lists, deterministic ties, and no-regression behavior when the trigger is absent.

---

## Background

Today `skills/cafleet/reference/director.md` contains a human-readable list of common Claude, Codex, and OpenCode models plus coarse intelligence labels. `cafleet member create` accepts pass-through `--model` and `--effort` values, but neither the CLI nor the Director workflow can compare model cost, assess task capability, or record why a model was chosen. That structure cannot support cost minimization constrained by reliable completion.

The initial catalog must be maintained locally from the official pricing and model information specified by the user: [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing) and [OpenAI pricing](https://developers.openai.com/api/docs/pricing). These sources publish standard token prices and pricing qualifiers; the catalog must retain the source timestamp and qualification rather than presenting estimates as an invoice guarantee.

---

## Specification

### Scope and activation

1. The feature applies to every CAFleet-native Director workflow that creates a member through `cafleet member create`, including existing design-doc, research, and future team workflows that load the `cafleet` Director guidance.
2. For an ordinary member, automatic selection is enabled only if the originating user request contains the case-insensitive standalone phrase `cost efficiency mode`. The Director passes that boolean and the original request into its selection step before rendering the spawn prompt.
3. Without the trigger, current workflow-specific model behavior is unchanged. The catalog may be consulted for validation and audit, but it must not alter a model choice.

The following precedence is normative, highest first:

1. A user-supplied explicit override wins. `--coding-agent` restricts eligible candidates to that backend; `--model` resolves through the catalog's exact token map and fixes both backend and model; a model/backend pair must resolve to the same mapped record or is rejected. A model token that cannot be mapped is still permitted only through the existing manual `member create` path, with `estimate_status: unavailable`; it is never a cost-mode result. `--effort` never selects or ranks a model: it is passed through only after the chosen backend's existing effort validation. Thus a backend-only override selects the lowest-cost capable catalog model within that backend, a model-only override infers its mapped backend, an effort-only override leaves model selection unchanged, and combined fields apply all of those rules.
2. A documented workflow override may win only when it is explicitly marked `manual_override` with a reason in the workflow definition. Legacy overlay placeholders such as `{monitor_model}` and `{reviewer_model}` are policy defaults, not overrides, and are removed by this design.
3. The monitor/reviewer policies below run when neither of the preceding overrides applies. Ordinary-member automatic selection runs only when the trigger is active; otherwise existing unpinned workflow behavior runs unchanged.

A user-pinned model is never deleted and replaced automatically. If credible underpowered evidence arises, the Director relays the evidence and candidate upgrade to the user and waits for an explicit replacement/override decision. A workflow manual override follows the same rule unless its definition explicitly authorizes replacement.

4. The monitoring and reviewer roles are policy exceptions whenever a CAFleet team is spawned:

   | Role | Required policy | Cost-mode behavior |
   |---|---|---|
   | `monitor` | Use the lowest estimated-cost model that meets the catalog's `monitor` capability baseline. | Applied regardless of the ordinary-member trigger; preserves the user's "cheapest model" requirement without selecting a model unable to run the monitoring protocol. |
   | `reviewer` | Use the highest global capability rank among eligible models that meets the reviewer baseline. | Applied regardless of the ordinary-member trigger; cost is recorded but not optimized. |
   | ordinary `member` | Minimize estimated task cost subject to all required capability floors. | Applied only when `cost efficiency mode` is active. |

`eligible` always means that the selected model is mapped to the requested backend, the backend satisfies its complete existing CAFleet readiness contract, the catalog data is fresh, and the record has the fields required by the role's policy.

### Local model list

Create one machine-readable source of truth at `skills/cafleet/reference/model-list.md`. It replaces the prose model lists in `skills/cafleet/reference/director.md`, which will retain only model-name/backend rules and a link to the model list/refresh skill; do not maintain a second, drifting list.

The model list is a Markdown document whose machine payload is a fixed set of simple Markdown tables embedded in the skill reference page, not a JSON file packaged with Python. Its human-readable preamble identifies the model list and its maintenance rule; it is followed by exactly three `##` sections in fixed order — `Metadata`, `Sources`, `Models` — each containing exactly one Markdown table with an exact expected column header and nothing else but blank lines. The `Metadata` table carries `schema_version`, `generated_at`, and `freshness_days` as fixed-order `Field | Value` rows. Cells are plain scalars: integers and USD-per-MTok decimals as literals, booleans as `yes`/`no`, and the em dash `—` for an absent value (an unpriced component or an alias-free model). Each `Models` row carries the model's backend, spawn token (`Model`), optional comma-separated aliases, active flag, unique rank, the five capability levels, the four current standard prices, and the model's total-token limit; a model's `key` is derived as `<backend>:<model>`. Role profiles and token profiles are reviewed code constants in `cafleet/model_selection.py`, not model-list data. The parser rejects a missing, duplicate, out-of-order, or unknown section, a wrong column header, a missing or malformed separator row, a row with the wrong cell count, table lines outside a section or prose inside one, and any malformed cell. The tables illustrated below are the machine payload.

The repository file is the release source; it is not a runtime default. Each `cafleet setup` release asset copies the entire `skills/cafleet/` directory independently to `~/.claude/skills/cafleet`, `~/.codex/skills/cafleet`, and `~/.config/opencode/skills/cafleet`. A Director derives its absolute model-list path from the exact `cafleet` skill root it loaded: `<loaded-cafleet-skill-root>/reference/model-list.md`. It passes that path explicitly to `cafleet model select`; the selector has no package-resource, repository, or CWD fallback. The Python wheel therefore contains parser/selection code but no model-list copy, and the loaded deployed CAFleet skill asset is the only runtime source.

The model list is distributed as an ordinary reference page of the `cafleet` skill — no release manifest, content fingerprint, or sidecar file accompanies it. Before selection, `cafleet model select` validates only that its explicit `--model-list` path is absolute and a readable regular file, and that the file parses as a valid, fresh model list. The audit record includes `model_list_path` in addition to the normalized snapshot and the per-source content hashes carried inside the payload.

Candidate backend eligibility equals the existing backend readiness contract: for every backend that survives model/capability filtering, the CLI calls `CODING_AGENTS[backend].ensure_available()` and excludes a failing backend before ranking. Version 1 is the only accepted schema; an unknown version is a hard model-list error.

```markdown
## Metadata

| Field | Value |
|---|---|
| schema_version | 1 |
| generated_at | 2026-07-19T00:00:00Z |
| freshness_days | 30 |

## Sources

| Source | URL | Retrieved at | Content SHA-256 |
|---|---|---|---|
| anthropic | https://platform.claude.com/docs/en/about-claude/pricing | 2026-07-19T00:00:00Z | <64-hex hash> |
| openai | https://developers.openai.com/api/docs/pricing | 2026-07-19T00:00:00Z | <64-hex hash> |

## Models

| Backend | Model | Aliases | Active | Rank | Cod | Pln | Rsc | Rev | Mon | In | Cached | Write | Out | Max tokens |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| codex | gpt-5.6-luna | — | yes | 55 | 3 | 3 | 3 | 3 | 4 | 1.0 | 0.1 | 0.0 | 6.0 | 400000 |
```

Required constraints:

- Every active selectable model has one row (one `key`) for one provider billing SKU. The row's `Model` cell plus its aliases are the model's exact CLI spawn tokens: every `(backend, token)` pair is unique, so aliases cannot double-count or receive a price intended for another SKU. `selected.model` is the row's `Model` cell, passed to `member create`. The initial inclusion authority is the finite set of model tokens currently documented in `skills/cafleet/reference/director.md` for direct Claude and Codex backends; arbitrary pass-through strings remain manual-only.
- Capability levels are integers 0–5 for exactly `coding`, `planning`, `research`, `review`, and `monitor`; a model is eligible only when every requested dimension is at least the required level. `Rank` is a unique, deterministic ordering used by the reviewer exception and tie-breaking. Capability levels and ranks are reviewed maintainer judgment; the two approved provider pages are pricing/availability sources only. Changing a level or rank requires reviewed policy approval in the same pull request.
- Each row prices all four token components at the currently effective standard rate. A priced component is a non-negative USD-per-MTok number; an unpriced component (`—`) makes the model ineligible if that component's requested tokens are nonzero; a priced component with `0` is an explicitly free component. A row whose components are all `—` (for example an OpenCode gateway model without an approved source price) stays visible in diagnostics but is never an automatic candidate. `total_tokens = input + cached_input + cache_write + output` must be at most `Max tokens`. The selector never treats a missing component as zero.
- Freshness is evaluated independently for both required sources in UTC: each `retrieved_at` must be no more than `freshness_days` before selection time and no more than five minutes after it. `generated_at` is audit metadata only; a newly generated model list with either stale source is stale.
- The validator rejects an unknown schema version, duplicate keys/ranks/tokens, an unknown backend or dimension value, an invalid source URL/hash, an invalid price cell, a stale required source, or no eligible model. It never silently falls back to a high-cost default.

### Task profile and selection algorithm

Add `cafleet/model_selection.py` as a pure domain module and a `cafleet model select` CLI group. The CLI file boundary reads the explicit Markdown path, while a pure `parse_model_list_markdown(text)` function validates/extracts the model-list tables and hands a typed model list to the I/O-free selector. The Director calls it before each `member create`; the selection result supplies the already-supported `--coding-agent` and `--model` flags to that existing command. Effort is never automatically selected: a manual effort override is independently passed to the existing backend validator after model selection.

The CLI accepts a role, task profile, token estimate, and output mode. A minimal interface is:

```text
cafleet model select \
  --model-list /absolute/path/to/skills/cafleet/reference/model-list.md \
  --role programmer \
  --estimated-input-tokens 12000 \
  --estimated-output-tokens 6000 \
  --json
```

`--role` is one of the following role-profile keys, defined as reviewed code constants in the domain module. The listed requirements and profile are the required defaults; repeated `--requires dimension=level` can only raise a listed or previously-zero dimension to 1–5, and explicit token counts can only replace the profile with non-negative integers. There is no free-form `--task-kind` or `--complexity` selector input: `task_kind` is a normative audit label owned by the role profile.

| Role-profile key | Task kind | Default requirements | Token profile |
|---|---|---|---|
| `monitor` | `monitoring` | monitor 2 | small |
| `drafter` | `design_doc_drafting` | planning 3, research 2, review 1 | standard |
| `reviewer` | `review` | review 4, planning 3 | standard |
| `analyzer` | `requirements_analysis` | planning 4, research 3, review 3 | standard |
| `programmer` | `implementation` | coding 4, planning 3, review 2 | large |
| `tester` | `test_design` | coding 3, planning 3, review 3 | standard |
| `verifier` | `verification` | coding 3, planning 4, review 4 | standard |
| `manager` | `research_coordination` | planning 4, research 3 | standard |
| `scout` | `source_discovery` | research 3, planning 2 | small |
| `researcher` | `research_synthesis` | research 4, planning 3, review 2 | large |
| `web_researcher` | `web_research` | research 4, planning 3 | large |
| `transcript` | `research_transcript` | planning 3, research 2, review 2 | standard |
| `presentation` | `presentation_authoring` | planning 3, research 2, review 2 | standard |
| `visual_reviewer` | `visual_review` | review 4, planning 2 | standard |

The Director may make only documented task-specific increases: routine bounded work may retain the profile, cross-component or high-impact work raises each relevant dimension to at least 4, and security-sensitive/novel/failure-intolerant work raises relevant dimensions to 5. A requested value below the profile or an unknown role/dimension is rejected.

The selection problem is explicitly constrained cost minimization. For task \(t\), let \(r_t[d]\) be the required level in each capability dimension \(d\), let \(a(m)\) mean model \(m\) is active, fresh, and runnable in the available backend, and let \(q(m,d)\) be the listed capability level. The eligible set is:

\[
E_t = \{m \mid a(m) \land pricing(m)=known \land \forall d,\ q(m,d) \ge r_t[d]\}
\]

For ordinary members, the selected model is:

\[
m^* = \underset{m \in E_t}{\operatorname{argmin}}\; \widehat{C}(m,t)
\]

where \(\widehat{C}\) is the estimated standard USD token cost for the task's input, cached-input, cache-write, and output token estimates. This is the operational form of “least cost subject to task success”: model-list eligibility is a conservative, reviewed proxy for the capability needed to finish the task reliably, not a guarantee of success. Runtime evidence that the proxy was too weak invokes the underpowered-member escalation below and raises the task floor before a replacement is selected.

The selector performs the following deterministic steps.

1. The CLI orchestration layer resolves the requested/manual backend candidates and calls each candidate's existing `CODING_AGENTS[backend].ensure_available()` contract. It supplies the resulting runtime-ready backend set to the pure, I/O-free domain selector; this preserves the OpenCode preset checks.
2. For automatic and special-role policies, load and validate the model list; reject stale/invalid data before candidate selection. Resolve the role profile, normalize only permitted requirement increases, and fill absent token values from its named token profile.
3. Normalize override shape before branching. A model pin, with or without its matching backend pin, is a full manual bypass: validate the existing backend/model/effort contract and spawn as current CAFleet does; return `manual_override` with an estimate only when a fresh mapped priced row exists, otherwise `estimate_status: unavailable`. An explicitly marked full workflow `manual_override` has the same behavior. A backend-only override remains a ready-backend candidate filter during active ordinary cost mode and monitor/reviewer policy selection; an effort-only override is post-selection validation and does not alter ranking. Outside an active ordinary/special policy, backend-only and effort-only fields retain current unpinned spawn behavior. This manual-bypass path is unavailable to an unmarked legacy workflow pin; automatic and special-role policies otherwise fail closed on stale data.

4. Filter automatic candidates by active row, ready backend, priced components, `Max tokens`, and every capability floor. For an ordinary member, compute `estimated_usd = input_tokens / 1_000_000 * input_price + cached_input_tokens / 1_000_000 * cached_input_price + cache_write_tokens / 1_000_000 * cache_write_price + output_tokens / 1_000_000 * output_price`; select the smallest estimate, then higher rank, then lexical key.
5. For a monitor, use its baseline and token profile then select the lowest cost. For a reviewer, use its baseline and select greatest rank, then lower estimated cost, then lexical key. The selector returns the backend, exact listed model token, no automatic effort, and the structured diagnostic; it never synthesizes a shell command.

In `--json` mode, every failure writes this common envelope to stdout before exiting: `{"error":{"code":"...","message":"...","details":{},"candidates":[]}}`. `candidates` contains each examined key and its exclusion reason when candidate enumeration occurred; `details` contains only normalized request/model-list metadata and never prompt text. The stable error contract is:

| Code | Click exit code | Meaning |
|---|---:|---|
| `MODEL_SELECTION_INVALID_REQUEST` | 2 | Unknown role/dimension, invalid override combination, negative token value, or invalid effort request. |
| `MODEL_LIST_PATH_UNAVAILABLE` | 1 | The required absolute Markdown model-list path is absent, unreadable, or not a regular file. |
| `MODEL_LIST_INVALID` | 1 | Missing, malformed, or schema-invalid model list. |
| `MODEL_LIST_STALE` | 1 | A required source fails the UTC freshness rule for automatic/special selection. |
| `MODEL_BACKEND_UNAVAILABLE` | 1 | No requested candidate passes its existing backend readiness contract. |
| `MODEL_NO_ELIGIBLE_CANDIDATE` | 1 | Ready listed candidates exist but none meets price, token-limit, or capability constraints. |
| `MODEL_SELECTION_AUDIT_UNAVAILABLE` | 1 | Automatic/special selection has `BASE == <unset>`. |
| `MODEL_UPGRADE_UNAVAILABLE` | 1 | A bounded replacement has no stronger candidate in its pinned snapshot. |

Human mode uses the same code in the Click error text. In cost-efficiency mode the Director relays an operator choice rather than spawning a guessed model. Outside cost-efficiency mode, existing behavior continues and the selector result is informational only.

### Decision audit trail

`cafleet model select --json` returns a stable, redaction-safe record:

```json
{
  "policy": "cost_minimized_subject_to_capability",
  "role": "drafter",
  "triggered_by": "cost efficiency mode",
  "task_profile": {"planning": 3, "research": 2, "review": 1},
  "token_estimate": {"input": 12000, "output": 6000, "source": "director"},
  "candidates": [
    {"key": "codex:gpt-5.6-luna", "eligible": true, "estimated_usd": 0.048},
    {"key": "codex:gpt-5.4-mini", "eligible": false, "reason": "planning capability 2 < 3"}
  ],
  "selection_id": "sel_20260719_001",
  "selected": {"key": "codex:gpt-5.6-luna", "backend": "codex", "model": "gpt-5.6-luna", "effort": null, "estimated_usd": 0.048},
  "model_list": {"schema_version": 1, "generated_at": "...", "source_hashes": {"openai": "...", "anthropic": "..."}, "snapshot": {"eligible_models": "full normalized candidate records"}},
  "model_list_path": "/home/user/.codex/skills/cafleet/reference/model-list.md",
  "spawn": {"state": "pending", "member_id": null, "error": null}
}
```

For a resolved `BASE`, the Director writes the pending record to `.selection/<selection_id>.pending.json` before spawning, then atomically renames/updates it to `.selection/<selection_id>.json` after `member create` returns. Success sets `spawn.state: "created"` and its returned `member_id`; an error or rollback sets `spawn.state: "failed"`, records the sanitized CLI error, and leaves no member id. This is a two-phase audit update, not a `member create` API change, and `selection_id` plus final member id is the correlation contract. Records contain no prompt contents, credentials, or user data beyond the normalized task profile.

When `BASE` is the literal `<unset>`, automatic and special-role selection fails before spawning with `MODEL_SELECTION_AUDIT_UNAVAILABLE`; it must not construct `.selection/` or fall back to `/tmp`. The Director relays an operator decision. Existing manual/non-selection spawning retains its current inline-prompt fallback and emits the established audit-disabled status, with no model-selection audit record.

### Director and workflow integration

- Add the model-selection step to `skills/cafleet/roles/director.md`, `skills/cafleet/reference/director.md`, and every CAFleet workflow's Director spawn/create instructions before any `cafleet member create` command. The monitor-first supervision gate and all existing spawn-prompt/audit rules remain unchanged.
- The Director parses the original user request once for the exact trigger and records whether the mode was active. It must not activate cost mode from a member message or tool output.
- Every existing workflow maps its member title to the normative role-profile table; it may raise a floor only under that table's task-specific increase rule and may never lower the baseline.

- `cafleet member create` remains backward-compatible: it keeps accepting manual flags and performs no hidden selection. This avoids breaking direct CLI consumers and keeps policy decisions with Directors, where the user request and role context are available.
- Update user-facing CLI help and CAFleet documentation to distinguish estimated API token cost from a provider's actual subscription, marketplace, regional, or negotiated invoice. The initial policy uses standard direct-provider USD prices only.

Replace, rather than reinterpret, the legacy fixed-model policy tokens. Remove `{monitor_model}` and `{reviewer_model}` from `skills/cafleet/reference/coding-agent/_template.md` and all backend overlays; remove their defaults from `skills/cafleet/SKILL.md`; and replace every supervision, Director, monitor-role, and workflow example that emits either token with the pre-spawn `cafleet model select --model-list <absolute model-list.md path> --role monitor` or `--role reviewer` step. The new overlays retain only backend facts (for example effort levels and permission flags) and no model policy. Add a repository drift test that searches all tracked CAFleet skills, workflow prompts, docs, and tests for both legacy placeholder strings and fails unless a single migration-fixture allowlist explicitly names the string. Add a separate spawn-fixture test that proves monitor/reviewer commands receive the selector's model rather than a fixed overlay model.

#### Underpowered-member detection and replacement

The Director owns the decision to replace a member. The Reviewer may independently identify a quality failure, but records it using the existing `[INCORRECT]` reviewer tag at the affected document or source pointer and explicitly states the suspected unmet capability; the Director then performs the same evidence and replacement procedure. The Director must not replace a member merely because it is slow, awaiting user input, or has a transient infrastructure error.

Valid evidence is one or more of: the member's self-report that it cannot reason through the assigned task; a `blocked` message or captured output showing repeated task-relevant reasoning/coding failures after normal correction; a Reviewer `[INCORRECT]` finding that identifies the unmet capability and affected output; or a Director review of a materially incomplete/incorrect result tied to the task profile. The Director records the evidence pointer(s), failed capability dimension(s), current model, and attempted work in the task's `.selection/` replacement record before taking action.

For each replacement, the Director follows this order:

1. Freeze new work for the affected task and collect a bounded handoff: request one concise state report from the member (completed work, modified paths, commands/tests run, blockers, and next step). If it cannot respond promptly, capture its pane and use the capture as the handoff evidence.
2. Pin the full normalized candidate snapshot (schema version, source hashes, exact model records, original model key, and ranks) from the original decision record for the lifetime of the task. Re-run the pure selector against that snapshot while rechecking current backend readiness; a model-list refresh cannot change rank history mid-attempt. Raise at least the failed capability floor by one, or to the Reviewer/Director's explicitly justified floor, and retain the same token estimate unless evidence requires a revised estimate. The replacement must have a strictly greater rank than the recorded failed model and satisfy the new floors. It may cost more; cost is minimized only within this stronger eligible set.
3. Write a replacement decision record containing the trigger, evidence pointers, old and new task profiles, candidate exclusions, old/new model and estimated cost, attempt number, and handoff artifact path. Do not include secrets or full prompt contents.
4. Delete the old member through the standard `cafleet member delete` lifecycle before creating the replacement, preventing concurrent agents from editing the same task. The existing monitor remains live; all normal spawn, audit, and prompt-substitution rules apply to the new member.
5. Spawn the replacement with the original assignment plus the bounded handoff and the same deliverable paths. It resumes the task rather than starting a parallel implementation. The Director routes the original task pointer and asks the Reviewer to re-evaluate the resumed output when the workflow normally reaches review.

The initial member plus at most two replacements are allowed per task. Each replacement must be strictly higher-ranked than its predecessor, and a `(task pointer, model key)` pair may never be retried. If the failed member has an unmapped manual model, an inactive/removed entry, no recorded rank, a stale/missing pinned snapshot, or any explicit user override, the Director does not auto-replace it and instead relays a user decision. If the stronger eligible set is empty, the maximum is reached, or evidence remains ambiguous, the Director likewise fails closed and relays an operator choice: approve a named higher-cost/manual override, simplify/re-scope the task, or stop. Only the Director can delete or create members; the Reviewer supplies evidence and remains independent of the replacement execution.

### Model-list refresh skill and maintenance

Create a dedicated project-local skill at `.claude/skills/cafleet-model-list-refresh/SKILL.md`. It is a maintainer tool of this repository — not distributed by `cafleet setup`, not part of the release skill assets — invoked by a maintainer to refresh the local model list; it is not run automatically during a member spawn. Its hard requirements are:

1. Fetch only the two user-approved official sources: `https://platform.claude.com/docs/en/about-claude/pricing` and `https://developers.openai.com/api/docs/pricing`. Do not use search results, third-party price sites, social posts, or scraped gateway prices as model-list authority.
2. Record URL, retrieval timestamp, content hash, the currently effective standard token prices, and availability/deprecation qualifiers. Capability levels and ranks are separately reviewed maintainer policy; do not present the pricing pages as benchmark evidence or copy long provider text into the repository.
3. Reapply the capability rubric explicitly. The maintainer must review every changed capability level, rank, model availability, and pricing basis. A new model remains inactive until it has all required fields and a reviewed classification.
4. Validate the fixed table layout (the three required sections with their exact column headers), schema, source allowlist, price units, non-negative values, backend/model syntax, unique keys, and source freshness. Generate a concise proposed diff and require explicit maintainer approval before atomically rewriting the tables in `skills/cafleet/reference/model-list.md`; preserve the prescribed preamble verbatim.
5. Update the `Metadata` table's `generated_at` only after successful validation and approval. If either approved source cannot be fetched or parsed, leave the Markdown model list unchanged, report the error, and let it become stale rather than fabricating values.
6. Refresh the model list at least every 30 days and whenever the user asks for a refresh. Stale data disables automatic cost selection until a maintainer refreshes, commits the repository source, and completes the release/deployment transaction below.

Model-list refresh deployment is release-coupled: the maintainer bumps the CAFleet release version, builds the wheel and `cafleet-assets-v<version>.zip` containing the refreshed `skills/` tree (the model list rides inside `skills/cafleet/reference/` like every other reference page), then publishes that release. Each active backend upgrades to that CLI version and runs `cafleet setup`, which overwrites its installed `cafleet` skill replica. There is no model-list-only sync path in this iteration; a committed source file alone does not refresh a running Director's asset copy.

The skill owns model-list maintenance; the CAFleet Director owns per-task selection; `member create` remains the execution boundary.

### Security, failure handling, and compatibility

- Treat model-list files and refresh output as untrusted until schema and source allowlist validation complete. Never execute text extracted from provider pages.
- Do not write price or capability data into the CAFleet database in this iteration. A committed model list plus per-task hidden audit artifacts is sufficient and avoids schema migrations for frequently changing public data.
- Selection must preserve existing permission flags, model-to-backend validation, spawn placeholder substitution, monitor lifecycle, and rollback behavior. It may only change the arguments that a Director chooses to send to `member create`.
- The use of source hashes and effective dates makes changed external pricing observable. The system does not claim real-time or account-specific billing accuracy.

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
