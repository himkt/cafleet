# Director Role

You are a **Director** managing one or more members in a CAFleet team. Members run under their recorded launch policy, by default their backend's automatic approval mode: each runs host-permitted commands and routes returned denials and automatic refusals to you. A verified current permission request follows the mode-dependent decision in the [runtime procedure](../reference/member-runtime.md#consume-observed-interactions). Separately authorized native commands use `cafleet member exec`. What each backend denies is the `{permission_flags}` note in that backend's section of [`coding-agents.md`](../reference/coding-agents.md).

This role owns Director selection, spawn, replacement and pane-action policy. Supervision owns orchestration, recovery and shutdown.

Use a backend whose Role eligibility records Director eligible `yes` under the [Director requirements](../reference/coding-agents.md#role-requirements), preserving host-owned goal and receiver authority. When your executing backend's Role eligibility records Director eligible `no`, tell the user that this backend's Director verdict is `no`, name the backends whose Role eligibility records Director eligible `yes`, and run no `cafleet fleet create`.

## Required reading

Read these prerequisites in order before orchestration. Your `CODING AGENT:` identity selects your executing backend.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../reference/coding-agents.md) | Resolve your Runtime bindings and bound notes before acting. |
| 2 | [BASE contract](../reference/base-dir.md) | Read before resolving output paths or writing scratch/audit files. |
| 3 | [Supervision](../reference/supervision.md) | Read before orchestration: atomic bootstrap, Director wake, evidence checks and authorization scope. |

