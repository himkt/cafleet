# Coding-agent backends

Every member pane runs a coding-agent binary selected from the backend registry. The backend is recorded
per member in `member_placements.coding_agent`; selection and inheritance via
`--coding-agent`, mixed-backend teams, and identity delivery are covered in
[Coding agents](../concepts/coding-agents.md). This page specifies each
backend's child argv, selected permission policy, model-flag format, and
version/config requirements.

## Role requirements {#role-requirements}

| Role | Required backend capability |
|---|---|
| Director | A goal command or equivalent native goal lifecycle controls, plus background execution while supervision continues. |
| Member | A compatible PTY lifecycle with an explicit detection-coverage declaration; launchability, detection and safe composer input are evaluated separately. |

Evaluate each role independently. A backend's native automatic review is its omitted member policy where the backend offers one; interactive and fixed allow/deny policies remain explicit choices under the runner contract. Preserve native guards, managed policy, explicit denies and sandbox boundaries. A backend with unknown detector/composer coverage permits manual terminal work and holds generated input.

Goal support is a host capability; preserve supplied activation, pause, completion and budget authority. A verified native receive adapter is evaluated separately: a host satisfying both Director prerequisites may use explicit manual `poll` or finite `wait` when its native adapter is unavailable. Background execution alone establishes no unsolicited wake or per-record reception guarantee.

## Spawn argv {#spawn-argv}

