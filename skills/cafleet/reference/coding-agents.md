# Coding Agents

Each backend section is self-contained. Select the backend by the subject of the action, then use its tables and bound notes:

| Purpose | Backend selector | Read and apply |
|---|---|---|
| Execute current instructions | The executing agent's `CODING AGENT:` identity, or its own identity when standalone | Its Runtime bindings and corresponding notes for decision surface, skill loader, and local execution tools. Required-reading row one gates this lookup; an ordinary member needs only its own runtime section. |
| Select/configure a spawned member | The backend selected under [Director model-selection policy](../roles/director.md#model-selection) | Its Model catalog and Role defaults; validate target effort and launch capabilities against its Runtime bindings. |

Apply the core skill's [Resolve your overlay](../SKILL.md#resolve-your-overlay) procedure.

Model catalogs, their provenance/context notes, canonical Role defaults, and shared freshness metadata are maintained exclusively by the `cafleet-model-list-refresh` skill from the official sources in each backend section, refreshed at least every 30 days (last refreshed: 2026-10-25). Runtime bindings, bound runtime notes, and worked resolutions belong to runtime documentation maintenance. The freshness date applies to model data; structural reorganization preserves that date.

Prices are standard provider USD rates per MTok and are planning estimates, not an invoice guarantee. Each backend's catalog is ordered most → least capable as reviewed judgment. Context windows are listed for the `claude` backend, whose model strings are the ones a context-window suffix can apply to. Selection policy, including cost efficiency mode and reviewer/member rules, lives in [Director model selection](../roles/director.md#model-selection).

`{permission_flags}` records omitted-policy launch defaults. An explicit `member create --permission-mode MODE` is resolved through the version-qualified backend catalog before spawn, retaining the remaining argv and sandbox boundaries; the selected policy belongs to the run rather than a fleet default. Detection, composer and request-bound answer capabilities are independently tested; the [runtime procedure](member-runtime.md) owns consumption, the Director's mode-dependent decision and response.

## Role requirements

| Role | Required backend capability |
|---|---|
| Director | Native goal lifecycle controls, such as a goal command, and background execution while supervision continues. Both capabilities are required. |
| Member | Runner-compatible PTY lifecycle and an explicit detection-coverage declaration, with launch, detection and safe composer input treated separately. |

Evaluate roles independently. A backend's native automatic review is its omitted Member policy where the backend offers one; interactive and fixed allow/deny policies remain explicit choices. Preserve native guards, managed policy, explicit denies and sandbox boundaries. Unknown detection/composer coverage permits manual native terminal work; ordinary CAFleet input then uses Escape-first delivery fallback, and semantic permission answers stay unavailable. The [member runtime procedure](member-runtime.md) owns event, input and continuation handling.

Goal support is a host capability; retain supplied activation, pause, completion and budget authority. Evaluate native reception separately: a host satisfying both Director prerequisites can use explicit manual `poll` or finite `wait` when no verified receive adapter is available. Background execution alone establishes no unsolicited wake or per-record reception guarantee.

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
| The omitted `auto` policy runs spawn-allowed broker tools and matching host allow rules, auto mode's classifier reviews every other call, and native rules, hooks and managed policy remain authoritative. A refused call is a permission refusal the member routes to the Director; an explicitly interactive native policy can instead block on a request-bound Director/user response or manual native input, and protected file edits retain their host-rule boundary. | `{permission_flags}`; [routing](prompt-routing.md#member-side-reconsider-then-route) |
| Legacy unwrapped members run `dontAsk`, so their open prompt is an `AskUserQuestion`; a Bash, Edit/Write, WebFetch or MCP tool approval follows the same rule. Escape-first input cancels it and Claude Code reports `The user doesn't want to proceed with this tool use.` with a STOP instruction before the CAFleet input arrives: only a following `[cafleet msg …]` line ending with the Escape-dismissal clause makes it delivery-caused, so re-issue the call or re-ask the question; a resume-clause-only preview leaves the rejection standing, and following `member prompt` / `member exec` input governs. | [Member command execution](../roles/member.md#command-execution); [Director dispatch](prompt-routing.md#director-side-dispatch); [unwrapped previews](cli.md#accepted-trade-offs) |
| The root Director receives broker records through Monitor stdout with zero automatic pane input, so a tool rejection or dismissed question in its pane comes from the operator or host. A wrapped member's runner delivers by profile without Escape into a verified empty composer. It queues input while it detects an open interaction, or while a planned composer is not verified empty, until that clears or fallback after 3 minutes. Without a plan it uses Escape-first delivery fallback. A rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause is delivery-caused, so re-issue the call or re-ask the question; one ending with the resume clause only leaves the rejection standing. | [Director wake](supervision.md#director-wake); [wrapped previews](member-runtime.md#reconcile-generated-input); [member command execution](../roles/member.md#command-execution) |

Evidence: Claude Code 2.1.296, checked 2026-10-22; [host continuation](https://code.claude.com/docs/en/goal) and [block cap](https://code.claude.com/docs/en/hooks-guide#stop-hook-hits-the-block-cap). Claude Code 2.1.302 with cafleet 0.29.0 reported an Escape-cancelled tool approval as a user rejection (issue #499). Local probes establish only exercised behavior.

#### Claude native Director reception

Discover Monitor through the installed host's tool-discovery mechanism and verify its authorization separately from Bash broker rules. After Director registration, start one Monitor with the actual literal Director ID. The tested invocation shape is:

```json
{"command":"cafleet message listen 1 --timeout 210","description":"CAFleet Director inbox","timeout_ms":240000}
```

Retain the task ID, pinned member/fleet IDs, receive status and unfinished actions in host-owned state. Require the `ready` record. Each stdout event supplies a protocol record and may start a new model turn before the process exits; consume complete records, reconcile the durable inbox and known/replayed IDs, record deferred work, ACK consumption and act within authorization. Keep Monitor active while returning to other work or ending the turn. An event during an open user question supplies no answer or authority to proceed past that decision.

The finite CLI lease normally ends before the tested Monitor lifetime. On `closed` or native stream-ended/completion notification, establish the old task and source process exited. Renew once while authorized work remains, then reconcile replay. Deduplicate closure and completion notices by the owned task ID, ignoring stale completion notices after ownership moves. A host timeout before CLI closure follows [receiver recovery](supervision.md#receiver-recovery); establish source-child termination before replacement and retain one owned source process.

Use verified native cancellation with observed child cleanup, or stop renewal and drain the finite lease. Preserve the supplied `/goal` activation, pause, completion and budget controls; receiving, ACKing and renewal use no goal lifecycle calls. Ordinary `Bash(run_in_background=true)` provides a different capability: observed completion notification did not establish per-line reception while the process lived. On a host satisfying the [Director requirements](#role-requirements), disclose unavailable or denied Monitor reception and use manual `poll`/finite `wait`.

Evidence: Claude Code 2.1.301 stream probes established separate event turns from one process. Actual Monitor expiry, cancellation, child cleanup and independently observed goal/decision preservation require acceptance evidence; the invocation shape alone establishes none of them.

## codex

### Runtime bindings

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{permission_flags}` | `--approve-for-me` |
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
| Run host-permitted workspace commands directly; automatic review decides escalations, and a necessary refused or denied command routes to the Director. Explicit interactive approval uses the original native surface and runtime-event procedure; the workspace sandbox and managed policy remain authoritative. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-reconsider-then-route) |

Evidence: Codex CLI 0.162.1, checked 2026-10-22; [host continuation](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex) and [command reference](https://learn.chatgpt.com/docs/developer-commands).

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
| `{permission_flags}` | `--agent cafleet`; `--agent cafleet-ask` when `--permission-mode cafleet-ask` is selected |
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
| Members perform assignments using the selected provider/model, workspace reads/edits, permitted tools and broker communication. A host satisfying the [Director requirements](#role-requirements) without a verified native stream adapter uses explicit manual `poll`/finite `wait` and states that reception limit. | [Director wake](supervision.md#director-wake) |
| OpenCode 2.1.4 loads presets from `~/.config/opencode`, the default preset base, and from `OPENCODE_CONFIG_DIR`. `member create` forwards a set `OPENCODE_CONFIG_DIR` into the member's environment, so the member loads the preset setup installed and spawn checked. Same-name agent definitions can merge across sources; inspect the effective configuration when customizing a preset. | `{permission_flags}`; preset discovery; member spawn |
| The preset uses the last matching permission rule, so specific denies for Director-owned explicit pane-input commands, including `member press`, win over the broad `cafleet *` allow. Denied member commands receive no permission prompt, while permitted broker commands run normally. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-reconsider-then-route) |
| MCP tools' participation in preset permission evaluation and compound-shell permission matching are unverified on opencode 2.1.4. Keep loaded configurations free of MCP servers and submit one command per invocation. | `{permission_flags}`; [command isolation](../SKILL.md#one-shot-command-isolation) |
| Each pane runs `opencode mini --standalone --agent <mode>` with `--prompt`, optional model selection and a private server; `<mode>` is `cafleet` or the selected `cafleet-ask`, and spawn requires only that mode's installed preset. | `{permission_flags}`; member spawn |
| Mini receives pushed notifications and direct prompts as submitted user turns. Captures can expose only the status bar; inspect retained session records for omitted input/output and broker poll/ACK for message consumption. | [Push notifications](cli.md#push-notifications); [prompt dispatch](cli.md#pane-input-and-previews) |
| Leading-`!` input runs a native shell command in `mini`. Preserve Director authorization and member-tool permissions because the native path's effective boundary and any second permission check remain unestablished. | [Native dispatch](prompt-routing.md#director-side-dispatch) |
| The `cafleet` preset allows its declared workspace/broker tools and denies unmatched shell commands; route a required denied command. A permission popup under `cafleet` is an issue to capture and relay. | `{permission_flags}`; [member routing](prompt-routing.md#member-side-reconsider-then-route) |
| The `cafleet-ask` preset keeps the same allowlist and Director-only denies and asks for every other shell command. Its native permission form is a runtime decision: the member waits while the Director answers it through the [runtime procedure](member-runtime.md), then continues from the result OpenCode returns. | `{permission_flags}`; [runtime procedure](member-runtime.md#consume-observed-interactions) |
| On OpenCode 2.1.4 and later a deny submits an empty rejection reason and ends the member's turn with `Step interrupted`; after verifying that nothing ran, continue the member with a Director message, which its runner delivers into the idle composer. A wrapped command or a reason to give is answered manually on the native surface. | [Runtime procedure](member-runtime.md#consume-observed-interactions) step 7 |

## Template

Add a `## <name>` backend with every subsection below. Fill every value, keep runtime bindings as short noun phrases, and bind each caveat to its token and consuming instruction. Shared selection and supervision policy stays at its linked owner.

### Runtime bindings

Supply all five tokens: `{decision_surface}` (recorded user-reaction surface; members relay), `{permission_flags}` (omitted launch-policy flags and explicit-selection behavior), `{pane_title}` (title mechanism or no analog), `{skill_loader}` (supported loader or complete absolute-path reads), and `{effort_levels}` (accepted values/flag or unsupported).

### Role defaults

Set `{reviewer_model}` and `{other_model}` to catalog tokens or aliases. Set their matching effort placeholders to a supported level or `—` when the backend does not support effort.

### Model catalog

List exact spawn tokens, valid aliases, reviewed most-to-least capability ordering, classes, and standard provider USD/MTok input/output prices. Include relevant context windows and availability constraints, required prefixes and official pricing/availability/configuration sources. Follow the freshness contract and [Director policy](../roles/director.md#model-selection).

### Note → applies at

Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind a `{permission_flags}` row stating the posture's denial semantics — what a member runs unprompted, what the harness denies, and when the member routes — to the member command-execution, prompt-routing, Director, and supervision instructions that point at it.