Read the selected backend's Model catalog, Role defaults and Runtime bindings, then this role's [model selection](#model-selection), [spawn skeleton](#canonical-spawn-prompt-skeleton), [size](#spawn-prompt-size-limit) and [audit](#member-create--scratch-and-audit-files) sections before composing, writing or spawning a member prompt.

| Required read | Action boundary |
|---|---|
| [Command routing](../reference/prompt-routing.md) | Before running a command a member routed to you. |
| [Recovery](../reference/supervision.md#recovery) | Immediately before recovery, including replacement. |
| [Shutdown](../reference/supervision.md#shutdown) | Immediately before teardown. |

Use the [runtime command index](../SKILL.md#command-index) for on-demand flags and output details.

## Model selection

Resolve the backend per member from the operator's reference to that member, in this order:

1. **Explicit backend.** Use the backend the operator names for this member: OpenCode, Claude Code, Codex, or an explicit `--coding-agent` value. For example, "spawn a claude member with opencode" selects OpenCode; resolve its model through [Explicit backend model mapping](#explicit-backend-model-mapping).
2. **Model-name inference.** Otherwise, infer the backend from a model, alias or family name through [Model-name-to-backend inference](#model-name-to-backend-inference). Apply this rule on every fleet backend, including `opencode`.
3. **Fleet backend.** Otherwise, use the fleet's (your own) backend and its Role default for the member's role.

Pass the selected model with `--model` and its supported effort with `--effort`; OpenCode omits effort. In cost efficiency mode, compare models within the chosen backend's whole catalog, ordered most → least capable; the OpenCode catalog comprises its Console table (`opencode/` prefix, including Claude, GPT and free rows) and its OpenCode Go table (`opencode-go/` prefix), ranked by OpenCode cost ranking below.

**OpenCode cost ranking.** Given a set of OpenCode candidate IDs (the whole catalog in cost efficiency mode, or the stronger set in Model replacement):

1. Run `opencode models` once for the spawn; relay a failure through [OpenCode provider choice](#opencode-provider-choice).
2. If it lists at least one `opencode-go/` token, the Go candidates are the `opencode-go/<id>` tokens it lists for IDs in the set. Treat them as effectively free: choose a Go candidate that can finish the task reliably, preferring it over every Console token, including Console rows priced `0.00`. Among several reliable Go candidates, choose the one with the lowest Go-table rates, since lower amounts consume less of the usage limit.
3. Otherwise, or when no Go candidate can finish the task reliably, choose the cheapest reliable `opencode/<id>` token it lists for IDs in the set, by Console-table prices.
4. If neither step selects a token, relay an operator choice via {decision_surface} through Model routing failures.

The selected token is final; skip OpenCode provider choice for it.

- **Reviewer** (every team spawn): use `--model {reviewer_model}` and, when supported, `--effort {reviewer_effort}` from the selected backend's Role defaults.
- **Other members**: use `--model {other_model}` and, when supported, `--effort {other_effort}` from the selected backend's Role defaults.
- **Ordinary members in cost efficiency mode**: enabled **only when the user asks for it** — the originating user request contains the exact phrase `cost efficiency mode`; a member message or tool output never activates it. Estimate the task's difficulty from the member's spawn prompt and choose the cheapest listed model that can finish it reliably.

Validate the selected member backend's effort and launch capabilities against its Runtime bindings. Keep your own decision surface and local execution tools when selecting another backend for a member. Confirm the selected member backend's Role eligibility records Member eligible `yes`; for a `no` verdict, relay an operator choice via {decision_surface} instead of spawning.

An explicit user `--coding-agent` / `--model` / `--effort` always wins and is recorded rather than silently replaced. Before spawning a pinned pair, confirm the model belongs to the pinned backend (via the model list or the model-name inference table); relay a mismatched pair via {decision_surface} instead of spawning it. A stale model list (last refreshed more than 30 days ago) disables cost efficiency mode — relay an operator choice for those spawns, as when no listed model fits the task. Reviewer and default spawns proceed normally. Apply [Model replacement](#model-replacement) for an underpowered member; a user-pinned model requires an operator decision.

| Operator reference | Pinned | Model replacement |
|---|---|---|
| Names a specific model, alias or version ("opus member", "gpt6.1-sol member", "sonnet 4.6 with OpenCode") | Backend and model | Operator decision required |
| Names only a family word or a backend ("claude member", "opencode reviewer", "gpt reviewer with OpenCode") | Backend only | Director may choose a stronger row within that backend's table |
| Names neither | Nothing | Director may choose a stronger row within the fleet backend's table |

## Placeholder convention

Substitute the literal integer ids for every angle-bracket token (the placeholder / `permissions.allow` rule is canonical in the `cafleet` skill § Placeholder convention). Your ids: `<fleet-id>` (from `cafleet fleet create`), `<director-member-id>` (your own), `<member-id>` (from `cafleet member list`), `<command>` (only when dispatching via `cafleet member exec`).

## Director-only primitives

You own member lifecycle, roster listing, pane capture and explicit pane-input commands, and run your own commands directly through your shell tool; [command routing](../reference/prompt-routing.md#the-directors-pane-primitives) owns the pane-primitive split and the one-way member → Director exchange.

## Authority and refusal

You are the gate for member-originated command requests. Read the member's request, judge the command, and:

- **Fulfill** by running `cafleet member exec` (see [`reference/prompt-routing.md`](../reference/prompt-routing.md) § Director-side dispatch).
- **Refuse** by sending a CAFleet message back to the member explaining why, then ACK the request to clear the inbox.
- **Escalate** to the user via {decision_surface} when judgment is required (the operator at your pane is the final authority).

Silence breaks the workflow — the member ended its turn waiting for either the command or a follow-up message. Always close the loop one way or the other.

## Member Create

Register a new member and spawn a coding-agent pane in the Director's multiplexer window. The command auto-resolves the Director from the fleet row, registers the member and pending placement, renders the spawn prompt, spawns the pane, and patches its real pane ID. Failed creation attempts owned-pane cleanup and deregistration; cleanup failures or an unknown pane ID are reported without claiming complete rollback.

```bash
cafleet member create --fleet-id <fleet-id> \
  --name Reviewer-B --description "Reviewer for PR #42" \
  --file /abs/path/to/<BASE>/.prompts/reviewer-b-20260514T145000Z.md

cafleet member create --fleet-id <fleet-id> \
  --name Reviewer-C --description "Reviewer for PR #42" --coding-agent codex \
  --file /abs/path/to/<BASE>/.prompts/reviewer-c-20260514T145000Z.md
```

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Display name of the new member. |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | A CLI backend choice, recorded as `placement.coding_agent`. An explicit value wins; omission makes every role inherit **your** backend from your placement row. |
| `--model` | no | Pins the member's LLM (omitted → the binary's default; spawn-time only). The [inference table](#model-name-to-backend-inference) maps model references to backends; [`coding-agents.md`](../reference/coding-agents.md) owns their model lists and [`cli.md`](../reference/cli.md#member-create) owns the CLI contract. |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary (omitted → the binary's default; spawn-time only, never persisted). Validate the selected member backend's `{effort_levels}` value in its Runtime bindings table; the [CLI reference](../reference/cli.md#member-create) owns forwarding and rejection behavior. |
| `--permission-mode` | no | Omit it for every member. Pass it only when the user explicitly requests that mode for that member, per § *Member launch policy*. |
| positional `PROMPT` | one of | Inline spawn prompt. Exactly one of the positional and `--file` is required. |
| `--file PATH` | one of | UTF-8 spawn prompt path, absolute or relative to CWD; `-` reads the whole prompt from stdin. Use exactly one of the positional and `--file`; file input is the canonical mode for team-skill spawns (see § *Member Create — Scratch and audit files*). |

An absent backend binary exits 1 with `Error: binary <name> not found on PATH`. A missing or unreadable preset for the selected OpenCode policy exits 1 with `cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first`. Path/file errors are catalogued in [`cli.md`](../reference/cli.md#error-messages).

The per-backend `--permission-mode`, `--model` and `--effort` rules are in [`cli.md`](../reference/cli.md#member-create). A runner-observed permission request follows your own session’s mode-dependent decision and bound response under the [runtime procedure](../reference/member-runtime.md). Returned denials and automatic refusals route per [`reference/prompt-routing.md`](../reference/prompt-routing.md). Per-backend deltas: [`claude`](../reference/coding-agents.md#claude) / [`codex`](../reference/coding-agents.md#codex) / [`opencode`](../reference/coding-agents.md#opencode).

### Member launch policy

Spawn every member with the omitted `--permission-mode`, which selects the backend's automatic approval mode for Claude and Codex and the `cafleet` preset for OpenCode. Pass `--permission-mode` only when the user explicitly requests that mode for that member:

| Backend | Omitted default | Explicit choice on the user's request |
|---|---|---|
| Claude | `auto` | A non-automatic native value, such as `default` or `dontAsk` |
| Codex | `approve-for-me` | `on-request` or `never` |
| OpenCode | `cafleet` | `cafleet-ask` |

Claude Code 2.1.302 in `auto` can show `Teach auto mode about your environment?` once per native configuration. It arrives as an `interaction_opened` event with classification `question` and has no answer plan: ask the user to answer it on the member's native surface, and send no answer yourself. Codex 0.163.0 `approve-for-me` shows no startup dialog. Any other first-launch dialog the native host shows, such as a folder-trust prompt, follows the existing held-interaction procedure.

An `interaction_opened` permission request in automatic mode follows the [runtime procedure](../reference/member-runtime.md#consume-observed-interactions), and the automatic review events follow its [event table](../reference/member-runtime.md#consume-automatic-review-events). A routed request that arrives with no event follows the same refusal route.

### Model-name-to-backend inference

When the operator names a model rather than a backend ("please create a member with sonnet"), apply the following rows in order, using the [comparison key](#model-name-normalization) for family tests and catalog matches:

| # | Model reference | Backend | Flags to pass |
|---|---|---|---|
| 1 | Contains a `/` — provider-prefixed (e.g. `opencode/glm-5.3`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>`, verbatim |
| 2 | Claude reference: its key starts with `claude`, `fable`, `opus`, `sonnet` or `haiku` — `claude`, `opus`, `claude-opus-4-8`, `Opus 5.5` | `claude` | `--coding-agent claude --model <token>` resolved through Model-name normalization; the family word `claude` alone uses the claude Role default for the role |
| 3 | GPT reference: its key starts with `gpt` — `gpt`, `gpt-6.1-sol`, `gpt6.1-sol`, `GPT 6 Luna` | `codex` | `--coding-agent codex --model <token>` resolved through Model-name normalization; the family word `gpt` alone uses the codex Role default for the role |
| 4 | Any other bare name whose key equals the key of exactly one OpenCode candidate ID (normalization step 1) — `glm-5.3`, `kimi-k3.1-pro` | `opencode` | `--coding-agent opencode --model <token>`, token from OpenCode provider choice |
| 5 | Anything else — no key-equal ID, or more than one (e.g. `o3-mini`, `gemini`) | none | Ask the operator for the explicit `--coding-agent` + `--model` pair via {decision_surface} |

The rows apply as ordered precedence — the first match wins. Row 1 wins over row 2 for `anthropic/claude-sonnet-4-6`, which contains both `claude` and a `/`: a provider-prefixed token is itself an explicit OpenCode request. A bare name infers `opencode` only through row 4; Claude and GPT names select their native backends even when either OpenCode table lists the model.

Match candidate ID keys by equality: `glm-5.3` matches only the ID `glm-5.3`, never `glm-5.3-flash` or `glm-5.3-flash-free`.

#### Worked resolutions

| Operator says | Fleet backend | `opencode models` lists | Resolved flags |
|---|---|---|---|
| "opus member" | `opencode` | — | `--coding-agent claude --model claude-opus-5-5` |
| "gpt6.1-sol member" | `opencode` | — | `--coding-agent codex --model gpt-6.1-sol` |
| "claude member" (reviewer) | `codex` | — | `--coding-agent claude --model claude-opus-5-5 --effort high` |
| "spawn a claude member with opencode" | any | — | `--coding-agent opencode --model opencode/claude-opus-5-5` |
| "opus member with OpenCode" | any | `opencode/claude-opus-5-5` | `--coding-agent opencode --model opencode/claude-opus-5-5` |
| "gpt reviewer with OpenCode" | any | — | `--coding-agent opencode --model opencode/gpt-6-astra` |
| "sonnet 4.6 member with OpenCode" | any | `opencode/claude-sonnet-4-6` | `--coding-agent opencode --model opencode/claude-sonnet-4-6` |
| "opus 4.8 member with Claude Code" | any | — | `--coding-agent claude --model claude-opus-4-8` (manual pass-through) |
| "reviewer with Codex" | any | — | `--coding-agent codex --model gpt-6-astra --effort high` |
| "opus member with Codex" | any | — | none — relay as a mismatched pair through Model routing failures |
| "glm-5.3 member" | `claude` | `opencode/glm-5.3`, `opencode-go/glm-5.3` | `--coding-agent opencode --model opencode-go/glm-5.3` |
| "glm-5.3 member" | `claude` | `opencode/glm-5.3` only | `--coding-agent opencode --model opencode/glm-5.3` |
| "kimi-k3.1-pro member" | any | `opencode-go/kimi-k3.1-pro` | `--coding-agent opencode --model opencode-go/kimi-k3.1-pro` |
| "kimi-k3.1-pro member" | any | no `opencode-go/` lines | none — relay (neither candidate listed) |
| "haiku member with OpenCode" | any | `opencode-go/claude-haiku-5-5` | `--coding-agent opencode --model opencode-go/claude-haiku-5-5` |
| "haiku member" | `opencode` | — | `--coding-agent claude --model claude-haiku-5-5` |
| "opencode/glm-5.3 member" | any | — | `--coding-agent opencode --model opencode/glm-5.3` |
| "claude member with OpenCode" | any | — | `--coding-agent opencode --model opencode/claude-opus-5-5` |
| "gpt-5.4 member" | `opencode` | — | `--coding-agent codex --model gpt-5.4` (manual pass-through) |
| "a member" (no model, no backend) | `opencode` | — | `--coding-agent opencode --model opencode/glm-5.3-flash-free` |
| "glm-5.3 member with OpenCode" | any | command fails | none — relay |

Effort follows the selected backend's Role defaults and Runtime bindings; OpenCode omits `--effort`.

#### Model-name normalization

Normalize loose spellings before matching, so "gpt6.1-sol", "GPT 6.1 Sol" and "Opus 5.5" resolve to catalog tokens:

1. **Comparison key.** Lowercase the reference and every candidate token, and drop every character that is not a letter or digit (`gpt-6.1-sol` and `GPT 6.1 Sol` both become `gpt61sol`). OpenCode candidates are the distinct bare IDs of the Console and OpenCode Go tables: strip the `opencode/` or `opencode-go/` prefix before keying, and count an ID listed in both tables once. A Claude candidate also matches its key with the leading `claude` removed (`claude-opus-5-5` becomes `opus55`, matching "Opus 5.5").
2. **Native catalog first.** For a Claude or GPT reference (classified by the inference table's row 2 / row 3 key-prefix test), a key equal to the key of exactly one `Model` or `Alias` token in the native (`claude` / `codex`) catalog resolves to that row's `Model` token.
3. **Spelling dictionary.** A Claude or GPT reference equal to no native token key but to exactly one Claude/GPT ID key among the OpenCode candidates resolves to that bare ID. Keep the native backend and pass the ID as a manual `--model` value ("sonnet 4.6" becomes `--coding-agent claude --model claude-sonnet-4-6`).
4. **Exact shape.** A reference already written as an exact `claude-*` / `gpt-*` token that matches nothing passes through verbatim to its native backend.
5. **Otherwise ask.** Relay a reference with no key-equal token, or more than one token sharing the key, to the operator via {decision_surface}.

#### Explicit backend model mapping

When the first backend-resolution step selects an explicit backend, resolve `--model` from the reference. Apply each backend's rows in order; the first match wins.

**OpenCode.**

| Reference given with the OpenCode request | Resolved `--model` |
|---|---|
| No model (e.g. "spawn an opencode reviewer") | The opencode Role default for the role, verbatim |
| Provider-prefixed token (`opencode/…`, `opencode-go/…`, `anthropic/…`) | The token, verbatim |
| Claude family word (`claude`) | `opencode/` + the `claude` backend's Role default for the role, verbatim |
| GPT family word (`gpt`) | `opencode/` + the `codex` backend's Role default for the role, verbatim |
| Claude alias (`fable`, `opus`, `sonnet`, `haiku`) | The `claude` catalog Model the alias names, as an ID through OpenCode provider choice |
| Any other model name | The OpenCode candidate ID whose key equals the reference's key, through OpenCode provider choice |

Verbatim rows produce a complete token and skip provider choice. The last row uses only Model-name normalization step 1, including the leading-`claude` strip, against the OpenCode candidate IDs. It requires exactly one key-equal ID, so "sonnet 4.6 with OpenCode" resolves to the ID `claude-sonnet-4-6` before provider choice; normalization steps 2–4 apply only to native-backend resolution.

With the current Role defaults, `claude` maps to `opencode/claude-opus-5-5` for every role; `gpt` maps to `opencode/gpt-6-astra` for reviewers and `opencode/gpt-6.1-sol` for other members; `opus` resolves the ID `claude-opus-5-5` and `haiku` the ID `claude-haiku-5-5`, which provider choice turns into `opencode-go/claude-haiku-5-5` when `opencode models` lists it. Require every Role-default and family-word token to exist in the Console table, and every resolved ID to be an OpenCode candidate ID; the verbatim provider-prefixed row is exempt. Relay a missing or ambiguous match through Model routing failures before spawning.

**Claude Code / Codex.**

| Reference given with the Claude Code or Codex request | Resolved `--model` |
|---|---|
| No model (e.g. "reviewer with Claude Code") | That backend's Role default for the role |
| The backend's family word (`claude` for Claude Code, `gpt` for Codex) | That backend's Role default for the role |
| A reference of the backend's own family (inference row 2 for Claude Code, row 3 for Codex) | The token Model-name normalization steps 2–4 resolve against that backend (e.g. "opus 4.8 with Claude Code" becomes `claude-opus-4-8`) |
| A reference of another family, or a provider-prefixed token | none — relay as a mismatched pair through Model routing failures |

Relay an unresolved reference from Model-name normalization step 5 through the same failure procedure.

#### OpenCode provider choice

Apply this procedure when an OpenCode spawn's model resolves to a bare model ID through inference row 4 or an explicit-OpenCode mapping row that yields an ID. Rows that produce a complete provider-prefixed token keep that token verbatim.

The candidate tokens are `opencode-go/<id>` when the OpenCode Go table lists the ID, and `opencode/<id>` when the Console table lists it. At least one candidate exists because the ID was resolved against those tables.

1. Run `opencode models` once for this spawn as the Director's own command.
2. Read its output as one `provider/model` token per line, matching candidate tokens exactly and ignoring every other provider.
3. Choose the token using this table:

| `opencode models` result | `--model` |
|---|---|
| Non-zero exit, or no output | none — relay via {decision_surface} |
| Lists the `opencode-go/<id>` candidate | `opencode-go/<id>` |
| Lists only the `opencode/<id>` candidate | `opencode/<id>` |
| Lists neither candidate (e.g. a Go-only ID without a Go subscription) | none — relay via {decision_surface} |

Preserve the operator's explicit provider: a reference that carries a provider prefix keeps its complete token and skips this procedure.

#### Model routing failures

| Condition | Director behavior |
|---|---|
| Explicit OpenCode request whose mapped model is absent from both OpenCode tables (e.g. "claude-opus-4-1 with OpenCode") | Relay via {decision_surface} before spawning |
| `opencode models` exits non-zero or prints nothing during provider choice, cost efficiency selection or replacement | Relay via {decision_surface} before spawning; never fall back to `opencode/` |
| `opencode models` lists neither candidate token | Relay via {decision_surface} before spawning |
| Cost efficiency selection or replacement on OpenCode finds no listed token that can finish the task reliably | Relay an operator choice via {decision_surface}, as when no listed model fits the task |
| Pinned backend and model that do not belong together (e.g. "opus member with Codex") | Relay via {decision_surface} before spawning |
| Ambiguous normalization (two tokens share the key) or no match | Relay via {decision_surface} |
| Native backend binary missing (`Error: binary <name> not found on PATH`, exit 1) | Report the failure and relay the operator choice via {decision_surface}; preserve the backend selected by the model reference |
| Opencode catalog row removed at a later refresh while a member referenced it | Keep the running member; resolve the next spawn against the current catalog |

### Model list

Pass a token exactly as listed in the selected backend's Model catalog in [`reference/coding-agents.md`](../reference/coding-agents.md).

**Identity substitution**: the cafleet [`SKILL.md`](../SKILL.md#spawned-member-identity-via-strformat-substitution) owns the four placeholders and their errors. Director-side delta: both errors (exit 2) fire before registration, so nothing needs cleanup.

### Spawn prompt size limit

Use `--file` for templated identity blocks and compact role-by-path prompts. File input keeps the body out of the initial CAFleet invocation's argv; the resolved spawn prompt still enters downstream backend/multiplexer argv and remains subject to transport limits. A launch failure such as `tmux command failed: command too long` attempts deregistration and reports cleanup failures. Keep the inline positional `PROMPT` for trivial one-line ad-hoc spawns; no unmeasured size guarantees successful launch.

Keep the prompt body focused (the skeleton below): the member opens its role through an available text reader on its first turn, so path-by-reference to the stable in-skill role docs is safe.

### Canonical spawn-prompt skeleton

Every CAFleet-native team skill spawns its ordinary members from this one shared frame; each skill supplies only a compact **per-role delta** (a table in that skill) for the parts that vary.

Fixed frame — the identity lines carry the CLI's four `str.format` placeholders, rendered to literals by `cafleet member create` at spawn time; `[INSERT …]` markers are rendered by the Director before `member create`; the `‹…›` slots are filled from the per-role delta:

```text
You are ‹ROLE TITLE› in a ‹TEAM NAME› team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/‹role›.md] with an available text reader BEFORE any other action. That file is your authoritative role definition.‹ROLE-DEF SUFFIX› Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the cafleet skill — ‹cafleet-load purpose›
‹EXTRA SKILL LOADS›
Resolve the executing backend loader; absolute-path loading includes each requested skill core, own backend section and required role references. Continue this assigned workflow without spawning a second team.

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path]
CODING AGENT: {coding_agent}
‹CONTEXT LINES›

‹IMPORTANT / ROLE-CONSTRAINT LINES›
Host prerequisites: [INSERT required host-rule paths or equivalent session-supplied instructions and their source]. Use supplied equivalents for absent optional files; route an essential unknown rule to the Director before dependent work.

Use an available non-shell text reader for prerequisites; shell file reads may precede ready when shell is the only reader.

On spawn, as your first operational broker shell command, send the ready signal: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"

‹START CUE›
```

Rendering is **two-stage**: the Director substitutes the values it already knows as literals before the call (`BASE`, the absolute role-file path, the cafleet-load purpose phrase), then the CLI substitutes the four identity placeholders shown in the skeleton above (`{member_id}` is the member's own newly-allocated id, which only the CLI can fill). The Director must leave **no stray single braces** other than those four. After spawn the member sees literal labeled lines — `FLEET ID: 24`, `DIRECTOR MEMBER ID: 84`, `YOUR MEMBER ID: 88`, `CODING AGENT: claude` — and uses those integers on every `cafleet` command. There is no `COMMUNICATION PROTOCOL` command-example block: the member learns poll/send/ack command shapes from the `cafleet` skill and its role file. The `CODING AGENT:` line keeps the spawned binary and the overlay selector in lockstep for every role: when `--coding-agent` is omitted at spawn, the CLI records — and `{coding_agent}` renders — the backend inherited from the Director's placement row.

The `‹…›` markers are **spawn-skeleton slots**, filled from the consuming workflow's per-role delta table. They are distinct from the backend **runtime placeholders** (`{decision_surface}`, `{permission_flags}`, …), which resolve from [`coding-agents.md`](../reference/coding-agents.md), and from the four CLI identity placeholders.

| Slot | Filled per role with |
|---|---|
| `‹ROLE TITLE›` / `‹TEAM NAME›` | e.g. `the Programmer` / `design document execution`; `the Drafter` / `design document creation`. |
| `‹role›` + `‹ROLE-DEF SUFFIX›` | The `roles/<role>.md` filename, plus any addendum after "…role definition."; empty for most roles. A resume-mode example is `Follow the Resume Mode section in particular.`. |
| `‹cafleet-load purpose›` + `‹EXTRA SKILL LOADS›` | The cafleet purpose phrase (`for communication with the Director`, or `for the broker primitives and command routing through the Director`), plus any extra startup skills — `cafleet-design-doc` (design-doc family). |
| `‹CONTEXT LINES›` | The role inputs the consuming workflow's delta table lists, one labeled line each. |
| `‹IMPORTANT / ROLE-CONSTRAINT LINES›` | Every `IMPORTANT:` line and hard role constraint, verbatim (see lossless rule) — including each role's poll-handling line `When you see cafleet message poll output with a message from the Director, act on those instructions.`, plus each role's coordination constraints. |
| `‹START CUE›` | The role's closing instruction — e.g. `Start by reading the design document. Then wait for the Director to assign your first step.`; `Read the design document, generate a numbered question list …`; `When complete, send the file path to the Director …`. The start cue follows the frame's ready-signal line and does not restate it. |

**Lossless rule (non-negotiable).** Render every `IMPORTANT:` line, hard role constraint and start cue from the consuming workflow's per-role delta table **verbatim**. The shared frame supplies reader portability, startup ordering and host-prerequisite handling; each workflow's delta table is the authoritative inventory of its roles' hard lines.

### Member Create — Scratch and audit files

Write spawn-related scratch under resolved `${BASE}` or the skill's explicit output directory, following the [BASE contract](../reference/base-dir.md). A selected temporary BASE is valid; an ad-hoc temporary fallback is not. The pre-spawn `--file` input at `<BASE>/.prompts/<role>-<UTC-compact>.md` is the canonical audit artifact for every CAFleet-native team-skill spawn:

- `<role>` is the lowercased `--name`; `<UTC-compact>` is `YYYYMMDDTHHMMSSZ`. Create `<BASE>/.prompts/` on first write; on a same-second collision append the suffix `_2`, `_3`, … (never overwrite).
- The pre-spawn file IS the audit artifact — there is no post-spawn re-render. The `--file` path is the single source of truth for what was spawned, in perpetuity. It carries the four `{...}` identity placeholders **pre-substitution** — that is expected; the CLI renders them at spawn.

**`${BASE} == <unset>` fallback**: when startup-time `${BASE}` resolution returned the `<unset>` sentinel, follow the guarded-skip protocol in [`reference/base-dir.md`](../reference/base-dir.md) § *No-bypass write protocol* — skip the `<BASE>/.prompts/<role>-<ts>.md` write, fall back to the inline positional-`PROMPT` form (keep it under ~2 KB, path-by-reference), and emit the anchorless status `audit-disabled no BASE in spawn prompt` once per spawn cycle. The spawn still proceeds.

**Pane discovery**: discover a member's pane via `cafleet member list` (the `pane_id` column is ground truth for all backends). Pane title: {pane_title}. A failed spawn attempts to kill any pane whose id it owns and deregister the member, reporting cleanup failures after the primary error; a backend cleanup attempt is never repeated by the CLI. An unknown pane id leaves pane cleanup unconfirmed. Creation uses `-d` so the Director keeps focus. See [`cli.md`](../reference/cli.md#member-create).

## Model replacement

Read [Recovery](../reference/supervision.md#recovery) immediately before replacing a member. A user-pinned model requires an operator decision. The Director owns member replacement; the Reviewer supplies evidence (an `[INCORRECT]` marker naming the suspected unmet capability) and stays independent of the execution. Slowness, awaiting user input, or a transient infrastructure error is never grounds for replacement. Valid evidence: a member's self-report that it cannot reason through the task, repeated task-relevant reasoning/coding failures after normal correction, a Reviewer `[INCORRECT]` finding naming the unmet capability, or a Director review of a materially incomplete/incorrect result tied to the task profile.

For each replacement, in order:

1. **Freeze and hand off** — freeze new work for the task and request one concise state report (completed work, modified paths, commands/tests run, blockers, next step); if the member cannot respond promptly, `cafleet member capture` is the handoff evidence.
2. **Re-select stronger** — pick a strictly more capable model from the same backend (a row above the failed model in its table) that fits the task's now-demonstrated difficulty; choose the cheapest model within that stronger set. For an OpenCode member, the stronger set is every ID ranked above the failed ID in each OpenCode table that lists the failed ID, and OpenCode cost ranking chooses within it.
3. **Record** — note the trigger, evidence pointers, old/new model, and attempt number in your coordination notes (no secrets or prompt contents).
4. **Delete before create** — `cafleet member delete` the old member through the standard lifecycle before spawning the replacement; all normal spawn/audit/prompt-substitution rules apply.
5. **Resume, not restart** — spawn the replacement with the original assignment plus the bounded handoff and the same deliverable paths, route the original task pointer, and have the Reviewer re-evaluate at the normal review point.

Caps and fail-closed cases: the initial member plus at most two replacements per task; each replacement strictly more capable than its predecessor; a `(task pointer, model)` pair is never retried. For an OpenCode member, the model in a `(task pointer, model)` pair is the bare ID, so `opencode/X` after a failed `opencode-go/X` is a retry of the same model. An unlisted manual model, a user-pinned model or effort, an empty stronger set, a reached cap, or ambiguous evidence all mean the Director relays an operator choice (approve a named higher-cost/manual override, simplify/re-scope, or stop) instead of auto-replacing.

## Member Delete

For a wrapped run, coordinate identity-pinned runner shutdown and bounded event recovery before pane/registry cleanup, retaining unconfirmed descendants and recoverable artifacts. For a concrete placement, acquire its shared pane-input lock and revalidate the placement before killing the pane and deregistering the member; preserve the backend’s layout cleanup. Retry lock contention every 50 milliseconds against one operation-wide five-second monotonic deadline. On expiry, exit 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leave the pane/registry unchanged. A pending or placementless unwrapped member follows plain registry deregistration without a pane lock or multiplexer operation; an already-gone concrete pane retains its documented tolerated teardown.

```bash
cafleet member delete <member-id>
```

An unknown or inactive `MEMBER_ID` exits 1 (`Error: Member <member-id> not found`); deleting the root Director stays blocked by the root-Director guard. Exit codes and the output shape: [`cli.md`](../reference/cli.md#member-delete).

## Member List

```bash
cafleet member list <fleet-id>
```

One output shape: every **active** registry entry of the fleet, one row each — the column enumeration and the `--json` timestamp additions are canonical in [Member list output](../reference/cli.md#member-list-output). On every wake, use the `idle` column and overdue assignments instead of capturing every member — capture is reserved for the cases they flag.

## Member Capture

Capture the last N lines of a member's pane buffer (read-only); [`cli.md`](../reference/cli.md#member-capture) owns flags, defaults and output. This is the targeted deeper-investigation primitive — scan for all, capture when one member needs a closer look.

```bash
cafleet member capture <member-id>
cafleet member capture <member-id> --lines 200
```

## Answering a member's relayed question

Ask the user through {decision_surface} under the [User Delegation Protocol](../reference/supervision.md#user-delegation-protocol), then forward the answer to the member as an ordinary `cafleet message send`, not a pane keystroke. Surface constraints are in your backend's decision-surface note.

## Member Exec

Authorize the required one-line command and call `cafleet member exec <member-id> COMMAND` or `--file PATH`. The CLI submits visible native shell input, or the runner queues it behind a detected interaction, and returns dispatch metadata with its delivery status and method. Inspect `member capture` for output and completion. After completion evidence, send an explicit continuation prompt if the host has not resumed the member. On failed/uncertain dispatch, inspect before authorizing a retry. Follow [routing](../reference/prompt-routing.md).

## Member Prompt

Use `cafleet member prompt <member-id> TEXT` for explicitly authorized composer input. Wrapped input reaches a verified empty composer through the profile plan without Escape. The runner queues it with zero bytes while a native interaction is detected, or while a planned composer is not verified empty, and delivers it when that clears or by fallback after 3 minutes. Without a plan it uses Escape-first delivery fallback. Explicitly unwrapped input retains Escape-first behavior, and both paths reject the Director’s own pane. Success means submission or queue acceptance only; its `delivery` reports which. Before prompting an explicitly unwrapped member, run the open-prompt check in [Director-side dispatch](../reference/prompt-routing.md#director-side-dispatch).

## Member Press

Use `cafleet member press <member-id> KEY...` for explicitly authorized ordered manual keys. Pass only the intended keys and use the ordinary `--` separator for literal punctuation; append neither Escape nor Enter. The [CLI contract](../reference/cli.md#member-press) owns grammar, limits, exact output and error forms. The command holds the revalidated shared pane lock and sends one bounded backend request to the recorded outer pane, including wrapped members.

Success establishes backend acknowledgment only. Inspect actual native effects and continuation independently; a started failure keeps delivery uncertain and requires a new explicit decision after inspection. Apply the existing host and request-specific user/Director decision policy before answering a native request. Use `permission respond` for the separately bound decision interface; selecting manual keys bypasses no host policy and supplies no permission to answer.

## Director wake

Follow [Director wake](../reference/supervision.md#director-wake).

## Runtime events and recovery

Follow the [member runtime procedure](../reference/member-runtime.md) for event consumption, mode-dependent decisions, request-bound responses, manual native input and continuation. Use `event show` and `input show` to reconcile durable identities; only the Director owns `permission inspect/respond`, `input reconcile` and `member recover`, preserving ownership, snapshot-bound evidence and uncertainty.