These are the child argv with omitted permission selection. The leading token names the backend binary; the launched `argv[0]` is the absolute path chosen by [native executable resolution](member-runtime.md#native-executable-resolution), the same path the version probe ran. The multiplexer starts the CAFleet PTY runner, which preserves the literal child argv, cwd and required environment. Explicit selection changes only the native permission argument through the [launch-policy contract](member-runtime.md#launch-policy-and-capability-registry).

| Backend | Spawn argv |
|---|---|
| `claude` | `claude --permission-mode auto --prompt-suggestions false --allowedTools <rule>... --name <member-name> <prompt>` |
| `codex` | `codex --approve-for-me --sandbox workspace-write <prompt>` |
| `opencode` | `opencode mini --standalone --agent cafleet --prompt <prompt>` |

Explicit choices change only the permission tokens:

| Backend | Explicit choice | Permission tokens |
|---|---|---|
| `claude` | Any other native mode, such as `default` or `dontAsk` | `--permission-mode <mode>` |
| `codex` | `on-request` or `never` | `--ask-for-approval <mode> --sandbox workspace-write` |
| `opencode` | `cafleet-ask` | `--agent cafleet-ask`; the `--agent` value always equals the resolved mode |

Shared contract:

- The omitted claude and codex policies are each backend's native automatic approval mode: an automatic reviewer decides each operation that no native rule settles, and a refusal returns to the member, which routes a necessary refused command to the Director. The runner reports observed automatic refusals and unavailable automatic review to the Director as [automatic review events](member-runtime.md#automatic-review-events). An explicit interactive policy may display native approval prompts.
- The explicit codex `on-request` and `never` choices inherit the user's Codex configuration, including an `approvals_reviewer` setting that can answer escalations without a native form.
- The native child receives the selected backend's config-location variable
  when the creator's environment sets it, so the member reads the
  configuration CAFleet validated and installed into — see
  [Member environment](cli-options.md#member-environment).
- Each backend honors the leading-`!` shell shortcut that
  [`cafleet member exec`](cli-options.md#member-exec) dispatches through.
- Every member can run its broker commands from the moment it is spawned:
  claude through [spawn-time allow rules](#spawn-time-allow-rules), codex
  through [the rules file](#cafleet-rules-file), and opencode through
  [the agent preset](#cafleet-agent-preset).
- `--model <m>` from `cafleet member create` is inserted immediately before
  the prompt. The value passes through verbatim — the binary rejects unknown
  models, so newly released models need no cafleet release. Omitted, no model
  tokens are emitted and the binary uses its configured default. Per-backend
  formats and create-time validation are in
  [Model selection](#model-selection).
- `--effort <level>` from `cafleet member create` forwards a reasoning-effort
  level, emitted immediately after the model tokens (before the prompt).
  Create-time validation uses the backend-specific accepted set before
  registration or multiplexer effects. Omission emits no effort tokens,
  leaving argv byte-identical to the no-effort form. Levels and exact errors are in
  [Reasoning effort](#reasoning-effort).
- A missing binary fails the spawn: exit 1 with
  `Error: binary <name> not found on PATH`.

Per-backend capabilities:

| Backend | OS-level sandbox | Sets the pane title | Shell-command posture | Preset / config prerequisite |
|---|---|---|---|---|
| `claude` | none | yes, via `--name <member-name>` | Runs the broker commands under its spawn-time allow rules and every command matching the operator's `permissions.allow`; auto mode's classifier reviews the rest, and a refused command routes to the Director | A permission profile in `~/.claude/settings.json` — a permission posture, not a spawn dependency ([Claude](#claude)) |
| `codex` | kernel-enforced ([Codex](#codex)) | no — locate the pane through `cafleet member list` (`pane_id` is ground truth) | Runs commands its workspace/host rules permit; automatic review decides escalations, and a refused required command routes to the Director | `~/.codex/rules/cafleet.rules` (`CODEX_HOME` relocates it), plus the `~/.codex/config.toml` settings and a trusted working directory — a permission posture, not a spawn dependency ([the rules file](#cafleet-rules-file)) |
| `opencode` | none | no — locate the pane through `cafleet member list` | `cafleet`: deny-by-default allowlist, everything outside it routes to the Director; `cafleet-ask`: the same allowlist, everything else opens a native permission form | The selected policy's preset, `~/.config/opencode/agents/<mode>.md` (`OPENCODE_CONFIG_DIR` relocates it) — a spawn precondition ([the agent presets](#cafleet-agent-preset)) |

## Director decision policy

The Director resolves its own effective approval mode from current host/session evidence, independently of member launch argv. Verified auto permits the Director to evaluate a current permission request and decide allow/deny; other or unknown modes require the user’s request-specific permission. Native and managed rules remain authoritative, including authorization to execute the response command.

The [permission workflow](../concepts/member-permission-events.md) owns consumption and risk review. Request-bound answer support is an independently tested exact-version profile capability; detection or composer support supplies no answer-key mapping. Unknown answer coverage preserves manual native input.

## Model selection {#model-selection}

| Backend | Accepted value format | Example values | Create-time validation |
|---|---|---|---|
| `claude` | Passes through verbatim | `haiku`, `sonnet`, `opus`, `fable` | none — the binary rejects unknown models |
| `codex` | Passes through verbatim | `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-luna` | none — the binary rejects unknown models |
| `opencode` | `<provider-id>/<model-id>`, split on the **first** `/` into two non-empty segments (model ids may contain further slashes) | `opencode/claude-opus-5-5`, `opencode/gpt-6.1-sol`, `opencode-go/glm-5.3`, `anthropic/claude-sonnet-4-6` | exit 2 with `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` |

## Reasoning effort {#reasoning-effort}

| Backend | Accepted levels | Forwarded as | Rejected with (exit 2) |
|---|---|---|---|
| `claude` | `low`, `medium`, `high`, `xhigh`, `max` | `--effort <level>`, immediately after the model tokens | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| `codex` | `minimal`, `low`, `medium`, `high`, `xhigh` | the single token `--config=model_reasoning_effort=<level>`, immediately after the model tokens | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| `opencode` | none — the backend exposes no reasoning-effort control | — | `Error: opencode does not support reasoning effort.` |

## Claude {#claude}

A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Use one native Monitor task for Director inbox records, retaining its handle across separate events and renewing after observed source-task exit. The installed CAFleet skill's Claude native instructions own discovery, invocation, renewal and evidence limits; the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains operation. Preserve supplied `/goal` state and pending user decisions; manual finite reception remains available on a host satisfying the [Director requirements](#role-requirements) when Monitor is unavailable or denied.

Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

CAFleet input cancels no Claude tool approval or `AskUserQuestion` in the root Director's pane, which receives no automatic pane input, or in a runner-managed member's pane, where previews wait for an empty composer and send no Escape. A legacy unwrapped member runs `dontAsk`, so its open prompt is an `AskUserQuestion`; its Escape-first previews, `member prompt` and `member exec` input can cancel it, and Claude Code then reports a user rejection before the input arrives. The member treats a rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause as delivery-caused and re-asks the question or re-issues the call. A runner-managed member's preview ends with the resume clause only and arrives after the prompt closed, so a rejection before it stands. A rejection followed by Director prompt or native-command input follows that input, and the Director checks the pane for an open prompt before such dispatch. Recreating a legacy unwrapped member makes it runner-managed and removes the Escape lead.

`--permission-mode auto` is the omitted Claude member policy. A member runs
its [spawn-time allow rules](#spawn-time-allow-rules) and every tool call
matching `permissions.allow` without review; Claude Code's auto mode
classifier reviews each other call, and a call it refuses returns to the
member as a refusal, which the member routes to the Director like any denial.
Native rules, hooks and managed policy remain authoritative: a `deny` or `ask`
rule still applies, and auto mode must be available for the account and model.
Where it is unavailable, the member can show native permission forms, and the
runner reports `automatic_review_unavailable` when its profile recognizes the
notice. The recommended profiles are in
[Claude Code configuration](../quickstart.md#claude-code).

Explicit native modes retain their own permission behavior. Under explicit
`dontAsk`, a member runs its spawn-time allow rules, Claude Code's built-in
read-only commands, file reads inside its working directory, and every tool
call matching `permissions.allow` without a prompt. Every other call — a shell
command or a file edit, including one matching `permissions.ask` — is denied
without a prompt. A write to one of Claude Code's protected paths, such as a
project's `.git` or `.claude` directory, is denied whatever the allow list
holds.

Launch-policy evidence is pinned to Claude Code 2.1.302, Codex CLI 0.163.0 and OpenCode 2.1.4. The inspected Claude parser accepts canonical `default` and alias `manual`, although help renders the canonical mode as `manual`; explicit argv preserves the selected spelling. [Claude’s permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents the alias. Codex help confirms `on-request` and `never` for `--ask-for-approval` and the `--approve-for-me` flag, which routes approval requests through automatic review using the workspace-write sandbox; every Codex choice retains the explicit workspace sandbox. OpenCode 2.1.4 prints `opencode v2.1.4` for `--version` and loads the agent preset that `--agent` names; its `cafleet` and `cafleet-ask` choices select presets rather than native flags, so both are eligible for every OpenCode version. These observations establish launch-policy eligibility. Detection, empty-composer and answer coverage for exactly these versions comes from their [bundled profiles](member-runtime.md#bundled-profiles-and-native-screen-fixtures); another version of a backend with bundled profiles runs under its newest one with `profile_match=fallback` ([profile version fallback](member-runtime.md#profile-version-fallback)).

### Spawn-time allow rules {#spawn-time-allow-rules}

cafleet passes the broker commands a claude member needs as session allow
rules on its spawn command line, so the member does not depend on any
`Bash(cafleet ...)` rule in the user's `settings.json`:

| Member | Rules |
|---|---|
| Broker capability | `Bash(cafleet message *)`, including finite `message wait` and `message listen` |
| Work capability | Host-configured read/edit/command rules |

`--allowedTools` takes several values, so it sits before `--name`, which ends
its value list and keeps the prompt positional. The rules are fixed at spawn:
a later loss of allow rules in the user's settings does not affect a running
member.

The flags do not cover these cases:

- A `deny` or `ask` rule at any settings level, including managed policy,
  still blocks the command. [`cafleet doctor`](cli-options.md#member-permissions)
  reports it before spawn.
- The managed setting `allowManagedPermissionRulesOnly` makes Claude Code
  ignore `--allowedTools`. `cafleet doctor` reports it.
- A blocking `PreToolUse` hook stops a tool call before permission rules are
  evaluated. `cafleet doctor` cannot evaluate a hook.
- Inspect overdue member readiness/replies directly; settings can change after spawn.
- Work commands (`mise`, `git commit`, package managers) stay under the
  user's own rules; a denied one is routed through
  [`cafleet member exec`](cli-options.md#member-exec).

### What each profile entry is for {#claude-profile-entries}

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet *)` | Other modes | The Director | One pattern pre-approves every subcommand the Director runs. |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool. |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory. |
| `Edit`, `Write` | Both | Every member that writes files | Under `auto`, a matching entry pre-approves the member's edit without classifier review, and without one the classifier reviews each edit; under an explicit `dontAsk`, an edit no entry matches is denied. No entry covers a protected path. |
| Prefixes for explicit native pane input under `ask`, including `member press` | Both | The Director | The operator confirms each dispatch. A member's own attempt reaches the `ask` rule: a native permission form under `auto`, a denial under an explicit `dontAsk`. |

The auto mode profile carries no `Bash(cafleet ...)` allow entry: the
classifier reviews each Director command that no rule matches, auto mode
approves the Director's file edits inside the working directory, and members
receive their broker commands as spawn-time allow rules.

Commands that send explicit native pane input stay under `ask` in both
profiles, including ordered manual keys through `member press`. An ask rule
outranks every allow rule, so these entries hold beside `Bash(cafleet *)`.

`Edit` and `Write` apply to every session that reads the settings file, so
they also pre-approve file edits in your own sessions. To confine the
approval, replace both with a path rule such as `Edit(~/work/**)`, which
covers every file-editing tool. No entry reaches a protected path: a member's
edit under a project's `.git` or `.claude` directory is always denied, and the
Director session makes that edit instead. When `CLAUDE_CONFIG_DIR` relocates
the config directory, write the `Read` entry against that directory's
`skills/**`.

### What members can run {#claude-member-permissions}

A member whose shell command is denied or refused by auto mode asks its Director to run it. The
Director runs it with [`cafleet member exec`](cli-options.md#member-exec),
which is under `ask`, so each routed command costs the operator one
confirmation. Add an allow entry for every project command members run as
part of their work — a test or lint task, for example, as `Bash(npm test *)` —
so those commands run without routing. Routing carries shell commands only: a
denied or refused file edit needs a matching `Edit` entry, or, on a protected
path, the Director to make the edit.

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a refused or denied shell command | Auto mode's classifier refused a command outside the spawn-time allow rules and the allow list, or a `deny` rule matched it | Add an allow entry for the command. Until then, each Director dispatch of it asks the operator to confirm. |
| A member reports a refused or denied file edit | Auto mode's classifier refused an edit no `Edit` entry matches, or the path is protected | Add `Edit` and `Write`, or widen the `Edit` path rule to cover the member's working directory. For a protected path, the Director makes the edit. |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Add a pattern for that subcommand per [`permissions.allow` coverage](cli-options.md#permissionsallow-coverage) |
| `defaultMode: "auto"` has no effect | It is set in a project or local settings file, or auto mode is unavailable for the organization or model | Move it to `~/.claude/settings.json`, removing it from the project file. When auto mode is unavailable, use the other-modes profile. |

Verify native Monitor authorization separately from Bash broker rules. Monitor events remain broker events during an open user decision. Actual expiry, cancellation, child cleanup and independently observed continuation preservation require acceptance evidence.

## Codex {#codex}

The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Retain one native execution session handle for Director inbox reception and explicitly read it while delegated work remains, independently of whether a host goal exists. The installed CAFleet skill's Codex native instructions own tool calls, bounded reads, leases and recovery; the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains operation. Preserve supplied goal activation, pause, completion and budget authority. A background process alone supplies no unsolicited per-line wake after a model turn ends; fully idle Codex reconciles its inbox on the next user/native activation.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Treat native input as a Director-authorized action rather than assume a second member-tool permission check.

`--sandbox workspace-write` confines writes to the workspace under a
kernel-enforced sandbox — codex is the only backend with one.
The omitted `--approve-for-me` routes approval requests through Codex's
automatic review; an explicit `--ask-for-approval never` disables interactive
approval prompts and `on-request` shows them (upstream write-up:
<https://developers.openai.com/codex/agent-approvals-security>).

Three `~/.codex/config.toml` prerequisites must be in place before the first
codex spawn, covered in
[Codex configuration](../quickstart.md#codex) and
[Trust the working directory](../quickstart.md#trust-the-working-directory):

| Setting | Required value | Why |
|---|---|---|
| `network_access` | `true` | The multiplexer socket counts as network access |
| `writable_roots` | Includes the cafleet DB directory | — |
| `trust_level` | `"trusted"` | The working directory must be trusted before spawning |

`trust_level` is keyed by absolute workspace path:

```toml
[projects."/abs/path/to/workspace"]
trust_level = "trusted"
```

### The `cafleet` rules file {#cafleet-rules-file}

`~/.codex/rules/cafleet.rules` grants the auto-approval posture for `cafleet`
commands (`CODEX_HOME` relocates the `~/.codex` base — see
[Config-dir resolution](cli-options.md#config-dir-resolution)). It ships as
an embedded static asset (`presets/codex/cafleet.rules`) in the released
binary and is installed offline by `cafleet setup`:

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "prompt"],
    decision = "prompt",
    justification = "cafleet member prompt keystrokes arbitrary text into a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs an arbitrary shell command in a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "press"],
    decision = "prompt",
    justification = "cafleet member press sends explicit native keys to a member pane",
)

prefix_rule(
    pattern = ["cafleet", "permission", "respond"],
    decision = "prompt",
    justification = "cafleet permission respond writes a native permission answer into a member pane",
)

prefix_rule(
    pattern = ["cafleet", "input", "reconcile"],
    decision = "prompt",
    justification = "cafleet input reconcile records a Director-owned input disposition",
)

prefix_rule(
    pattern = ["cafleet", "member", "recover"],
    decision = "prompt",
    justification = "cafleet member recover reconciles a member run under exclusive ownership",
)
```

Codex applies the strictest decision when more than one rule matches
(`forbidden` > `prompt` > `allow`): the Director-owned explicit pane-input,
native-answer, input-reconciliation and run-recovery commands match the broad
allow and their specific prompt rules, so operator approval remains required. Commands without a more specific rule match the broad `["cafleet"]`
allow for every fleet,
since every id rides past the matched prefix as a positional or trailing
argument.

The file is **owned by `cafleet setup`**: it is overwritten on every install,
so operator customizations belong in a separate rules file under
`~/.codex/rules/` — Codex loads every `*.rules` file in that directory at
startup and applies the strictest decision across all of them. The rules file
is a permission posture, not a spawn dependency: `cafleet member create
--coding-agent codex` requires only the `codex` binary on PATH.

#### Minimum rule set {#minimum-rule-set}

A rules directory maintained without the installed file — for example one
managed from a dotfiles repository — needs at least these rules:

```text
prefix_rule(
    pattern = ["cafleet", "message"],
    decision = "allow",
    match = ["cafleet message poll 5", "cafleet message send --from-member-id 1 --to-member-id 2 ready"],
    not_match = ["cafleet member list 1", "cafleet doctor"],
)

prefix_rule(
    pattern = ["cafleet", "member", "prompt"],
    decision = "prompt",
    justification = "cafleet member prompt keystrokes arbitrary text into a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs an arbitrary shell command in a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "press"],
    decision = "prompt",
    justification = "cafleet member press sends explicit native keys to a member pane",
)
```

The `allow` rules cover the broker commands members run themselves — the same
commands a claude member receives as
[spawn-time allow rules](#spawn-time-allow-rules); the `prompt` rules keep
explicit native pane-input commands under operator approval.

The set is sufficient for members and is not the recommendation for a
Director session. Codex has no classifier to review what these rules leave
unmatched, so every other `cafleet` command a Director runs follows that
session's own approval policy and sandbox instead of running pre-approved
outside it. Keep the installed file for a Director session; it is a superset
of these rules, so they add nothing when it is also present.

## Opencode {#opencode}

OpenCode members perform supplied assignments with their selected provider/model, workspace reads/edits, permitted tools and broker communication. The Director coordinates next steps and routed native shell commands. Leading-`!` input runs a native shell command in `mini`. The native path's effective permission boundary and any second member-tool permission check remain unestablished; preserve the preset's member-tool permissions and the Director's command authorization.

The pane runs `opencode mini --standalone`, the long-lived minimal interactive
interface with a private server per pane. The prompt travels through `--prompt`
because `mini` takes no positional message.

### The `cafleet` and `cafleet-ask` agent presets {#cafleet-agent-preset}

Each OpenCode launch policy names an agent preset of the same name:

| Policy | Spawn argument | Preset file | Embedded asset |
|---|---|---|---|
| `cafleet` (default) | `--agent cafleet` | `~/.config/opencode/agents/cafleet.md` | `presets/opencode/cafleet.md` |
| `cafleet-ask` | `--agent cafleet-ask` | `~/.config/opencode/agents/cafleet-ask.md` | `presets/opencode/cafleet-ask.md` |

`~/.config/opencode` is the directory OpenCode 2.1.4 loads agent definitions
from by default. `OPENCODE_CONFIG_DIR` relocates that base — see
[Config-dir resolution](cli-options.md#config-dir-resolution); the spawn
precondition checks the same resolved path `setup` installs to, and
`member create` forwards a set `OPENCODE_CONFIG_DIR` into the
[member environment](cli-options.md#member-environment), so the member's
OpenCode loads the preset CAFleet checked. Both presets
are embedded in the released binary. `cafleet setup` installs every preset
offline, replacing any existing copy; rerun setup after upgrading CAFleet to
refresh them. The selected policy's preset is a spawn precondition: `cafleet
member create --coding-agent opencode` reads `<preset base>/agents/<mode>.md`
for the resolved mode and fails with `cannot read opencode agent preset at
<preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` when that
file is missing or unreadable. The unselected preset is not consulted.

Preset discovery includes the relocated directory and default home configuration.
Agent definitions with the same name can merge across those sources; inspect the
effective configuration when customizing a preset.

Each preset's `shell` ruleset starts with a `"*"` base, then an explicit
allowlist translated from the operator's Claude Code `permissions.allow` set
(`cafleet *`, its declared Git commands, file-inspection utilities, and the
project's cargo-backed mise tasks). opencode selects the **last** matching
rule, so this order is the safety floor. The two presets differ only in their
description, their shell base and their permission paragraph; every allowlist
entry, every Director-only deny and every non-shell permission is identical:

| Preset | Shell base | An unmatched shell command |
|---|---|---|
| `cafleet` | `"*": "deny"` | Is denied without a form; every check resolves to `allow` or `deny`, never `ask` |
| `cafleet-ask` | `"*": "ask"` | Opens a native permission form |

Specific deny entries for the Director-owned commands — explicit pane input (`cafleet member prompt *`,
`cafleet member exec *`, `cafleet member press *`), request-bound native answers
(`cafleet permission respond *`), input reconciliation
(`cafleet input reconcile *`) and run recovery (`cafleet member recover *`) —
follow the `cafleet *` allow, so they win: the preset configures members, and
dispatching input into another pane, answering a native request, settling an
input receipt or reconciling a run is the Director's act. A member's denied command receives no permission prompt, while permitted broker commands run normally. Tool permission and assignment authority are separate: the assigned role and Director instructions determine which allowed operations a member may perform.

Non-shell permissions (`read`, `edit`, `external_directory`, `webfetch`,
`websearch`, `repo_clone`, `question`, `plan_enter`, `plan_exit`) never ask
under either preset, so only a shell command can open a form and the `question`
tool raises no competing question form.

Under `cafleet-ask`, a shell permission form is a runtime decision. The member
runtime detects it, notifies the Director with an `interaction_opened` event,
and the Director answers it through `cafleet permission respond` under the
[permission workflow](../concepts/member-permission-events.md); the member
continues from the result OpenCode returns. The [OpenCode 2.1.4 profile](member-runtime.md#bundled-profiles-and-native-screen-fixtures) answers allow with Enter on `Allow once`. It answers deny by moving the focus to `Reject`, selecting it and submitting the rejection-reason dialog with an empty reason. After a deny OpenCode shows `Step interrupted` and does not continue on its own. CAFleet holds the Director's message because OpenCode prompt submission is `unknown`, so the member continues from input on its native surface. A command that wraps in the form, a reason the user wants to give, and every OpenCode version other than 2.1.4 are answered manually on the native surface. Under `cafleet`, a permission
popup in an opencode pane is a regression escape: capture the pane, escalate,
and extend the allowlist by operator decision, leaving the popup for that
decision.

### Safety-floor caveats {#safety-floor-caveats}

Both postures are allowlists with no OS-level sandbox. Standalone
un-enumerated commands fall to the `"*"` base: denied under `cafleet`, a
native permission form under `cafleet-ask`. Apply these
limits when choosing tools and authorizing commands:

| Surface | Example | Usage limit |
|---|---|---|
| MCP-contributed tools | — | Their participation in preset permission evaluation is unverified on opencode 2.1.4. Keep loaded configurations free of MCP servers. |
| Compound shell commands | <code>git log --stat; curl … &#124; sh</code> | Permission matching for compound lines is unverified on opencode 2.1.4. Submit one command per invocation and authorize each actual command. |
| Interpreter or hook execution via allowed tooling | `mise //cafleet:test *` executes workspace-writable test code; `git commit *` runs `.git/hooks` | Authorize the code and hooks that an allowed command can execute. |

cafleet ships no MCP stanzas. Operators MUST keep every loaded opencode
configuration free of MCP servers.

For kernel-enforced isolation, use the `codex` backend.

Spawn, preset loading, broker input and native shell input are validated against
opencode 2.1.4; opencode 2 is the minimum supported version.
