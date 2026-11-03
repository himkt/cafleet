# Coding Agents

Select your own backend's Runtime bindings and bound notes to execute instructions. For spawning, select the backend under [Director model selection](../roles/director.md#model-selection), then check its catalog, defaults, eligibility and runtime capabilities. Apply [Resolve your overlay](../SKILL.md#resolve-your-overlay).

Model data is refreshed at least every 30 days (last refreshed: 2026-10-25). Prices are planning estimates in standard provider USD/MTok; catalogs use reviewed most-to-least capability order. Claude context notes govern model suffixes.

`{permission_flags}` describes omitted-policy defaults; explicit `member create --permission-mode MODE` selects one of that backend's [permission choices](cli.md#member-create) while preserving argv/sandbox boundaries. Policy belongs to the run. Independently tested detection, composer and answer capabilities follow [runtime handling](../roles/director.md#runtime-events).

## Role requirements

A role verdict is `yes` only when evidence establishes every Eligible-tier requirement; otherwise record `no` with the missing evidence/failure. Directors need native goal lifecycle controls and streamed background output while supervision continues, established by a CAFleet stream probe: the candidate starts `cafleet message listen` through its native background facility, keeps working on unrelated tasks, and first observes each of another member's broker records in that listener's output, then consumes it without user input, while the listener is still running, during the work or at the session's next turn boundary, whether the host pushes the records or the model reads the running process's output. A record found through a foreground `poll`, `wait` or `show` invalidates the probe; output that reaches the session only at process exit, such as a completion notification for a finite receive, does not meet the bar; waking an idle session is not required. Members need a runner-compatible PTY, an explicit detection-coverage declaration and a classified default launch mode (`auto` — native automatic review or a fixed allow/deny policy — whenever the backend offers one; otherwise `manual`, native interactive approval) whose bundled profile detects every approval form the mode renders and declares request-bound allow and deny answers, outside the profile's recorded layout limits. An approval form is a native permission request that blocks the member until answered; question, trust, update and startup dialogs are outside it. A recorded layout limit is a profile-documented rendering no request rule matches (narrower than `W_min`, wrapped or clipped operation, uncaptured geometry), answered manually on the native surface. A default mode with no approval form meets the bar without request rules. Approval-prompt coverage counts toward the Member verdict; other bundled support leaves verdicts unchanged, and a failed probe or unselected profile still launches with `coverage=unknown`. Evidence carries forward until re-check; eligible Directors without verified reception follow [receiver recovery](../roles/director.md#receiver-recovery), preserving goal/decision authority.

