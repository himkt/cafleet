# Readable model / coding-agent routing guidance with available-model tables

**Status**: Approved
**Progress**: 13/13 tasks complete
**Last Updated**: 2026-06-20

## Overview

Replace the prose-heavy "Model-name-to-backend inference" table in the cafleet skill's `director.md` with a clean routing summary table, and add three curated per-backend available-model tables (`claude` / `codex` / `opencode`). `skills/cafleet/reference/director.md` becomes the single authoritative home; every other surface keeps brief illustrative examples and stays self-contained, so no full model list is duplicated.

## Success Criteria

- [ ] `director.md` § Model-name-to-backend inference presents a 4-row routing summary table — one model-name shape per row, with no single cell combining the infer-rule, the flags, and the omit-default caveat as the original did — replacing the current table body (lines 41-46).
- [ ] `director.md` gains an "Available models per backend" subsection with three tables: claude (simple list including `best`/`default`/`opusplan`/`sonnet[1m]`/`opus[1m]`), codex (`gpt-5.5`/`gpt-5.4`/`gpt-5.4-mini`/`gpt-5.3-codex-spark`), opencode (curated: OpenAI + Anthropic + Google rows + two collapse notes).
- [ ] The routing rule documents precedence: slash-form → `opencode` (explicit-only); else `gpt-*` → `codex`; else Claude alias / `claude-*` full name → `claude` (default); else unknown bare name → ask the operator.
- [ ] `best`/`default`/`opusplan`/`sonnet[1m]`/`opus[1m]` route to `claude`, not the ask-operator branch.
- [ ] No full available-model list exists outside `director.md`; `cli-options.md`, `coding-agents.md`, `README.md`, `quickstart.md`, `mixed-backend-team.md` retain only brief examples.
- [ ] The old `| fable, opus, sonnet, or a claude-* full name | claude | … |` row and its three sibling rows no longer appear in any live skill/doc surface — a backtick-tolerant repo grep for that wording returns only design docs (this doc plus the historical `0000082-coding-agent-model-option`, which introduced the table and is preserved per `removal.md`) and the pre-spawn prompt renders under `prompts/`.
- [ ] Every surface in the Surface Inventory (S3) is updated or explicitly verified as needing no change.

---

## Background

`director.md` § Model-name-to-backend inference (heading at line 37, table body at lines 41-46) packs the routing rule into a 4-row table whose cells carry multi-clause parentheticals — the row the user called out reads `| fable, opus, sonnet, or a claude-* full name | claude | --coding-agent claude --model <name> (--coding-agent claude may be omitted — it is the default) |`. The table tells the Director how to *infer* a backend from a model name but never lists which models each backend actually offers, so the Director cannot answer "what can I pass to `--model` for codex?" without leaving the skill. Authoritative vendor model data was gathered in `research/vendor-models.md` and is the source for the new tables (no re-fetch).

---

## Specification

### S1. `director.md` — routing summary table (replaces the table body, lines 41-46)

Keep the heading `### Model-name-to-backend inference` (referenced by `director.md` line 30 and `skills/cafleet/SKILL.md` line 61) and keep the existing lead-in sentence (the one beginning "When the operator names a model rather than a backend …, resolve the backend from the model-name shape:") — it still introduces the table. Replace only the table body (the current lines 41-46) with the clean summary below, ordered by match precedence; then append the precedence rule and the S2 subsection after it:

