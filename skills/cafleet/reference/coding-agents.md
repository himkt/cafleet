# Coding Agents

Execute instructions with your own backend's Runtime bindings and bound notes. To spawn, select the backend under [Director model selection](../roles/director.md#model-selection), check its catalog, defaults, eligibility and runtime capabilities, and apply [Resolve your overlay](../SKILL.md#resolve-your-overlay).

Model data is refreshed at least every 30 days (last refreshed: 2026-10-25). Prices are planning estimates in standard provider USD/MTok; catalogs use reviewed most-to-least capability order. Claude context notes govern model suffixes.

`{permission_flags}` describes omitted-policy defaults; explicit `member create --permission-mode MODE` selects one of that backend's [permission choices](cli.md#member-create) while preserving argv/sandbox boundaries. Policy belongs to the run; independently tested detection, composer and answer capabilities follow [runtime handling](../roles/director.md#runtime-events).

## Role requirements

A role verdict is `yes` only when evidence establishes every Eligible-tier requirement; otherwise `no` with the missing evidence/failure. A Director needs two independent requirements, goal and listen; a `no` verdict names each unmet one. Evidence carries forward until re-check.

| Requirement | Tier 1 | Tier 2 | Tier 3 |
|---|---|---|---|
| Listen | Native subscribe (preferred): the backend pushes each `cafleet message listen` record into the session as a turn | Extension subscribe: a CAFleet plugin that `cafleet setup` installs does the same | Built-in compromise (degraded): the model reads the running listener's output or receives batches, spending turns and needing heartbeat-style wake and log reconciliation |
| Goal | Native goal lifecycle controls | A CAFleet extension verified live for the full lifecycle, persistence and continuation | — |

