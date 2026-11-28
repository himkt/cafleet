# Coding Agents

Use your own backend's Runtime bindings and notes; before a spawn, apply [Resolve your overlay](../SKILL.md#resolve-your-overlay) to the selected backend.

Model data is refreshed at least every 30 days (last refreshed: 2026-11-17). Prices are planning estimates in standard provider USD/MTok; catalogs use reviewed most-to-least capability order. Claude context notes govern model suffixes.

`{permission_flags}` gives the omitted-policy default; `member create --permission-mode MODE` selects another of the backend's [permission choices and mode classes](cli.md#member-create).

## Role requirements

A role is eligible (`yes`) only when evidence establishes every requirement; otherwise `no`, naming each unmet one. A Director needs:

- **Pane delivery**: the bundled profile declares Director-screen coverage on tmux and herdr, and a live Director delivery probe passed on both for the installed native version. The fleet's member runners then type each pending message's preview into the Director's pane only while its composer is verified empty with no dialog open; a preview starts a turn on an idle session and arrives no later than the end of a busy turn.
- **Goal**: native goal lifecycle controls, or a CAFleet extension verified live for lifecycle, persistence and continuation.

An eligible Director whose delivery is held or unavailable reads by manual poll ([Director wake](../roles/director.md#director-wake)).

A Member needs a runner-compatible PTY and a default mode classed `auto` or `manual` whose bundled profile declares its coverage, detects every approval form (a native permission request blocking the member) and answers allow and deny per request. Renderings past the profile's layout limits are answered manually; a backend without a bundled profile launches with `coverage=unknown`.

## claude

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode auto` (default mode `auto`, class `auto`) |
| `{pane_title}` | `claude --name "<member_id> - <member-name>"` sets `#{pane_title}` |
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
| claude-fable-5-2 | fable | Latest Mythos-class frontier; highest capability on every dimension | 1M | 8.00 | 40.00 |
| claude-fable-5-1 | — | Prior Mythos-class generation at a higher price | 1M | 10.00 | 50.00 |
| claude-fable-5 | — | Earlier Mythos-class generation at a higher price | 1M | 10.00 | 50.00 |
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
| `AskUserQuestion` offers up to four options plus a free-text Other. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| A separate evaluator checks the goal against the transcript; native background work defers it. | [Director wake](../roles/director.md#director-wake) |
| A preview typed into the Director pane during a running turn waits for the end of the turn. An open tool approval or `AskUserQuestion` form in the Director pane holds every preview until it closes. | [Director wake](../roles/director.md#director-wake) |
| Native `!` shell probes showed the output followed by a model turn; its trigger and permission check are unestablished. | [Command authorization](../roles/director.md#director-side-dispatch) |
| Default `auto` (class `auto`, as are `dontAsk` and `bypassPermissions`; `manual`: `default`, `manual`, `acceptEdits`, `plan`) runs spawn-allowed broker tools and host allows, and its classifier reviews other calls; route refused calls. `manual`-class forms (`Read file`, `Edit file`, `Create file`, three-choice Bash) and outside-workspace file access wait for a request-bound answer. | `{permission_flags}`; [routing](../roles/member.md#reconsider-then-route); [runtime handling](../roles/director.md#runtime-events) |
| Resolve native folder trust before unattended startup. | `{permission_flags}`; [observed interactions](../roles/director.md#consume-observed-interactions) |
| `Make auto mode your default permission mode?` (user settings pin a non-`auto` `defaultMode`) opens a `question` with `review_question`; inspection lists `decline` (`No, keep …`), and `Yes, set auto mode as my default permission mode` has no answer. In Director auto mode answer `--choice decline`; otherwise relay it. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [observed interactions](../roles/director.md#consume-observed-interactions) |
| `Teach auto mode about your environment?` lists `not-now` (7-day snooze) and `dont-show-again`; `1. Yes` has no answer, and the user answers its wizard, a separate no-choice `question`, natively. In Director auto mode answer `--choice not-now`, asking the user before `dont-show-again`; otherwise relay both. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [observed interactions](../roles/director.md#consume-observed-interactions) |
| Several tool calls in one turn show one form at a time, headed `N of M`. An allow answers only the current form; a deny interrupts the turn and refuses every uncompleted call. | [Runtime handling](../roles/director.md#runtime-events) |
| `Claude has a question for you — it shows once you send or clear what you're typing.` means a draft hides the auto-default dialog: ask the user to clear the draft, so the dialog arrives for `--choice decline`, or send it with Enter (again while the notice stays), after which the dialog returns at the next launch. | [Observed interactions](../roles/director.md#consume-observed-interactions) |
| An AskUserQuestion form (numbered options, `Chat about this`, `Enter to select · … · Esc to cancel` or `Review your answers`) opens a `question` with `review_question` and no listed choice. | [Observed interactions](../roles/director.md#consume-observed-interactions) |
| `The user doesn't want to proceed with this tool use.` is delivery-caused only when an Escape-dismissal preview follows. | [Command execution](../roles/member.md#command-execution) |

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` (2.1.296); `pane delivery` on tmux 3.8 and herdr 0.9.5 (2.1.325, checked 2026-11-23); a Director pane of at least 80 columns keeps a preview at the plain limit intact |
| Member | `yes` | Omitted `auto` automatic review; the bundled Claude profile |

## codex

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--approve-for-me` (default mode `approve-for-me`, class `auto`) |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | absolute-path reads of each requested skill core, own backend section and role-required references |
| `{effort_levels}` | `minimal`, `low`, `medium`, `high`, `xhigh` (spawn flag `--effort <level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `gpt-6.2-astra` |
| `{reviewer_effort}` | `high` |
| `{other_model}` | `gpt-6.2-sol` |
| `{other_effort}` | `medium` |

### Model catalog

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| gpt-6.2-astra | Latest frontier tier across code, apps, and research; strongest reviewer | 8.00 | 40.00 |
| gpt-6-astra | Prior Astra generation at a higher price | 10.00 | 50.00 |
| gpt-6.2-sol | Everyday coding and multi-source work tier | 2.00 | 10.00 |
| gpt-6.1-sol | Prior Sol generation at the same price | 2.00 | 10.00 |
| gpt-6.1-terra | Fast efficient tier for writing, visual understanding, and computer use | 1.00 | 5.00 |
| gpt-6-luna | Fast efficient tier for focused, high-volume tasks | 0.10 | 0.50 |

Prices apply to prompts up to 272K input tokens; longer prompts bill at a
higher long-context rate.

Sources: [OpenAI pricing](https://developers.openai.com/api/docs/pricing) and [Codex model availability](https://learn.chatgpt.com/docs/models.md).

### Note → applies at

| Note | Applies at |
|---|---|
| A preview typed into the Director pane during a running turn joins that turn without interrupting it; previews interleave with native goal continuation turns. | [Director wake](../roles/director.md#director-wake) |
| Native `!` shell shows command, exit code and output; its sandbox boundary and permission check are unestablished. | [Native authorization and continuation](../roles/director.md#director-side-dispatch) |
| Default `approve-for-me` (class `auto`, as is `never`; `on-request` is `manual`): run host-permitted workspace commands; automatic review decides escalations; route a necessary refused command. Explicit `on-request` uses the native form and runtime-event procedure. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| Restart Codex to load the installed `<codex home>/rules/cafleet.rules`, whose allows run outside the sandbox. Run one CAFleet command per invocation; wrappers and variable expansion can affect matching. | `{permission_flags}`; [command execution](../roles/member.md#command-execution) |
| The Director's approval policy permits escalation requests, with `approvals_reviewer` selecting user or automatic review; `never` cannot obtain a new approval. Member `--permission-mode` sets only that member's policy. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy) |
| A Director's tool commands run under the shared `codex app-server` daemon, so `fleet create` needs Director pane identification. | Director launch; [Director pane identification](../roles/director.md#director-pane-identification) |
| Three consecutive automatic denials interrupt the turn; ask a member idle without a report whether refused operations are still needed. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy) |
| Resolve native `Trust this folder?` before unattended startup; worktree trust follows the native repository scope. | `{permission_flags}`; [observed interactions](../roles/director.md#consume-observed-interactions) |

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` (0.162.1); `pane delivery` on tmux 3.8 and herdr 0.9.5 (0.169.0, checked 2026-11-23); a Director pane of at least 80 columns keeps a preview at the plain limit intact |
| Member | `yes` | Omitted `--approve-for-me` automatic review; the bundled Codex profile |

## opencode

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--agent cafleet-ask` (default mode `cafleet-ask`, class `manual`); `--agent cafleet` (class `auto`) under `--permission-mode cafleet` |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | absolute-path reads of each requested skill core, own backend section and role-required references |
| `{effort_levels}` | unsupported — omit `--effort` |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `opencode-go/kimi-k3.1-pro` |
| `{reviewer_effort}` | `—` |
| `{other_model}` | `opencode-go/glm-5.3` |
| `{other_effort}` | `—` |

### Model catalog

The table lists every OpenCode Console model not past its published deprecation date; the `opencode/` prefix is part of the `--model` value. Any other `<provider-id>/<model-id>` value is a manual spawn with explicit `--coding-agent opencode --model` flags on `member create`. `0.00` marks an explicitly free rate; a model free at both rates is a limited-time OpenCode Console offer. A Claude or GPT row gives its OpenCode spawn token; a bare model name selects the native backend under [Director model selection](../roles/director.md#model-selection). [OpenCode Go](#opencode-go) lists the subscription-provider models.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode/claude-fable-5-2 | Latest Mythos-class frontier; highest capability on every dimension | 8.00 | 40.00 |
| opencode/gpt-6.2-astra | Latest frontier tier across code, apps, and research; strongest reviewer | 8.00 | 40.00 |
| opencode/claude-fable-5-1 | Prior Mythos-class generation at a higher price | 10.00 | 50.00 |
| opencode/gpt-6-astra | Prior Astra generation at a higher price | 10.00 | 50.00 |
| opencode/claude-fable-5 | Earlier Mythos-class generation at a higher price | 10.00 | 50.00 |
| opencode/claude-opus-5-5 | Everyday frontier; strong coding, planning, and review | 4.00 | 20.00 |
| opencode/gpt-6.2-sol | Everyday coding and multi-source work tier | 2.00 | 10.00 |
| opencode/gpt-6.1-sol | Prior Sol generation at the same price | 2.00 | 10.00 |
| opencode/gemini-4-argon | Frontier general tier for complex coding and multimodal work | 2.00 | 10.00 |
| opencode/gemini-4-carbon | Frontier general tier for complex coding and multimodal work | 2.00 | 10.00 |
| opencode/claude-opus-5 | Prior frontier generation at a higher price | 5.00 | 25.00 |
| opencode/gpt-6-sol | Prior Sol generation for complex coding and agentic work | 2.00 | 10.00 |
| opencode/muse-wonder-1.0 | Strong general tier for complex coding and reasoning | 4.00 | 20.00 |
| opencode/gpt-5.6-sol | Prior high-capability Sol tier for demanding coding work | 4.00 | 20.00 |
| opencode/claude-opus-4-8 | Earlier Opus tier for demanding coding and review | 5.00 | 25.00 |
| opencode/kimi-k3 | Strong agentic tier for complex coding workflows | 3.00 | 15.00 |
| opencode/grok-4.7 | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/gpt-5.5-pro | Earlier high-compute tier for demanding reasoning | 30.00 | 180.00 |
| opencode/claude-opus-4-7 | Earlier Opus tier for complex engineering work | 5.00 | 25.00 |
| opencode/claude-sonnet-5-5 | Efficient mid tier for routine work | 2.00 | 10.00 |
| opencode/gpt-5.6-terra | Prior balanced tier for coding and reasoning | 2.00 | 12.00 |
| opencode/claude-opus-4-6 | Earlier Opus tier for complex coding tasks | 5.00 | 25.00 |
| opencode/gpt-5.5 | Earlier general tier for coding and agentic work | 5.00 | 30.00 |
| opencode/gpt-5.4-pro | Earlier high-compute tier for deep reasoning | 30.00 | 180.00 |
| opencode/gemini-4-hydrogen | Efficient Gemini 4 tier for coding and multimodal tasks | 1.50 | 7.50 |
| opencode/gemini-3.8-flash | Fast general tier for coding and multimodal tasks | 0.75 | 3.75 |
| opencode/grok-4.6 | Earlier strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/gpt-5.4 | Earlier general tier for coding and tool use | 2.50 | 15.00 |
| opencode/claude-sonnet-5 | Prior mid-tier generation at the same price | 2.00 | 10.00 |
| opencode/glm-5.3 | Strong general coding tier with agentic workflows | 1.40 | 4.40 |
| opencode/qwen3.8-max | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/deepseek-v4-pro | Strong reasoning tier for coding and tool use | 1.74 | 3.48 |
| opencode/claude-opus-4-5 | Earlier Opus tier for broad coding tasks | 5.00 | 25.00 |
| opencode/gemini-3.7-flash | Earlier fast tier for coding and multimodal tasks | 0.75 | 3.75 |
| opencode/grok-4.5 | Earlier general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode/claude-sonnet-4-6 | Earlier balanced tier for coding and tool use | 3.00 | 15.00 |
| opencode/gpt-5.3-codex | Earlier coding tier for agentic engineering | 1.75 | 14.00 |
| opencode/kimi-k2.6 | Earlier agentic tier for coding workflows | 0.95 | 4.00 |
| opencode/muse-spark-1.3 | Balanced general tier for routine coding | 1.25 | 4.25 |
| opencode/gemini-3.6-flash | Earlier fast general and multimodal tier | 0.75 | 3.75 |
| opencode/gpt-5.2 | Earlier general tier for coding and reasoning | 1.75 | 14.00 |
| opencode/glm-5.1 | Earlier general coding tier | 1.40 | 4.40 |
| opencode/claude-sonnet-4-5 | Earlier balanced tier for routine engineering | 3.00 | 15.00 |
| opencode/gemini-3.5-flash | Earlier efficient general and multimodal tier | 1.50 | 9.00 |
| opencode/minimax-m3 | Efficient agentic tier for routine coding workflows | 0.30 | 1.20 |
| opencode/mistral-large-4 | Balanced general tier for routine coding and reasoning | 0.68 | 2.09 |
| opencode/muse-spark-1.2 | Mid-price general coding tier | 1.25 | 4.25 |
| opencode/gpt-5.3-codex-spark | Earlier fast coding tier for bounded tasks | 1.75 | 14.00 |
| opencode/minimax-m2.7 | Earlier efficient agentic coding tier | 0.30 | 1.20 |
| opencode/deepseek-v4.1-flash | Efficient reasoning and coding tier | 0.30 | 1.20 |
| opencode/deepseek-v4.1-flash-free | DeepSeek V4.1 Flash free tier; zero-retention provider | 0.00 | 0.00 |
| opencode/gpt-6-luna | Fast efficient tier for focused, high-volume tasks | 0.10 | 0.50 |
| opencode/claude-haiku-5-5 | Fast lowest-cost tier; routine checks and bounded tasks | 0.10 | 0.50 |
| opencode/muse-ember-1.0 | Fast low-cost Muse tier for routine tasks | 0.10 | 0.50 |
| opencode/gpt-5.6-luna | Prior efficient Luna tier for focused coding tasks | 0.20 | 1.20 |
| opencode/qwen3.6-plus | Efficient general tier for routine coding and reasoning | 0.50 | 3.00 |
| opencode/gpt-5.4-mini | Earlier small tier for focused coding and tool use | 0.75 | 4.50 |
| opencode/qwen3.8-flash | Fast low-cost tier for routine coding tasks | 0.15 | 0.47 |
| opencode/glm-5.3-flash-free | GLM-5.3-Flash free efficient tier; zero-retention provider | 0.00 | 0.00 |
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/glm-5.3-flash | Fast low-cost GLM tier for routine work | 0.15 | 0.50 |
| opencode/gpt-5.1 | Earlier general tier for bounded coding tasks | 1.07 | 8.50 |
| opencode/gpt-5 | Earlier general tier for routine coding tasks | 1.07 | 8.50 |
| opencode/claude-haiku-4-5 | Earlier fast tier for routine checks and small tasks | 1.00 | 5.00 |
| opencode/gemini-3.5-flash-lite | Lightweight tier for bounded multimodal and routine tasks | 0.30 | 2.50 |
| opencode/mimo-v2.6-flash-free | Free efficient tier for routine coding tasks | 0.00 | 0.00 |
| opencode/nemotron-3.5-super-vl-free | Free vision-language tier for multimodal tasks | 0.00 | 0.00 |
| opencode/nemotron-3.5-lightning-free | Free fast tier for focused routine tasks | 0.00 | 0.00 |
| opencode/gpt-5.4-nano | Earlier lightweight tier for simple bounded tasks | 0.20 | 1.25 |
| opencode/gpt-5-nano | Lightweight tier for extraction and routine checks | 0.05 | 0.40 |
| opencode/gpt-6-luna-decision | Specialized tier for typed decisions and scoring | 0.10 | 0.00 |
| opencode/gemini-decision-1 | Specialized tier for typed decisions and scoring | 0.10 | 0.00 |
| opencode/jev-1.13 | Specialized tier for typed decisions and scoring | 0.042 | 0.00 |
| opencode/jev-1.13-free | Free specialized tier for typed decisions and scoring | 0.00 | 0.00 |
| opencode/grok-build-0.1 | Early coding preview; capability unverified | 1.00 | 2.00 |
| opencode/beam-free | Free experimental tier; capability unverified | 0.00 | 0.00 |
| opencode/flint-40b-free | Free 40B experimental tier; capability unverified | 0.00 | 0.00 |
| opencode/longcat-2.5-free | Free preview tier; capability unverified | 0.00 | 0.00 |
| opencode/iceman-free | Free experimental tier; capability unverified | 0.00 | 0.00 |
| opencode/inkling-2-small-free | Free small experimental tier; capability unverified | 0.00 | 0.00 |
| opencode/hy4-flash-free | Free Hy4 Flash tier; capability unverified | 0.00 | 0.00 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |

Source: [OpenCode Console models and pricing](https://opencode.ai/v2/docs/console/models/).

#### OpenCode Go

The table lists every model the OpenCode Go models endpoint returns; the `opencode-go/` prefix is part of the `--model` value. These tokens are available only when the operator's configured providers include an OpenCode Go subscription; the Director checks availability per spawn under [Director model selection](../roles/director.md#model-selection).

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode-go/gemini-4-argon | Frontier general tier for complex coding and multimodal work | 2.00 | 10.00 |
| opencode-go/kimi-k3.1-pro | Strong agentic tier for complex coding workflows | 2.00 | 10.00 |
| opencode-go/kimi-k3 | Strong agentic tier for complex coding workflows | 3.00 | 15.00 |
| opencode-go/grok-4.7 | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode-go/glm-5.3 | Strong general coding tier with agentic workflows | 1.40 | 4.40 |
| opencode-go/qwen4-max | Flagship Qwen tier for coding and agentic work | 1.50 | 4.50 |
| opencode-go/qwen3.8-max | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode-go/deepseek-v4-pro | Strong reasoning tier for coding and tool use | 0.66 | 1.98 |
| opencode-go/mimo-v2.6-pro | Strong general tier for coding and reasoning | 0.435 | 0.87 |
| opencode-go/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode-go/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode-go/hy4 | Efficient general tier for coding workflows | 0.834 | 2.501 |
| opencode-go/muse-spark-1.3-contributor | Balanced general tier for routine coding | 0.10 | 0.20 |
| opencode-go/minimax-m3.1-flash | Efficient agentic tier for routine coding workflows | 0.15 | 0.60 |
| opencode-go/minimax-m3 | Efficient agentic tier for routine coding workflows | 0.30 | 1.20 |
| opencode-go/muse-spark-1.2-contributor | Earlier general tier for routine coding | 0.10 | 0.20 |
| opencode-go/mimo-v2.6-flash | Efficient general tier for routine coding | 0.14 | 0.28 |
| opencode-go/hy3 | Efficient general tier for routine coding workflows | 0.14 | 0.58 |
| opencode-go/longcat-2.0 | Efficient general tier for routine coding | 0.30 | 1.20 |
| opencode-go/deepseek-v4.1-flash | Efficient reasoning and coding tier | 0.15 | 0.60 |
| opencode-go/gpt-6-luna | Fast efficient tier for focused, high-volume tasks | 0.10 | 0.50 |
| opencode-go/claude-haiku-5-5 | Fast lowest-cost tier; routine checks and bounded tasks | 0.10 | 0.50 |
| opencode-go/gpt-5.6-luna | Prior efficient Luna tier for focused coding tasks | 0.20 | 1.20 |
| opencode-go/qwen3.8-flash | Fast low-cost tier for routine coding tasks | 0.15 | 0.47 |
| opencode-go/qwen3.7-plus | Efficient general tier for routine coding and reasoning | 0.40 | 1.60 |
| opencode-go/glm-5.3-flash | Fast low-cost GLM tier for routine work | 0.15 | 0.50 |
| opencode-go/hy4-preview | Preview coding tier; capability unverified | 0.834 | 2.501 |
| opencode-go/step-5-preview | Preview coding tier; capability unverified | 1.00 | 2.70 |
| opencode-go/beam-free | Free experimental tier; capability unverified | 0.00 | 0.00 |
| opencode-go/longcat-2.5-free | Free preview tier; capability unverified | 0.00 | 0.00 |
| opencode-go/iceman-contributor-free | Free experimental tier; capability unverified | 0.00 | 0.00 |
| opencode-go/hy4-flash-free | Free Hy4 Flash tier; capability unverified | 0.00 | 0.00 |

Prices are per-token USD per MTok counted against subscription usage limits. Extra Usage bills usage beyond the limits to the pay-as-you-go balance.

`opencode-go/deepseek-v4-pro` and `opencode-go/deepseek-v4.1-flash` bill at 2× the listed rates Monday–Friday during 01:00–04:00 and 06:00–10:00 UTC. `opencode-go/gemini-4-argon` bills at 2× the listed rates from 2026-12-01.

Sources: [OpenCode Go models endpoint](https://opencode.ai/inference/go/v1/models) and [OpenCode Go usage limits](https://opencode.ai/v2/docs/console/go).

### Note → applies at

| Note | Applies at |
|---|---|
| [Setup](cli.md#setup-and-doctor) installs the plugin `<preset base>/plugins/cafleet.ts`, providing `/cafleet-goal`, `cafleet_goal_status`, `cafleet_goal_progress` and `cafleet_goal_complete`. | setup; [Director launch and goal](#opencode-director-launch-and-goal) |
| Plugin tools need the `cafleet` permission action, allowed by default on a Director and denied by member presets; an operator `ask`/`deny` override needs `{ "action": "cafleet", "resource": "*", "effect": "allow" }`. Each tool takes the literal `director_id` (plus `database_url` when non-default) and otherwise refuses with `cafleet tools run only in a Director session: <stderr>`. | [Director wake](../roles/director.md#director-wake); [member routing](../roles/member.md#reconsider-then-route) |
| An active goal continues through queued `[cafleet goal]` turns; pause, resume, clear and complete drop undelivered continuations, though one queued at a pause may still run. The goal pauses after 3 continuations without `cafleet_goal_progress` or `cafleet_goal_complete`, after 50, or on a user interrupt, queuing `[cafleet goal] paused: …`; `/cafleet-goal resume` resets both counts. Spawning a member queues nothing into the Director. | [Director wake](../roles/director.md#director-wake); [Director launch and goal](#opencode-director-launch-and-goal) |
| OpenCode loads presets from `~/.config/opencode` and `OPENCODE_CONFIG_DIR`, which `member create` forwards when set; same-name agents can merge, so inspect the effective configuration when customizing a preset. | `{permission_flags}`; preset discovery; member spawn |
| Presets allow `cafleet message send *`, `poll *` and `show *`, with any `--database-url <url>` after the subcommand. MCP permission evaluation and compound-shell matching are unverified: keep MCP servers out of loaded configurations and submit one command per invocation. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [command isolation](../SKILL.md#one-shot-command-isolation) |
| Mini receives previews and prompts as submitted user turns; a preview typed during a running turn waits for the end of the turn, and the Director's own agent name in the status line is recognized. Captures may show only the status bar; check session records for input/output and `message show` `completed` for consumption. | [Push notifications](cli.md#push-notifications); [prompt dispatch](cli.md#member-prompt); [Director wake](../roles/director.md#director-wake) |
| Leading-`!` runs a native shell command in `mini`; its permission boundary is unestablished. | [Native dispatch](../roles/director.md#director-side-dispatch) |
| Default `cafleet-ask` asks for every shell command beyond the broker allows; the member waits while the Director answers through the [runtime procedure](../roles/director.md#runtime-events). A `message send --file` naming a `.env` file copies its contents despite `.env` file-tool denials. | `{permission_flags}`; [runtime procedure](../roles/director.md#consume-observed-interactions) |
| Explicit `cafleet` denies every shell command beyond the broker allows without a form; route required denials, and relay any permission popup as an issue. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| A deny submits an empty reason and ends the member's turn with `Step interrupted`. The bundled profile detects the form at widths of at least `W_min` = 36 columns (rules `opencode-shell-permission`, `opencode-shell-permission-compact`, `opencode-reject-reason-compact`); a narrower pane or a clipped command opens no `interaction_opened` and is answered manually, as is any reason. | [Runtime procedure](../roles/director.md#consume-observed-interactions) step 7 |

#### OpenCode Director launch and goal

1. Launch the Director with `opencode mini --standalone` (2.2.0+) from the project directory with the default agent.
2. The operator manages the goal with `/cafleet-goal <objective>` and its `pause`, `resume`, `clear` and `status` subcommands; `cafleet_goal_status` also reports it.
3. Report each step toward an active goal with `cafleet_goal_progress`; finish with `cafleet_goal_complete`.
4. Rerun `cafleet setup --coding-agent opencode` for a missing tool or an executable-record error.

After a restart, the session's next event restores an active goal.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `extension` through the `cafleet` plugin under `opencode mini --standalone` (2.2.0 minimum); `pane delivery` on tmux 3.8 and herdr 0.9.5 (2.3.0, checked 2026-11-23); a Director pane of at least 80 columns keeps a preview at the plain limit intact |
| Member | `yes` | Omitted `cafleet-ask`, whose form the bundled OpenCode profile detects and answers; runner acceptance on herdr only |

## hermes

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the `clarify` tool with one single-select question, whose `user_response` records the choice (members relay) |
| `{permission_flags}` | no permission argv; recorded mode `smart` (default, class `auto`), with the operator's `approvals.mode` governing |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | absolute-path reads of each requested skill core, own backend section and role-required references |
| `{effort_levels}` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` (spawn flag `--effort <level>`) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `—` (omit `--model`; the operator's configured default applies) |
| `{reviewer_effort}` | `—` |
| `{other_model}` | `—` (omit `--model`; the operator's configured default applies) |
| `{other_effort}` | `—` |

### Model catalog

No maintained catalog. `--model VALUE` passes through verbatim as Hermes's `--model VALUE`, provider-prefixed ids included, and Hermes rejects unknown models; CAFleet never emits `--provider`. Omitted, the member uses the default provider and model in the operator's Hermes configuration (`hermes config get model`).

### Note → applies at

| Note | Applies at |
|---|---|
| `clarify` adds a free-text Other and waits up to an hour; only `status` `answered` records a decision. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| A Director runs the TUI client (`hermes chat --tui` or `display.interface: tui`), whose composer and approval form the bundled profile recognizes for previews; members use the classic CLI. A preview typed during a running turn waits for the end of the turn, and continuations queued before `/goal clear` still run. | Director launch; [Director wake](../roles/director.md#director-wake) |
| Default `smart`: the guardian reviews commands matching Hermes's dangerous-command patterns and may open an ESCALATE or, after DENY, an owner-override once/deny prompt for the Director to answer. An unanswered prompt fails closed at `approvals.timeout` (default 300 s) as BLOCKED; route it and hardline, sudo-guard or `approvals.deny` blocks as refusals. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [runtime procedure](../roles/director.md#consume-observed-interactions) |
| CAFleet needs no `approvals.deny` entries; a `cafleet …` glob there also blocks the Director with `BLOCKED: … user-defined deny rule '<glob>'`, and the Director asks the operator to remove it. | `{decision_surface}`; `{permission_flags}`; [Director dispatch](../roles/director.md#director-side-dispatch) |
| Each pane runs the [member create](cli.md#member-create) Hermes argv as a fresh conversation under the forwarded `HERMES_HOME`. | `{permission_flags}`; member spawn |
| One gateway per Hermes home runs every session's terminal tools, so `fleet create` without `--pane` refuses a Director with the `hermes gateway run` shared-server error. | Director launch; [Director pane identification](../roles/director.md#director-pane-identification); member spawn |
| Rerun `cafleet setup --coding-agent hermes` when Hermes rewrites an installed skill. | setup |
| Session work can outlive the pane's client; [member delete](cli.md#member-delete) owns settlement. After its client-already-exited warning, stop leftover work with `hermes sessions list --source tool`, then `hermes chat --cli --resume <session id> --in <cwd>` and `/stop`. | [Member Delete](../roles/director.md#member-delete); [Shutdown](../roles/director.md#shutdown) |
| Leading-`!` runs a native shell command in the classic CLI and the TUI. | [Native dispatch](../roles/director.md#director-side-dispatch) |

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` `/goal` on the TUI client (2026.9.24); `pane delivery` on tmux 3.8 and herdr 0.9.5 (2026.9.24, checked 2026-11-23); a Director pane of at least 80 columns keeps a preview at the plain limit intact; the Director runs the TUI client, whose screen the Director rules read |
| Member | `yes` | Default `smart` approval-prompt coverage (ESCALATE and override); the bundled Hermes profile |

## pi

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the pi extension's `cafleet_ask` tool (literal Director ID, one question, its choices); an `answered` result records the decision (members relay) |
| `{permission_flags}` | `--cafleet-gate cafleet-ask` (default mode `cafleet-ask`, class `manual`); `--cafleet-gate cafleet` (class `auto`) under `--permission-mode cafleet` |
| `{pane_title}` | `pi --name "<member_id> - <member-name>"`; pi titles the terminal `π - <member_id> - <member-name> - <cwd basename>` |
| `{skill_loader}` | pi's skill listing (name, description, `SKILL.md` path); read that `SKILL.md`, your backend section and role-required references with `read` |
| `{effort_levels}` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`, clamped to the model) |

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `—` (omit `--model`; the operator's configured default applies) |
| `{reviewer_effort}` | `—` |
| `{other_model}` | `—` (omit `--model`; the operator's configured default applies) |
| `{other_effort}` | `—` |

### Model catalog

No maintained catalog. `--model VALUE` passes through verbatim as pi's `--model VALUE` (`provider/id`, an exact id or a fuzzy name) without a `:<thinking>` suffix, and pi rejects unknown models; pass the thinking level with `--effort`. Omitted, the member uses `defaultProvider` and `defaultModel` from `<pi base>/settings.json`, where `<pi base>` is `$PI_CODING_AGENT_DIR`, else `~/.pi/agent`.

### Note → applies at

| Note | Applies at |
|---|---|
| The CAFleet pi extension (`<pi base>/extensions/cafleet.ts`) auto-loads in every pi session, providing `cafleet_ask`, `/cafleet-goal` and the goal tools; each takes the literal `director_id`, plus `database_url` for a non-default database, refusing outside a Director with `cafleet tools run only in a Director session: <stderr>`. | `{decision_surface}`; [setup and doctor](cli.md#setup-and-doctor); [Director wake](../roles/director.md#director-wake); [member routing](../roles/member.md#reconsider-then-route) |
| In both modes the member gate runs only `cafleet message send`, `poll` and `show`, each as one simple command with any `--database-url <url>` after the subcommand. A simple command has no unquoted `;`, `&`, `|`, `<`, `>`, `(`, `)` or backtick, no backtick inside double quotes, no unterminated quote or trailing unquoted backslash, and no `$` outside single quotes except before a name (`$NAME`). Explicit `cafleet` mode blocks other bash with `the CAFleet member gate runs only cafleet message send, poll and show; route the command to the Director if it is required`. Both modes block tools other than bash, read, grep, find, ls, edit and write, and block without a form any command holding a control character (C0 except tab, DEL or C1) with `the CAFleet member gate refuses a command holding a control character; route the command to the Director if it is required`. `read`, `edit` and `write` need a non-empty `path`; file-tool refusals end in `route the read …` or `route the change …`. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [Director dispatch](../roles/director.md#director-side-dispatch) |
| Both modes block `edit` and `write` below the directories named (case-insensitive) in `the CAFleet member gate protects .git, .pi, .agents, .claude, .codex and .opencode from edit and write; route the change to the Director if it is required`; project-root files such as `AGENTS.md` stay writable. Paths resolve as in pi's file tools, without following symlinks. A `file:` path is refused with `the CAFleet member gate refuses a file: URL path for <tool>; route the read …` or `route the change …`; pass a plain path. The case-insensitive `.env` rule refuses only file-tool calls naming a `.env` file; searches, `message send --file` and Director-allowed commands still read `.env` contents. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [Director dispatch](../roles/director.md#director-side-dispatch) |
| The gate refuses newlines, so send a one-line body as positional `TEXT` and a multi-line body with `--file PATH` written inside the working directory. | [Send](../SKILL.md#send-unicast); [message send](cli.md#message-send) |
| Default `cafleet-ask`: an unmatched bash command opens the `CAFleet member gate` form, and the member waits for the Director's answer; a deny returns `the operator denied this command` and the run continues. | `{permission_flags}`; [runtime procedure](../roles/director.md#consume-observed-interactions); [native interactions](../roles/member.md#native-interactions) |
| The bundled pi profile detects the gate form at widths of at least `W_min` = 49 columns; its deny presses `j` then Enter, never Escape. A narrower pane, a wrapped command line or a form taller than the pane opens no `interaction_opened`; answer it manually. | [Runtime procedure](../roles/director.md#consume-observed-interactions) |
| Each pane runs the [member create](cli.md#member-create) pi argv as a fresh session under the forwarded `PI_CODING_AGENT_DIR`; an extension that cannot register `--cafleet-gate` exits at startup (`runner_exited`). | `{permission_flags}`; [member create](cli.md#member-create) |
| Leading-`!` runs the command and stores its output without starting a turn; after `member exec`, send an explicit continuation. | [Director dispatch](../roles/director.md#director-side-dispatch); [member exec](cli.md#member-exec) |
| Escape interrupts a running turn and dismisses an open gate form; Enter during work queues a steering message. | [Command execution](../roles/member.md#command-execution); [member prompt](cli.md#member-prompt) |
| A preview typed into the Director pane during a running turn waits for the end of the turn. | [Director wake](../roles/director.md#director-wake) |

#### pi Director launch and goal

1. Launch the Director with plain `pi` from the project directory and register with plain `cafleet fleet create`.
2. The operator manages the goal with `/cafleet-goal <objective>` and its `pause`, `resume`, `clear` and `status` subcommands; `cafleet_goal_status` also reports it.
3. Report each step toward an active goal with `cafleet_goal_progress`; finish with `cafleet_goal_complete`.
4. Rerun `cafleet setup --coding-agent pi` for a missing tool or an executable-record error.

After `pi --continue`, the session's next turn continues an active goal.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `extension` `/cafleet-goal` from the CAFleet pi extension under plain `pi` (1.5.0); `pane delivery` on tmux 3.8 and herdr 0.9.5 (1.5.2, checked 2026-11-23); a Director pane of at least 80 columns keeps a preview at the plain limit intact; pi has no native approval dialog, so the interaction hold is `—` |
| Member | `yes` | Omitted `cafleet-ask`, whose gate form the bundled pi profile detects and answers |