| Model name shape | Backend | Flags to pass |
|---|---|---|
| Contains a `/` — provider-prefixed (e.g. `opencode/gpt-5.5`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>` |
| `gpt-*` (e.g. `gpt-5.5`, `gpt-5.4-mini`) | `codex` | `--coding-agent codex --model <name>` |
| Claude alias or `claude-*` full name — `fable`, `opus`, `sonnet`, `haiku`, `best`, `default`, `opusplan`, `sonnet[1m]`, `opus[1m]`, `claude-opus-4-8`, … | `claude` (default) | `--model <name>` (`--coding-agent claude` is the default and may be omitted) |
| Any other bare name — no shape match (e.g. `gemini-2.5-pro`, `o3-mini`) | none — do NOT infer | Ask the operator for the explicit `--coding-agent` + `--model` pair |

Followed by the routing rule as ordered precedence:

> Resolve the backend in this order — the first match wins:
> 1. **Name contains a `/`** → `opencode`. The provider-prefixed form is the explicit "use opencode" signal; opencode is never inferred from a bare name.
> 2. **Name matches `gpt-*`** → `codex`.
> 3. **Name is a Claude alias (`fable` / `opus` / `sonnet` / `haiku` / `best` / `default` / `opusplan` / `sonnet[1m]` / `opus[1m]`) or a `claude-*` full name** → `claude`, the default backend (`--coding-agent` may be omitted).
> 4. **Anything else** → do not infer; ask the operator for the explicit `--coding-agent` + `--model` pair.

Precedence matters for the slash case: `anthropic/claude-sonnet-4-6` contains both `claude` and a `/`, and rule 1 (slash → opencode) wins over rule 3.

### S2. `director.md` — available models per backend (new subsection)

Add `### Available models per backend` immediately after the routing table. Three minimal tables — no snapshot-date line, no "cafleet does not enforce / tracks vendor docs" caveat, no alias→resolves-to or pinnable-full-name columns.

**Claude Code (`--coding-agent claude`)** — simple list:

| Model | For |
|---|---|
| `fable` | hardest, longest-running tasks |
| `opus` | complex reasoning |
| `sonnet` | everyday coding |
| `haiku` | fast, simple tasks |
| `best` | Fable 5 if the org has access, else the latest Opus |
| `default` | clears the override; returns to the account-tier model |
| `opusplan` | `opus` in Plan Mode, `sonnet` during execution |
| `sonnet[1m]` | `sonnet` with a 1M-token context window |
| `opus[1m]` | `opus` with a 1M-token context window |

Full names: `claude-fable-5`, `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5`.

**Codex (`--coding-agent codex`)**:

| Model | Notes |
|---|---|
| `gpt-5.5` | newest frontier; default / recommended |
| `gpt-5.4` | flagship frontier — professional coding & reasoning |
| `gpt-5.4-mini` | fast, efficient mini — responsive tasks and subagents |
| `gpt-5.3-codex-spark` | text-only research preview (ChatGPT Pro) — near-instant iteration |

**OpenCode Zen (`--coding-agent opencode`)** — the OpenCode Zen catalog ([opencode.ai/docs/zen](https://opencode.ai/docs/zen/)). Every Zen model is passed with the `opencode/` gateway prefix, i.e. `opencode/<model-id>` (e.g. `opencode/gpt-5.5`, `opencode/claude-sonnet-4-6`, `opencode/gemini-3.5-flash`). The Models column lists the bare `<model-id>`; prepend `opencode/`:

| Provider | Models (pass as `opencode/<model-id>`) |
|---|---|
| OpenAI | `gpt-5.5`, `gpt-5.5-pro`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.4-nano`, `gpt-5.3-codex` |
| Anthropic | `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5` |
| Google | `gemini-3.5-flash` |

Other providers: Qwen, DeepSeek, Kimi, GLM, MiniMax, Grok.
Free (limited beta): Big Pickle, DeepSeek V4 Flash Free, MiMo-V2.5 Free, North Mini Code Free, Nemotron 3 Ultra Free.

The S1 routing rule accepts any `<provider-id>/<model-id>` for the `opencode` backend, including direct-provider forms such as `anthropic/claude-sonnet-4-6` or `openai/gpt-5.5`; the Zen catalog above is normalized to the `opencode/` gateway prefix, and the direct-provider examples elsewhere in `director.md` / `README.md` / `coding-agents.md` stay valid.

### S3. Surface inventory — keeping the change total

Every surface that mentions model/backend routing, the `--model` flag, or specific model names, with the action for each. This is the no-stale-duplicate checklist.

| Surface | Current state | Action |
|---|---|---|
| `skills/cafleet/reference/director.md` § heading (line 37) + table body (lines 41-46) | wordy inference table | **Replace** the table body (lines 41-46) with the S1 routing table + precedence rule; keep the heading and the line-39 lead-in; **add** the S2 available-model subsection. |
| `skills/cafleet/reference/director.md` line 30 | cross-ref: "The model-name-to-backend inference table below maps a bare model name to its backend and lists a per-backend example." | Verify it still reads correctly; reword minimally to also name the available-model subsection if helpful. |
| `docs/spec/cli-options.md` member-create (lines 545-557) | `--coding-agent` / `--model` rows; spawn-per-backend table uses `<m>` placeholder | No model name list is present → nothing to trim. Leave the `--model` / spawn-per-backend content as-is; it already points readers to `coding-agents.md` § Model selection. |
| `docs/concepts/coding-agents.md` § Model selection (lines 36-54) | brief inline examples (`sonnet`, `gpt-5.4-mini`, `anthropic/claude-sonnet-4-6`, `opencode/big-pickle`) | Keep the brief examples; confirm no full list is present; add NO cross-link into the skill tree. Do NOT add full tables. |
| `docs/reference/coding-agents/codex.md` line 32 | "Example models … `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex-spark`" | Keep as a brief example set (already short and matches vendor data); verify the names are current. |
| `docs/reference/coding-agents/opencode.md` line 34 | example values incl. `big-pickle` and free-beta names | Keep as brief examples; verify names are current; trim if the list grows toward a full catalog. |
| `README.md` line 83 | examples `sonnet`, `gpt-5.4-mini`, `anthropic/claude-sonnet-4-6`, `opencode/big-pickle` | Keep brief examples; no full list. |
| `docs/get-started/quickstart.md` line 67 | `--model sonnet` example | Audit; brief example only — no change expected. |
| `docs/how-to/mixed-backend-team.md` line 60 | `--coding-agent` context | Audit; no model list — no change expected. |
| `skills/cafleet/SKILL.md` line 61 | "the model-name-to-backend inference … live in `reference/director.md`" | Verify the cross-ref is accurate; optionally extend to "and the per-backend available-model tables". |
| `skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md` line 8 | `monitor_model` overlay values (`haiku` / `gpt-5.4-mini` / `anthropic/claude-haiku-4-5`) | These are monitor-model overlay values, not routing/available-model guidance. No change. |

### S4. Design decisions

- **Single authoritative home = `director.md`** (user decision). The routing summary and the available-model tables live only there; every other surface keeps brief illustrative examples. This prevents the seven-surface drift a duplicated catalog would cause.
- **Curated opencode table** (user decision): OpenAI / Anthropic / Google rows are explicit; the long tail (Qwen / DeepSeek / Kimi / GLM / MiniMax / Grok) and the fastest-churning free-beta names collapse into two summary notes.
- **Extended claude routing** (user decision): `best` / `default` / `opusplan` / `sonnet[1m]` / `opus[1m]` route to `claude`; only genuinely-unknown bare names reach the ask-operator branch.
- **Minimal tables** (user decision): no snapshot-date line, no "cafleet does not enforce / tracks vendor docs" caveat, no alias→resolves-to or pinnable-full-name columns. The per-backend validation framing (claude/codex pass any string through; opencode validates the `<provider-id>/<model-id>` format) already lives in `cli-options.md` and `coding-agents.md` and is not duplicated into the tables.
- **No cross-link from human docs into the skill tree** (user decision). `director.md` is an agent-facing skill file; the human operator docs (`README.md`, `quickstart.md`, `coding-agents.md`, `mixed-backend-team.md`) stay self-contained with their existing brief examples and add no pointer into `skills/`. `director.md` is the single authoritative home, and "single authoritative home + no duplicated full list" holds without crossing the audience boundary the project keeps separate.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-06-20T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Rewrite `director.md` routing + add available-model tables

- [x] Replace the `director.md` table body (lines 41-46) with the S1 routing summary table and the ordered-precedence routing rule; keep the line-37 heading and the line-39 lead-in sentence. <!-- completed: 2026-06-20T11:44 -->
- [x] Add the S2 `### Available models per backend` subsection with the claude / codex / opencode tables. <!-- completed: 2026-06-20T11:44 -->
- [x] Verify `director.md` line 30 cross-ref text still reads correctly against the new subsection; reword minimally if needed. <!-- completed: 2026-06-20T11:44 -->

### Step 2: Human-facing docs consistency sweep

- [x] `docs/concepts/coding-agents.md` § Model selection: keep the brief examples, confirm no full list is present, and add NO cross-link into the skill tree. <!-- completed: 2026-06-20T12:02 -->
- [x] `docs/spec/cli-options.md` member-create: confirm no duplicated model list; leave the `--model` / spawn-per-backend content as-is. <!-- completed: 2026-06-20T12:02 -->
- [x] `docs/reference/coding-agents/codex.md` + `opencode.md`: confirm the example-model sets stay brief and the names match vendor data; trim any drift toward a full catalog. <!-- completed: 2026-06-20T12:02 -->
- [x] `README.md` line 83 + `docs/get-started/quickstart.md` line 67 + `docs/how-to/mixed-backend-team.md` line 60: confirm brief examples only, no full list. <!-- completed: 2026-06-20T12:02 -->

### Step 3: Agent-facing skill consistency

- [x] `skills/cafleet/SKILL.md` line 61: verify the `director.md` cross-ref is accurate; optionally note the available-model tables. <!-- completed: 2026-06-20T12:04 -->
- [x] `skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md`: confirm the `monitor_model` values are untouched (no change expected). <!-- completed: 2026-06-20T12:04 -->

### Step 4: Verification

- [x] A backtick-tolerant repo-wide grep (the live row backticks each token, e.g. `` `fable` ``/`` `opus` ``/`` `claude-*` ``, so the pattern must tolerate backticks) confirms the four old inference-table rows no longer appear in any live skill/doc surface — the only remaining hits are design docs (this doc plus the historical `0000082-coding-agent-model-option`, preserved per `removal.md`) and the pre-spawn prompt renders under `prompts/`. <!-- completed: 2026-06-20T12:12 -->
- [x] Repo-wide grep confirms no per-backend full available-model list exists outside `director.md`. <!-- completed: 2026-06-20T12:09 -->
- [x] All internal cross-references introduced in Steps 1-3 resolve to real anchors / files. <!-- completed: 2026-06-20T12:09 -->
- [x] `mise //cafleet:lint` passes — a docs-only change touches no Python, so this is a guard against accidental source edits. <!-- completed: 2026-06-20T12:09 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-20 | Initial draft |
| 2026-06-20 | Reviewer round 1: cite the precise replacement range (table body, lines 41-46) and keep the lead-in; normalize the S2 opencode table to the `opencode/` Zen gateway prefix; decide no human→skill cross-link; align the grep criterion (backtick-tolerant, design-doc-only). |
