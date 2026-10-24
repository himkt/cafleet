# Coding Agents

Each backend section is self-contained. Select the backend by the subject of the action, then use its tables and bound notes:

| Purpose | Backend selector | Read and apply |
|---|---|---|
| Execute current instructions | The executing agent's `CODING AGENT:` identity, or its own identity when standalone | Its Runtime bindings and corresponding notes for decision surface, skill loader, and local execution tools. Required-reading row one gates this lookup; an ordinary member needs only its own runtime section. |
| Select/configure a spawned member | The backend selected under [Director model-selection policy](../roles/director.md#model-selection) | Its Model catalog and Role defaults; validate target effort and launch capabilities against its Runtime bindings. |
| Interpret a captured member pane | The observed member's recorded backend | Its Pane-state capture cues, while retaining the observer's own execution tools and decision surface. |

Resolve the five runtime placeholders from Runtime bindings for the relevant operation and model/effort placeholders from the selected backend’s Role defaults. Apply notes at their named instructions and emit concrete command values using the core skill's [Resolve your overlay](../SKILL.md#resolve-your-overlay) procedure and its documented neutral defaults for explicitly allowed missing/unknown-backend cases. Report a missing required backend section, malformed table, or broken supported-backend reference as a documentation defect.

For example, a Codex Director selecting an OpenCode reviewer uses the OpenCode catalog and reviewer defaults with the Codex decision surface and local execution tools. A Claude Director inspecting Codex applies the target backend’s capture cues. User overrides and selection decisions follow [Director model-selection policy](../roles/director.md#model-selection).

Model catalogs, their provenance/context notes, canonical Role defaults, and shared freshness metadata are maintained exclusively by the `cafleet-model-list-refresh` skill from the official sources in each backend section, refreshed at least every 30 days (last refreshed: 2026-10-20). Runtime bindings, bound runtime notes, pane cues, and worked resolutions belong to runtime documentation maintenance. The freshness date applies to model data; structural reorganization preserves that date.

Prices are standard provider USD rates per MTok and are planning estimates, not an invoice guarantee. Each backend's catalog is ordered most → least capable as reviewed judgment. Context windows are listed for the `claude` backend, whose model strings are the ones a context-window suffix can apply to. Selection policy, including cost efficiency mode and reviewer/member rules, lives in [Director model selection](../roles/director.md#model-selection).

## claude

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool |
| `{effort_levels}` | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `claude-opus-5-5` |
| `{reviewer_effort}` | `high` |
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
| claude-haiku-5-5 | haiku | Fast lowest-cost tier; 1M context; routine checks and quick bounded tasks | 1M | 0.10 | 0.50 |

Every row above runs at its 1M window on every plan on the Anthropic API,
so its `--model` value needs no `[1m]` suffix. `claude-haiku-5-5` prices
above apply to prompts up to 100K tokens; longer prompts bill at 0.50 / 2.50.

Sources: [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing.md) and [Claude Code model configuration](https://code.claude.com/docs/en/model-config.md) — context windows and `[1m]` applicability.

### Note → applies at

| Note | Applies at |
|---|---|
| `AskUserQuestion` supports up to four options; its built-in Other path accepts free text. Members relay a concrete question to the Director. | `{decision_surface}`; [user relay](supervision.md#user-delegation-protocol) |
| A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. External CAFleet panes supply evidence when the Director reads broker/pane state. | [Director receiving](supervision.md#asynchronous-wait-rule) |
| Without native background work an unmet condition can immediately start another turn; consecutive tool-free stops reach the configured block cap, eight by default. Retain the finite receive task handle while delegated work remains. | [Turn boundaries](supervision.md#asynchronous-wait-rule) |
| Composer controls act at the beginning of submitted input; a local probe merged control text into a concurrent preview. Serialize complete input and inspect its capture. | [Native dispatch](prompt-routing.md#director-side-dispatch) |
| Native shell probes displayed command/stdout/stderr followed by a model turn; its trigger and a second permission check were not established. The Director authorizes routed native input under its host rules. | [Command authorization](prompt-routing.md#director-side-dispatch) |
| `dontAsk` permits spawn-allowed broker tools, built-in read-only work and matching host allow rules; denied work routes to the Director. Protected file edits need the Director or a host-rule change. | `{permission_flags}`; [routing](prompt-routing.md#member-side-reconsider-then-route) |

Evidence: Claude Code 2.1.296, checked 2026-10-22; [host continuation](https://code.claude.com/docs/en/goal) and [block cap](https://code.claude.com/docs/en/hooks-guide#stop-hook-hits-the-block-cap). Local probes establish only exercised behavior.

### Pane-state capture cues

Use fresh styled capture as transport evidence. Approval/work outranks idle cues; preserve drafts and treat ambiguity as unknown.

| State | claude capture cue |
|---|---|
| `idle` | Empty `>` composer with recognized dim/grey host suggestion or placeholder decoration and no active/approval evidence |
| `working` | Spinner, streaming, shell/tool activity or `esc to interrupt` |
| `awaiting_user` | Question/confirmation/approval box awaiting a selection |
| `draft` | User-input styled unsent text in the composer, including text identical to a host suggestion |
| `unknown` | Missing, truncated, unrecognized or ambiguous composer/style evidence |

Follow [Director evidence handling](supervision.md#idle-semantics). Empty readiness establishes no task completion.

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
| `{other_model}` | `gpt-6.1-sol` |
| `{other_effort}` | `xhigh` |

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
|---|---|
| A member sends a concrete question to the Director; the Director relays the operator's answer as a message. | `{decision_surface}`; [user relay](supervision.md#user-delegation-protocol) |
| The objective belongs to the thread with model-callable lifecycle tools and optional token-budget accounting; a two-turn tool-bearing probe verified continuation and model-reported completion. Preserve the user's objective, activation authority and budget, retain live tool/session handles and collect broker/pane evidence. | [Director receiving](supervision.md#asynchronous-wait-rule) |
| Native shell input displayed command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the effective TUI boundary and protocol path were not established. | [Native authorization](prompt-routing.md#director-side-dispatch) |
| The Director authorizes routed native input under its host rules; harmless command visibility establishes no second member-tool permission check. Inspect output before explicit continuation. | [Dispatch and continuation](prompt-routing.md#director-side-dispatch) |
| Run host-permitted workspace commands directly and route required denied operations. The configured sandbox and approval posture remain authoritative. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-reconsider-then-route) |

Evidence: Codex CLI 0.162.1, checked 2026-10-22; [host continuation](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex) and [command reference](https://learn.chatgpt.com/docs/developer-commands).

### Pane-state capture cues

Use fresh styled capture as transport evidence. Approval/work outranks idle cues; preserve drafts and treat ambiguity as unknown.

| State | codex capture cue |
|---|---|
| `idle` | Empty native composer or verified decoration-styled placeholder without active/approval evidence |
| `working` | Active turn/working indicator, streaming, tool or native shell execution |
| `awaiting_user` | Confirmation or approval prompt awaiting input |
| `draft` | Unsent user-styled composer text, including text identical to a placeholder |
| `unknown` | Missing, truncated, unrecognized or ambiguous composer/style evidence |

Follow [Director evidence handling](supervision.md#idle-semantics). Empty readiness establishes no task completion.

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
| `{reviewer_model}` | `opencode/deepseek-v4.1-flash-free` |
| `{reviewer_effort}` | `—` |
| `{other_model}` | `opencode/glm-5.3-flash-free` |
| `{other_effort}` | `—` |

### Model catalog

A curated subset of the [OpenCode Console](https://opencode.ai/v2/docs/console/models/)
catalog; the `opencode/` prefix is part of the `--model` value. Any other
`<provider-id>/<model-id>` value remains a manual spawn with explicit
`--coding-agent opencode --model` flags on `member create`. A price of
0.00 is an explicitly free model, currently offered by OpenCode Console for
a limited time.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode/muse-spark-1.2 | Mid-price general coding tier | 1.25 | 4.25 |
| opencode/deepseek-v4.1-flash-free | DeepSeek V4.1 Flash free tier; zero-retention provider | 0.00 | 0.00 |
| opencode/glm-5.3-flash-free | GLM-5.3-Flash free efficient tier; zero-retention provider | 0.00 | 0.00 |
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/glm-5.3-flash | Fast low-cost GLM tier for routine work | 0.15 | 0.50 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |

Source: [OpenCode Console models and pricing](https://opencode.ai/v2/docs/console/models/).

### Note → applies at

| Note | Applies at |
|---|---|
| A member routes concrete questions to the Director and receives the operator's answer as a message. | `{decision_surface}`; [user relay](supervision.md#user-delegation-protocol) |
| Members perform assignments using the selected provider/model, workspace reads/edits, permitted tools and broker communication. The Director coordinates next steps and retains finite receiving. | [Director receiving](supervision.md#asynchronous-wait-rule) |
| The TUI documents leading `!` native shell input and conversation-visible output. A local native-input permission probe was outside this investigation; preserve member-tool permissions and Director authorization. | [Native dispatch](prompt-routing.md#director-side-dispatch) |
| The `cafleet` preset allows its declared workspace/broker tools and denies unmatched shell commands; route a required denied command. A permission popup is an issue to capture and relay. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-reconsider-then-route) |

Provenance: [OpenCode shell input](https://docs.opencode.ai/docs/tui/#shell-commands).

### Pane-state capture cues

Use fresh styled capture as transport evidence. Approval/work outranks idle cues; preserve drafts and treat ambiguity as unknown.

| State | opencode capture cue |
|---|---|
| `idle` | Affirmative empty composer or verified decoration-styled placeholder without active/approval evidence |
| `working` | Streaming, generation, tool/shell activity or interrupt cue |
| `awaiting_user` | Question/permission/selection popup awaiting a choice |
| `draft` | Unsent user-input styled text in the recognized composer |
| `unknown` | Missing, truncated, unrecognized or ambiguous composer/style evidence |

Follow [Director evidence handling](supervision.md#idle-semantics). Empty readiness establishes no task completion.

## Template

Add a `## <name>` backend with every subsection below. Fill every value, keep runtime bindings as short noun phrases, and bind each caveat to its token and consuming instruction. Shared selection and supervision policy stays at its linked owner.

### Runtime bindings

Supply all five tokens: `{decision_surface}` (recorded user-reaction surface; members relay), `{permission_flags}` (exact auto-approval flags), `{pane_title}` (title mechanism or no analog), `{skill_loader}` (supported loader or complete absolute-path reads), and `{effort_levels}` (accepted values/flag or unsupported).

### Role defaults

Set `{reviewer_model}` and `{other_model}` to catalog tokens or aliases. Set their matching effort placeholders to a supported level or `—` when the backend does not support effort.

### Model catalog

List exact spawn tokens, valid aliases, reviewed most-to-least capability ordering, classes, and standard provider USD/MTok input/output prices. Include relevant context windows and availability constraints, required prefixes and official pricing/availability/configuration sources. Follow the freshness contract and [Director policy](../roles/director.md#model-selection).

### Note → applies at

Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind the pane-cue table to Director capture inspection and the Director's reading of a pane capture. Bind a `{permission_flags}` row stating the posture's denial semantics — what a member runs unprompted, what the harness denies, and when the member routes — to the member command-execution, prompt-routing, Director, and supervision instructions that point at it.

### Pane-state capture cues

Provide backend-specific styled cues for `idle`, `working`, `awaiting_user`, `draft` and `unknown`. Bind receiving and native-input facts to their consuming supervision/routing instructions. Preserve input when evidence is ambiguous.
