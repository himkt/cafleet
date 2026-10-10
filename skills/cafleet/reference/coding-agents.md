# Coding Agents

Each backend section is self-contained. Select the backend by the subject of the action, then use its tables and bound notes:

| Purpose | Backend selector | Read and apply |
|---|---|---|
| Execute current instructions | The executing agent's `CODING AGENT:` identity, or its own identity when standalone | Its Runtime bindings and corresponding notes for decision surface, skill loader, and local execution tools. Required-reading row one gates this lookup; an ordinary member needs only its own runtime section. |
| Select/configure a spawned member | The backend selected under [Director model-selection policy](../roles/director.md#model-selection) | Its Model catalog and Role defaults; validate target effort and launch capabilities against its Runtime bindings. |
| Interpret a captured member pane | The observed member's recorded backend | Its Pane-state capture cues, while retaining the observer's own execution tools and decision surface. |

Resolve the five runtime placeholders from Runtime bindings for the relevant operation and the model and effort placeholders from the selected spawn backend's Role defaults. Apply notes at their named instructions and emit concrete command values using the core skill's [Resolve your overlay](../SKILL.md#resolve-your-overlay) procedure and its documented neutral defaults for explicitly allowed missing/unknown-backend cases. Report a missing required backend section, malformed table, or broken supported-backend reference as a documentation defect.

For example, a Codex Director selecting an OpenCode reviewer uses the OpenCode catalog and reviewer defaults with the Codex decision surface and local execution tools. A Claude monitor observing a Codex member uses Codex capture cues with its own Claude tools. Monitor bootstrap and recovery select the Director's backend by construction: pass its monitor model and supported effort at bootstrap and recovery, retaining backend inheritance. User overrides and selection decisions follow [Director model-selection policy](../roles/director.md#model-selection).

Model catalogs, their provenance/context notes, canonical Role defaults, and shared freshness metadata are maintained exclusively by the `cafleet-model-list-refresh` skill from the official sources in each backend section, refreshed at least every 30 days (last refreshed: 2026-10-08). Runtime bindings, bound runtime notes, pane cues, and worked resolutions belong to runtime documentation maintenance. The freshness date applies to model data; structural reorganization preserves that date.

Prices are standard provider USD rates per MTok and are planning estimates, not an invoice guarantee. Each backend's catalog is ordered most → least capable as reviewed judgment. Context windows are listed for the `claude` backend, whose model strings are the ones a context-window suffix can apply to. Selection policy, including cost efficiency mode and monitor/reviewer rules, lives in [Director model selection](../roles/director.md#model-selection).

## claude

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |
| `{effort_levels}` | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `claude-opus-5-5` |
| `{reviewer_effort}` | `high` |
| `{monitor_model}` | `claude-haiku-5-5` |
| `{monitor_effort}` | `low` |
| `{other_model}` | `claude-opus-5-5` |
| `{other_effort}` | `high` |

### Model catalog

Either the model name or its alias is a valid `--model` token.

| Model | Alias | Class | Context | Input $/MTok | Output $/MTok |
|---|---|---|---|---|---|
| claude-fable-5-1 | fable | Mythos-class frontier; highest capability on every dimension | 1M | 10.00 | 50.00 |
| claude-fable-5 | — | Prior Mythos-class generation at the same price tier | 1M | 10.00 | 50.00 |
| claude-opus-5-5 | opus | Everyday frontier; strong coding, planning, and review | 1M | 4.00 | 20.00 |
| claude-opus-5 | — | Prior frontier generation at a higher price | 1M | 5.00 | 25.00 |
| claude-sonnet-5-5 | sonnet | Efficient mid tier for routine work | 1M | 2.00 | 10.00 |
| claude-sonnet-5 | — | Prior mid-tier generation at the same price | 1M | 2.00 | 10.00 |
| claude-haiku-5-5 | haiku | Fast lowest-cost tier; 1M context; monitoring and quick bounded tasks | 1M | 0.10 | 0.50 |
| claude-haiku-4-5 | — | Prior Haiku generation at a higher price | 200K | 1.00 | 5.00 |

Every 1M row above runs at that window on every plan on the Anthropic API,
so its `--model` value needs no `[1m]` suffix; `claude-haiku-4-5` has no 1M
variant and never takes one. `claude-haiku-5-5` prices above apply to
prompts up to 100K tokens; longer prompts bill at 0.50 / 2.50.

Sources: [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing.md) and [Claude Code model configuration](https://code.claude.com/docs/en/model-config.md) — context windows and `[1m]` applicability.

### Note → applies at

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| *Pane-state capture cues* (below) — the concrete claude-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's reading of a pane capture — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director reading a capture — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | claude capture cue |
|---|---|
| `awaiting_user` | A bordered prompt box awaiting a keypress: an `AskUserQuestion` selection list (numbered options with a `❯`-marked choice and an "Other" free-text entry) or a tool/permission approval prompt (`Do you want to proceed?` with `❯ 1. Yes` / `2. No`). A choice is pending; the composer is not at rest. |
| `finished` | The bare composer at rest: an empty `>` input line with its placeholder hint and the idle status line beneath (model + context, the `⏵⏵` mode indicator), with **no** question/approval box above it and **no** running spinner or `esc to interrupt` indicator. |
| `working` | Affirmative active-work evidence: a running spinner, `esc to interrupt`, streaming response, tool execution, or generation indicator. A truncated or ambiguous capture that might hide one of these cues is also `working`. |
| `stall_candidate` | Quiet, non-finished transcript content with no question/approval box, no empty at-rest composer, and no spinner, tool, streaming, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `cafleet/roles/monitor.md` § *On each wake*.

## codex

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--ask-for-approval never --sandbox workspace-write` |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading each requested skill core, own backend section and role-required references by the absolute paths the spawn prompt provides |
| `{effort_levels}` | `minimal`, `low`, `medium`, `high`, `xhigh` (spawn flag `--effort <level>`, forwarded as `--config=model_reasoning_effort=<level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `gpt-6-astra` |
| `{reviewer_effort}` | `high` |
| `{monitor_model}` | `gpt-6-luna` |
| `{monitor_effort}` | `low` |
| `{other_model}` | `gpt-6.1-sol` |
| `{other_effort}` | `medium` |

### Model catalog

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| gpt-6-astra | Latest frontier tier across code, apps, and research; strongest reviewer | 10.00 | 50.00 |
| gpt-6.1-sol | Complex coding and agentic workflows tier for everyday work | 2.00 | 10.00 |
| gpt-6-luna | Fast efficient tier for focused, high-volume tasks | 0.10 | 0.50 |

Prices apply to prompts up to 272K input tokens; longer prompts bill at a
higher long-context rate.

Sources: [OpenAI pricing](https://developers.openai.com/api/docs/pricing) and [Codex model availability](https://learn.chatgpt.com/docs/models.md).

### Note → applies at

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. Ask a concrete, answerable question, not free-form prose. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| *Pane-state capture cues* (below) — the concrete codex-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's reading of a pane capture — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director reading a capture — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | codex capture cue |
|---|---|
| `awaiting_user` | A codex confirmation/approval prompt awaiting a keypress — a `[y/n]`-style command-approval or an out-of-sandbox escalation request. Under `--ask-for-approval never --sandbox workspace-write` codex auto-approves in-workspace work, so a visible approval prompt means codex hit something the policy could not clear and is genuinely waiting. |
| `finished` | The empty codex composer at rest — the input prompt with no streaming output above it and no active-turn / "working" indicator. |
| `working` | Affirmative active-work evidence: the active-turn or `working` indicator, streaming model output, a running tool, or generation in progress. A truncated or ambiguous capture that may still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no confirmation prompt, no empty at-rest composer, and no active-turn, tool, streaming, generation, or other work cue. |

Tie-breaks and the two-quiet-families rule: `cafleet/roles/monitor.md` § *On each wake*.

## opencode

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--agent cafleet` |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading each requested skill core, own backend section and role-required references by the absolute paths the spawn prompt provides |
| `{effort_levels}` | unsupported — omit `--effort` |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `opencode/muse-spark-1.3-contributor-free` |
| `{reviewer_effort}` | `—` |
| `{monitor_model}` | `opencode/big-pickle` |
| `{monitor_effort}` | `—` |
| `{other_model}` | `opencode/mimo-v2.5-free` |
| `{other_effort}` | `—` |

### Model catalog

A curated subset of the [OpenCode Zen](https://opencode.ai/docs/zen.md)
catalog; the `opencode/` prefix is part of the `--model` value. Any other
`<provider-id>/<model-id>` value remains a manual spawn with explicit
`--coding-agent opencode --model` flags on `member create`. A price of
0.00 is an explicitly free model, currently offered by Zen for a limited
time.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode/muse-spark-1.2 | Mid-price general coding tier | 1.25 | 4.25 |
| opencode/muse-spark-1.3-contributor-free | Muse Spark 1.3 free for contributors | 0.00 | 0.00 |
| opencode/mimo-v2.5-free | MiMo-V2.5 free general model | 0.00 | 0.00 |
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |

Source: [OpenCode Zen models and pricing](https://opencode.ai/docs/zen.md).

### Note → applies at

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. The `--agent cafleet` safety floor shows no popup; if a popup ever appears it is a regression to escalate, not a decision point. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| *Pane-state capture cues* (below) — the concrete opencode-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's reading of a pane capture — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director reading a capture — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | opencode capture cue |
|---|---|
| `awaiting_user` | An opencode permission/selection popup awaiting a choice. The `--agent cafleet` floor suppresses permission popups, so a visible popup is both the `awaiting_user` signal **and** a regression to escalate (see the decision-surface note above). |
| `finished` | The empty opencode prompt at rest — no streaming response above it and no active generation indicator. |
| `working` | Affirmative active-work evidence: streaming response text, an active generation indicator, a running tool, or any other visible in-progress state. A truncated or ambiguous capture that might still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no popup, no empty at-rest prompt, and no streaming, tool, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `cafleet/roles/monitor.md` § *On each wake*.

## Template

Add a `## <name>` backend with every subsection below. Fill every value, keep runtime bindings as short noun phrases, and bind each caveat to its token and consuming instruction. Shared selection and supervision policy stays at its linked owner.

### Runtime bindings

Supply all five tokens: `{decision_surface}` (recorded user-reaction surface; members relay), `{permission_flags}` (exact auto-approval flags), `{pane_title}` (title mechanism or no analog), `{skill_loader}` (supported loader or complete absolute-path reads), and `{effort_levels}` (accepted values/flag or unsupported).

### Role defaults

Set `{reviewer_model}`, `{monitor_model}`, and `{other_model}` to catalog tokens or aliases. Set their matching effort placeholders to a supported level or `—` when the backend does not support effort.

### Model catalog

List exact spawn tokens, valid aliases, reviewed most-to-least capability ordering, classes, and standard provider USD/MTok input/output prices. Include relevant context windows and availability constraints, required prefixes and official pricing/availability/configuration sources. Follow the freshness contract and [Director policy](../roles/director.md#model-selection).

### Note → applies at

Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind the pane-cue table to monitor on-wake classification and the Director's reading of a pane capture.

### Pane-state capture cues

Provide concrete content-only cues for `awaiting_user`, `finished`, `working` and `stall_candidate`; native `agent_status` is outside this classification. Finished requires an empty at-rest composer without a pending prompt or active work; working includes streaming, generation, tools and ambiguous/truncated captures that may still be active. Stall-candidate means quiet non-finished content without a prompt or active-work cue. Link [the monitor's on-wake protocol](../roles/monitor.md#on-each-wake) for both tie-breaks (`awaiting_user` over `finished`, `working` over `stall_candidate`) and quiet confirmation.