## claude

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode auto` (default mode `auto`, class `auto`) |
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
| `AskUserQuestion` supports up to four options; its built-in Other path accepts free text. Members relay a concrete question to the Director. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Receive Director records one at a time through one owned Monitor task using the native recipe below. | [Director wake](../roles/director.md#director-wake); [native reception](#claude-native-director-reception) |
| Native shell probes displayed command/stdout/stderr followed by a model turn; its trigger and a second permission check were not established. The Director authorizes routed native input under its host rules. | [Command authorization](../roles/director.md#director-side-dispatch) |
| Default mode `auto` (class `auto`; explicit `dontAsk` and `bypassPermissions` are also `auto`, while `default`, `manual`, `acceptEdits` and `plan` are `manual`) runs spawn-allowed broker tools and matching host allows; its classifier reviews other calls while native rules, hooks and managed policy remain authoritative. Route refused calls; explicit interactive policies wait for a request-bound Director/user answer or manual input, and protected edits retain host boundaries. Outside-workspace file access is a native permission request; CAFleet uses only its single-request choices. | `{permission_flags}`; [routing](../roles/member.md#reconsider-then-route); [runtime handling](../roles/director.md#runtime-events) |
| In `auto`, Claude Code 2.1.302 can show `Teach auto mode about your environment?`; after `3. Don't show again` it does not reappear for that configuration. It arrives as an `interaction_opened` event with classification `question` and has no answer plan: ask the user to answer it on the member's native surface, and send no answer yourself. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [observed interactions](../roles/director.md#consume-observed-interactions) |
| A delivery-dismissed prompt can show `The user doesn't want to proceed with this tool use.`; only an Escape-dismissal preview establishes that the delivery caused it. | [Command execution](../roles/member.md#command-execution) |

#### Claude native Director reception

Discover Monitor through the host's tool discovery and authorize it separately from Bash broker rules. After registration, start one Monitor with the literal Director ID:

```json
{"command":"cafleet message listen 1 --timeout 210","description":"CAFleet Director inbox","timeout_ms":240000}
```

Retain its task ID, pinned member/fleet IDs, receive status and unfinished actions. Require `ready`; consume complete stdout records, reconcile known/replayed IDs against the durable inbox, record deferred work, ACK consumption and act within authorization. Keep Monitor active across other work and turn endings. Events during a user question preserve that pending decision.

On `closed` or stream completion, verify task/source-child termination before replacement. Renew while authorized work remains, reconcile replay and deduplicate closure/completion by owned task ID. A host timeout before CLI closure follows [receiver recovery](../roles/director.md#receiver-recovery).

Retire through verified native cancellation with observed child cleanup, or stop renewal and drain the finite lease. `Bash(run_in_background=true)` completion notifications establish no per-line reception. Unavailable or denied Monitor reception follows [receiver recovery](../roles/director.md#receiver-recovery).

### Role eligibility

| Role | Eligible | Evidence |
|---|---|---|
| Director | `yes` | Goal lifecycle: [Claude Code goal](https://code.claude.com/docs/en/goal), 2.1.296, checked 2026-10-22. Streamed background output: Monitor stream probes, 2.1.301, checked 2026-10-25. |
| Member | `yes` | PTY lifecycle, coverage declaration and omitted `auto` automatic review: live runner acceptance on tmux and herdr (omitted-default launch, auto-approved and auto-refused cases), 2.1.302, checked 2026-10-29. Bundled support: the Claude profile from `2.1.302`. |

## codex

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--approve-for-me` (default mode `approve-for-me`, class `auto`) |
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
| A member sends a concrete question to the Director; the Director relays the operator's answer as a message. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| The objective belongs to the thread with model-callable lifecycle tools and optional token-budget accounting. Retain native session handles and explicitly read the owned receiver while delegated work remains. | [Director wake](../roles/director.md#director-wake); [native reception](#codex-native-director-reception) |
| Native shell input displayed command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the effective TUI boundary and protocol path were not established. The Director authorizes routed native input under its host rules; harmless command visibility establishes no second member-tool permission check. Inspect output before explicit continuation. | [Native authorization and continuation](../roles/director.md#director-side-dispatch) |
| Default mode `approve-for-me` (class `auto`; explicit `never` is also `auto`, explicit `on-request` is `manual`): run host-permitted workspace commands directly; automatic review decides escalations, and a necessary refused or denied command routes to the Director. Explicit interactive approval uses the original native surface and runtime-event procedure; the workspace sandbox and managed policy remain authoritative. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| `approve-for-me` on Codex 0.163.0 shows no startup dialog. After three consecutive automatic denials Codex interrupts the turn, so the member stops without routing a request; the Director receives one `automatic_refusal_observed` per denial. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [automatic review events](../roles/director.md#consume-automatic-review-events) |

#### Codex native Director reception

After registration, start `cafleet message listen <director-id> --timeout 300` through native `exec_command` with the literal ID and a short initial yield, such as 1,000 ms. Retain `session_id` and initial stdout; require `ready`.

While delegated work remains, read that session through `write_stdin` with empty `chars` and bounded yields, such as 10,000 ms. Tool read timeouts leave the receiver running; its CLI lease remains separate. Accumulate incomplete JSONL, consume complete records, reconcile replay/known IDs with `poll --json`, record deferred actions and ACK consumption. Continue independent authorized work and updates between reads.

Keep this loop active with or without a supplied goal. A background process supplies no unsolicited per-line wake after turn end; after required turn endings or tool-blocking user decisions, reconcile on the next user/native activation.

On `closed`/exit, verify termination and follow [receiver recovery](../roles/director.md#receiver-recovery). Renew clean expiry only while authorized work remains; unexpected failures require a fresh health/inbox check. Preserve session/identity, status, partial JSONL and unfinished actions across compaction. Retire by verified native cancellation or drain the finite lease without renewal; verify exit and discard partial final data.

Reception uses stdout/broker reads and makes no pane input.

### Role eligibility

| Role | Eligible | Evidence |
|---|---|---|
| Director | `yes` | Goal lifecycle: [Codex goals](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), 0.162.1, checked 2026-10-22. Streamed background output: retained `exec_command` session stream probes, 0.162.1, checked 2026-10-25. |
| Member | `yes` | PTY lifecycle, coverage declaration and omitted `--approve-for-me` automatic review: live runner acceptance on tmux and herdr (omitted-default launch and auto-refused cases, host-limited cases recorded as limits), 0.163.0, checked 2026-10-29. Bundled support: the Codex profile from `0.163.0`. |

## opencode

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--agent cafleet` (default mode `cafleet`, class `auto`); `--agent cafleet-ask` (class `manual`) when `--permission-mode cafleet-ask` is selected |
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

The table lists every model in the [OpenCode Console](https://opencode.ai/v2/docs/console/models/)
catalog except models past their published deprecation date; the `opencode/`
prefix is part of the `--model` value. Any other `<provider-id>/<model-id>`
value remains a manual spawn with explicit `--coding-agent opencode --model`
flags on `member create`. A price of `0.00` marks an explicitly free rate;
models with both rates free are offered by OpenCode Console for a limited time.
A Claude or GPT row here gives its OpenCode spawn token; a bare model-name
reference selects the native backend under [Director model selection](../roles/director.md#model-selection).
The [OpenCode Go table](#opencode-go) below lists the subscription-provider models.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode/claude-fable-5-1 | Mythos-class frontier; highest capability on every dimension | 10.00 | 50.00 |
| opencode/gpt-6-astra | Latest frontier tier across code, apps, and research; strongest reviewer | 10.00 | 50.00 |
| opencode/claude-fable-5 | Prior Mythos-class generation at the same price tier | 10.00 | 50.00 |
| opencode/claude-opus-5-5 | Everyday frontier; strong coding, planning, and review | 4.00 | 20.00 |
| opencode/gpt-6.1-sol | Complex coding and agentic workflows tier for everyday work | 2.00 | 10.00 |
| opencode/gemini-4-argon | Frontier general tier for complex coding and multimodal work | 2.00 | 10.00 |
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
| opencode/gemini-3.8-flash | Fast general tier for coding and multimodal tasks | 1.50 | 7.50 |
| opencode/grok-4.6 | Earlier strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/gpt-5.4 | Earlier general tier for coding and tool use | 2.50 | 15.00 |
| opencode/claude-sonnet-5 | Prior mid-tier generation at the same price | 2.00 | 10.00 |
| opencode/glm-5.3 | Strong general coding tier with agentic workflows | 1.40 | 4.40 |
| opencode/qwen3.8-max | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/deepseek-v4-pro | Strong reasoning tier for coding and tool use | 1.74 | 3.48 |
| opencode/claude-opus-4-5 | Earlier Opus tier for broad coding tasks | 5.00 | 25.00 |
| opencode/gemini-3.7-flash | Earlier fast tier for coding and multimodal tasks | 1.50 | 7.50 |
| opencode/grok-4.5 | Earlier general tier for coding and reasoning | 2.00 | 6.00 |
| opencode/gemini-3.1-pro | Earlier general reasoning and multimodal tier | 2.00 | 12.00 |
| opencode/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode/claude-sonnet-4-6 | Earlier balanced tier for coding and tool use | 3.00 | 15.00 |
| opencode/gpt-5.3-codex | Earlier coding tier for agentic engineering | 1.75 | 14.00 |
| opencode/kimi-k2.6 | Earlier agentic tier for coding workflows | 0.95 | 4.00 |
| opencode/muse-spark-1.3 | Balanced general tier for routine coding | 1.25 | 4.25 |
| opencode/gemini-3.6-flash | Earlier fast general and multimodal tier | 1.50 | 7.50 |
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
| opencode/gpt-5.6-luna | Prior efficient Luna tier for focused coding tasks | 0.20 | 1.20 |
| opencode/qwen3.6-plus | Efficient general tier for routine coding and reasoning | 0.50 | 3.00 |
| opencode/gpt-5.4-mini | Earlier small tier for focused coding and tool use | 0.75 | 4.50 |
| opencode/qwen3.8-flash | Fast low-cost tier for routine coding tasks | 0.15 | 0.47 |
| opencode/glm-5.3-flash-free | GLM-5.3-Flash free efficient tier; zero-retention provider | 0.00 | 0.00 |
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/glm-5.3-flash | Fast low-cost GLM tier for routine work | 0.15 | 0.50 |
| opencode/gpt-5.1 | Earlier general tier for bounded coding tasks | 1.07 | 8.50 |
| opencode/gpt-5 | Earlier general tier for routine coding tasks | 1.07 | 8.50 |
| opencode/gemini-3-flash | Earlier efficient general and multimodal tier | 0.50 | 3.00 |
| opencode/claude-haiku-4-5 | Earlier fast tier for routine checks and small tasks | 1.00 | 5.00 |
| opencode/gemini-3.5-flash-lite | Lightweight tier for bounded multimodal and routine tasks | 0.30 | 2.50 |
| opencode/mimo-v2.6-flash-free | Free efficient tier for routine coding tasks | 0.00 | 0.00 |
| opencode/nemotron-3-ultra-free | Free reasoning tier for bounded coding and analysis | 0.00 | 0.00 |
| opencode/nemotron-3.5-super-vl-free | Free vision-language tier for multimodal tasks | 0.00 | 0.00 |
| opencode/nemotron-3.5-lightning-free | Free fast tier for focused routine tasks | 0.00 | 0.00 |
| opencode/gpt-5.4-nano | Earlier lightweight tier for simple bounded tasks | 0.20 | 1.25 |
| opencode/gpt-5-nano | Lightweight tier for extraction and routine checks | 0.05 | 0.40 |
| opencode/jev-1.13 | Specialized tier for typed decisions and scoring | 0.042 | 0.00 |
| opencode/jev-1.13-free | Free specialized tier for typed decisions and scoring | 0.00 | 0.00 |
| opencode/grok-build-0.1 | Early coding preview; capability unverified | 1.00 | 2.00 |
| opencode/beam-free | Free experimental tier; capability unverified | 0.00 | 0.00 |
| opencode/flint-40b-free | Free 40B experimental tier; capability unverified | 0.00 | 0.00 |
| opencode/longcat-2.5-preview-free | Free preview tier; capability unverified | 0.00 | 0.00 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |

Source: [OpenCode Console models and pricing](https://opencode.ai/v2/docs/console/models/).

#### OpenCode Go

The table lists every model on the [OpenCode Go](https://opencode.ai/v2/docs/console/go) page; the `opencode-go/` prefix is part of the `--model` value. These tokens are available only when the operator's configured providers include an OpenCode Go subscription; the Director checks availability per spawn under [Director model selection](../roles/director.md#model-selection).

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode-go/gemini-4-argon | Frontier general tier for complex coding and multimodal work | 2.00 | 10.00 |
| opencode-go/kimi-k3.1-pro | Strong agentic tier for complex coding workflows | 2.00 | 10.00 |
| opencode-go/kimi-k3 | Strong agentic tier for complex coding workflows | 3.00 | 15.00 |
| opencode-go/grok-4.7 | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode-go/glm-5.3 | Strong general coding tier with agentic workflows | 1.40 | 4.40 |
| opencode-go/qwen3.8-max | Strong general tier for coding and reasoning | 2.00 | 6.00 |
| opencode-go/deepseek-v4-pro | Strong reasoning tier for coding and tool use | 1.32 | 3.96 |
| opencode-go/mimo-v2.6-pro | Strong general tier for coding and reasoning | 0.435 | 0.87 |
| opencode-go/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode-go/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode-go/mimo-v2.5-pro | Prior MiMo Pro tier for coding and reasoning | 0.435 | 0.87 |
| opencode-go/muse-spark-1.3-contributor | Balanced general tier for routine coding | 0.10 | 0.20 |
| opencode-go/minimax-m3.1-flash | Efficient agentic tier for routine coding workflows | 0.15 | 0.60 |
| opencode-go/minimax-m3 | Efficient agentic tier for routine coding workflows | 0.30 | 1.20 |
| opencode-go/muse-spark-1.2-contributor | Earlier general tier for routine coding | 0.10 | 0.20 |
| opencode-go/mimo-v2.6-flash | Efficient general tier for routine coding | 0.14 | 0.28 |
| opencode-go/mimo-v2.5 | Prior MiMo tier for routine coding | 0.14 | 0.28 |
| opencode-go/hy3 | Efficient general tier for routine coding workflows | 0.14 | 0.58 |
| opencode-go/longcat-2.0 | Efficient general tier for routine coding | 0.30 | 1.20 |
| opencode-go/deepseek-v4.1-flash | Efficient reasoning and coding tier | 0.30 | 1.20 |
| opencode-go/gpt-6-luna | Fast efficient tier for focused, high-volume tasks | 0.10 | 0.50 |
| opencode-go/claude-haiku-5-5 | Fast lowest-cost tier; routine checks and bounded tasks | 0.10 | 0.50 |
| opencode-go/gpt-5.6-luna | Prior efficient Luna tier for focused coding tasks | 0.20 | 1.20 |
| opencode-go/qwen3.8-flash | Fast low-cost tier for routine coding tasks | 0.15 | 0.47 |
| opencode-go/qwen3.7-plus | Efficient general tier for routine coding and reasoning | 0.40 | 1.60 |
| opencode-go/glm-5.3-flash | Fast low-cost GLM tier for routine work | 0.15 | 0.50 |
| opencode-go/hy4-preview | Preview coding tier; capability unverified | 0.834 | 2.501 |
| opencode-go/step-5-preview | Preview coding tier; capability unverified | 1.00 | 2.70 |
| opencode-go/beam-free | Free experimental tier; capability unverified | 0.00 | 0.00 |
| opencode-go/longcat-2.5-preview-free | Free preview tier; capability unverified | 0.00 | 0.00 |

Prices are per-token USD per MTok counted against subscription usage limits.

Source: [OpenCode Go](https://opencode.ai/v2/docs/console/go).

### Note → applies at

| Note | Applies at |
|---|---|
| A member routes concrete questions to the Director and receives the operator's answer as a message. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| Members perform assignments using the selected provider/model, workspace reads/edits, permitted tools and broker communication. OpenCode's Director verdict is `no` (Role eligibility below), so an OpenCode session follows the Director role's ineligible-backend rule. | [Director wake](../roles/director.md#director-wake) |
| OpenCode loads presets from `~/.config/opencode`, the default preset base, and from `OPENCODE_CONFIG_DIR`. `member create` forwards a set `OPENCODE_CONFIG_DIR` into the member's environment, so the member loads the preset setup installed and spawn checked. Same-name agent definitions can merge across sources; inspect the effective configuration when customizing a preset. | `{permission_flags}`; preset discovery; member spawn |
| The broad `cafleet *` allow lets members run broker commands; Director-only commands refuse inside the member through the runner guard, whose error tells the member to send the request to its Director. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| MCP tools' participation in preset permission evaluation and compound-shell permission matching are unverified. Keep loaded configurations free of MCP servers and submit one command per invocation. | `{permission_flags}`; [command isolation](../SKILL.md#one-shot-command-isolation) |
| Each pane runs `opencode mini --standalone --agent <mode>` with `--prompt`, optional model selection and a private server; `<mode>` is `cafleet` or the selected `cafleet-ask`, and spawn requires only that mode's installed preset. | `{permission_flags}`; member spawn |
| Mini receives pushed notifications and direct prompts as submitted user turns. Captures can expose only the status bar; inspect retained session records for omitted input/output and broker poll/ACK for message consumption. | [Push notifications](cli.md#push-notifications); [prompt dispatch](cli.md#pane-input-and-previews) |
| Leading-`!` input runs a native shell command in `mini`. Preserve Director authorization and member-tool permissions because the native path's effective boundary and any second permission check remain unestablished. | [Native dispatch](../roles/director.md#director-side-dispatch) |
| The default `cafleet` preset (class `auto`) allows its declared workspace/broker tools and denies unmatched shell commands; route a required denied command. A permission popup under `cafleet` is an issue to capture and relay. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| The explicit `cafleet-ask` preset (class `manual`) keeps the same allowlist and asks for every other shell command. Its native permission form is a runtime decision: the member waits while the Director answers it through the [runtime procedure](../roles/director.md#runtime-events), then continues from the result OpenCode returns. | `{permission_flags}`; [runtime procedure](../roles/director.md#consume-observed-interactions) |
| On OpenCode a deny submits an empty rejection reason and ends the member's turn with `Step interrupted`; after verifying that nothing ran, continue the member with a Director message, which its runner delivers into the idle composer. The bundled OpenCode profile detects the form at every pane width of at least `W_min` = 36 columns: rule `opencode-shell-permission` covers the wide (77 and above) and split (56–76) layouts, and `opencode-shell-permission-compact` covers the compact layout (36–55), with the same allow and deny stages; its rejection-reason dialog is `opencode-reject-reason-compact`. A narrower pane, where the choices no longer fit on one row, and a command clipped by the form's command box open no `interaction_opened` event and are answered manually on the native surface, as is a reason to give. A wrapped command that the box shows in full is detected, with `operation_rows` above 1; native fixtures back the layouts at 120, 57, 55 and 36 columns. | [Runtime procedure](../roles/director.md#consume-observed-interactions) step 7 |

### Role eligibility

| Role | Eligible | Evidence |
|---|---|---|
| Director | `no` | Goal lifecycle controls: not established for 2.1.4, with no goal evidence recorded, checked 2026-10-31. Streamed background output: observed failure, CAFleet stream probes of `shell` `background=true` (records only at process exit), 2.1.4, checked 2026-11-03. |
| Member | `yes` | PTY lifecycle, coverage declaration and the omitted `cafleet` preset, which resolves every shell check to allow or deny: live runner acceptance on herdr only (`coverage=tested`, no native form, native denial itself not exercised), 2.1.4, checked 2026-10-29. Bundled support: the OpenCode profile from `2.1.4`. |

## hermes

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the `clarify` tool with one single-select question; its result records the chosen `user_response` (members relay) |
| `{permission_flags}` | no permission argv; the recorded mode is `smart` (default, class `auto`) or `manual` (class `manual`), and the operator configuration's `approvals.mode` governs the session, with hardline floors, the sudo guard and `approvals.deny` enforced |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading each requested skill core, own backend section and role-required references by the absolute paths the spawn prompt provides |
| `{effort_levels}` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` (spawn flag `--effort <level>`, forwarded as `--reasoning <level>`) |

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
| `clarify` offers the listed choices plus a free-text Other and waits up to an hour; a result whose `status` is `answered` records the decision. A member routes concrete questions to the Director and receives the operator's answer as a message. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| A Hermes Director reads its background `listen` receiver through `process_manage` `action=log` between work steps (§ Hermes native Director reception); records reach the session when the Director reads, and an idle session is woken only by the receiver's exit notification. | [Director wake](../roles/director.md#director-wake) |
| Default mode `smart` (class `auto`): commands outside Hermes's dangerous-command patterns run unreviewed; the guardian model reviews the rest and may open an ESCALATE prompt or, after a guardian DENY, an owner-override prompt offering once/deny. The member waits while the Director answers allow-once or deny through the runtime procedure; an unanswered prompt fails closed at `approvals.timeout` (default 300 s) with a BLOCKED result, which the member routes like any refusal. Hardline floors, the sudo guard and `approvals.deny` block before review and route like any refusal. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [runtime procedure](../roles/director.md#consume-observed-interactions) |
| CAFleet needs no `approvals.deny` entries. Hermes applies `approvals.deny` to every session, so a `cafleet …` glob there also blocks a Hermes Director's own command with `BLOCKED: … user-defined deny rule '<glob>'`; the Director then asks the operator, through its decision surface, to remove that CAFleet glob from `approvals.deny`. | `{decision_surface}`; `{permission_flags}`; [Director dispatch](../roles/director.md#director-side-dispatch) |
| Each pane runs `hermes chat --cli --source tool --in <cwd>` with optional `--model` and `--reasoning`, then `-q <prompt>` as a fresh conversation. `member create` forwards a set `HERMES_HOME`, so the member uses the configuration setup merged into and the preflight checked. | `{permission_flags}`; member spawn |
| One Hermes gateway per Hermes home runs every session's terminal tools. Setup merges CAFleet's member variables into `terminal.client_env_passthrough`, so each member's tools see its own `CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID` and pane variables; `member create` refuses a configuration whose list lacks one of them, naming `cafleet setup --coding-agent hermes`. | setup; member spawn |
| Background terminal commands run through a login shell that rebuilds `PATH`; invoke `cafleet` by its absolute path in a background command. | Member startup; [native reception](../roles/director.md#director-wake) |
| Hermes can rewrite an installed skill during a session. Rerun `cafleet setup --coding-agent hermes` to restore the embedded copies. | setup |
| A Hermes session can keep work running after its pane's client exits; [member delete](cli.md#member-delete) owns settlement, its exit and its warning. When deletion warns that the client had already exited, that session's in-flight turn runs to completion; to stop its background processes, find the session with `hermes sessions list --source tool`, resume it with `hermes chat --cli --resume <session id> --in <cwd>` and run `/stop`. CAFleet never stops a Hermes gateway or touches sessions it does not own. | [Member Delete](../roles/director.md#member-delete); [Shutdown](../roles/director.md#shutdown) |
| Leading-`!` input runs a native shell command in the classic REPL. The Director authorizes routed native input under its host rules. | [Native dispatch](../roles/director.md#director-side-dispatch) |

#### Hermes native Director reception

After registration, start one receiver with the `terminal` tool, `background=true` and `notify=true`, running `<absolute cafleet path> message listen <director-id> --timeout 300` with the literal ID and the `cafleet` path your own shell resolves. Retain the returned process `session_id`, the Director ID, the output position already read and unfinished actions.

While delegated work remains, read the running receiver between work steps with `process_manage` `action=log` with `offset` advanced past output already read on the retained `session_id`. Accumulate incomplete JSONL, consume complete records, reconcile replayed and known IDs with `cafleet message poll <director-id> --json`, record deferred actions and ACK consumption. Continue independent authorized work between reads.

The receiver's exit notification follows its `closed` record or exit: verify termination and follow [receiver recovery](../roles/director.md#receiver-recovery), renewing while delegated work remains. Retire the receiver with `process_manage`, `action=kill` on the retained `session_id`, and verify that it exited. Preserve the process `session_id`, output position, partial JSONL and unfinished actions across compaction. When the background facility is unavailable or denied, follow [receiver recovery](../roles/director.md#receiver-recovery).

Reception uses process output and broker reads, makes no pane input and leaves `/goal` `gate`/`wait` authority with the host.

### Role eligibility

| Role | Eligible | Evidence |
|---|---|---|
| Director | `yes` | Goal lifecycle: native `/goal` lifecycle controls in installed source and in-session `/help goal`, 2026.9.24, checked 2026-11-02. Streamed background output: CAFleet stream probe with `terminal` `background=true` read through `process_manage` `action=log`, 2026.9.24, checked 2026-11-03. |
| Member | `yes` | PTY lifecycle, coverage declaration and default `smart` approval-prompt coverage (ESCALATE and override, allow-once and deny): live runner acceptance on tmux and herdr, 2026.9.24, checked 2026-11-02. Bundled support: the Hermes profile from `2026.9.24`. |
