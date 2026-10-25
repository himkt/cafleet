# Coding Agents

Each backend section is self-contained. Select the backend by the subject of the action, then use its tables and bound notes:

| Purpose | Backend selector | Read and apply |
|---|---|---|
| Execute current instructions | The executing agent's `CODING AGENT:` identity, or its own identity when standalone | Its Runtime bindings and corresponding notes for decision surface, skill loader, and local execution tools. Required-reading row one gates this lookup; an ordinary member needs only its own runtime section. |
| Select/configure a spawned member | The backend selected under [Director model-selection policy](../roles/director.md#model-selection) | Its Model catalog and Role defaults; validate target effort and launch capabilities against its Runtime bindings. |

Resolve the five runtime placeholders from Runtime bindings for the relevant operation and model/effort placeholders from the selected backend’s Role defaults. Apply notes at their named instructions and emit concrete command values using the core skill's [Resolve your overlay](../SKILL.md#resolve-your-overlay) procedure and its documented neutral defaults for explicitly allowed missing/unknown-backend cases. Report a missing required backend section, malformed table, or broken supported-backend reference as a documentation defect.

For example, a Codex Director selecting an OpenCode reviewer uses the OpenCode catalog and reviewer defaults with the Codex decision surface and local execution tools. User overrides and selection decisions follow [Director model-selection policy](../roles/director.md#model-selection).

Model catalogs, their provenance/context notes, canonical Role defaults, and shared freshness metadata are maintained exclusively by the `cafleet-model-list-refresh` skill from the official sources in each backend section, refreshed at least every 30 days (last refreshed: 2026-10-25). Runtime bindings, bound runtime notes, and worked resolutions belong to runtime documentation maintenance. The freshness date applies to model data; structural reorganization preserves that date.

Prices are standard provider USD rates per MTok and are planning estimates, not an invoice guarantee. Each backend's catalog is ordered most → least capable as reviewed judgment. Context windows are listed for the `claude` backend, whose model strings are the ones a context-window suffix can apply to. Selection policy, including cost efficiency mode and reviewer/member rules, lives in [Director model selection](../roles/director.md#model-selection).

## claude

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode auto` |
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
| A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Per-record reception follows the native recipe below. | [Director wake](supervision.md#director-wake) |
| Receive Director records through one owned Monitor task using the native recipe below. Preserve the supplied `/goal` state and pending user decisions across events and renewal. | [Director wake](supervision.md#director-wake); [native reception](#claude-native-director-reception) |
| Native shell probes displayed command/stdout/stderr followed by a model turn; its trigger and a second permission check were not established. The Director authorizes routed native input under its host rules. | [Command authorization](prompt-routing.md#director-side-dispatch) |
| Members execute task commands in native `auto` mode and route observed denials with command/cwd/environment/refusal evidence. Native deny/ask rules, hooks and managed policy retain precedence; unavailable auto can start in Manual, and unresolved prompts require Director attention. Preserve the actual request and authority rather than inferring automatic success. | `{permission_flags}`; [routing](prompt-routing.md#member-side-observed-denial) |

Evidence: Claude Code 2.1.296, checked 2026-10-22; [host continuation](https://code.claude.com/docs/en/goal) and [block cap](https://code.claude.com/docs/en/hooks-guide#stop-hook-hits-the-block-cap). The launch selects [auto permission mode](https://code.claude.com/docs/en/permission-modes); mode selection alone establishes no effective live availability. Local probes establish only exercised behavior.

#### Claude native Director reception

Discover Monitor through the installed host's tool-discovery mechanism and verify its authorization separately from Bash broker rules. After Director registration, start one Monitor with the actual literal Director ID. The tested invocation shape is:

```json
{"command":"cafleet message listen 1 --timeout 210","description":"CAFleet Director inbox","timeout_ms":240000}
```

Retain the task ID, pinned member/fleet IDs, receive status and unfinished actions in host-owned state. Require the `ready` record. Each stdout event supplies a protocol record and may start a new model turn before the process exits; consume complete records, reconcile the durable inbox and known/replayed IDs, record deferred work, ACK consumption and act within authorization. Keep Monitor active while returning to other work or ending the turn. An event during an open user question supplies no answer or authority to proceed past that decision.

The finite CLI lease normally ends before the tested Monitor lifetime. On `closed` or native stream-ended/completion notification, establish the old task and source process exited. Renew once while authorized work remains, then reconcile replay. Deduplicate closure and completion notices by the owned task ID, ignoring stale completion notices after ownership moves. A host timeout before CLI closure follows [receiver recovery](supervision.md#receiver-recovery); establish source-child termination before replacement and retain one owned source process.

Use verified native cancellation with observed child cleanup, or stop renewal and drain the finite lease. Preserve the supplied `/goal` activation, pause, completion and budget controls; receiving, ACKing and renewal use no goal lifecycle calls. Ordinary `Bash(run_in_background=true)` provides a different capability: observed completion notification did not establish per-line reception while the process lived. Where Monitor is unavailable or denied, disclose the limitation and use manual `poll`/finite `wait`.

Evidence: Claude Code 2.1.301 stream probes established separate event turns from one process. Actual Monitor expiry, cancellation, child cleanup and independently observed goal/decision preservation require acceptance evidence; the invocation shape alone establishes none of them.

## codex

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--approve-for-me --sandbox workspace-write` |
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
| The objective belongs to the thread with model-callable lifecycle tools and optional token-budget accounting. Retain native session handles and explicitly read the owned receiver while delegated work remains, preserving goal activation, pause, completion and budget authority. | [Director wake](supervision.md#director-wake); [native reception](#codex-native-director-reception) |
| Native shell input displayed command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the effective TUI boundary and protocol path were not established. | [Native authorization](prompt-routing.md#director-side-dispatch) |
| The Director authorizes routed native input under its host rules; harmless command visibility establishes no second member-tool permission check. Inspect output before explicit continuation. | [Dispatch and continuation](prompt-routing.md#director-side-dispatch) |
| Members execute task commands with automatic approval review and an explicit `workspace-write` sandbox. Preserve sandbox restrictions and managed policy; route observed refusals with command/cwd/environment/refusal evidence and surface unavailable review or unresolved approval. Native file tools remain within role scope. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-observed-denial) |

Evidence: Codex CLI 0.162.1, checked 2026-10-22; [host continuation](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex) and [command reference](https://learn.chatgpt.com/docs/developer-commands). Installed CLI help maps `--approve-for-me` to automatic approval review with workspace protection; [auto-review](https://learn.chatgpt.com/docs/sandboxing/auto-review) availability depends on the host policy. Launch arguments alone establish no live review result.

#### Codex native Director reception

After Director registration, start `cafleet message listen <director-id> --timeout 300` once through native `exec_command`, using the actual literal ID and a short initial yield such as 1,000 ms. Retain its live `session_id` and initial stdout; require `ready` before relying on reception.

While delegated work remains, call `write_stdin` on that same session with empty `chars` and a bounded yield such as 10,000 ms. A tool read/yield timeout leaves the receiver running and differs from its 300-second CLI lease. Accumulate incomplete JSONL across chunks, consume every complete record, reconcile known/replayed IDs and `poll --json`, record deferred actions, ACK consumption and dispatch ready work. Continue independent authorized work and user-facing updates between bounded reads, then read the same session again.

Keep the tool-bearing receive loop active independently of whether a host goal was supplied. A background process alone supplies no unsolicited per-line wake after the model turn ends. When the host requires ending the turn or blocks tools for a required user decision, wait for the next user/native activation and reconcile the durable inbox then; fully idle Codex wake is outside this adapter.

On `closed`/exit, establish termination and follow [receiver recovery](supervision.md#receiver-recovery). Renew a clean expired lease only while authorized delegated work remains; unexpected failures require a fresh successful health/inbox check first. Retain the session ID, pinned member/fleet IDs, receive status, incomplete JSONL and unfinished actions across context compaction. When normal retirement lacks a verified native cancel primitive, stop renewal and drain the finite lease to observed exit. Verified native cancellation may retire it sooner; confirm termination and discard partial final data.

Reception, ACK and renewal preserve the supplied goal's activation, pause, completion and budget authority. They never call a goal lifecycle tool, start a goal, resume a paused goal or change its budget. Required user decisions gate dependent work while independent authorized work may continue. Automatic Director reception uses stdout and broker reads only, with no pane controls or queue bridge.

Evidence: Codex CLI 0.162.1 stream probes established one retained session/process across receive–act–receive. Independently observed goal-state and open-decision preservation require acceptance evidence; absence of goal lifecycle calls alone proves no state preservation.

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

The table uses base (shortest-prompt) rates. Long-context rates below are input/output USD per MTok.

| Model | Prompt length | Input $/MTok | Output $/MTok |
|---|---|---|---|
| `opencode/gpt-6-astra` | > 272K tokens | 20.00 | 75.00 |
| `opencode/gpt-6.1-sol` | > 272K tokens | 4.00 | 15.00 |
| `opencode/gpt-6-sol` | > 272K tokens | 4.00 | 15.00 |
| `opencode/gpt-5.6-sol` | > 272K tokens | 8.00 | 30.00 |
| `opencode/grok-4.7` | > 200K tokens | 4.00 | 12.00 |
| `opencode/gpt-5.6-terra` | > 272K tokens | 4.00 | 18.00 |
| `opencode/gpt-5.5` | > 272K tokens | 10.00 | 45.00 |
| `opencode/grok-4.6` | > 200K tokens | 4.00 | 12.00 |
| `opencode/gpt-5.4` | > 272K tokens | 5.00 | 22.50 |
| `opencode/grok-4.5` | > 200K tokens | 4.00 | 12.00 |
| `opencode/gemini-3.1-pro` | > 200K tokens | 4.00 | 18.00 |
| `opencode/claude-sonnet-4-5` | > 200K tokens | 6.00 | 22.50 |
| `opencode/gpt-6-luna` | > 272K tokens | 0.20 | 0.75 |
| `opencode/claude-haiku-5-5` | > 100K tokens | 0.50 | 2.50 |
| `opencode/gpt-5.6-luna` | > 272K tokens | 0.40 | 1.80 |

Promotional rows use the currently billed rates. List prices below are input/output USD per MTok.

| Model | Discount | List input $/MTok | List output $/MTok | Discount end |
|---|---|---|---|---|
| `opencode/gemini-4-argon` | 50% off | 4.00 | 20.00 | 2026-11-30 |
| `opencode/mistral-large-4` | 50% off | 1.36 | 4.18 | Not published |

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

Prices are per-token amounts in USD per MTok counted against the subscription's usage limits. Plans are Go ($10/month) and Go Plus ($40/month), for single-member workspaces. Each model's usage is capped at 20% of its monthly limit per 5 hours, 50% per week and 100% per month; with Extra Usage enabled, usage beyond the limits bills the pay-as-you-go balance.

| Model | Go monthly limit $ | Go Plus monthly limit $ |
|---|---|---|
| `opencode-go/gemini-4-argon` | 30 | 120 |
| `opencode-go/kimi-k3.1-pro` | 15 | 60 |
| `opencode-go/kimi-k3` | 15 | 60 |
| `opencode-go/grok-4.7` | 15 | 60 |
| `opencode-go/glm-5.3` | 15 | 120 |
| `opencode-go/qwen3.8-max` | 15 | 60 |
| `opencode-go/deepseek-v4-pro` | 15 | 60 |
| `opencode-go/mimo-v2.6-pro` | 15 | 60 |
| `opencode-go/glm-5.2` | 60 | 180 |
| `opencode-go/kimi-k2.7-code` | 60 | 180 |
| `opencode-go/mimo-v2.5-pro` | 15 | 60 |
| `opencode-go/muse-spark-1.3-contributor` | 60 | 120 |
| `opencode-go/minimax-m3.1-flash` | 20 | 80 |
| `opencode-go/minimax-m3` | 60 | 180 |
| `opencode-go/muse-spark-1.2-contributor` | 60 | 120 |
| `opencode-go/mimo-v2.6-flash` | 60 | 120 |
| `opencode-go/mimo-v2.5` | 60 | 120 |
| `opencode-go/hy3` | 60 | 240 |
| `opencode-go/longcat-2.0` | 60 | 240 |
| `opencode-go/deepseek-v4.1-flash` | 80 | 160 |
| `opencode-go/gpt-6-luna` | 15 | 60 |
| `opencode-go/claude-haiku-5-5` | 15 | 60 |
| `opencode-go/gpt-5.6-luna` | 15 | 60 |
| `opencode-go/qwen3.8-flash` | 30 | 90 |
| `opencode-go/qwen3.7-plus` | 60 | 180 |
| `opencode-go/glm-5.3-flash` | 60 | 180 |
| `opencode-go/hy4-preview` | 30 | 120 |
| `opencode-go/step-5-preview` | 15 | 60 |
| `opencode-go/beam-free` | Unlimited | Unlimited |
| `opencode-go/longcat-2.5-preview-free` | Unlimited | Unlimited |

The table uses base (shortest-prompt) rates. Long-context rates below are input/output USD per MTok.

| Model | Prompt length | Input $/MTok | Output $/MTok |
|---|---|---|---|
| `opencode-go/grok-4.7` | > 200K tokens | 4.00 | 12.00 |
| `opencode-go/gpt-6-luna` | > 272K tokens | 0.20 | 0.75 |
| `opencode-go/claude-haiku-5-5` | > 100K tokens | 0.50 | 2.50 |
| `opencode-go/gpt-5.6-luna` | > 272K tokens | 0.40 | 1.80 |
| `opencode-go/qwen3.7-plus` | > 256K tokens | 1.20 | 4.80 |

Promotional rows use the currently billed rates. List prices below are input/output USD per MTok.

| Model | Discount | List input $/MTok | List output $/MTok | Discount end |
|---|---|---|---|---|
| `opencode-go/gemini-4-argon` | 50% off | 4.00 | 20.00 | 2026-11-30 |

Rows with Peak and Off-Peak prices use the Peak rate as the conservative planning estimate. Off-Peak rates are input/output USD per MTok.

| Model | Off-Peak input $/MTok | Off-Peak output $/MTok | Off-Peak hours |
|---|---|---|---|
| `opencode-go/deepseek-v4-pro` | 0.66 | 1.98 | All hours except 01:00–04:00 and 06:00–10:00 UTC Monday–Friday; weekends included |
| `opencode-go/deepseek-v4.1-flash` | 0.15 | 0.60 | All hours except 01:00–04:00 and 06:00–10:00 UTC Monday–Friday; weekends included |

In Settings → Privacy, `deepseek-v4.1-flash` and `deepseek-v4-pro` require Global under Regions; `muse-spark-1.3-contributor` and `muse-spark-1.2-contributor` require "Allow paid endpoints that train on request data".

Source: [OpenCode Go](https://opencode.ai/v2/docs/console/go).

### Note → applies at

| Note | Applies at |
|---|---|
| A member routes concrete questions to the Director and receives the operator's answer as a message. | `{decision_surface}`; [user relay](supervision.md#user-delegation-protocol) |
| Members perform assignments using the selected provider/model, workspace reads/edits, permitted tools and broker communication. A Director without a verified native stream adapter uses explicit manual `poll`/finite `wait` and states that reception limit. | [Director wake](supervision.md#director-wake) |
| `OPENCODE_CONFIG_DIR` adds a preset search directory alongside default home configuration. Same-name agent definitions can merge across sources; inspect the effective configuration when customizing a preset. | `{permission_flags}`; preset discovery |
| The preset's shell map has a deny base, literal routine/broker allow patterns and explicit native-dispatch denies. Last-match precedence keeps `cafleet member prompt *` and `cafleet member exec *` denied despite the broad `cafleet *` allow. Role authority still scopes permitted tools; route observed denials with command/cwd/environment/refusal evidence. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-observed-denial) |
| MCP tools' participation in preset permission evaluation and compound-shell permission matching are unverified on opencode 2.1.4. Keep loaded configurations free of MCP servers and submit one command per invocation. | `{permission_flags}`; [command isolation](../SKILL.md#one-shot-command-isolation) |
| Each pane runs `opencode mini --standalone --agent cafleet` with `--prompt`, optional model selection and a private server. | `{permission_flags}`; member spawn |
| Mini uses the selected preset's fixed allow/deny policy. Execute permitted role-authorized commands and route observed refusals with actual evidence. Surface unresolved approval to the Director and retain native deny rules; supervision handles prompts that prevent the member from sending. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-observed-denial) |
| Mini receives pushed notifications and direct prompts as submitted user turns. Captures can expose only the status bar; inspect retained session records for omitted input/output and broker poll/ACK for message consumption. | [Push notifications](runtime/spec/multiplexer-backends.md#push-notifications); prompt dispatch |
| Leading-`!` input runs a native shell command in `mini`. Preserve Director authorization and member-tool permissions because the native path's effective boundary and any second permission check remain unestablished. | [Native dispatch](prompt-routing.md#director-side-dispatch) |
| The `cafleet` preset retains native file boundaries and its declared shell allowlist. Broad patterns grant no Director-only actions or Git write authority. Use native readers for prerequisites and permitted shell reads when shell is the only reader; route an actual denial before dependent startup work. Capture and relay unresolved permission prompts. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-observed-denial) |

## hermes

### Role defaults

| Placeholder | Value |
|---|---|
| `{reviewer_model}` | `openai/gpt-6.1-sol` |
| `{reviewer_effort}` | `high` |
| `{other_model}` | `openai/gpt-6.1-sol` |
| `{other_effort}` | `high` |

### Model catalog

Hermes model data last verified: 2026-10-25. This catalog-specific date applies to the Hermes model data; preserve the shared freshness date when refreshing only Hermes. Refresh the selected catalog at least every 30 days under `cafleet-model-list-refresh`.

The table contains every Text model in the official Nous Portal snapshot. Copy each exact ID as the Hermes `--model` value, including provider prefixes and suffixes. Embeddings are excluded. Prices are Nous Portal-billed USD per million tokens and apply only with verified effective Nous Portal routing. Cost-based selection with another provider or an unverified automatic route requires an operator choice.

The approved order places `openai/gpt-6.1-sol` first for its observed native tool use, followed by every remaining Text model in snapshot source order for traceability. Read those remaining positions as source order. The first row's Class records scoped tool-use evidence from authorized native probes; automatic role selection of that row requires the applicable Hermes runtime milestones to pass. Every remaining row requires explicit manual selection while its tool-loop eligibility remains unverified. The role defaults express the approved choices for use after those runtime gates pass.

Price cells retain the exact decimals decoded from the saved payload, including additional precision beyond the Portal UI's two-decimal display. Each published zero rate is written `0.00`. Use the current billed rates in this table and the published list/offer qualifiers below.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---:|---:|
| `openai/gpt-6.1-sol` | Verified tool use on the accepted Hermes runtime | 2 | 10 |
| `moonshotai/kimi-k3.1-pro` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `minimax/minimax-m3.1-flash` | Tool-loop eligibility unverified; manual selection | 0.09 | 0.5 |
| `deepseek/deepseek-v4.1-flash-1017` | Tool-loop eligibility unverified; manual selection | 0.02 | 1 |
| `elorian/glimpse` | Tool-loop eligibility unverified; manual selection | 0.6 | 3 |
| `reflection/beam-1.0-501b-a23b` | Tool-loop eligibility unverified; manual selection | 0.24 | 0.98 |
| `inception/mercury-3` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.35 |
| `meta/muse-spark-open` | Tool-loop eligibility unverified; manual selection | 1.25 | 4.25 |
| `nvidia/nemotron-3.5-super-vl` | Tool-loop eligibility unverified; manual selection | 0.06 | 0.35 |
| `unbiased/pareto-26.10` | Tool-loop eligibility unverified; manual selection | 0.7999999999999999 | 3.1999999999999997 |
| `~unbiased/pareto-latest` | Tool-loop eligibility unverified; manual selection | 0.7999999999999999 | 3.1999999999999997 |
| `unbiased/pareto-26.9` | Tool-loop eligibility unverified; manual selection | 2.5 | 7.5 |
| `liquid/lfm-2.5-24b-a2b` | Tool-loop eligibility unverified; manual selection | 0.049999999999999996 | 0.7 |
| `google/gemini-4-argon` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `meta/muse-wonder` | Tool-loop eligibility unverified; manual selection | 4 | 20 |
| `stepfun/step-5-preview` | Tool-loop eligibility unverified; manual selection | 1 | 2.7 |
| `anthropic/claude-haiku-5.5` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.5 |
| `google/gemini-nano-banana-2.1` | Tool-loop eligibility unverified; manual selection | 1.5 | 7.5 |
| `mistralai/mistral-large-4-0` | Tool-loop eligibility unverified; manual selection | 1.3599999999999999 | 4.18 |
| `inclusionai/ling-3.1-flash` | Tool-loop eligibility unverified; manual selection | 0.075 | 0.22499999999999998 |
| `openai/gpt-6.1-sol-pro` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `anthropic/claude-sonnet-5.5` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `perceptron/perceptron-mk1.5` | Tool-loop eligibility unverified; manual selection | 0.15 | 1.5 |
| `fireworks/ember-1` | Tool-loop eligibility unverified; manual selection | 3 | 15 |
| `z-ai/glm-5.3-prime` | Tool-loop eligibility unverified; manual selection | 2.8 | 8.8 |
| `qwen/qwen3.8-max-prime` | Tool-loop eligibility unverified; manual selection | 4 | 12 |
| `aion-labs/aion-3.5-mini` | Tool-loop eligibility unverified; manual selection | 0.7 | 1.4 |
| `aion-labs/aion-3.5` | Tool-loop eligibility unverified; manual selection | 3 | 6 |
| `upstage/solar-mini4` | Tool-loop eligibility unverified; manual selection | 0.049999999999999996 | 0.19999999999999998 |
| `cohere/command-a-plus` | Tool-loop eligibility unverified; manual selection | 0.3 | 1.5 |
| `openai/gpt-6-luna-pro` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.5 |
| `openai/gpt-6-luna` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.5 |
| `openai/gpt-6-sol-pro` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `openai/gpt-6-sol` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `anthropic/claude-opus-5.5` | Tool-loop eligibility unverified; manual selection | 4 | 20 |
| `xiaomi/mimo-v2.6-pro-ultraspeed` | Tool-loop eligibility unverified; manual selection | 4.35 | 8.7 |
| `xiaomi/mimo-v2.6-flash` | Tool-loop eligibility unverified; manual selection | 0.091 | 0.182 |
| `xiaomi/mimo-v2.6-pro` | Tool-loop eligibility unverified; manual selection | 0.387 | 0.7829999999999999 |
| `qwen/qwen3.8-omni-flash` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.47 |
| `prism-ml/ternary-bonsai-2-27b` | Tool-loop eligibility unverified; manual selection | 0.075 | 0.5 |
| `z-ai/glm-5.3-flashx` | Tool-loop eligibility unverified; manual selection | 0.37 | 1.25 |
| `~deepseek/deepseek-pro-latest` | Tool-loop eligibility unverified; manual selection | 0.1188 | 8 |
| `~deepseek/deepseek-flash-latest` | Tool-loop eligibility unverified; manual selection | 0.02 | 1 |
| `~openai/gpt-astra-latest` | Tool-loop eligibility unverified; manual selection | 10 | 50 |
| `~openai/gpt-sol-latest` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `~openai/gpt-terra-latest` | Tool-loop eligibility unverified; manual selection | 2 | 12 |
| `~openai/gpt-luna-latest` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.5 |
| `sakana/fugu-ultra-v2` | Tool-loop eligibility unverified; manual selection | 5 | 30 |
| `sakana/fugu-max` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `inclusionai/ling-3.0-flash-vl` | Tool-loop eligibility unverified; manual selection | 0.06 | 0.18 |
| `inception/mercury-2.5` | Tool-loop eligibility unverified; manual selection | 0.04 | 0.15 |
| `nex-agi/nex-n2.5-pro` | Tool-loop eligibility unverified; manual selection | 0.075 | 0.25 |
| `openai/gpt-6-astra` | Tool-loop eligibility unverified; manual selection | 10 | 50 |
| `openai/gpt-6-astra-pro` | Tool-loop eligibility unverified; manual selection | 10 | 50 |
| `qwen/qwen3.8-max-0902` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `meta/muse-spark-1.3` | Tool-loop eligibility unverified; manual selection | 1.25 | 4.25 |
| `google/gemini-3.8-flash` | Tool-loop eligibility unverified; manual selection | 0.75 | 3.75 |
| `anthropic/claude-fable-5.1` | Tool-loop eligibility unverified; manual selection | 10 | 50 |
| `ibm-granite/granite-4.2-8b` | Tool-loop eligibility unverified; manual selection | 0.06 | 0.25 |
| `tencent/hy4-preview` | Tool-loop eligibility unverified; manual selection | 0.417 | 1.25 |
| `~z-ai/glm-flash-latest` | Tool-loop eligibility unverified; manual selection | 0.051300000000000005 | 0.0701 |
| `qwen/qwen3.8-flash` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.47 |
| `z-ai/glm-5.3-flash` | Tool-loop eligibility unverified; manual selection | 0.12 | 0.39999999999999997 |
| `deepseek/deepseek-v4-flash-vision-exp` | Tool-loop eligibility unverified; manual selection | 0.21559999999999999 | 0.6468 |
| `~z-ai/glm-latest` | Tool-loop eligibility unverified; manual selection | 0.0703 | 4.3843 |
| `qwen/qwen3.8-27b` | Tool-loop eligibility unverified; manual selection | 0.02 | 0.5 |
| `google/gemini-3.7-flash` | Tool-loop eligibility unverified; manual selection | 0.75 | 3.75 |
| `bytedance-seed/seed-2-1-turbo` | Tool-loop eligibility unverified; manual selection | 0.5 | 2.5 |
| `qwen/qwen3.8-2.4t-a95b` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `bytedance-seed/seed-2.0-code` | Tool-loop eligibility unverified; manual selection | 0.5 | 3 |
| `deepseek/deepseek-v4-pro-0813` | Tool-loop eligibility unverified; manual selection | 0.1188 | 8 |
| `nvidia/nemotron-3.5-lightning` | Tool-loop eligibility unverified; manual selection | 0.024999999999999998 | 0.12 |
| `upstage/solar-pro4` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `meta/muse-glimmer-30b` | Tool-loop eligibility unverified; manual selection | 0.3 | 1.2 |
| `meta/muse-spark-1.2` | Tool-loop eligibility unverified; manual selection | 1.25 | 4.25 |
| `~deepseek/deepseek-v4-flash-latest` | Tool-loop eligibility unverified; manual selection | 0.0422 | 0.1763 |
| `thinkingmachines/inkling-small` | Tool-loop eligibility unverified; manual selection | 0.44999999999999996 | 1.2 |
| `qwen/qwen3.7-flash` | Tool-loop eligibility unverified; manual selection | 0.03 | 0.13 |
| `anthropic/claude-opus-5` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `inclusionai/ling-3.0-flash` | Tool-loop eligibility unverified; manual selection | 0.020999999999999998 | 0.063 |
| `poolside/laguna-s-2.1` | Tool-loop eligibility unverified; manual selection | 0.09 | 0.18 |
| `poolside/laguna-s-2.1:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `google/gemini-3.6-flash` | Tool-loop eligibility unverified; manual selection | 0.75 | 3.75 |
| `google/gemini-3.5-flash-lite` | Tool-loop eligibility unverified; manual selection | 0.3 | 2.5 |
| `meituan/longcat-2.0` | Tool-loop eligibility unverified; manual selection | 0.3 | 1.2 |
| `thinkingmachines/inkling` | Tool-loop eligibility unverified; manual selection | 1 | 4.05 |
| `moonshotai/kimi-k3` | Tool-loop eligibility unverified; manual selection | 0.15200000000000002 | 16 |
| `openai/gpt-5.6-luna-pro` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 1.2 |
| `openai/gpt-5.6-luna` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 1.2 |
| `openai/gpt-5.6-terra-pro` | Tool-loop eligibility unverified; manual selection | 2 | 12 |
| `openai/gpt-5.6-terra` | Tool-loop eligibility unverified; manual selection | 2 | 12 |
| `openai/gpt-5.6-sol-pro` | Tool-loop eligibility unverified; manual selection | 4 | 20 |
| `openai/gpt-5.6-sol` | Tool-loop eligibility unverified; manual selection | 4 | 20 |
| `~x-ai/grok-latest` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `aion-labs/aion-3.0-mini` | Tool-loop eligibility unverified; manual selection | 0.7 | 1.4 |
| `aion-labs/aion-3.0` | Tool-loop eligibility unverified; manual selection | 3 | 6 |
| `tencent/hy3` | Tool-loop eligibility unverified; manual selection | 0.13199999999999998 | 0.5279999999999999 |
| `poolside/laguna-xs-2.1` | Tool-loop eligibility unverified; manual selection | 0.06 | 0.12 |
| `poolside/laguna-xs-2.1:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `anthropic/claude-sonnet-5` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `sakana/fugu-ultra` | Tool-loop eligibility unverified; manual selection | 5 | 30 |
| `google/gemini-3-pro-image` | Tool-loop eligibility unverified; manual selection | 2 | 12 |
| `moonshotai/kimi-k2.7-code` | Tool-loop eligibility unverified; manual selection | 0.6712 | 3.35 |
| `~anthropic/claude-fable-latest` | Tool-loop eligibility unverified; manual selection | 10 | 50 |
| `anthropic/claude-fable-5` | Tool-loop eligibility unverified; manual selection | 10 | 50 |
| `nvidia/nemotron-3-ultra-550b-a55b` | Tool-loop eligibility unverified; manual selection | 0.5 | 2.2 |
| `qwen/qwen3.7-plus` | Tool-loop eligibility unverified; manual selection | 0.32 | 1.28 |
| `minimax/minimax-m3` | Tool-loop eligibility unverified; manual selection | 0.22999999999999998 | 0.96 |
| `stepfun/step-3.7-flash` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 1.15 |
| `anthropic/claude-opus-4.8` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `qwen/qwen3.7-max` | Tool-loop eligibility unverified; manual selection | 1.475 | 4.425 |
| `x-ai/grok-build-0.1` | Tool-loop eligibility unverified; manual selection | 1 | 2 |
| `google/gemini-3.5-flash` | Tool-loop eligibility unverified; manual selection | 1.5 | 9 |
| `google/gemini-3.1-flash-lite` | Tool-loop eligibility unverified; manual selection | 0.25 | 1.5 |
| `openai/gpt-chat-latest` | Tool-loop eligibility unverified; manual selection | 5 | 30 |
| `x-ai/grok-4.3` | Tool-loop eligibility unverified; manual selection | 1.25 | 2.5 |
| `mistralai/mistral-medium-3-5` | Tool-loop eligibility unverified; manual selection | 1.5 | 7.5 |
| `~anthropic/claude-haiku-latest` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.5 |
| `~openai/gpt-mini-latest` | Tool-loop eligibility unverified; manual selection | 0.75 | 4.5 |
| `~google/gemini-pro-latest` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `~moonshotai/kimi-latest` | Tool-loop eligibility unverified; manual selection | 0.15200000000000002 | 16 |
| `~google/gemini-flash-latest` | Tool-loop eligibility unverified; manual selection | 0.75 | 3.75 |
| `~anthropic/claude-sonnet-latest` | Tool-loop eligibility unverified; manual selection | 2 | 10 |
| `qwen/qwen3.5-plus-20260420` | Tool-loop eligibility unverified; manual selection | 0.3 | 1.7999999999999998 |
| `qwen/qwen3.6-flash` | Tool-loop eligibility unverified; manual selection | 0.1875 | 1.125 |
| `qwen/qwen3.6-35b-a3b` | Tool-loop eligibility unverified; manual selection | 0.049999999999999996 | 0.7 |
| `qwen/qwen3.6-27b` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 2 |
| `openai/gpt-5.5-pro` | Tool-loop eligibility unverified; manual selection | 30 | 180 |
| `openai/gpt-5.5` | Tool-loop eligibility unverified; manual selection | 5 | 30 |
| `deepseek/deepseek-v4-pro` | Tool-loop eligibility unverified; manual selection | 0.94 | 1.89 |
| `deepseek/deepseek-v4-flash` | Tool-loop eligibility unverified; manual selection | 0.0211 | 0.44999999999999996 |
| `tencent/hy3-preview` | Tool-loop eligibility unverified; manual selection | 0.18 | 0.6 |
| `xiaomi/mimo-v2.5-pro` | Tool-loop eligibility unverified; manual selection | 0.3045 | 0.609 |
| `xiaomi/mimo-v2.5` | Tool-loop eligibility unverified; manual selection | 0.119 | 0.238 |
| `~anthropic/claude-opus-latest` | Tool-loop eligibility unverified; manual selection | 4 | 20 |
| `moonshotai/kimi-k2.6` | Tool-loop eligibility unverified; manual selection | 0.4493 | 1.8920000000000001 |
| `anthropic/claude-opus-4.7` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `z-ai/glm-5.1` | Tool-loop eligibility unverified; manual selection | 0.9646 | 3.0316 |
| `google/gemma-4-26b-a4b-it` | Tool-loop eligibility unverified; manual selection | 0.041999999999999996 | 0.22 |
| `google/gemma-4-31b-it` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.33999999999999997 |
| `qwen/qwen3.6-plus` | Tool-loop eligibility unverified; manual selection | 0.325 | 1.95 |
| `z-ai/glm-5v-turbo` | Tool-loop eligibility unverified; manual selection | 1.2 | 4 |
| `arcee-ai/trinity-large-thinking` | Tool-loop eligibility unverified; manual selection | 0.25 | 0.7999999999999999 |
| `x-ai/grok-4.20` | Tool-loop eligibility unverified; manual selection | 1.25 | 2.5 |
| `rekaai/reka-edge` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.09999999999999999 |
| `minimax/minimax-m2.7` | Tool-loop eligibility unverified; manual selection | 0.21 | 0.84 |
| `openai/gpt-5.4-nano` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 1.25 |
| `openai/gpt-5.4-mini` | Tool-loop eligibility unverified; manual selection | 0.75 | 4.5 |
| `mistralai/mistral-small-2603` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `z-ai/glm-5-turbo` | Tool-loop eligibility unverified; manual selection | 1.2 | 4 |
| `nvidia/nemotron-3-super-120b-a12b` | Tool-loop eligibility unverified; manual selection | 0.08 | 0.44999999999999996 |
| `bytedance-seed/seed-2.0-lite` | Tool-loop eligibility unverified; manual selection | 0.25 | 2 |
| `qwen/qwen3.5-9b` | Tool-loop eligibility unverified; manual selection | 0.08 | 0.13 |
| `openai/gpt-5.4-pro` | Tool-loop eligibility unverified; manual selection | 30 | 180 |
| `openai/gpt-5.4` | Tool-loop eligibility unverified; manual selection | 2.5 | 15 |
| `inception/mercury-2` | Tool-loop eligibility unverified; manual selection | 0.25 | 0.75 |
| `google/gemini-3.1-flash-lite-preview` | Tool-loop eligibility unverified; manual selection | 0.25 | 1.5 |
| `bytedance-seed/seed-2.0-mini` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.39999999999999997 |
| `qwen/qwen3.5-35b-a3b` | Tool-loop eligibility unverified; manual selection | 0.065 | 0.75 |
| `qwen/qwen3.5-27b` | Tool-loop eligibility unverified; manual selection | 0.195 | 1.56 |
| `qwen/qwen3.5-122b-a10b` | Tool-loop eligibility unverified; manual selection | 0.26 | 2.08 |
| `qwen/qwen3.5-flash-02-23` | Tool-loop eligibility unverified; manual selection | 0.065 | 0.26 |
| `google/gemini-3.1-pro-preview-customtools` | Tool-loop eligibility unverified; manual selection | 2 | 12 |
| `openai/gpt-5.3-codex` | Tool-loop eligibility unverified; manual selection | 1.75 | 14 |
| `aion-labs/aion-2.0` | Tool-loop eligibility unverified; manual selection | 0.7999999999999999 | 1.5999999999999999 |
| `google/gemini-3.1-pro-preview` | Tool-loop eligibility unverified; manual selection | 2 | 12 |
| `anthropic/claude-sonnet-4.6` | Tool-loop eligibility unverified; manual selection | 3 | 15 |
| `qwen/qwen3.5-plus-02-15` | Tool-loop eligibility unverified; manual selection | 0.26 | 1.56 |
| `qwen/qwen3.5-397b-a17b` | Tool-loop eligibility unverified; manual selection | 0.39 | 2.34 |
| `minimax/minimax-m2.5` | Tool-loop eligibility unverified; manual selection | 0.27 | 0.95 |
| `z-ai/glm-5` | Tool-loop eligibility unverified; manual selection | 0.6 | 1.92 |
| `anthropic/claude-opus-4.6` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `qwen/qwen3-coder-next` | Tool-loop eligibility unverified; manual selection | 0.12 | 0.7999999999999999 |
| `stepfun/step-3.5-flash` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.3 |
| `moonshotai/kimi-k2.5` | Tool-loop eligibility unverified; manual selection | 0.49 | 2.5 |
| `upstage/solar-pro-3` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `openai/gpt-audio` | Tool-loop eligibility unverified; manual selection | 2.5 | 10 |
| `openai/gpt-audio-mini` | Tool-loop eligibility unverified; manual selection | 0.6 | 2.4 |
| `z-ai/glm-4.7-flash` | Tool-loop eligibility unverified; manual selection | 0.060500000000000005 | 0.39999999999999997 |
| `openai/gpt-5.2-codex` | Tool-loop eligibility unverified; manual selection | 1.75 | 14 |
| `bytedance-seed/seed-1.6-flash` | Tool-loop eligibility unverified; manual selection | 0.075 | 0.3 |
| `bytedance-seed/seed-1.6` | Tool-loop eligibility unverified; manual selection | 0.25 | 2 |
| `minimax/minimax-m2.1` | Tool-loop eligibility unverified; manual selection | 0.3 | 1.2 |
| `z-ai/glm-4.7` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 1.75 |
| `google/gemini-3-flash-preview` | Tool-loop eligibility unverified; manual selection | 0.5 | 3 |
| `nvidia/nemotron-3-nano-30b-a3b` | Tool-loop eligibility unverified; manual selection | 0.049999999999999996 | 0.19999999999999998 |
| `openai/gpt-5.2-chat` | Tool-loop eligibility unverified; manual selection | 1.75 | 14 |
| `openai/gpt-5.2-pro` | Tool-loop eligibility unverified; manual selection | 21 | 168 |
| `openai/gpt-5.2` | Tool-loop eligibility unverified; manual selection | 1.75 | 14 |
| `mistralai/devstral-2512` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 2 |
| `relace/relace-search` | Tool-loop eligibility unverified; manual selection | 1 | 3 |
| `z-ai/glm-4.6v` | Tool-loop eligibility unverified; manual selection | 0.3 | 0.8999999999999999 |
| `openai/gpt-5.1-codex-max` | Tool-loop eligibility unverified; manual selection | 1.25 | 10 |
| `amazon/nova-2-lite-v1` | Tool-loop eligibility unverified; manual selection | 0.3 | 2.5 |
| `mistralai/ministral-14b-2512` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 0.19999999999999998 |
| `mistralai/ministral-8b-2512` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.15 |
| `mistralai/ministral-3b-2512` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.09999999999999999 |
| `mistralai/mistral-large-2512` | Tool-loop eligibility unverified; manual selection | 0.5 | 1.5 |
| `deepseek/deepseek-v3.2` | Tool-loop eligibility unverified; manual selection | 0.20879999999999999 | 0.30960000000000004 |
| `anthropic/claude-opus-4.5` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `openai/gpt-5.1` | Tool-loop eligibility unverified; manual selection | 1.25 | 10 |
| `openai/gpt-5.1-codex` | Tool-loop eligibility unverified; manual selection | 1.25 | 10 |
| `openai/gpt-5.1-codex-mini` | Tool-loop eligibility unverified; manual selection | 0.25 | 2 |
| `moonshotai/kimi-k2-thinking` | Tool-loop eligibility unverified; manual selection | 0.6 | 2.5 |
| `amazon/nova-premier-v1` | Tool-loop eligibility unverified; manual selection | 2.5 | 12.5 |
| `mistralai/voxtral-small-24b-2507` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.3 |
| `openai/gpt-oss-safeguard-20b` | Tool-loop eligibility unverified; manual selection | 0.075 | 0.3 |
| `minimax/minimax-m2` | Tool-loop eligibility unverified; manual selection | 0.255 | 1.02 |
| `anthropic/claude-haiku-4.5` | Tool-loop eligibility unverified; manual selection | 1 | 5 |
| `qwen/qwen3-vl-8b-instruct` | Tool-loop eligibility unverified; manual selection | 0.25 | 0.75 |
| `qwen/qwen3-vl-30b-a3b-thinking` | Tool-loop eligibility unverified; manual selection | 0.29 | 1 |
| `qwen/qwen3-vl-30b-a3b-instruct` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `openai/gpt-5-pro` | Tool-loop eligibility unverified; manual selection | 15 | 120 |
| `z-ai/glm-4.6` | Tool-loop eligibility unverified; manual selection | 0.43 | 1.75 |
| `anthropic/claude-sonnet-4.5` | Tool-loop eligibility unverified; manual selection | 3 | 15 |
| `deepseek/deepseek-v3.2-exp` | Tool-loop eligibility unverified; manual selection | 0.27 | 0.41 |
| `qwen/qwen3-vl-235b-a22b-instruct` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 0.88 |
| `deepseek/deepseek-v3.1-terminus` | Tool-loop eligibility unverified; manual selection | 0.27 | 1 |
| `qwen/qwen3-coder-flash` | Tool-loop eligibility unverified; manual selection | 0.195 | 0.975 |
| `qwen/qwen3-next-80b-a3b-thinking` | Tool-loop eligibility unverified; manual selection | 0.15 | 1.2 |
| `qwen/qwen3-next-80b-a3b-instruct` | Tool-loop eligibility unverified; manual selection | 0.09 | 1.1 |
| `moonshotai/kimi-k2-0905` | Tool-loop eligibility unverified; manual selection | 0.6 | 2.5 |
| `deepseek/deepseek-chat-v3.1` | Tool-loop eligibility unverified; manual selection | 0.25 | 0.95 |
| `mistralai/mistral-medium-3.1` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 2 |
| `z-ai/glm-4.5v` | Tool-loop eligibility unverified; manual selection | 0.6 | 1.7999999999999998 |
| `openai/gpt-5` | Tool-loop eligibility unverified; manual selection | 1.25 | 10 |
| `openai/gpt-5-mini` | Tool-loop eligibility unverified; manual selection | 0.25 | 2 |
| `openai/gpt-5-nano` | Tool-loop eligibility unverified; manual selection | 0.049999999999999996 | 0.39999999999999997 |
| `openai/gpt-oss-120b` | Tool-loop eligibility unverified; manual selection | 0.03 | 0.15 |
| `openai/gpt-oss-20b` | Tool-loop eligibility unverified; manual selection | 0.018 | 0.09 |
| `anthropic/claude-opus-4.1` | Tool-loop eligibility unverified; manual selection | 15 | 75 |
| `mistralai/codestral-2508` | Tool-loop eligibility unverified; manual selection | 0.3 | 0.8999999999999999 |
| `qwen/qwen3-coder-30b-a3b-instruct` | Tool-loop eligibility unverified; manual selection | 0.07 | 0.28 |
| `qwen/qwen3-30b-a3b-instruct-2507` | Tool-loop eligibility unverified; manual selection | 0.09 | 0.3 |
| `z-ai/glm-4.5` | Tool-loop eligibility unverified; manual selection | 0.6 | 2.2 |
| `z-ai/glm-4.5-air` | Tool-loop eligibility unverified; manual selection | 0.13 | 0.85 |
| `qwen/qwen3-235b-a22b-thinking-2507` | Tool-loop eligibility unverified; manual selection | 0.44999999999999996 | 3.5 |
| `qwen/qwen3-coder` | Tool-loop eligibility unverified; manual selection | 0.22 | 1.7999999999999998 |
| `google/gemini-2.5-flash-lite` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.39999999999999997 |
| `qwen/qwen3-235b-a22b-2507` | Tool-loop eligibility unverified; manual selection | 0.0875 | 0.35 |
| `moonshotai/kimi-k2` | Tool-loop eligibility unverified; manual selection | 0.5700000000000001 | 2.3 |
| `mistralai/mistral-small-3.2-24b-instruct` | Tool-loop eligibility unverified; manual selection | 0.075 | 0.19999999999999998 |
| `minimax/minimax-m1` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 2.2 |
| `google/gemini-2.5-flash` | Tool-loop eligibility unverified; manual selection | 0.3 | 2.5 |
| `google/gemini-2.5-pro` | Tool-loop eligibility unverified; manual selection | 1.25 | 10 |
| `openai/o3-pro` | Tool-loop eligibility unverified; manual selection | 20 | 80 |
| `google/gemini-2.5-pro-preview` | Tool-loop eligibility unverified; manual selection | 1.25 | 10 |
| `deepseek/deepseek-r1-0528` | Tool-loop eligibility unverified; manual selection | 0.5 | 2.1500000000000004 |
| `mistralai/mistral-medium-3` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 2 |
| `qwen/qwen3-30b-a3b` | Tool-loop eligibility unverified; manual selection | 0.12 | 0.5 |
| `qwen/qwen3-14b` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.22 |
| `qwen/qwen3-32b` | Tool-loop eligibility unverified; manual selection | 0.08 | 0.28 |
| `openai/o4-mini-high` | Tool-loop eligibility unverified; manual selection | 1.1 | 4.4 |
| `openai/o3` | Tool-loop eligibility unverified; manual selection | 2 | 8 |
| `openai/o4-mini` | Tool-loop eligibility unverified; manual selection | 1.1 | 4.4 |
| `openai/gpt-4.1` | Tool-loop eligibility unverified; manual selection | 2 | 8 |
| `openai/gpt-4.1-mini` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 1.5999999999999999 |
| `openai/gpt-4.1-nano` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.39999999999999997 |
| `meta-llama/llama-4-maverick` | Tool-loop eligibility unverified; manual selection | 0.1875 | 0.6525 |
| `meta-llama/llama-4-scout` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.3 |
| `deepseek/deepseek-chat-v3-0324` | Tool-loop eligibility unverified; manual selection | 0.25 | 1 |
| `mistralai/mistral-small-3.1-24b-instruct` | Tool-loop eligibility unverified; manual selection | 0.351 | 0.5549999999999999 |
| `google/gemma-3-12b-it` | Tool-loop eligibility unverified; manual selection | 0.049999999999999996 | 0.15 |
| `google/gemma-3-27b-it` | Tool-loop eligibility unverified; manual selection | 0.08 | 0.16 |
| `mistralai/mistral-saba` | Tool-loop eligibility unverified; manual selection | 0.19999999999999998 | 0.6 |
| `openai/o3-mini-high` | Tool-loop eligibility unverified; manual selection | 1.1 | 4.4 |
| `qwen/qwen-plus` | Tool-loop eligibility unverified; manual selection | 0.26 | 0.78 |
| `openai/o3-mini` | Tool-loop eligibility unverified; manual selection | 1.1 | 4.4 |
| `deepseek/deepseek-chat` | Tool-loop eligibility unverified; manual selection | 0.2574 | 1.0287 |
| `openai/o1` | Tool-loop eligibility unverified; manual selection | 15 | 60 |
| `meta-llama/llama-3.3-70b-instruct` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.32 |
| `amazon/nova-lite-v1` | Tool-loop eligibility unverified; manual selection | 0.06 | 0.24 |
| `amazon/nova-micro-v1` | Tool-loop eligibility unverified; manual selection | 0.035 | 0.14 |
| `amazon/nova-pro-v1` | Tool-loop eligibility unverified; manual selection | 0.7999999999999999 | 3.1999999999999997 |
| `openai/gpt-4o-2024-11-20` | Tool-loop eligibility unverified; manual selection | 2.5 | 10 |
| `mistralai/mistral-large-2407` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `qwen/qwen-2.5-7b-instruct` | Tool-loop eligibility unverified; manual selection | 0.09999999999999999 | 0.19999999999999998 |
| `qwen/qwen-2.5-72b-instruct` | Tool-loop eligibility unverified; manual selection | 0.36 | 0.39999999999999997 |
| `cohere/command-r-08-2024` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `cohere/command-r-plus-08-2024` | Tool-loop eligibility unverified; manual selection | 2.5 | 10 |
| `openai/gpt-4o-2024-08-06` | Tool-loop eligibility unverified; manual selection | 2.5 | 10 |
| `meta-llama/llama-3.1-70b-instruct` | Tool-loop eligibility unverified; manual selection | 0.39999999999999997 | 0.39999999999999997 |
| `meta-llama/llama-3.1-8b-instruct` | Tool-loop eligibility unverified; manual selection | 0.02 | 0.04 |
| `mistralai/mistral-nemo` | Tool-loop eligibility unverified; manual selection | 0.0166 | 0.0216 |
| `openai/gpt-4o-mini` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `openai/gpt-4o-mini-2024-07-18` | Tool-loop eligibility unverified; manual selection | 0.15 | 0.6 |
| `openai/gpt-4o` | Tool-loop eligibility unverified; manual selection | 2.5 | 10 |
| `openai/gpt-4o-2024-05-13` | Tool-loop eligibility unverified; manual selection | 5 | 15 |
| `mistralai/mixtral-8x22b-instruct` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `openai/gpt-4-turbo` | Tool-loop eligibility unverified; manual selection | 10 | 30 |
| `mistralai/mistral-large` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `openai/gpt-3.5-turbo-0613` | Tool-loop eligibility unverified; manual selection | 1 | 2 |
| `openai/gpt-3.5-turbo-16k` | Tool-loop eligibility unverified; manual selection | 3 | 4 |
| `openai/gpt-4` | Tool-loop eligibility unverified; manual selection | 30 | 60 |
| `upstage/solar-mini4:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `meituan/longcat-2.0:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `stepfun/step-3.7-flash:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `openai/gpt-6-astra-fast` | Tool-loop eligibility unverified; manual selection | 20 | 100 |
| `openai/gpt-6-astra-flex` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `openai/gpt-6-astra-pro-fast` | Tool-loop eligibility unverified; manual selection | 20 | 100 |
| `openai/gpt-6-astra-pro-flex` | Tool-loop eligibility unverified; manual selection | 5 | 25 |
| `z-ai/glm-5.2` | Tool-loop eligibility unverified; manual selection | 0.8999999999999999 | 2.83 |
| `meituan/longcat-2.5-preview:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `x-ai/grok-4.5` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `x-ai/grok-4.6` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `x-ai/grok-4.7` | Tool-loop eligibility unverified; manual selection | 2 | 6 |
| `moonshotai/kimi-k3:US` | Tool-loop eligibility unverified; manual selection | 3.63 | 18.150000000000002 |
| `deepseek/deepseek-v4-flash-0731:US` | Tool-loop eligibility unverified; manual selection | 0.363 | 1.089 |
| `deepseek/deepseek-v4.1-flash:US` | Tool-loop eligibility unverified; manual selection | 0.363 | 1.089 |
| `z-ai/glm-5.3-flash:US` | Tool-loop eligibility unverified; manual selection | 0.24750000000000003 | 0.8250000000000001 |
| `z-ai/glm-5.3:US` | Tool-loop eligibility unverified; manual selection | 2.31 | 7.26 |
| `deepseek/deepseek-v4-flash-0731` | Tool-loop eligibility unverified; manual selection | 0.07 | 0.14 |
| `z-ai/glm-5.3` | Tool-loop eligibility unverified; manual selection | 0.9099999999999999 | 2.8600000000000003 |
| `deepseek/deepseek-v4.1-flash` | Tool-loop eligibility unverified; manual selection | 0.106 | 0.513 |
| `reflection/beam-1.0-501b-a23b:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |
| `ornith/ornith-1.5-122b:free` | Tool-loop eligibility unverified; manual selection | 0.00 | 0.00 |

#### Discount and list-rate facts

Use the currently billed rates in the main table. The following comparisons preserve both published list rates, the Portal input-discount badge and its raw `minDiscountFraction`. Interpret the badge as the input reduction; output reductions can differ. The saved catalog publishes no discount end date or time/context price-tier schedule. Treat those qualifiers as `Not published` for this snapshot.

| Model | Billed input $/MTok | Billed output $/MTok | List input $/MTok | List output $/MTok | Portal input discount % | Portal minimum discount fraction | Discount end |
|---|---:|---:|---:|---:|---:|---:|---|
| `minimax/minimax-m3.1-flash` | 0.09 | 0.5 | 0.15 | 0.6 | 40 | 0.16666666666666663 | Not published |
| `inception/mercury-3` | 0.09999999999999999 | 0.35 | 0.19999999999999998 | 0.7 | 50 | 0.5 | Not published |
| `google/gemini-4-argon` | 2 | 10 | 4 | 20 | 50 | 0.5 | Not published |
| `upstage/solar-mini4` | 0.049999999999999996 | 0.19999999999999998 | 0.09999999999999999 | 0.39999999999999997 | 50 | 0.5 | Not published |
| `xiaomi/mimo-v2.6-flash` | 0.091 | 0.182 | 0.14 | 0.28 | 35 | 0.3500000000000001 | Not published |
| `xiaomi/mimo-v2.6-pro` | 0.387 | 0.7829999999999999 | 0.435 | 0.87 | 11 | 0.10000000000000009 | Not published |
| `inception/mercury-2.5` | 0.04 | 0.15 | 0.19999999999999998 | 0.75 | 80 | 0.8 | Not published |
| `google/gemini-3.8-flash` | 0.75 | 3.75 | 1.5 | 7.5 | 50 | 0.5 | Not published |
| `tencent/hy4-preview` | 0.417 | 1.25 | 0.834 | 2.501 | 50 | 0.5 | Not published |
| `z-ai/glm-5.3-flash` | 0.12 | 0.39999999999999997 | 0.15 | 0.5 | 20 | 0.19999999999999996 | Not published |
| `qwen/qwen3.8-27b` | 0.02 | 0.5 | 0.425 | 2.5500000000000003 | 95 | 0.803921568627451 | Not published |
| `google/gemini-3.7-flash` | 0.75 | 3.75 | 1.5 | 7.5 | 50 | 0.5 | Not published |
| `upstage/solar-pro4` | 0.15 | 0.6 | 0.3 | 1.2 | 50 | 0.5 | Not published |
| `poolside/laguna-s-2.1` | 0.09 | 0.18 | 0.09999999999999999 | 0.19999999999999998 | 10 | 0.09999999999999998 | Not published |
| `poolside/laguna-xs-2.1` | 0.06 | 0.12 | 0.09999999999999999 | 0.19999999999999998 | 40 | 0.4 | Not published |
| `minimax/minimax-m3` | 0.22999999999999998 | 0.96 | 0.3 | 1.2 | 23 | 0.20000000000000007 | Not published |
| `qwen/qwen3.6-27b` | 0.19999999999999998 | 2 | 0.44999999999999996 | 2.7 | 56 | 0.2592592592592593 | Not published |
| `moonshotai/kimi-k2.6` | 0.4493 | 1.8920000000000001 | 0.95 | 4 | 53 | 0.5269999999999999 | Not published |
| `z-ai/glm-5.1` | 0.9646 | 3.0316 | 1.4 | 4.4 | 31 | 0.31099999999999994 | Not published |
| `google/gemma-4-26b-a4b-it` | 0.041999999999999996 | 0.22 | 0.15 | 0.6 | 72 | 0.6333333333333333 | Not published |
| `qwen/qwen3.5-35b-a3b` | 0.065 | 0.75 | 0.1625 | 1.3 | 60 | 0.42307692307692313 | Not published |
| `z-ai/glm-5` | 0.6 | 1.92 | 1 | 3.1999999999999997 | 40 | 0.4 | Not published |
| `z-ai/glm-4.7` | 0.39999999999999997 | 1.75 | 0.6 | 2.2 | 33 | 0.20454545454545459 | Not published |
| `minimax/minimax-m2` | 0.255 | 1.02 | 0.3 | 1.2 | 15 | 0.15000000000000002 | Not published |
| `z-ai/glm-4.6` | 0.43 | 1.75 | 0.6 | 2.2 | 28 | 0.20454545454545459 | Not published |
| `z-ai/glm-4.5-air` | 0.13 | 0.85 | 0.19999999999999998 | 1.1 | 35 | 0.2272727272727273 | Not published |
| `mistralai/mistral-small-3.2-24b-instruct` | 0.075 | 0.19999999999999998 | 0.09999999999999999 | 0.3 | 25 | 0.25 | Not published |
| `mistralai/mistral-nemo` | 0.0166 | 0.0216 | 0.15 | 0.15 | 89 | 0.856 | Not published |

#### Free-rate facts

Every row below publishes both rates as zero and has its own exact ID. A free suffix remains part of the model token. The saved catalog supplies no offer end date; this snapshot does not guarantee future availability or assert that each free offer is permanent or time limited.

| Model | Input $/MTok | Output $/MTok | Offer end |
|---|---:|---:|---|
| `poolside/laguna-s-2.1:free` | 0.00 | 0.00 | Not published |
| `poolside/laguna-xs-2.1:free` | 0.00 | 0.00 | Not published |
| `upstage/solar-mini4:free` | 0.00 | 0.00 | Not published |
| `meituan/longcat-2.0:free` | 0.00 | 0.00 | Not published |
| `stepfun/step-3.7-flash:free` | 0.00 | 0.00 | Not published |
| `meituan/longcat-2.5-preview:free` | 0.00 | 0.00 | Not published |
| `reflection/beam-1.0-501b-a23b:free` | 0.00 | 0.00 | Not published |
| `ornith/ornith-1.5-122b:free` | 0.00 | 0.00 | Not published |

Source: [Nous Portal Models](https://portal.nousresearch.com/models). Catalog identity, pricing and qualifiers come from that official source; reviewed classes, ordering, eligibility and role defaults are maintainer policy.

## Template

Add a `## <name>` backend with every subsection below. Fill every value, keep runtime bindings as short noun phrases, and bind each caveat to its token and consuming instruction. Shared selection and supervision policy stays at its linked owner.

### Runtime bindings

Supply all five tokens: `{decision_surface}` (recorded user-reaction surface; members relay), `{permission_flags}` (selected permission mechanism and exact flags), `{pane_title}` (title mechanism or no analog), `{skill_loader}` (supported loader or complete absolute-path reads), and `{effort_levels}` (accepted values/flag or unsupported).

### Role defaults

Set `{reviewer_model}` and `{other_model}` to catalog tokens or aliases. Set their matching effort placeholders to a supported level or `—` when the backend does not support effort.

### Model catalog

List exact spawn tokens, valid aliases, reviewed most-to-least capability ordering, classes, and standard provider USD/MTok input/output prices. Include relevant context windows and availability constraints, required prefixes and official pricing/availability/configuration sources. Follow the freshness contract and [Director policy](../roles/director.md#model-selection).

### Note → applies at

Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind a `{permission_flags}` row describing the selected native permission policy and its limits alongside the shared behavior: members execute role-authorized task commands, route observed permission denials with actual evidence, and surface unresolved approval to the Director. When automatic review is selected, surface an unavailable automatic mode to the Director. Preserve native guards, explicit denies, managed policies and file/sandbox boundaries. Link the member command-execution, prompt-routing, Director and supervision instructions that consume it.
