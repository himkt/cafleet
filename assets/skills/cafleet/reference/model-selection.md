# Model Selection Procedures

[Director model selection](../roles/director.md#model-selection) owns the policy, backend-resolution order and cost-efficiency trigger. Read these procedures only under a trigger below.

| Trigger | Sections |
|---|---|
| The operator names a model, alias, family word or backend for a member | [Inference](#model-name-to-backend-inference), [worked resolutions](#worked-resolutions), [normalization](#model-name-normalization), [explicit mapping](#explicit-backend-model-mapping), [routing failures](#model-routing-failures) |
| The originating user request contains `cost efficiency mode` | [OpenCode token selection](#opencode-token-selection) over the whole catalog, routing failures |
| The selected member backend is `opencode` | OpenCode token selection |
| A [Model replacement](../roles/director.md#model-replacement) re-selection | OpenCode token selection on `opencode`, routing failures |
| A backend/model mismatch or an unsuitable or missing model | Routing failures |

## Model-name-to-backend inference

When the operator names a model but no backend ("please create a member with sonnet"), apply the first matching row, testing [comparison keys](#model-name-normalization):

| # | Model reference | Backend | Flags to pass |
|---|---|---|---|
| 1 | Contains `/` (`opencode/glm-5.3`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>`, verbatim |
| 2 | Key starts with `claude`, `fable`, `opus`, `sonnet` or `haiku` (`opus`, `claude-opus-4-8`, `Opus 5.5`) | `claude` | `--coding-agent claude --model <token>` via normalization; bare `claude` uses the claude Role default |
| 3 | Key starts with `gpt` (`gpt-6.1-sol`, `gpt6.1-sol`, `GPT 6 Luna`) | `codex` | `--coding-agent codex --model <token>` via normalization; bare `gpt` uses the codex Role default |
| 4 | Key equals the key of exactly one OpenCode candidate ID (`glm-5.3`, `kimi-k3.1-pro`) | `opencode` | `--coding-agent opencode --model <token>` via OpenCode token selection |
| 5 | Anything else (`o3-mini`, `gemini`) | none | Request an explicit backend/model pair through routing failures |

Claude and GPT names select their native backends even when an OpenCode table lists them. Inference never selects `hermes` or `pi`; they require an operator-named or fleet backend ([backend-resolution](../roles/director.md#model-selection) steps 1 and 3). Keys match by equality: `glm-5.3` never matches `glm-5.3-flash`.

## Worked resolutions

Resolve role-default placeholders from the selected member backend and omit unsupported effort.

| Operator reference | Fleet backend | Result |
|---|---|---|
| `claude` reviewer | `codex` | `--coding-agent claude --model {reviewer_model} --effort {reviewer_effort}` |
| `gpt` ordinary member | `opencode` | `--coding-agent codex --model {other_model} --effort {other_effort}` |
| OpenCode reviewer | any | `--coding-agent opencode --model {reviewer_model}` |
| `glm-5.3`, both provider candidates listed | any | `--coding-agent opencode --model opencode-go/glm-5.3` |
| `opus` with Codex | any | Mismatched pair; routing failures |
| `anthropic/claude-sonnet-4-6` | any | Explicit OpenCode token, verbatim |

## Model-name normalization

1. **Comparison key.** Lowercase and keep only letters and digits (`GPT 6.1 Sol` → `gpt61sol`). OpenCode candidates are the distinct bare IDs of the Console and OpenCode Go tables, keyed after stripping `opencode/` or `opencode-go/`. A Claude candidate also matches its key without the leading `claude` (`claude-opus-5-5` → `opus55`).
2. **Native catalog first.** A Claude or GPT reference (inference row 2 / 3) whose key equals exactly one `Model` or `Alias` key in the `claude` / `codex` catalog resolves to that row's `Model` token.
3. **Spelling dictionary.** Otherwise, a key equal to exactly one Claude/GPT OpenCode candidate ID passes that bare ID as a manual `--model` on the native backend ("sonnet 4.6" → `--coding-agent claude --model claude-sonnet-4-6`).
4. **Exact shape.** An exact `claude-*` / `gpt-*` token that matches nothing passes through verbatim to its native backend.
5. **Otherwise ask.** Route no match or several key-equal tokens through routing failures.

## Explicit backend model mapping

When [backend-resolution](../roles/director.md#model-selection) step 1 selects an explicit backend, resolve `--model` from that backend's first matching row.

**OpenCode.**

| Reference | Resolved `--model` |
|---|---|
| No model ("spawn an opencode reviewer") | The opencode Role default, verbatim |
| Provider-prefixed token (`opencode/…`, `opencode-go/…`, `anthropic/…`) | The token, verbatim |
| `claude` | `opencode/` + the `claude` Role default, verbatim |
| `gpt` | `opencode/` + the `codex` Role default, verbatim |
| Claude alias (`fable`, `opus`, `sonnet`, `haiku`) | The `claude` catalog Model it names, through OpenCode token selection |
| Any other name | The one OpenCode candidate ID with an equal step-1 key, through OpenCode token selection ("sonnet 4.6 with OpenCode" → `claude-sonnet-4-6`) |

Each Role-default and family-word token must exist in the Console table and each resolved bare ID in the candidate set; route a missing or ambiguous match, except a verbatim provider-prefixed reference, through routing failures.

**Claude Code / Codex.**

| Reference | Resolved `--model` |
|---|---|
| No model, or the backend's family word (`claude` / `gpt`) | That backend's Role default |
| The backend's own family (inference row 2 / 3) | Normalization steps 2–4 against that backend ("opus 4.8 with Claude Code" → `claude-opus-4-8`) |
| Another family, or a provider-prefixed token | none — a mismatched pair; routing failures |

**Hermes and pi.** Neither has a maintained catalog; their Role defaults are `—`.

| Reference | Resolved `--model` |
|---|---|
| No model, a family word or a backend-only request | None: omit `--model` and `--effort` so the operator's configured default applies |
| pi token ending in `:<thinking>` (`off`, `minimal`, `low`, `medium`, `high`, `xhigh` or `max` after its last `:`) | The token without the suffix, with the level as `--effort`; `member create` refuses the suffix in `--model` |
| Any other token | The token, verbatim; the backend rejects an unknown model at launch |

## OpenCode token selection

Complete provider-prefixed tokens stay verbatim and skip provider choice. A bare ID has `opencode-go/<id>` and/or `opencode/<id>` candidates only where its catalog tables list it.

1. Run `opencode models` once per spawn as your own command, reading one `provider/model` token per line and ignoring other providers.
2. For a bare ID, choose its listed Go candidate, else its listed Console candidate.
3. For cost efficiency selection or replacement over the specified ID set (whole catalog or stronger set), prefer every reliable listed Go candidate over every Console candidate, Console `0.00` rows included; the lower Go-table rate breaks ties.
4. With no reliable Go candidate, choose the cheapest reliable listed Console candidate by table price.

A cost-ranked token is final, with no step-2 pass. Route a failed or empty `opencode models` result, a bare ID with neither candidate listed, or an empty reliable set through routing failures.

## Model routing failures

| Condition | Director behavior |
|---|---|
| Mapped OpenCode model in neither table ("claude-opus-4-1 with OpenCode"); `opencode models` lists no candidate; cost efficiency selection or replacement finds no reliable listed token; a mismatched backend/model pair ("opus member with Codex"); normalization ambiguous or unmatched | Relay an operator choice via {decision_surface} before spawning |
| `opencode models` exits non-zero or prints nothing | Relay via {decision_surface} before spawning; never fall back to `opencode/` |
| Native backend binary missing (`Error: binary <name> not found on PATH`, exit 1) | Report it and relay via {decision_surface}, keeping the backend the model reference selected |
| A catalog row a running member uses is removed at refresh | Keep the member; resolve the next spawn against the current catalog |
