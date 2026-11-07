# Model Selection Procedures

[Director model selection](../roles/director.md#model-selection) owns the policy, the backend-resolution order and the cost-efficiency trigger. Read these procedures only under the triggers below; a spawn with none of them needs only that policy.

| Trigger | Sections |
|---|---|
| The operator names a model, alias, family word or backend for a member | [Inference](#model-name-to-backend-inference), [worked resolutions](#worked-resolutions), [normalization](#model-name-normalization), [explicit mapping](#explicit-backend-model-mapping), [routing failures](#model-routing-failures) |
| The originating user request contains `cost efficiency mode` | [OpenCode token selection](#opencode-token-selection) for whole-catalog comparison, routing failures |
| The selected member backend is `opencode` | OpenCode token selection |
| A [Model replacement](../roles/director.md#model-replacement) re-selection | OpenCode token selection when the backend is `opencode`, routing failures |
| A backend/model mismatch or an unsuitable or missing model | Routing failures |

## Model-name-to-backend inference

When the operator names a model rather than a backend ("please create a member with sonnet"), apply the first matching row, using the [comparison key](#model-name-normalization) for family tests and catalog matches:

| # | Model reference | Backend | Flags to pass |
|---|---|---|---|
| 1 | Contains a `/` — provider-prefixed (e.g. `opencode/glm-5.3`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>`, verbatim |
| 2 | Claude reference: its key starts with `claude`, `fable`, `opus`, `sonnet` or `haiku` — `claude`, `opus`, `claude-opus-4-8`, `Opus 5.5` | `claude` | `--coding-agent claude --model <token>` resolved through Model-name normalization; the family word `claude` alone uses the claude Role default for the role |
| 3 | GPT reference: its key starts with `gpt` — `gpt`, `gpt-6.1-sol`, `gpt6.1-sol`, `GPT 6 Luna` | `codex` | `--coding-agent codex --model <token>` resolved through Model-name normalization; the family word `gpt` alone uses the codex Role default for the role |
| 4 | Any other bare name whose key equals the key of exactly one OpenCode candidate ID (normalization step 1) — `glm-5.3`, `kimi-k3.1-pro` | `opencode` | `--coding-agent opencode --model <token>`, token from OpenCode token selection |
| 5 | Anything else — no key-equal ID, or more than one (e.g. `o3-mini`, `gemini`) | none | Request an explicit backend/model pair through Model routing failures |

Apply these rows only when the operator names no backend. Inference never selects `hermes`: a Hermes member requires an operator-named backend or a Hermes fleet backend ([backend-resolution](../roles/director.md#model-selection) steps 1 and 3), and a provider-prefixed model for a Hermes-named member follows the Hermes explicit mapping.

Row 1 wins over row 2 for `anthropic/claude-sonnet-4-6`, which contains both `claude` and a `/`: a provider-prefixed token is itself an explicit OpenCode request. A bare name infers `opencode` only through row 4; Claude and GPT names select their native backends even when either OpenCode table lists the model.

Match candidate ID keys by equality: `glm-5.3` matches only the ID `glm-5.3`, never `glm-5.3-flash` or `glm-5.3-flash-free`.

## Worked resolutions

Resolve role-default placeholders from the selected member backend and omit unsupported effort.

| Operator reference | Fleet backend | Result |
|---|---|---|
| `claude` reviewer | `codex` | `--coding-agent claude --model {reviewer_model} --effort {reviewer_effort}` |
| `gpt` ordinary member | `opencode` | `--coding-agent codex --model {other_model} --effort {other_effort}` |
| OpenCode reviewer | any | `--coding-agent opencode --model {reviewer_model}` |
| `glm-5.3`, both provider candidates listed | any | `--coding-agent opencode --model opencode-go/glm-5.3` |
| `opus` with Codex | any | Mismatched pair; Model routing failures |
| `anthropic/claude-sonnet-4-6` | any | Explicit OpenCode token, verbatim |

## Model-name normalization

Normalize loose spellings such as "gpt6.1-sol", "GPT 6.1 Sol" and "Opus 5.5" to catalog tokens:

1. **Comparison key.** Lowercase the reference and every candidate token, and drop every character that is not a letter or digit (`gpt-6.1-sol` and `GPT 6.1 Sol` both become `gpt61sol`). OpenCode candidates are the distinct bare IDs of the Console and OpenCode Go tables: strip the `opencode/` or `opencode-go/` prefix before keying, and count an ID listed in both tables once. A Claude candidate also matches its key with the leading `claude` removed (`claude-opus-5-5` becomes `opus55`, matching "Opus 5.5").
2. **Native catalog first.** For a Claude or GPT reference (classified by the inference table's row 2 / row 3 key-prefix test), a key equal to the key of exactly one `Model` or `Alias` token in the native (`claude` / `codex`) catalog resolves to that row's `Model` token.
3. **Spelling dictionary.** A Claude or GPT reference equal to no native token key but to exactly one Claude/GPT ID key among the OpenCode candidates resolves to that bare ID. Keep the native backend and pass the ID as a manual `--model` value ("sonnet 4.6" becomes `--coding-agent claude --model claude-sonnet-4-6`).
4. **Exact shape.** A reference already written as an exact `claude-*` / `gpt-*` token that matches nothing passes through verbatim to its native backend.
5. **Otherwise ask.** Route a reference with no key-equal token or several tokens sharing the key through Model routing failures.

## Explicit backend model mapping

When [backend-resolution](../roles/director.md#model-selection) step 1 selects an explicit backend, resolve `--model` from the reference with that backend's first matching row.

**OpenCode.**

| Reference given with the OpenCode request | Resolved `--model` |
|---|---|
| No model (e.g. "spawn an opencode reviewer") | The opencode Role default for the role, verbatim |
| Provider-prefixed token (`opencode/…`, `opencode-go/…`, `anthropic/…`) | The token, verbatim |
| Claude family word (`claude`) | `opencode/` + the `claude` backend's Role default for the role, verbatim |
| GPT family word (`gpt`) | `opencode/` + the `codex` backend's Role default for the role, verbatim |
| Claude alias (`fable`, `opus`, `sonnet`, `haiku`) | The `claude` catalog Model the alias names, as an ID through OpenCode token selection |
| Any other model name | The OpenCode candidate ID whose key equals the reference's key, through OpenCode token selection |

The last row uses only Model-name normalization step 1, including the leading-`claude` strip, against the OpenCode candidate IDs. It requires exactly one key-equal ID, so "sonnet 4.6 with OpenCode" resolves to the ID `claude-sonnet-4-6` before provider choice; normalization steps 2–4 apply only to native-backend resolution.

Require each Role-default and family-word token to exist in the Console table, and each resolved bare ID in the OpenCode candidate set. A verbatim provider-prefixed reference is exempt; route missing or ambiguous matches through Model routing failures.

**Claude Code / Codex.**

| Reference given with the Claude Code or Codex request | Resolved `--model` |
|---|---|
| No model (e.g. "reviewer with Claude Code") | That backend's Role default for the role |
| The backend's family word (`claude` for Claude Code, `gpt` for Codex) | That backend's Role default for the role |
| A reference of the backend's own family (inference row 2 for Claude Code, row 3 for Codex) | The token Model-name normalization steps 2–4 resolve against that backend (e.g. "opus 4.8 with Claude Code" becomes `claude-opus-4-8`) |
| A reference of another family, or a provider-prefixed token | none — relay as a mismatched pair through Model routing failures |

**Hermes.** Hermes has no maintained catalog, and its Role defaults are `—`.

| Reference given with the Hermes request | Resolved `--model` |
|---|---|
| No model, a family word or a backend-only request | None: omit `--model` and `--effort`, so the operator's configured default applies |
| Any model token, provider-prefixed or bare | The token, verbatim; Hermes rejects an unknown model at launch |

## OpenCode token selection

Use this owner for provider choice and cost ranking. Complete provider-prefixed tokens from explicit references, defaults or family-word mapping remain verbatim and skip provider choice. A bare resolved ID has `opencode-go/<id>` and/or `opencode/<id>` candidates only where its catalog tables list them.

1. Run `opencode models` once per spawn as your own command. Parse one exact `provider/model` token per line and ignore unrelated providers; route a failed/empty result through Model routing failures.
2. For a resolved bare ID, choose its listed Go candidate first, otherwise its listed Console candidate. Route neither-listed through Model routing failures.
3. For cost efficiency selection or replacement, use the specified candidate-ID set: the whole catalog or the stronger set. Prefer every reliable listed Go candidate over every Console candidate, including Console rows priced `0.00`; Go subscription candidates are effectively free, and lower Go-table rates break reliable-candidate ties because they consume less usage.
4. If no Go candidate can finish reliably, choose the cheapest reliable listed Console candidate by its table prices. Route an empty reliable set through Model routing failures.

The selected cost-ranked token is final and receives no second provider-choice pass.

## Model routing failures

| Condition | Director behavior |
|---|---|
| An explicit OpenCode request's mapped model is absent from both OpenCode tables (e.g. "claude-opus-4-1 with OpenCode"); `opencode models` lists neither candidate token; cost efficiency selection or replacement on OpenCode finds no listed token that can finish the task reliably; a pinned backend and model do not belong together (e.g. "opus member with Codex"); normalization is ambiguous (two tokens share the key) or matches nothing | Relay an operator choice via {decision_surface} before spawning |
| `opencode models` exits non-zero or prints nothing during provider choice, cost efficiency selection or replacement | Relay via {decision_surface} before spawning; never fall back to `opencode/` |
| Native backend binary missing (`Error: binary <name> not found on PATH`, exit 1) | Report the failure and relay the operator choice via {decision_surface}; preserve the backend selected by the model reference |
| OpenCode catalog row removed at a later refresh while a member referenced it | Keep the running member; resolve the next spawn against the current catalog |
