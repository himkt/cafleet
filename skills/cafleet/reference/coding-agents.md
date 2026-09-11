# Coding Agents

Each backend section is self-contained. Select the backend by the subject of the action, then use its tables and bound notes:

| Purpose | Backend selector | Read and apply |
|---|---|---|
| Execute current instructions | The executing agent's `CODING AGENT:` identity, or its own identity when standalone | Its Runtime bindings and corresponding notes for decision surface, skill loader, and local execution tools. Required-reading row one gates this lookup; an ordinary member needs only its own runtime section. |
| Select/configure a spawned member | The backend selected under [Director model-selection policy](../roles/director.md#model-selection) | Its Model catalog and Role defaults; validate target effort and launch capabilities against its Runtime bindings. |
| Interpret a captured member pane | The observed member's recorded backend | Its Pane-state capture cues, while retaining the observer's own execution tools and decision surface. |

Resolve the seven runtime placeholders from Runtime bindings for the relevant operation and the two model placeholders from the selected spawn backend's Role defaults. Apply notes at their named instructions and emit concrete command values using the core skill's [Resolve your overlay](../SKILL.md#resolve-your-overlay) procedure and its documented neutral defaults for explicitly allowed missing/unknown-backend cases. Report a missing required backend section, malformed table, or broken supported-backend reference as a documentation defect.

For example, a Codex Director selecting an OpenCode reviewer uses the OpenCode catalog, reviewer default, and effort capability with the Codex decision surface and local execution tools. A Claude monitor observing a Codex member uses Codex capture cues and Claude background execution for its own loop. Monitor bootstrap and recovery select the Director's backend by construction: use its monitor default for `--monitor-model` at bootstrap and `--model` at recovery, retaining backend inheritance. User overrides and selection decisions follow [Director model-selection policy](../roles/director.md#model-selection).

Model catalogs, their provenance/context notes, canonical Role defaults, and shared freshness metadata are maintained exclusively by the `cafleet-model-list-refresh` skill from the official sources in each backend section, refreshed at least every 30 days (last refreshed: 2026-09-05). Runtime bindings, bound runtime notes, pane cues, and worked resolutions belong to runtime documentation maintenance. The freshness date applies to model data; structural reorganization preserves that date.

Prices are standard provider USD rates per MTok and are planning estimates, not an invoice guarantee. Each backend's catalog is ordered most → least capable as reviewed judgment. Context windows are listed for the `claude` backend, whose model strings are the ones a context-window suffix can apply to. Selection policy, including cost efficiency mode and monitor/reviewer rules, lives in [Director model selection](../roles/director.md#model-selection).

## claude

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{bg_run}` | the Bash tool's `run_in_background: true` |
| `{bg_stop}` | `TaskStop` |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |
| `{effort_levels}` | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `fable` |
| `{monitor_model}` | `haiku` |

### Model catalog

Either the model name or its alias is a valid `--model` token.

| Model | Alias | Class | Context | Input $/MTok | Output $/MTok |
|---|---|---|---|---|---|
| claude-fable-5-1 | fable | Mythos-class frontier; highest capability on every dimension | 1M | 10.00 | 50.00 |
| claude-fable-5 | — | Prior Mythos-class generation at the same price tier | 1M | 10.00 | 50.00 |
| claude-opus-5 | opus | Everyday frontier; strong coding, planning, and review | 1M | 5.00 | 25.00 |
| claude-opus-4-8 | — | Prior frontier generation at the same price tier | 1M | 5.00 | 25.00 |
| claude-sonnet-5 | sonnet | Efficient mid tier for routine work | 1M | 2.00 | 10.00 |
| claude-haiku-4-5 | haiku | Fast low-cost tier; monitoring and quick bounded tasks | 200K | 1.00 | 5.00 |

Every 1M row above runs at that window by default on the Anthropic API, so
its `--model` value needs no `[1m]` suffix; `claude-haiku-4-5` has no 1M
variant and never takes one. The one case that calls for the suffix is an
Opus spawn on a Pro plan, where the 1M window is opt-in and billed to usage
credits — there the Director passes `--model 'claude-opus-5[1m]'`, quoted,
because the brackets otherwise glob in zsh. Confirm the operator's plan
before spending their credits that way.

Sources: [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing.md) and [Claude Code model configuration](https://code.claude.com/docs/en/model-config.md) — context windows and `[1m]` applicability.

### Note → applies at

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| *Pane-state capture cues* (below) — the concrete claude-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director at its pre-ping gate — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | claude capture cue |
|---|---|
| `awaiting_user` | A bordered prompt box awaiting a keypress: an `AskUserQuestion` selection list (numbered options with a `❯`-marked choice and an "Other" free-text entry) or a tool/permission approval prompt (`Do you want to proceed?` with `❯ 1. Yes` / `2. No`). A choice is pending; the composer is not at rest. |
| `finished` | The bare composer at rest: an empty `>` input line with its placeholder hint and the idle status line beneath (model + context, the `⏵⏵` mode indicator), with **no** question/approval box above it and **no** running spinner or `esc to interrupt` indicator. |
| `working` | Affirmative active-work evidence: a running spinner, `esc to interrupt`, streaming response, tool execution, or generation indicator. A truncated or ambiguous capture that might hide one of these cues is also `working`. |
| `stall_candidate` | Quiet, non-finished transcript content with no question/approval box, no empty at-rest composer, and no spinner, tool, streaming, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

### Worked resolution

Launch `cafleet monitor <fleet-id>` via the Bash tool with `run_in_background: true`, confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in the task output, then send `monitor live` to the Director.

## codex

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--ask-for-approval never --sandbox workspace-write` |
| `{bg_run}` | a retained Codex-managed execution session created without shell `&` |
| `{bg_stop}` | interrupting or terminating the retained managed execution session |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading each requested skill core, own backend section and role-required references by the absolute paths the spawn prompt provides |
| `{effort_levels}` | `minimal`, `low`, `medium`, `high`, `xhigh` (spawn flag `--effort <level>`, forwarded as `--config=model_reasoning_effort=<level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `gpt-6-astra` |
| `{monitor_model}` | `gpt-5.6-luna` |

### Model catalog

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| gpt-6-astra | Latest frontier tier across code, apps, and research; strongest reviewer | 10.00 | 50.00 |
| gpt-5.6-sol | Most capable GPT-5.6 tier for complex coding and research | 4.00 | 20.00 |
| gpt-5.6-terra | Balanced agentic coding tier for everyday work | 2.00 | 12.00 |
| gpt-5.6-luna | Fast affordable agentic coding tier | 0.20 | 1.20 |

Sources: [OpenAI pricing](https://developers.openai.com/api/docs/pricing) and [Codex model availability](https://learn.chatgpt.com/docs/models.md).

### Note → applies at

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. Ask a concrete, answerable question, not free-form prose. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| Run the monitor command without shell `&` and retain the active managed execution session's returned session ID. Inspect the initial output for `monitor loop started`; if the line is absent and the session remains active, perform one immediate poll. A missing session ID or an early exit is a failed start; if the session is active but unconfirmed after that poll, terminate it. Withhold `monitor live` and report startup failure unless the line was observed. Whenever a broker message reopens a later monitor-member turn, poll the retained session ID once before any other work. If it exited, relaunch with the same bounded confirmation and send `monitor restarted` only after the replacement is confirmed. The monitor member alone owns the session ID and polling; it never sends the ID to the Director or creates a timer or sleep loop. | `{bg_run}` / `{bg_stop}` — `cafleet/roles/monitor.md` § Startup, in order / § Who watches the watcher; `cafleet/reference/supervision.md` § Spawn Protocol / § Idle Semantics |
| *Pane-state capture cues* (below) — the concrete codex-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director at its pre-ping gate — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | codex capture cue |
|---|---|
| `awaiting_user` | A codex confirmation/approval prompt awaiting a keypress — a `[y/n]`-style command-approval or an out-of-sandbox escalation request. Under `--ask-for-approval never --sandbox workspace-write` codex auto-approves in-workspace work, so a visible approval prompt means codex hit something the policy could not clear and is genuinely waiting. |
| `finished` | The empty codex composer at rest — the input prompt with no streaming output above it and no active-turn / "working" indicator. |
| `working` | Affirmative active-work evidence: the active-turn or `working` indicator, streaming model output, a running tool, or generation in progress. A truncated or ambiguous capture that may still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no confirmation prompt, no empty at-rest composer, and no active-turn, tool, streaming, generation, or other work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

### Worked resolution

Launch `cafleet monitor <fleet-id>` without shell `&` as a Codex-managed execution and retain the returned session ID. Inspect the initial output for `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)`; when the line is absent and the session is active, perform one immediate poll of that retained session. Treat a missing session ID or early exit as startup failure. If the execution remains active but unconfirmed after the poll, terminate it. Send `monitor live` to the Director only after observing the startup line; otherwise withhold it and report the failure.

## opencode

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--agent cafleet` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading each requested skill core, own backend section and role-required references by the absolute paths the spawn prompt provides |
| `{effort_levels}` | unsupported — omit `--effort` |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `opencode/glm-5.2` |
| `{monitor_model}` | `opencode/big-pickle` |

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
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |

Source: [OpenCode Zen models and pricing](https://opencode.ai/docs/zen.md).

### Note → applies at

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. The `--agent cafleet` safety floor shows no popup; if a popup ever appears it is a regression to escalate, not a decision point. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| *Pane-state capture cues* (below) — the concrete opencode-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director at its pre-ping gate — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | opencode capture cue |
|---|---|
| `awaiting_user` | An opencode permission/selection popup awaiting a choice. The `--agent cafleet` floor suppresses permission popups, so a visible popup is both the `awaiting_user` signal **and** a regression to escalate (see the decision-surface note above). |
| `finished` | The empty opencode prompt at rest — no streaming response above it and no active generation indicator. |
| `working` | Affirmative active-work evidence: streaming response text, an active generation indicator, a running tool, or any other visible in-progress state. A truncated or ambiguous capture that might still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no popup, no empty at-rest prompt, and no streaming, tool, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

### Worked resolution

Launch `cafleet monitor <fleet-id> &` as a backgrounded `!` shell command, confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in its output, then send `monitor live` to the Director.

## Template

Add a `## <name>` backend with all six subsections below. Fill every value, keep runtime bindings as short noun phrases, and bind each caveat to its token and consuming instruction. Shared selection and supervision policy stays at its linked owner.

### Runtime bindings

Supply all seven tokens: `{decision_surface}` (recorded user-reaction surface; members relay), `{permission_flags}` (exact auto-approval flags), `{bg_run}` and `{bg_stop}` (matching execution primitives), `{pane_title}` (title mechanism or no analog), `{skill_loader}` (supported loader or complete absolute-path reads), and `{effort_levels}` (accepted values/flag or unsupported).

### Role defaults

Set `{reviewer_model}` to the most capable catalog token/alias and `{monitor_model}` to the lightweight monitoring token/alias. Both must belong to the catalog.

### Model catalog

List exact spawn tokens, valid aliases, reviewed most-to-least capability ordering, classes, and standard provider USD/MTok input/output prices. Include relevant context windows and availability constraints, required prefixes and official pricing/availability/configuration sources. Follow the freshness contract and [Director policy](../roles/director.md#model-selection).

### Note → applies at

Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind the pane-cue table to monitor on-wake classification and the Director's capture gate.

### Pane-state capture cues

Provide concrete content-only cues for `awaiting_user`, `finished`, `working` and `stall_candidate`; native `agent_status` is outside this classification. Finished requires an empty at-rest composer without a pending prompt or active work; working includes streaming, generation, tools and ambiguous/truncated captures that may still be active. Stall-candidate means quiet non-finished content without a prompt or active-work cue. Link [the capture gate](supervision.md#the-pre-ping-capture-gate) for both tie-breaks (`awaiting_user` over `finished`, `working` over `stall_candidate`) and quiet confirmation.

### Worked resolution

Give a complete monitor launch using concrete backend execution values, the exact `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` confirmation and `monitor live` handoff. Include any bounded confirmation/failed-start handling that the backend requires.