A subscribe listen starts a turn on an idle session, delivers no later than the end of a busy turn without interrupting it, and keeps emission order. Eligible Directors without verified reception follow [receiver recovery](../roles/director.md#receiver-recovery), preserving goal/decision authority.

Members need a runner-compatible PTY, an explicit detection-coverage declaration and a classified default launch mode (`auto` — native automatic review or a fixed allow/deny policy — whenever the backend offers one; otherwise `manual`, native interactive approval) whose bundled profile detects every approval form the mode renders and declares request-bound allow and deny answers, outside the profile's recorded layout limits. An approval form is a native permission request that blocks the member until answered; question, trust, update and startup dialogs are outside it. A recorded layout limit is a profile-documented rendering no request rule matches (narrower than `W_min`, wrapped or clipped operation, uncaptured geometry), answered manually on the native surface. A default mode with no approval form meets the bar without request rules. Approval-prompt coverage counts toward the Member verdict; other bundled support leaves verdicts unchanged, and a failed probe or unselected profile still launches with `coverage=unknown`.

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
| `AskUserQuestion` supports up to four options; its built-in Other path accepts free text. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Receive Director records one at a time through one owned Monitor task using the native recipe below. | [Director wake](../roles/director.md#director-wake); [native reception](#claude-native-director-reception) |
| Native shell probes displayed command/stdout/stderr followed by a model turn; its trigger and a second permission check were not established. The Director authorizes routed native input under its host rules. | [Command authorization](../roles/director.md#director-side-dispatch) |
| Default mode `auto` (class `auto`; also `auto`: explicit `dontAsk`, `bypassPermissions`; `manual`: `default`, `manual`, `acceptEdits`, `plan`) runs spawn-allowed broker tools and matching host allows; its classifier reviews other calls under authoritative native rules, hooks and managed policy. Route refused calls; explicit interactive policies, including the `manual`-mode `Read file`, `Edit file`, `Create file` and three-choice Bash forms, wait for a request-bound Director/user answer or manual input, protected edits keep host boundaries, and outside-workspace file access is a native permission request answered only with its single-request choices. | `{permission_flags}`; [routing](../roles/member.md#reconsider-then-route); [runtime handling](../roles/director.md#runtime-events) |
| When user settings pin a non-`auto` `defaultMode`, Claude Code can show `Make auto mode your default permission mode?` after a member's turn as a `question` `interaction_opened` with `review_question`; inspection lists choice `decline` (`No, keep …`), and `Yes, set auto mode as my default permission mode` has no answer. In Director auto mode answer `--choice decline`; otherwise relay it, and the user picks `decline` or answers natively. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [observed interactions](../roles/director.md#consume-observed-interactions) |
| In `auto`, Claude Code can show `Teach auto mode about your environment?` after a turn; inspection lists `not-now` (7-day snooze) and `dont-show-again`, and `1. Yes` has no answer; ask the user to answer its wizard, a separate no-choice `question`, natively. In Director auto mode answer `--choice not-now` and ask the user before `dont-show-again`; otherwise relay both choices. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [observed interactions](../roles/director.md#consume-observed-interactions) |
| Several tool calls in one turn show one permission form at a time, each heading ending in `N of M`, and each form is its own permission request. An allow answers only the current form, and the next form arrives as a new request; a deny on any form interrupts the turn and refuses every call that has not completed. | [Runtime handling](../roles/director.md#runtime-events) |
| `Claude has a question for you — it shows once you send or clear what you're typing.` is a no-choice `question`: a draft hides the auto-default dialog. Ask the user to clear the draft on the member's surface, so the dialog appears as its own event for `--choice decline`, or to send it with Enter (again while the notice stays), after which the dialog returns at the member's next launch. | [Observed interactions](../roles/director.md#consume-observed-interactions) |
| A delivery-dismissed prompt can show `The user doesn't want to proceed with this tool use.`; only an Escape-dismissal preview establishes that the delivery caused it. | [Command execution](../roles/member.md#command-execution) |

#### Claude native Director reception

Discover Monitor through the host's tool discovery and authorize it separately from Bash broker rules. After registration, start one Monitor with the literal Director ID:

```json
{"command":"cafleet message listen 1 --timeout 1770","description":"CAFleet Director inbox","timeout_ms":1800000}
```

Monitor caps `timeout_ms` at 1,800,000 (30 minutes); the 1,770-second CLI lease closes 30 seconds inside that deadline, so the `closed` `timeout` record and stream completion precede Monitor expiry.

Require `ready`, retain the task ID, pinned member/fleet IDs, receive status and unfinished actions, and keep Monitor active across other work, idle periods and turn endings while the fleet exists. Consume complete stdout records, reconcile known/replayed IDs against the durable inbox, record deferred work, ACK consumption and act within authorization; events during a user question preserve that pending decision.

On `closed` or stream completion, verify task/source-child termination, then renew while the fleet exists, reconciling replay and deduplicating closure/completion by owned task ID. A host timeout before CLI closure and unavailable or denied Monitor reception follow [receiver recovery](../roles/director.md#receiver-recovery); use verified native cancellation with observed child cleanup only to replace a receiver there.

Reception ends with the fleet: after `closed` `fleet_deleted`, verify task/source-child exit and start no replacement. `Bash(run_in_background=true)` completion notifications establish no per-line reception.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` (2.1.296); `listen` `native` `subscribe` through one Monitor task running `cafleet message listen` (2.1.312) |
| Member | `yes` | Omitted `auto` automatic review; the Claude profile from `2.1.302` |

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
| The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting; Director records arrive as queued user turns from the `--codex-thread` forwarder, interleaving with goal continuation turns at turn boundaries. | [Director wake](../roles/director.md#director-wake); [native reception](#codex-native-director-reception) |
| Start the forwarder on the escalated path under the host's approval rules when the sandbox cannot start it or its `codex queue` children (for example bwrap cannot create namespaces) or unshares the PID namespace, hiding the Codex ancestor so the forwarder fails with `--codex-thread requires a codex ancestor process`. | `{permission_flags}`; [native reception](#codex-native-director-reception) |
| Codex 0.165.0 caps a thread's queue at 100 submissions; the forwarder retries a refused record, so a burst arrives complete and in order, and one record refused for 600 s ends it with a `[cafleet listen closed]` turn when the notice gets through. | [Native reception](#codex-native-director-reception); [receiver recovery](../roles/director.md#receiver-recovery) |
| Native shell input displayed command, exit code, duration and output; installed protocol metadata describes unsandboxed dispatch and public CLI guidance current approval/sandbox settings, so the effective TUI boundary and protocol path are not established. The Director authorizes routed native input under its host rules, where harmless command visibility establishes no second member-tool permission check, and inspects output before explicit continuation. | [Native authorization and continuation](../roles/director.md#director-side-dispatch) |
| Default mode `approve-for-me` (class `auto`; explicit `never` is also `auto`, explicit `on-request` is `manual`): run host-permitted workspace commands directly; automatic review decides escalations, and a necessary refused or denied command routes to the Director. Explicit interactive approval uses the original native surface and runtime-event procedure; the workspace sandbox and managed policy remain authoritative. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| `approve-for-me` on Codex 0.163.0 shows no startup dialog. After three consecutive automatic denials Codex interrupts the turn, so the member stops without routing a request; the Director receives one `automatic_refusal_observed` per denial. | `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [automatic review events](../roles/director.md#consume-automatic-review-events) |

#### Codex native Director reception

1. After registration, start the forwarder through native `exec_command`, with the literal Director ID and a short initial yield such as 1,000 ms: `cafleet message listen <director-id> --codex-thread "$CODEX_THREAD_ID"`. Codex sets `CODEX_THREAD_ID` in every exec shell. Omit `--timeout`.
2. Retain the exec `session_id` and require `ready` in the initial output; read the session again only during recovery.
3. Each record arrives as a queued user turn beginning `[cafleet listen record]`, starting a turn when you are idle and waiting for the end of a running turn otherwise. Consume it, reconcile with `cafleet message poll <director-id> --json`, record deferred actions, ACK and act within authorization. A `message_ref` record names a message too large to forward; read it with `cafleet message show <id> --json`.
4. Records interleave with native `/goal` continuation turns at turn boundaries. Reception never starts, resumes, pauses, clears or re-budgets a goal.
5. A forwarded `closed` record or a `[cafleet listen closed]` turn follows [receiver recovery](../roles/director.md#receiver-recovery): read the retained exec session once for the final output, verify the forwarder exited, then start one replacement while the fleet exists. After a forwarded `closed` `fleet_deleted`, verify exit and start no replacement. When even the closed notice could not be forwarded, the per-wake `poll --json` and overdue-reply checks under [Director wake](../roles/director.md#director-wake) surface the gap.

The forwarder's stdout, record forwarding and exit with its pinned Codex session follow [`message listen`](cli.md#message-listen). Preserve the exec `session_id` and unfinished actions across compaction. Reception makes no pane input.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` (0.162.1); `listen` `native` `subscribe` through `codex queue` on the escalated path (0.165.0); the sandboxed path is unverified |
| Member | `yes` | Omitted `--approve-for-me` automatic review; the Codex profile from `0.163.0` |

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

The table lists every OpenCode Console model not past its published deprecation date; the `opencode/` prefix is part of the `--model` value. Any other `<provider-id>/<model-id>` value is a manual spawn with explicit `--coding-agent opencode --model` flags on `member create`. `0.00` marks an explicitly free rate; a model free at both rates is a limited-time OpenCode Console offer. A Claude or GPT row gives its OpenCode spawn token; a bare model name selects the native backend under [Director model selection](../roles/director.md#model-selection). [OpenCode Go](#opencode-go) lists the subscription-provider models.

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

The table lists every model on the OpenCode Go page; the `opencode-go/` prefix is part of the `--model` value. These tokens are available only when the operator's configured providers include an OpenCode Go subscription; the Director checks availability per spawn under [Director model selection](../roles/director.md#model-selection).

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
| An OpenCode Director runs OpenCode 2.2.0 or later as `opencode mini --standalone` from the project directory with the default agent, so plugins load in its private server with the launcher's environment. The shared background service passes plugins no client environment and cannot host a Director. | Director launch; [native reception](#opencode-native-director-reception) |
| `cafleet setup` installs the CAFleet plugin as `<preset base>/plugins/cafleet.ts` with a `cafleet.executable` record naming the `cafleet` it runs. It provides `cafleet_listen`, `/cafleet-goal`, `cafleet_goal_status`, `cafleet_goal_progress` and `cafleet_goal_complete`. | setup; [native reception](#opencode-native-director-reception) |
| Each plugin tool asserts the `cafleet` permission action, which OpenCode's base allow-all rule resolves to allow on a default Director; an operator configuration whose broad `ask` or `deny` rule overrides it needs `{ "action": "cafleet", "resource": "*", "effect": "allow" }`, and both member presets deny `cafleet`. `cafleet_listen` and the goal tools each take the literal Director ID as `director_id`, plus `database_url` for a non-default database, and run `cafleet member show <director_id> --json`; unless it exits 0 with `"kind": "director"`, they refuse with `cafleet tools run only in a Director session: <stderr>`. | [Director wake](../roles/director.md#director-wake); [member routing](../roles/member.md#reconsider-then-route) |
| An active goal continues through queued `[cafleet goal]` continuation turns, each tagged with the goal's generation, which pause, resume, clear and complete bump monotonically per session. The plugin's `context` hook drops every undelivered continuation after the last assistant message whose generation differs from the stored one or whose goal is not `active`, keeping earlier history; live acceptance on 2.2.0 established that pause stops further continuations, not that a continuation already queued at the pause is dropped. | [Director wake](../roles/director.md#director-wake); [native reception](#opencode-native-director-reception) |
| The goal pauses after 3 continuation turns without `cafleet_goal_progress` or `cafleet_goal_complete` or after 50 continuations, each queuing a `[cafleet goal] paused: …` notice, and a user interrupt pauses it. `/cafleet-goal resume` resets both counts, so a resumed goal gets a fresh budget of 50. | [Director wake](../roles/director.md#director-wake); [native reception](#opencode-native-director-reception) |
| After a restart, the session's first event, tool call or command restores an active goal; goal storage is shared by every OpenCode server on the same data directory, while restoration stays scoped to the sessions a server serves, so spawning a member queues nothing into the Director session. The `context` hook and event loop act only on a session holding a goal stored by the operator-typed `/cafleet-goal` command or by a goal tool whose `director_id` verified. | [Director wake](../roles/director.md#director-wake); [native reception](#opencode-native-director-reception) |
| OpenCode loads presets from `~/.config/opencode`, the default preset base, and from `OPENCODE_CONFIG_DIR`, which `member create` forwards into the member's environment when set, so the member loads the preset setup installed and spawn checked. Same-name agent definitions can merge across sources; inspect the effective configuration when customizing a preset. | `{permission_flags}`; preset discovery; member spawn |
| The broad `cafleet *` allow lets members run broker commands; Director-only commands refuse a member's own `--from-member-id` with an error telling the member to send the request to its Director. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| MCP tools' participation in preset permission evaluation and compound-shell permission matching are unverified. Keep loaded configurations free of MCP servers and submit one command per invocation. | `{permission_flags}`; [command isolation](../SKILL.md#one-shot-command-isolation) |
| Each pane runs `opencode mini --standalone --agent <mode>` with `--prompt`, optional model selection and a private server; `<mode>` is `cafleet` or the selected `cafleet-ask`, and spawn requires only that mode's installed preset. | `{permission_flags}`; member spawn |
| Mini receives pushed notifications and direct prompts as submitted user turns. Captures can expose only the status bar; inspect retained session records for omitted input/output and broker poll/ACK for message consumption. | [Push notifications](cli.md#push-notifications); [prompt dispatch](cli.md#member-prompt) |
| Leading-`!` input runs a native shell command in `mini`. Preserve Director authorization and member-tool permissions because the native path's effective boundary and any second permission check remain unestablished. | [Native dispatch](../roles/director.md#director-side-dispatch) |
| The default `cafleet` preset (class `auto`) allows its declared workspace/broker tools and denies unmatched shell commands; route a required denied command. A permission popup under `cafleet` is an issue to capture and relay. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route) |
| The explicit `cafleet-ask` preset (class `manual`) keeps the same allowlist and asks for every other shell command. Its native permission form is a runtime decision: the member waits while the Director answers it through the [runtime procedure](../roles/director.md#runtime-events), then continues from the result OpenCode returns. | `{permission_flags}`; [runtime procedure](../roles/director.md#consume-observed-interactions) |
| On OpenCode a deny submits an empty rejection reason and ends the member's turn with `Step interrupted`; after verifying that nothing ran, continue the member with a Director message, which its runner delivers into the idle composer. The bundled OpenCode profile detects the form at every pane width of at least `W_min` = 36 columns, with the layout coverage below this table. | [Runtime procedure](../roles/director.md#consume-observed-interactions) step 7 |

Shell-permission layout coverage, for the same runtime-procedure step: rule `opencode-shell-permission` covers the wide (77 and above) and split (56–76) layouts and `opencode-shell-permission-compact` the compact layout (36–55), with the same allow and deny stages; the rejection-reason dialog is `opencode-reject-reason-compact`. A narrower pane, where the choices no longer fit on one row, and a command clipped by the form's command box open no `interaction_opened` event and are answered manually on the native surface, as is a reason to give. A wrapped command the box shows in full is detected with `operation_rows` above 1; native fixtures back the layouts at 120, 57, 55 and 36 columns.

#### OpenCode native Director reception

1. Launch the Director with `opencode mini --standalone` from the project directory, with the default agent.
2. After registration, call `cafleet_listen` with the literal Director ID as `director_id`, adding `database_url` when the fleet uses a non-default database. Require `ok` and the `ready` record; a repeat call returns the existing receiver with `existing: true`.
3. Each record arrives as its own turn beginning `[cafleet listen record]`, starting a turn when you are idle and waiting for the end of a running turn otherwise. Consume it, reconcile with `cafleet message poll <director-id> --json`, record deferred actions, ACK and act within authorization.
4. The operator sets, pauses, resumes and clears the goal with `/cafleet-goal <objective>`, `/cafleet-goal pause`, `/cafleet-goal resume` and `/cafleet-goal clear`; `/cafleet-goal status` and `cafleet_goal_status` report it.
5. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`, passing `director_id`, and `database_url` when needed, on every goal tool. Reception itself calls no goal tool.
6. A `[cafleet listen closed]` turn, an `ok: false` result or a missing `cafleet_listen` tool follows [receiver recovery](../roles/director.md#receiver-recovery). A missing tool or an executable-record error is fixed by `cafleet setup --coding-agent opencode`, which `cafleet doctor` names.

After a restart on the same session, re-arm with `cafleet_listen` on the first turn; that turn also restores an active goal. Exiting the client ends the receiver. Reception makes no pane input.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `extension` through the `cafleet` plugin and `listen` `extension` `subscribe` through `cafleet_listen`, both under `opencode mini --standalone` (2.2.0 minimum) |
| Member | `yes` | The omitted `cafleet` preset, accepted on herdr only; the OpenCode profile from `2.1.4` |

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
| `clarify` offers the listed choices plus a free-text Other and waits up to an hour; a result whose `status` is `answered` records the decision, as verified in the classic CLI and the TUI client. | `{decision_surface}`; [user relay](../roles/director.md#user-delegation-protocol) |
| A Hermes Director runs the TUI client (`hermes chat --tui`, or `display.interface: tui`), because turns injected into a classic CLI session run without rendering in its pane; members use the classic CLI, and Director records arrive through the CAFleet plugin's `cafleet_listen` tool (§ Hermes native Director reception). Native `/goal` set, status, pause, resume and clear work in the TUI client, and continuation turns already queued before `/goal clear` still run after it. | Director launch; [Director wake](../roles/director.md#director-wake) |
| `cafleet setup --coding-agent hermes` installs the CAFleet plugin into `<hermes home>/plugins/cafleet/` with an `executable` record naming the `cafleet` it runs, adds `cafleet` to `plugins.enabled` and sets `plugins.entries.cafleet.allow_gateway_injection` to `true`, failing when `plugins.disabled` lists `cafleet`. `cafleet_listen` verifies its literal `director_id` with `cafleet member show <director_id> --json` and otherwise returns `cafleet_listen runs only in a Director session: <stderr>`. | setup; [member routing](../roles/member.md#reconsider-then-route) |
| Default mode `smart` (class `auto`): commands outside Hermes's dangerous-command patterns run unreviewed; the guardian model reviews the rest and may open an ESCALATE prompt or, after a guardian DENY, an owner-override prompt offering once/deny. The member waits while the Director answers allow-once or deny through the runtime procedure; an unanswered prompt fails closed at `approvals.timeout` (default 300 s) with a BLOCKED result; that result and the hardline floors, the sudo guard and `approvals.deny`, which block before review, route like any refusal. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [runtime procedure](../roles/director.md#consume-observed-interactions) |
| CAFleet needs no `approvals.deny` entries: Hermes applies `approvals.deny` to every session, so a `cafleet …` glob there also blocks a Hermes Director's own command with `BLOCKED: … user-defined deny rule '<glob>'`, and the Director then asks the operator, through its decision surface, to remove that glob. | `{decision_surface}`; `{permission_flags}`; [Director dispatch](../roles/director.md#director-side-dispatch) |
| Each pane runs `hermes chat --cli --source tool --in <cwd>` with optional `--model` and `--reasoning`, then `-q <prompt>` as a fresh conversation. `member create` forwards a set `HERMES_HOME`, so the member uses the configuration setup enabled the plugin in and the preflight resolved. | `{permission_flags}`; member spawn |
| One Hermes gateway per Hermes home runs every session's terminal tools, which may carry another pane's multiplexer variables, so a Hermes Director registers with `cafleet fleet create … --pane <pane-id>` (§ Hermes native Director reception), which refuses a Hermes session without `--pane`. Members take identity and database from their prompt literals. | Director launch; member spawn |
| Hermes can rewrite an installed skill during a session. Rerun `cafleet setup --coding-agent hermes` to restore the embedded copies. | setup |
| A Hermes session can keep work running after its pane's client exits; [member delete](cli.md#member-delete) owns settlement, its exit and its warning, and CAFleet never stops a Hermes gateway or touches sessions it does not own. When deletion warns that the client had already exited, that session's in-flight turn runs to completion; to stop its background processes, find the session with `hermes sessions list --source tool`, resume it with `hermes chat --cli --resume <session id> --in <cwd>` and run `/stop`. | [Member Delete](../roles/director.md#member-delete); [Shutdown](../roles/director.md#shutdown) |
| Leading-`!` input runs a native shell command in the classic CLI REPL and the TUI client, where it was verified. The Director authorizes routed native input under its host rules. | [Native dispatch](../roles/director.md#director-side-dispatch) |

#### Hermes native Director reception

1. Run the Director in the TUI client.
2. Read your pane ID with the leading-`!` local shell, which runs in your pane's own environment: `! echo $TMUX_PANE` in tmux or `! echo $HERDR_PANE_ID` in herdr. Register with `cafleet fleet create --name <name> --coding-agent hermes --pane <pane-id> --json`, adding `--mux-server <server>` when the pane is on a server other than the default.
3. Call `cafleet_listen` with the literal Director ID as `director_id`, adding `database_url` when the fleet uses a non-default database. Require `ok: true` and the `ready` record; a repeat call returns the existing receiver with `existing: true`.
4. Each record arrives as its own turn beginning `[cafleet listen record]`; a record that arrives during a busy turn runs after that turn ends, without interrupting it. Consume it, reconcile with `cafleet message poll <director-id> --json`, record deferred actions, ACK and act within authorization.
5. A `[cafleet listen closed]` turn, an `ok: false` result or a missing `cafleet_listen` tool follows [receiver recovery](../roles/director.md#receiver-recovery). A missing tool or an executable-record error is fixed by `cafleet setup --coding-agent hermes`, which `cafleet doctor` names.

`cafleet_listen` starts a receiver only when `plugins.entries.cafleet.allow_gateway_injection` is `true`, otherwise returning `ok: false` naming `cafleet setup --coding-agent hermes`. The receiver runs inside the Hermes gateway without `--timeout` and commits each record before injecting the next, so records keep emission order; a refused injection, such as a full admission queue during a long busy turn, retries every 1 s with later records waiting behind it.

| Receiver end | Session sees |
|---|---|
| Session finalizes, or no client attached for 30 s (a grace that also covers a client reconnecting during a retry) | Nothing |
| An injection raises or exceeds its 10 s deadline, or one record stays refused for 600 s | A `[cafleet listen closed]` turn, when that notice gets through |
| Plugin unload, such as a re-discovery after `hermes plugins install` or `enable` | A `[cafleet listen closed] the plugin unloaded: …` turn; the session stays attached |
| `closed` `fleet_deleted` | Normal end; start no replacement |

Reception makes no pane input and leaves `/goal` authority with the host.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` `/goal` and `listen` `extension` `subscribe` through `cafleet_listen`, both on the TUI client (2026.9.24) |
| Member | `yes` | Default `smart` approval-prompt coverage (ESCALATE and override); the Hermes profile from `2026.9.24` |

## pi

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the `cafleet_ask` tool of the CAFleet pi extension, with the literal Director ID, one question and its choices; an `answered` result records the decision (members relay) |
| `{permission_flags}` | `--cafleet-gate cafleet` (default mode `cafleet`, class `auto`: the CAFleet member gate's deny-by-default bash allowlist and protected `.git`/`.pi`/`.agents` directories, no form); `--cafleet-gate cafleet-ask` (class `manual`: an unmatched bash command opens the gate's approval form) when `--permission-mode cafleet-ask` is selected |
| `{pane_title}` | `pi --name <member-name>`; pi titles the terminal `π - <name> - <cwd basename>` |
| `{skill_loader}` | pi's skill loader: pi lists each installed skill's name, description and `SKILL.md` path; read that `SKILL.md`, its own backend section and role-required references with the `read` tool |
| `{effort_levels}` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`, forwarded as `--thinking <level>`; pi clamps the level to the model) |

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
| The CAFleet pi extension (`<pi base>/extensions/cafleet.ts` with its `cafleet.executable` record) auto-loads in every pi session. It provides `cafleet_listen`, `cafleet_ask`, `/cafleet-goal` and the goal tools, each verifying its literal `director_id` and otherwise refusing with `cafleet tools run only in a Director session: <stderr>`. | `{decision_surface}`; [setup and doctor](cli.md#setup-and-doctor); [Director wake](../roles/director.md#director-wake); [member routing](../roles/member.md#reconsider-then-route) |
| Default `cafleet` mode (class `auto`): the member gate runs each allowlisted simple command. It blocks every other bash command and every tool outside bash, read, grep, find, ls, edit and write, with a reason ending in `route the command to the Director if it is required`; route a required blocked command. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [Director dispatch](../roles/director.md#director-side-dispatch) |
| In both launch modes, `edit` and `write` are blocked below `.git/`, `.pi/` and `.agents/` with `the CAFleet member gate protects .git, .pi and .agents from edit and write; route the change to the Director if it is required`, because Git hooks and project pi resources run outside the allowlist in later sessions. The path check follows no symbolic link. | `{permission_flags}`; [member routing](../roles/member.md#reconsider-then-route); [Director dispatch](../roles/director.md#director-side-dispatch) |
| Explicit `cafleet-ask` (class `manual`): an unmatched bash command opens the `CAFleet member gate` form. The member waits while the Director answers it; a deny returns `the operator denied this command` to the member, which continues its run. | `{permission_flags}`; [runtime procedure](../roles/director.md#consume-observed-interactions); [native interactions](../roles/member.md#native-interactions) |
| The bundled pi profile detects the gate form at every pane width of at least `W_min` = 49 columns, and its deny answer moves the focus with `j` before Enter, never sending Escape. A narrower pane, a command line that wraps or a form taller than the pane opens no `interaction_opened` event and is answered manually on the native surface. | [Runtime procedure](../roles/director.md#consume-observed-interactions) |
| A pane runs `pi --cafleet-gate <mode> --no-approve --tui-mode fullscreen --name <name>`; optional `--model` and `--thinking` follow, then `--` and the prompt, as a fresh session. `member create` forwards a set `PI_CODING_AGENT_DIR`, and a gated member whose extension cannot register `--cafleet-gate` exits at startup (`runner_exited`). | `{permission_flags}`; [member create](cli.md#member-create) |
| Leading-`!` input runs the command and stores its output without starting a turn. After `member exec`, send an explicit continuation message so the member acts on the output. | [Director dispatch](../roles/director.md#director-side-dispatch); [member exec](cli.md#member-exec) |
| Escape interrupts a running pi turn and dismisses an open gate form, and Enter while the member works queues the text as a steering message. | [Command execution](../roles/member.md#command-execution); [member prompt](cli.md#member-prompt) |
| A pi Director runs plain `pi` from the project directory, registers with plain `fleet create`, and replaces its receiver after `/reload`, `/new`, `/resume` or `/fork` when the runtime-replaced notice arrives. | [Spawn Protocol](../roles/director.md#spawn-protocol); [receiver recovery](../roles/director.md#receiver-recovery) |

#### pi native Director reception

1. Launch the Director with plain `pi` from the project directory, so the CAFleet pi extension loads, and register with `cafleet fleet create --name <name> --coding-agent pi --json`.
2. After registration, call `cafleet_listen` with the literal Director ID as `director_id`, adding `database_url` when the fleet uses a non-default database. Require `ok` and the `ready` record; a repeat call returns the existing receiver with `existing: true`.
3. Each record arrives as a `[cafleet listen record]` message: it starts a turn when you are idle and otherwise arrives at the end of the running turn. Consume it, reconcile with `cafleet message poll <director-id> --json`, record deferred actions, ACK and act within authorization.
4. The operator sets, pauses, resumes and clears the goal with `/cafleet-goal <objective>`, `/cafleet-goal pause`, `/cafleet-goal resume` and `/cafleet-goal clear`; `/cafleet-goal status` and `cafleet_goal_status` report it.
5. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`, passing `director_id`, and `database_url` when needed, on every goal tool. Reception itself calls no goal tool.
6. A `[cafleet listen closed]` message, including the runtime-replaced notice after `/reload`, `/new`, `/resume` or `/fork`, an `ok: false` result or a missing tool follows [receiver recovery](../roles/director.md#receiver-recovery). A missing tool or an executable-record error is fixed by `cafleet setup --coding-agent pi`, which `cafleet doctor` names.

After a restart on the same session (`pi --continue`), re-arm with `cafleet_listen` on the first turn; that turn also continues an active goal. Quitting pi ends the receiver. Reception makes no pane input.

### Role eligibility

| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `extension` `/cafleet-goal` and `listen` `extension` `subscribe` through `cafleet_listen`, both from the CAFleet pi extension under plain `pi` (1.5.0); re-arm reception after `/reload`, `/new`, `/resume`, `/fork` or `pi --continue` |
| Member | `yes` | The omitted `cafleet` gate mode, which renders no approval form; the pi profile from `1.5.0` |
